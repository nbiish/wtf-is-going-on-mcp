//! Operator paste-bins: durable three-bin slots (BIN 1..3) holding copy-paste
//! content handed from a human to agents, or between agents and machines on
//! the federated server.
//!
//! Scoped Multi-Context 3-Bins Architecture:
//! Every context individually owns its own trio of 3 bins (BIN 1..3):
//!   - `scope: "user"`: global courier clipboard for human operator reference.
//!   - `scope: "chat:<session_id>"`: dedicated 3 bins per agent chat / session channel.
//!   - `scope: "machine:<device_name>"`: dedicated 3 bins per federated machine workspace.
//!   - custom scopes supported seamlessly.
//!
//! Bins are plain state, not events: the canonical copy lives in
//! `$WTF_HOME/bins.json`, written atomically on every update, so hub
//! restarts lose nothing. Pasted content is treated as confidential: the
//! file is `0600` like every other store under `$WTF_HOME`. Oversized
//! pastes are rejected (never truncated) so instructions cannot be cut
//! short silently.

use crate::config;
use crate::json::Value;
use crate::util::{clamp, now_secs};
use std::path::PathBuf;
use std::sync::Mutex;

pub const DEFAULT_BIN_IDS: [u8; 3] = [1, 2, 3];
/// Hard cap per bin, in characters. Oversized writes fail closed.
pub const MAX_BIN_CHARS: usize = 65_536;

pub fn normalize_scope(s: &str) -> String {
    let t = s.trim();
    if t.is_empty() {
        "user".to_string()
    } else {
        clamp(t, 64)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Bin {
    pub scope: String,
    pub id: u8,
    pub content: String,
    /// Who last wrote this bin: a device name or "dashboard".
    pub updated_by: String,
    pub updated_at: u64,
}

impl Bin {
    pub fn empty(id: u8) -> Bin {
        Self::empty_scoped("user", id)
    }

    pub fn empty_scoped(scope: &str, id: u8) -> Bin {
        Bin {
            scope: normalize_scope(scope),
            id,
            content: String::new(),
            updated_by: String::new(),
            updated_at: 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.content.is_empty()
    }

    /// State/wire shape (content included so dashboards render verbatim).
    pub fn to_state_json(&self) -> Value {
        Value::obj(vec![
            ("scope", Value::from(self.scope.as_str())),
            ("id", Value::from(self.id as i64)),
            ("content", Value::from(self.content.as_str())),
            ("updated_by", Value::from(self.updated_by.as_str())),
            ("updated_at", Value::from(self.updated_at as i64)),
            ("size", Value::from(self.content.chars().count() as i64)),
        ])
    }

    fn to_file_json(&self) -> Value {
        Value::obj(vec![
            ("scope", Value::from(self.scope.as_str())),
            ("id", Value::from(self.id as i64)),
            ("content", Value::from(self.content.as_str())),
            ("updated_by", Value::from(self.updated_by.as_str())),
            ("updated_at", Value::from(self.updated_at as i64)),
        ])
    }

    fn from_json(v: &Value) -> Option<Bin> {
        let id = v.get("id")?.as_i64()?;
        if !Bins::valid_id(id) {
            return None;
        }
        let raw_scope = v.get("scope").and_then(|x| x.as_str()).unwrap_or("user");
        let scope = normalize_scope(raw_scope);
        Some(Bin {
            scope,
            id: id as u8,
            content: v
                .get("content")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            updated_by: v
                .get("updated_by")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            updated_at: v.get("updated_at").and_then(|x| x.as_i64()).unwrap_or(0) as u64,
        })
    }
}

pub struct Bins {
    inner: Mutex<Vec<Bin>>,
    path: PathBuf,
}

impl Bins {
    pub fn load() -> Bins {
        Self::load_at(&config::bins_path())
    }

    pub fn load_at(path: &PathBuf) -> Bins {
        let mut bins: Vec<Bin> = Vec::new();
        if let Ok(Some(v)) = config::load_json(path) {
            if let Some(arr) = v.get("bins").and_then(|x| x.as_arr()) {
                for item in arr {
                    if let Some(b) = Bin::from_json(item) {
                        bins.push(b);
                    }
                }
            }
        }
        for id in DEFAULT_BIN_IDS {
            if !bins.iter().any(|b| b.scope == "user" && b.id == id) {
                bins.push(Bin::empty_scoped("user", id));
            }
        }
        bins.sort_by(|a, b| a.scope.cmp(&b.scope).then_with(|| a.id.cmp(&b.id)));
        Bins {
            inner: Mutex::new(bins),
            path: path.clone(),
        }
    }

    pub fn valid_id(id: i64) -> bool {
        (1..=255).contains(&id)
    }

    pub fn trio_for_slot(slot: u16) -> [u8; 3] {
        let base = 3 + slot.saturating_mul(3);
        let b = base.min(253);
        [b as u8, (b + 1) as u8, (b + 2) as u8]
    }

    pub fn get(&self, id: u8) -> Option<Bin> {
        self.get_scoped("user", id)
    }

    pub fn get_scoped(&self, scope: &str, id: u8) -> Option<Bin> {
        let sc = normalize_scope(scope);
        let inner = self.inner.lock().unwrap();
        if let Some(b) = inner.iter().find(|b| b.scope == sc && b.id == id) {
            Some(b.clone())
        } else if DEFAULT_BIN_IDS.contains(&id) {
            Some(Bin::empty_scoped(&sc, id))
        } else {
            None
        }
    }

    pub fn list_scope(&self, scope: &str) -> Vec<Bin> {
        let sc = normalize_scope(scope);
        let inner = self.inner.lock().unwrap();
        let mut list = Vec::new();
        for id in DEFAULT_BIN_IDS {
            if let Some(b) = inner.iter().find(|b| b.scope == sc && b.id == id) {
                list.push(b.clone());
            } else {
                list.push(Bin::empty_scoped(&sc, id));
            }
        }
        for b in inner.iter() {
            if b.scope == sc && !DEFAULT_BIN_IDS.contains(&b.id) {
                list.push(b.clone());
            }
        }
        list.sort_by_key(|b| b.id);
        list
    }

    pub fn scopes(&self) -> Vec<String> {
        let inner = self.inner.lock().unwrap();
        let mut list = vec!["user".to_string()];
        for b in inner.iter() {
            if !list.contains(&b.scope) {
                list.push(b.scope.clone());
            }
        }
        list
    }

    pub fn all(&self) -> Vec<Bin> {
        self.inner.lock().unwrap().clone()
    }

    pub fn set(&self, id: u8, content: &str, by: &str) -> Result<Bin, String> {
        self.set_scoped("user", id, content, by)
    }

    pub fn set_scoped(&self, scope: &str, id: u8, content: &str, by: &str) -> Result<Bin, String> {
        if !Self::valid_id(id as i64) {
            return Err(format!(
                "bin must be one of: {}",
                DEFAULT_BIN_IDS
                    .iter()
                    .map(|i| i.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if content.chars().count() > MAX_BIN_CHARS {
            return Err(format!("bin content too large (max {MAX_BIN_CHARS} chars)"));
        }
        let sc = normalize_scope(scope);
        let bin = Bin {
            scope: sc.clone(),
            id,
            content: content.to_string(),
            updated_by: clamp(by, 64),
            updated_at: now_secs(),
        };
        let mut inner = self.inner.lock().unwrap();
        let mut found = false;
        let mut projected: Vec<Value> = Vec::new();
        for b in inner.iter() {
            if b.scope == sc && b.id == id {
                projected.push(bin.to_file_json());
                found = true;
            } else {
                projected.push(b.to_file_json());
            }
        }
        if !found {
            projected.push(bin.to_file_json());
        }
        config::save_json(
            &self.path,
            &Value::obj(vec![("bins", Value::Arr(projected))]),
            0o600,
        )?;
        match inner.iter_mut().find(|b| b.scope == sc && b.id == id) {
            Some(slot) => *slot = bin.clone(),
            None => {
                inner.push(bin.clone());
                inner.sort_by(|a, b| a.scope.cmp(&b.scope).then_with(|| a.id.cmp(&b.id)));
            }
        }
        Ok(bin)
    }

    pub fn to_state_json(&self) -> Value {
        Value::Arr(self.list_scope("user").iter().map(|b| b.to_state_json()).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "wtf-bins-{tag}-{}-{}",
            std::process::id(),
            crate::rand::hex(6)
        ));
        std::fs::create_dir_all(&d).unwrap();
        d.join("bins.json")
    }

    #[test]
    fn set_get_and_persistence() {
        let p = temp_path("persist");
        let b = Bins::load_at(&p);
        assert!(b.get(1).unwrap().is_empty());
        let saved = b.set(2, "work from this spec", "dashboard").unwrap();
        assert_eq!(saved.updated_by, "dashboard");
        assert!(saved.updated_at > 0);
        // survives a reload
        let b2 = Bins::load_at(&p);
        assert_eq!(b2.get(2).unwrap().content, "work from this spec");
        assert_eq!(b2.get(1).unwrap().content, "");
        // 0600 like every store under WTF_HOME
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&p).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }

    #[test]
    fn scoped_bins_isolation_and_persistence() {
        let p = temp_path("scoped");
        let b = Bins::load_at(&p);
        assert!(b.get_scoped("chat:lane-1", 1).unwrap().is_empty());
        b.set_scoped("chat:lane-1", 1, "chat secret key", "claude-code").unwrap();
        b.set_scoped("machine:pi", 1, "pi wifi credentials", "dashboard").unwrap();
        b.set(1, "global user spec", "dashboard").unwrap();

        assert_eq!(b.get_scoped("chat:lane-1", 1).unwrap().content, "chat secret key");
        assert_eq!(b.get_scoped("machine:pi", 1).unwrap().content, "pi wifi credentials");
        assert_eq!(b.get(1).unwrap().content, "global user spec");

        // List scope guarantees 3 bins
        let chat_bins = b.list_scope("chat:lane-1");
        assert_eq!(chat_bins.len(), 3);
        assert_eq!(chat_bins[0].content, "chat secret key");
        assert_eq!(chat_bins[1].content, "");

        let scopes = b.scopes();
        assert!(scopes.contains(&"user".to_string()));
        assert!(scopes.contains(&"chat:lane-1".to_string()));
        assert!(scopes.contains(&"machine:pi".to_string()));

        // Reload from disk
        let b2 = Bins::load_at(&p);
        assert_eq!(b2.get_scoped("chat:lane-1", 1).unwrap().content, "chat secret key");
        assert_eq!(b2.get_scoped("machine:pi", 1).unwrap().content, "pi wifi credentials");
        assert_eq!(b2.get(1).unwrap().content, "global user spec");
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }

    #[test]
    fn rejects_bad_id_and_oversize() {
        let p = temp_path("reject");
        let b = Bins::load_at(&p);
        assert!(b.set(0, "x", "d").is_err());
        assert!(b.set(255, "x", "d").is_ok());
        let big = "x".repeat(MAX_BIN_CHARS + 1);
        assert!(b.set(1, &big, "d").is_err());
        let at_cap = "x".repeat(MAX_BIN_CHARS);
        assert!(b.set(1, &at_cap, "d").is_ok());
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }

    #[test]
    fn corrupt_and_missing_files_tolerated() {
        let p = temp_path("tolerant");
        std::fs::write(&p, "not json at all {{{").unwrap();
        let b = Bins::load_at(&p);
        assert_eq!(b.all().len(), 3);
        assert!(b.all().iter().all(|x| x.is_empty()));
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }

    #[test]
    fn set_rejects_before_mutating_on_disk_error() {
        let p = temp_path("failclosed");
        std::fs::remove_file(&p).ok();
        std::fs::create_dir_all(&p).unwrap();
        let b = Bins::load_at(&p);
        assert!(b.set(1, "hello", "d").is_err());
        assert!(b.get(1).unwrap().is_empty());
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
        std::fs::remove_dir(&p).ok();
    }
}
