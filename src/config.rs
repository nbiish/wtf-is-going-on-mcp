//! Filesystem layout and atomic JSON persistence for `wtf`.
//!
//! State lives under `$WTF_HOME` if set, else `$HOME/.config/wtf-mcp`
//! (`%USERPROFILE%\.config\wtf-mcp` on Windows). Writes are atomic (`.tmp` + rename)
//! with 0700 dir / 0600 file permissions on Unix.

use crate::json::{self, Value};
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

pub fn home() -> PathBuf {
    if let Ok(h) = std::env::var("WTF_HOME") {
        if !h.trim().is_empty() {
            return PathBuf::from(h);
        }
    }
    let base = std::env::var("HOME")
        .ok()
        .filter(|h| !h.trim().is_empty())
        .or_else(|| {
            std::env::var("USERPROFILE")
                .ok()
                .filter(|h| !h.trim().is_empty())
        })
        .unwrap_or_else(|| ".".to_string());
    Path::new(&base).join(".config").join("wtf-mcp")
}

pub fn fleet_path() -> PathBuf {
    home().join("fleet.json")
}

/// Load a JSON document; `Ok(None)` if the file does not exist.
pub fn load_json(path: &Path) -> Result<Option<Value>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => json::parse(&text)
            .map(Some)
            .map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Atomically write a JSON document. `mode` is the Unix file mode (e.g. `0o600`).
pub fn save_json(path: &Path, val: &Value, mode: u32) -> Result<(), String> {
    let dir = path
        .parent()
        .ok_or_else(|| format!("bad path {}", path.display()))?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    #[cfg(unix)]
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    let tmp = path.with_extension("json.tmp");
    {
        let mut f = std::fs::File::create(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
        f.write_all(val.to_json().as_bytes())
            .and_then(|_| f.sync_all())
            .map_err(|e| format!("{}: {e}", tmp.display()))?;
    }
    #[cfg(unix)]
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode))
        .map_err(|e| format!("{}: {e}", tmp.display()))?;
    let _ = mode;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Short identifier validation: 1..=64 ASCII alphanumeric / `.` / `_` / `-`,
/// and must not start with `-` (prevents option injection into CLI tools).
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('-')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(tag: &str) -> PathBuf {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = std::env::temp_dir().join(format!("wtf-cfg-{tag}-{n}"));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn valid_name_rules() {
        assert!(valid_name("mac"));
        assert!(valid_name("windows-128gb"));
        assert!(valid_name("user_1.local"));
        assert!(!valid_name(""));
        assert!(!valid_name("-oProxyCommand=bad"));
        assert!(!valid_name("bad name"));
        assert!(!valid_name("a/b"));
        assert!(!valid_name(&"a".repeat(65)));
    }

    #[test]
    fn home_respects_wtf_home() {
        let _guard = crate::util::test_env_lock();
        let prev = std::env::var_os("WTF_HOME");
        std::env::set_var("WTF_HOME", "/tmp/custom-wtf-home");
        assert_eq!(home(), PathBuf::from("/tmp/custom-wtf-home"));
        assert_eq!(fleet_path(), PathBuf::from("/tmp/custom-wtf-home/fleet.json"));
        match prev {
            Some(v) => std::env::set_var("WTF_HOME", v),
            None => std::env::remove_var("WTF_HOME"),
        }
    }

    #[test]
    fn json_save_and_load_roundtrip() {
        let d = temp_dir("roundtrip");
        let p = d.join("fleet.json");
        assert!(load_json(&p).unwrap().is_none());
        let doc = Value::obj(vec![("ok", Value::from(true))]);
        save_json(&p, &doc, 0o600).unwrap();
        let loaded = load_json(&p).unwrap().unwrap();
        assert_eq!(loaded.get("ok").and_then(|v| v.as_bool()), Some(true));
        #[cfg(unix)]
        {
            let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let _ = std::fs::remove_dir_all(&d);
    }
}
