//! Keyless fleet pickup store: goals, SSH aliases, and PATH-based tool probes.
//!
//! Stores only non-secret pickup data in `$WTF_HOME/fleet.json` (mode 0600).
//! Private keys, IdentityFile paths, tokens, and cookies are rejected by
//! `valid_host` / `valid_name` and verified absent via `forbidden_keys`.

use crate::config::{self, valid_name};
use crate::json::Value;
use crate::rand;
use crate::util::{clamp, now_secs};
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;

pub const MAX_GOALS: usize = 256;
pub const MAX_ALIASES: usize = 64;
pub const MAX_TITLE: usize = 160;
pub const MAX_DETAIL: usize = 2_000;
pub const MAX_REPO: usize = 128;
pub const MAX_HOST: usize = 253;

/// Reported tool name, and the executables that count as "present".
const SKILL_BINS: &[(&str, &[&str])] = &[
    ("herdr", &["herdr"]),
    ("orca", &["orca", "orca-ide"]),
    ("browsermcp", &["browsermcp"]),
    ("agent-desktop", &["agent-desktop"]),
    ("browser-skill", &["bsk"]),
];

/// Object keys that must never appear in a pickup payload.
const FORBIDDEN_KEYS: &[&str] = &[
    "key",
    "secret",
    "token",
    "password",
    "psk",
    "cookie",
    "identity",
    "identityfile",
    "capability",
    "device_key",
    "dashboard_key",
];

#[derive(Clone, Debug, PartialEq)]
pub struct Goal {
    pub id: String,
    pub title: String,
    pub detail: String,
    pub repo: String,
    pub target: String,
    pub status: String,
    pub created_at: u64,
    pub updated_at: u64,
}

impl Goal {
    pub fn to_json(&self) -> Value {
        Value::obj(vec![
            ("id", Value::from(self.id.as_str())),
            ("title", Value::from(self.title.as_str())),
            ("detail", Value::from(self.detail.as_str())),
            ("repo", Value::from(self.repo.as_str())),
            ("target", Value::from(self.target.as_str())),
            ("status", Value::from(self.status.as_str())),
            ("created_at", Value::from(self.created_at as i64)),
            ("updated_at", Value::from(self.updated_at as i64)),
        ])
    }

    fn from_json(v: &Value) -> Option<Self> {
        let status = v.get("status").and_then(|x| x.as_str()).unwrap_or("open");
        if status != "open" && status != "done" {
            return None;
        }
        Some(Goal {
            id: v.get("id").and_then(|x| x.as_str())?.to_string(),
            title: v.get("title").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            detail: v.get("detail").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            repo: v.get("repo").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            target: v.get("target").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            status: status.to_string(),
            created_at: v.get("created_at").and_then(|x| x.as_i64()).unwrap_or(0) as u64,
            updated_at: v.get("updated_at").and_then(|x| x.as_i64()).unwrap_or(0) as u64,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SshAlias {
    pub alias: String,
    pub user: String,
    pub host: String,
    pub port: u16,
}

impl SshAlias {
    pub fn to_json(&self) -> Value {
        Value::obj(vec![
            ("alias", Value::from(self.alias.as_str())),
            ("user", Value::from(self.user.as_str())),
            ("host", Value::from(self.host.as_str())),
            ("port", Value::from(self.port as i64)),
        ])
    }

    fn from_json(v: &Value) -> Option<Self> {
        let port = v.get("port").and_then(|x| x.as_i64()).unwrap_or(22);
        if !(1..=65535).contains(&port) {
            return None;
        }
        Some(SshAlias {
            alias: v.get("alias").and_then(|x| x.as_str())?.to_string(),
            user: v.get("user").and_then(|x| x.as_str())?.to_string(),
            host: v.get("host").and_then(|x| x.as_str())?.to_string(),
            port: port as u16,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SkillSight {
    pub device: String,
    pub name: String,
    pub present: bool,
}

impl SkillSight {
    pub fn to_json(&self) -> Value {
        Value::obj(vec![
            ("device", Value::from(self.device.as_str())),
            ("name", Value::from(self.name.as_str())),
            ("present", Value::from(self.present)),
        ])
    }
}

pub struct FleetStore {
    goals: Mutex<Vec<Goal>>,
    aliases: Mutex<Vec<SshAlias>>,
    path: PathBuf,
}

impl FleetStore {
    pub fn load() -> FleetStore {
        Self::load_at(&config::fleet_path())
    }

    pub fn load_at(path: &PathBuf) -> FleetStore {
        let mut goals = Vec::new();
        let mut aliases = Vec::new();
        if let Ok(Some(v)) = config::load_json(path) {
            if let Some(arr) = v.get("goals").and_then(|x| x.as_arr()) {
                for item in arr {
                    if let Some(g) = Goal::from_json(item) {
                        goals.push(g);
                    }
                }
            }
            if let Some(arr) = v.get("aliases").and_then(|x| x.as_arr()) {
                for item in arr {
                    if let Some(a) = SshAlias::from_json(item) {
                        aliases.push(a);
                    }
                }
            }
        }
        goals.truncate(MAX_GOALS);
        aliases.truncate(MAX_ALIASES);
        FleetStore {
            goals: Mutex::new(goals),
            aliases: Mutex::new(aliases),
            path: path.clone(),
        }
    }

    fn persist(&self) -> Result<(), String> {
        let goals = self.goals.lock().unwrap();
        let aliases = self.aliases.lock().unwrap();
        let g: Vec<Value> = goals.iter().map(Goal::to_json).collect();
        let a: Vec<Value> = aliases.iter().map(SshAlias::to_json).collect();
        drop(aliases);
        drop(goals);
        config::save_json(
            &self.path,
            &Value::obj(vec![("goals", Value::Arr(g)), ("aliases", Value::Arr(a))]),
            0o600,
        )
    }

    pub fn goals(&self) -> Vec<Goal> {
        let mut v = self.goals.lock().unwrap().clone();
        v.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then(b.id.cmp(&a.id)));
        v
    }

    pub fn aliases(&self) -> Vec<SshAlias> {
        let mut v = self.aliases.lock().unwrap().clone();
        v.sort_by(|a, b| a.alias.cmp(&b.alias));
        v
    }

    pub fn add_goal(
        &self,
        title: &str,
        detail: &str,
        repo: &str,
        target: &str,
    ) -> Result<Goal, String> {
        let title = clamp(title.trim(), MAX_TITLE);
        if title.is_empty() {
            return Err("title is required".into());
        }
        let detail = clamp(detail.trim(), MAX_DETAIL);
        let repo = clamp(repo.trim(), MAX_REPO);
        let target = target.trim();
        if !target.is_empty() && !valid_name(target) {
            return Err("target must be a machine name (letters, digits, . _ -)".into());
        }
        let now = now_secs();
        let mut goals = self.goals.lock().unwrap();
        if goals.len() >= MAX_GOALS {
            return Err("goal list is full".into());
        }
        let mut id = rand::hex(8);
        while goals.iter().any(|g| g.id == id) {
            id = rand::hex(8);
        }
        let goal = Goal {
            id,
            title,
            detail,
            repo,
            target: target.to_string(),
            status: "open".into(),
            created_at: now,
            updated_at: now,
        };
        goals.push(goal.clone());
        drop(goals);
        self.persist()?;
        Ok(goal)
    }

    pub fn done_goal(&self, id: &str) -> Result<Goal, String> {
        let id = id.trim();
        if id.is_empty() || id.len() > 32 {
            return Err("missing goal id".into());
        }
        let mut goals = self.goals.lock().unwrap();
        let g = goals
            .iter_mut()
            .find(|g| g.id == id)
            .ok_or_else(|| format!("no goal {id}"))?;
        g.status = "done".into();
        g.updated_at = now_secs();
        let out = g.clone();
        drop(goals);
        self.persist()?;
        Ok(out)
    }

    pub fn remove_goal(&self, id: &str) -> Result<bool, String> {
        let id = id.trim();
        if id.is_empty() || id.len() > 32 {
            return Err("missing goal id".into());
        }
        let mut goals = self.goals.lock().unwrap();
        let before = goals.len();
        goals.retain(|g| g.id != id);
        let removed = goals.len() != before;
        drop(goals);
        if removed {
            self.persist()?;
        }
        Ok(removed)
    }

    pub fn upsert_alias(
        &self,
        alias: &str,
        user: &str,
        host: &str,
        port: u16,
    ) -> Result<SshAlias, String> {
        let alias = alias.trim();
        let user = user.trim();
        let host = host.trim();
        if !valid_name(alias) {
            return Err("alias must be a short name (letters, digits, . _ -)".into());
        }
        if !valid_name(user) {
            return Err("user must be a short name (letters, digits, . _ -)".into());
        }
        if !valid_host(host) {
            return Err("host must be a hostname or IPv4 address, with no path".into());
        }
        if port == 0 {
            return Err("port must be 1-65535".into());
        }
        let row = SshAlias {
            alias: alias.to_string(),
            user: user.to_string(),
            host: host.to_string(),
            port,
        };
        let mut aliases = self.aliases.lock().unwrap();
        if let Some(existing) = aliases.iter_mut().find(|a| a.alias == row.alias) {
            *existing = row.clone();
        } else {
            if aliases.len() >= MAX_ALIASES {
                return Err("alias table is full".into());
            }
            aliases.push(row.clone());
        }
        drop(aliases);
        self.persist()?;
        Ok(row)
    }

    pub fn remove_alias(&self, alias: &str) -> Result<bool, String> {
        let alias = alias.trim();
        if !valid_name(alias) {
            return Err("alias must be a short name (letters, digits, . _ -)".into());
        }
        let mut aliases = self.aliases.lock().unwrap();
        let before = aliases.len();
        aliases.retain(|a| a.alias != alias);
        let removed = aliases.len() != before;
        drop(aliases);
        if removed {
            self.persist()?;
        }
        Ok(removed)
    }
}

pub fn valid_host(host: &str) -> bool {
    if host.is_empty() || host.len() > MAX_HOST {
        return false;
    }
    if host.contains(['/', '\\', ' ', '@', ':', ';', '\n', '\r']) {
        return false;
    }
    if host.starts_with('-') || host.ends_with('-') || host.ends_with('.') {
        return false;
    }
    let ipv4 = host.split('.').collect::<Vec<_>>();
    if ipv4.len() == 4 && ipv4.iter().all(|o| o.parse::<u8>().is_ok() && !o.is_empty()) {
        return true;
    }
    host.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
    })
}

/// Names found on PATH. The argument list is the allowlist above; nothing
/// from user or file input is passed to the process.
pub fn probe_skills(device: &str) -> Vec<SkillSight> {
    let device = if device.is_empty() { "local" } else { device };
    SKILL_BINS
        .iter()
        .map(|(name, bins)| SkillSight {
            device: device.to_string(),
            name: (*name).to_string(),
            present: bins.iter().any(|b| bin_present(b)),
        })
        .collect()
}

fn bin_present(name: &str) -> bool {
    let cmd = if cfg!(windows) { "where" } else { "which" };
    Command::new(cmd)
        .arg(name)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn forbidden_keys(v: &Value) -> Vec<String> {
    let mut out = Vec::new();
    walk_keys(v, &mut out);
    out
}

fn walk_keys(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Obj(pairs) => {
            for (k, child) in pairs {
                let lk = k.to_ascii_lowercase();
                if FORBIDDEN_KEYS.iter().any(|f| lk == *f || lk.contains(*f)) {
                    out.push(k.clone());
                }
                walk_keys(child, out);
            }
        }
        Value::Arr(items) => {
            for child in items {
                walk_keys(child, out);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmp() -> PathBuf {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = std::env::temp_dir().join(format!("wtf-fleet-{n}"));
        std::fs::create_dir_all(&p).unwrap();
        p.join("fleet.json")
    }

    #[test]
    fn alias_and_goal_crud_roundtrip_and_keyless() {
        let path = tmp();
        let store = FleetStore::load_at(&path);
        let alias = store
            .upsert_alias("windows", "kenwa", "peopleofthedawn.local", 22)
            .unwrap();
        assert_eq!(alias.port, 22);
        assert!(store
            .upsert_alias("bad", "kenwa", "/home/kenwa/.ssh/id_ed25519", 22)
            .is_err());
        assert!(store.upsert_alias("bad", "kenwa", "host name", 22).is_err());
        assert!(store.upsert_alias("-bad", "kenwa", "host.local", 22).is_err());

        let goal = store
            .add_goal("finish the lean cli", "stateless", "wtf-is-going-on-mcp", "mac")
            .unwrap();
        let done = store.done_goal(&goal.id).unwrap();
        assert_eq!(done.status, "done");

        let reloaded = FleetStore::load_at(&path);
        assert_eq!(reloaded.aliases().len(), 1);
        assert_eq!(reloaded.goals()[0].status, "done");
        assert!(forbidden_keys(&reloaded.goals()[0].to_json()).is_empty());
        assert!(forbidden_keys(&reloaded.aliases()[0].to_json()).is_empty());

        assert!(reloaded.remove_goal(&goal.id).unwrap());
        assert!(!reloaded.remove_goal(&goal.id).unwrap());
        assert!(reloaded.remove_alias("windows").unwrap());
        assert!(!reloaded.remove_alias("windows").unwrap());
        let reloaded2 = FleetStore::load_at(&path);
        assert!(reloaded2.goals().is_empty());
        assert!(reloaded2.aliases().is_empty());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn host_rules() {
        assert!(valid_host("peopleofthedawn.local"));
        assert!(valid_host("192.168.1.248"));
        assert!(!valid_host(""));
        assert!(!valid_host("a/b"));
        assert!(!valid_host("~/.ssh/id_ed25519"));
        assert!(!valid_host("host:22"));
        assert!(!valid_host("-badhost"));
    }

    #[test]
    fn forbidden_key_detector_catches_secret_fields() {
        let bad = Value::obj(vec![("identityFile", Value::from("/tmp/key"))]);
        assert_eq!(forbidden_keys(&bad), vec!["identityFile".to_string()]);
    }
}
