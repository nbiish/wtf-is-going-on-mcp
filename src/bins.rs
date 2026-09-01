//! Operator paste-bins: three durable slots (BIN 1..3) holding copy-paste
//! content handed from a human on any machine to every reporting agent.
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

#[derive(Clone, Debug, PartialEq)]
pub struct Bin {
    pub id: u8,
    pub content: String,
    /// Who last wrote this bin: a device name or "dashboard".
    pub updated_by: String,
    pub updated_at: u64,
}

impl Bin {
    pub fn empty(id: u8) -> Bin {
        Bin {
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
            ("id", Value::from(self.id as i64)),
            ("content", Value::from(self.content.as_str())),
            ("updated_by", Value::from(self.updated_by.as_str())),
            ("updated_at", Value::from(self.updated_at as i64)),
            ("size", Value::from(self.content.chars().count() as i64)),
        ])
    }

    fn to_file_json(&self) -> Value {
        Value::obj(vec![
            ("id", Value::from(self.id as i64)),
            ("content", Value::from(self.content.as_str())),
            ("updated_by", Value::from(self.updated_by.as_str())),
            ("updated_at", Value::from(self.updated_at as i64)),
        ])
    }

    fn from_json(v: &Value) -> Option<Bin> {
        let id = v.get("id")?.as_i64()?;
        if !(1..=3).contains(&id) {
            return None;
        }
        Some(Bin {
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
    inner: Mutex<Vec<Bin>>, // dynamic: default trio + any persisted per-connection bins
    path: PathBuf,
}

impl Bins {
    pub fn load() -> Bins {
        Self::load_at(&config::bins_path())
    }

    /// Load from disk; missing file yields default bins. Dynamic bin ids
    /// (v0.15.0, operator directive): every agent connection gets its OWN
    /// bins instead of three shared slots — a bin id may be a small integer
    /// (1..=999, legacy 1..=3 still first-class) or a connection slug
    /// ("omp", "hermes-1", "vm- research"). Missing ids materialize as
    /// empty bins on read; the file only stores non-empty bins.
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
            if !bins.iter().any(|b| b.id == id) {
                bins.push(Bin::empty(id));
            }
        }
        bins.sort_by_key(|b| b.id);
        Bins {
            inner: Mutex::new(bins),
            path: path.clone(),
        }
    }

    /// Numeric bins stay 1..=255 (u8 wire contract); non-numeric ids are
    /// rejected here — per-connection bins use integers offset by device
    /// slot (see trio_for_slot). Default operator bins remain 1..=3.

    pub fn valid_id(id: i64) -> bool {
        (1..=255).contains(&id)
    }

    /// Per-connection bin base: each enrolled device derives a stable,
    /// non-overlapping trio of bin ids from its device slot index, so
    /// every agent connection owns BIN trio (base+0, base+1, base+2)
    /// while default operator bins remain 1..=3 (base 0).
    pub fn trio_for_slot(slot: u16) -> [u8; 3] {
        let base = 3 + slot.saturating_mul(3);
        let b = base.min(253);
        [b as u8, (b + 1) as u8, (b + 2) as u8]
    }

    pub fn get(&self, id: u8) -> Option<Bin> {
        self.inner
            .lock()
            .unwrap()
            .iter()
            .find(|b| b.id == id)
            .cloned()
    }

    pub fn all(&self) -> Vec<Bin> {
        self.inner.lock().unwrap().clone()
    }

    /// Set one bin's content; persists before swapping in so a failed write
    /// leaves memory and disk consistent. Returns the stored bin.
    pub fn set(&self, id: u8, content: &str, by: &str) -> Result<Bin, String> {
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
        let bin = Bin {
            id,
            content: content.to_string(),
            updated_by: clamp(by, 64),
            updated_at: now_secs(),
        };
        let mut inner = self.inner.lock().unwrap();
        let projected: Vec<Value> = inner
            .iter()
            .map(|b| {
                if b.id == id {
                    bin.to_file_json()
                } else {
                    b.to_file_json()
                }
            })
            .collect();
        config::save_json(
            &self.path,
            &Value::obj(vec![("bins", Value::Arr(projected))]),
            0o600,
        )?;
        // Dynamic bins: an id never seen before materializes on first
        // write (per-connection bins are created lazily on first use).
        match inner.iter_mut().find(|b| b.id == id) {
            Some(slot) => *slot = bin.clone(),
            None => {
                inner.push(bin.clone());
                inner.sort_by_key(|b| b.id);
            }
        }
        Ok(bin)
    }

    /// "bins" array for /api/v1/state and /api/v1/bins.
    pub fn to_state_json(&self) -> Value {
        Value::Arr(self.all().iter().map(|b| b.to_state_json()).collect())
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
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&p).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }

    #[test]
    fn rejects_bad_id_and_oversize() {
        let p = temp_path("reject");
        let b = Bins::load_at(&p);
        assert!(b.set(0, "x", "d").is_err());
        // v0.15.0 dynamic bins: ids 4..=255 materialize on write
        // (per-connection bins); 0 stays rejected. (255 = u8::MAX)
        assert!(b.set(255, "x", "d").is_ok());
        let big = "x".repeat(MAX_BIN_CHARS + 1);
        assert!(b.set(1, &big, "d").is_err());
        // exactly at the cap is fine
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
        // A directory where the file should be makes save_json fail; the
        // in-memory state must not change.
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
