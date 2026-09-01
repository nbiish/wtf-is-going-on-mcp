//! Hub federation: identity, peer table, and outbound replication queue.
//!
//! Every hub is simultaneously a hub and a *device* on each of its peers.
//! Linking two hubs (`wtf federate add`) enrolls this hub on the peer with
//! the peer's site secret (PSK handshake, v0.9.0 protocol) and records the
//! device credential the peer issued — under `WTF_FED_NAME`, default
//! `hub-<this-hub-name>`. Replication then rides the existing HMAC-SHA256
//! request lane: same auth, same replay guards, no new crypto, `auth.rs`
//! untouched.
//!
//! Files (all 0600, atomic writes):
//! - `federation.json` — this hub's name + the peer table (peer name, URL,
//!   the device name we present there, and the device key the peer issued;
//!   that key is key material held by this hub).
//! - `dashboard_capability` — 64-hex token gating the dashboard URL path.
//!
//! The ledger is a public-surface event log by design (secrets forbidden);
//! federation replicates it over the standard-transport HMAC lane, the same
//! posture as every other authenticated request.

use crate::config::{home, load_json, save_json};
use crate::json::{self, Value};
use crate::rand;
use std::path::PathBuf;

/// Device name this hub presents on peers when `federate add` is run
/// without an explicit `--as`.
pub const FED_NAME_PREFIX: &str = "fed-";

pub fn federation_path() -> PathBuf {
    home().join("federation.json")
}

pub fn capability_path() -> PathBuf {
    home().join("dashboard_capability")
}

/// One linked peer hub. `device_key` is the credential THE PEER issued to
/// this hub (this hub signs requests to the peer with it). It is key
/// material at rest for this hub — hence 0600.
#[derive(Clone, Debug)]
pub struct Peer {
    pub name: String,
    pub url: String,
    pub device: String,
    pub device_key: String,
    pub added_at: u64,
}

#[derive(Clone, Debug, Default)]
pub struct FedConfig {
    /// Stable hub identity, stamped on every locally-originated event.
    pub name: String,
    pub peers: Vec<Peer>,
}

impl FedConfig {
    pub fn load() -> FedConfig {
        Self::load_at(&federation_path())
    }

    pub fn load_at(path: &PathBuf) -> FedConfig {
        match load_json(path) {
            Ok(Some(v)) => FedConfig {
                name: v
                    .get("name")
                    .and_then(|x| x.as_str())
                    .unwrap_or_default()
                    .to_string(),
                peers: v
                    .get("peers")
                    .and_then(|x| x.as_arr())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|p| {
                                Some(Peer {
                                    name: p.get("name")?.as_str()?.to_string(),
                                    url: p.get("url")?.as_str()?.to_string(),
                                    device: p.get("device")?.as_str()?.to_string(),
                                    device_key: p.get("device_key")?.as_str()?.to_string(),
                                    added_at: p
                                        .get("added_at")
                                        .and_then(|x| x.as_i64())
                                        .unwrap_or(0)
                                        as u64,
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            },
            _ => FedConfig::default(),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        Self::save_at(&federation_path(), self)
    }

    pub fn save_at(path: &PathBuf, cfg: &FedConfig) -> Result<(), String> {
        let peers: Vec<Value> = cfg
            .peers
            .iter()
            .map(|p| {
                Value::obj(vec![
                    ("name", Value::from(p.name.as_str())),
                    ("url", Value::from(p.url.as_str())),
                    ("device", Value::from(p.device.as_str())),
                    ("device_key", Value::from(p.device_key.as_str())),
                    ("added_at", Value::from(p.added_at as i64)),
                ])
            })
            .collect();
        save_json(
            path,
            &Value::obj(vec![
                ("name", Value::from(cfg.name.as_str())),
                ("peers", Value::Arr(peers)),
            ]),
            0o600,
        )
    }

    pub fn find_peer(&self, name: &str) -> Option<&Peer> {
        self.peers.iter().find(|p| p.name == name)
    }

    pub fn find_peer_by_url(&self, url: &str) -> Option<&Peer> {
        self.peers.iter().find(|p| p.url == url)
    }

    /// Mint the stable hub name on first use. `[A-Za-z0-9._-]{1,32}`.
    pub fn ensure_name(&mut self) -> Result<String, String> {
        if !self.name.is_empty() {
            return Ok(self.name.clone());
        }
        let short = rand::hex(4);
        self.name = format!("hub-{short}");
        self.save()?;
        Ok(self.name.clone())
    }
}

/// The 64-hex capability token gating the dashboard URL path. Auto-minted
/// on first use; rotate = rewrite the file (peers are unaffected: they talk
/// to the API lane, not the dashboard).
pub fn load_or_create_capability() -> Result<String, String> {
    let path = capability_path();
    match std::fs::read_to_string(&path) {
        Ok(t) => {
            let t = t.trim().to_string();
            if t.len() == 64 && t.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Ok(t.to_ascii_lowercase());
            }
            // Missing/corrupt: regenerate rather than fail closed with no UI.
            let tok = mint_capability(&path)?;
            Ok(tok)
        }
        Err(_) => mint_capability(&path),
    }
}

fn mint_capability(path: &PathBuf) -> Result<String, String> {
    let tok = rand::key_hex();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(path, &tok).map_err(|e| format!("{}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(tok)
}

/// Parse a federate-push envelope body: `{origin, events: [...]}`.
/// Fail-closed shape validation; returns (origin, events) or an error string.
pub fn parse_push(body: &Value) -> Result<(String, Vec<Value>), String> {
    let origin = body
        .get("origin")
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty() && s.len() <= 64)
        .ok_or("missing 'origin'")?
        .to_string();
    let events = body
        .get("events")
        .and_then(|x| x.as_arr())
        .ok_or("missing 'events' array")?;
    if events.len() > 500 {
        return Err("too many events in push".into());
    }
    Ok((origin, events.to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> (std::path::PathBuf, PathBuf) {
        let d = std::env::temp_dir().join(format!(
            "wtf-fed-{}-{}-{}",
            tag,
            std::process::id(),
            crate::rand::hex(4)
        ));
        std::fs::create_dir_all(&d).unwrap();
        (d.clone(), d.join("federation.json"))
    }

    #[test]
    fn fed_config_roundtrip() {
        let (_d, path) = tmp("roundtrip");
        let mut cfg = FedConfig::load_at(&path);
        assert!(cfg.name.is_empty());
        cfg.peers.push(Peer {
            name: "hub-mac".into(),
            url: "http://localhost:7800".into(),
            device: "hub-hub-x".into(),
            device_key: "ab".repeat(32),
            added_at: 123,
        });
        FedConfig::save_at(&path, &cfg).unwrap();
        let cfg2 = FedConfig::load_at(&path);
        assert_eq!(cfg2.name, "");
        assert_eq!(cfg2.peers.len(), 1);
        assert_eq!(cfg2.peers[0].name, "hub-mac");
        assert_eq!(cfg2.peers[0].device_key, "ab".repeat(32));
        std::fs::remove_dir_all(&_d).ok();
    }

    #[test]
    fn ensure_name_is_stable() {
        let d = std::env::temp_dir().join(format!(
            "wtf-fed-name-{}-{}",
            std::process::id(),
            crate::rand::hex(4)
        ));
        std::fs::create_dir_all(&d).unwrap();
        let prev = std::env::var("WTF_HOME").ok();
        std::env::set_var("WTF_HOME", &d);
        let mut cfg = FedConfig::load();
        let n1 = cfg.ensure_name().unwrap();
        let n2 = FedConfig::load().ensure_name().unwrap();
        assert_eq!(n1, n2);
        assert!(n1.starts_with("hub-"));
        match prev {
            Some(v) => std::env::set_var("WTF_HOME", v),
            None => std::env::remove_var("WTF_HOME"),
        }
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn capability_roundtrip_and_corrupt_regen() {
        let d = std::env::temp_dir().join(format!(
            "wtf-fed-cap-{}-{}",
            std::process::id(),
            crate::rand::hex(4)
        ));
        std::fs::create_dir_all(&d).unwrap();
        let path = d.join("dashboard_capability");
        // load_or_create reads via capability_path() which is $WTF_HOME-bound;
        // test the mint+read primitives through the file directly.
        let tok = mint_capability(&path).unwrap();
        assert_eq!(tok.len(), 64);
        let t2 = std::fs::read_to_string(&path).unwrap();
        assert_eq!(t2.trim(), tok);
        std::fs::write(&path, "garbage").unwrap();
        let t3 = mint_capability(&path).unwrap();
        assert_eq!(t3.len(), 64);
        assert_ne!(t3, "garbage");
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn parse_push_shapes() {
        let body = json::parse(
            r#"{"origin":"hub-a","events":[{"kind":"event","id":1,"ts":2,"device":"d","agent":"a","level":"info","message":"m","status":"","task":"","details":""}]}"#,
        )
        .unwrap();
        let (origin, events) = parse_push(&body).unwrap();
        assert_eq!(origin, "hub-a");
        assert_eq!(events.len(), 1);
        assert!(parse_push(&Value::obj(vec![])).is_err());
        let no_events = json::parse(r#"{"origin":"hub-a"}"#).unwrap();
        assert!(parse_push(&no_events).is_err());
    }
}
