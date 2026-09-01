//! Filesystem layout, atomic persistence, hub config, key store, bridge config.
//!
//! All state lives under one root dir: `$WTF_HOME` if set, else
//! `$HOME/.config/wtf-mcp`. Files holding key material are written 0600 into
//! a 0700 directory. Writes are atomic (tmp file + rename).

use crate::json::{self, Value};
use crate::rand;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

pub const DEFAULT_PORT: u16 = 7800;

pub fn home() -> PathBuf {
    if let Ok(h) = std::env::var("WTF_HOME") {
        if !h.trim().is_empty() {
            return PathBuf::from(h);
        }
    }
    let hd = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    Path::new(&hd).join(".config").join("wtf-mcp")
}

pub fn config_path() -> PathBuf {
    home().join("config.json")
}

pub fn keys_path() -> PathBuf {
    home().join("keys.json")
}

pub fn bridge_path() -> PathBuf {
    home().join("bridge.json")
}

pub fn events_path() -> PathBuf {
    home().join("events.jsonl")
}

pub fn bins_path() -> PathBuf {
    home().join("bins.json")
}

pub fn sessions_path() -> PathBuf {
    home().join("sessions.json")
}

pub fn enroll_tokens_path() -> PathBuf {
    home().join("enroll_tokens.json")
}

/// Ensure the home dir exists with 0700.
pub fn ensure_home() -> std::io::Result<PathBuf> {
    let h = home();
    std::fs::create_dir_all(&h)?;
    std::fs::set_permissions(&h, std::fs::Permissions::from_mode(0o700))?;
    Ok(h)
}

/// Load a JSON document; Ok(None) if the file does not exist.
pub fn load_json(path: &Path) -> Result<Option<Value>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => json::parse(&text)
            .map(Some)
            .map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Atomically write a JSON document. `mode` is the unix file mode (0600 etc).
pub fn save_json(path: &Path, val: &Value, mode: u32) -> Result<(), String> {
    let dir = path
        .parent()
        .ok_or_else(|| format!("bad path {}", path.display()))?;
    std::fs::create_dir_all(dir)
        .and_then(|_| std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)))
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    let tmp = path.with_extension("json.tmp");
    {
        let mut f = std::fs::File::create(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
        f.write_all(val.to_json().as_bytes())
            .and_then(|_| f.sync_all())
            .map_err(|e| format!("{}: {e}", tmp.display()))?;
    }
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode))
        .map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Hub configuration persisted at config.json.
#[derive(Clone, Debug)]
pub struct HubConfig {
    pub bind_ip: String,
    pub port: u16,
    pub dashboard_key: String,
    pub created_at: u64,
    /// URL handed out to joining devices (overlay IP, public https host).
    /// When set, it wins over the auto-detected LAN address in lan_url().
    pub advertised_url: Option<String>,
    /// Site enrollment secret (256-bit hex): holders may self-enroll via the
    /// signed PSK handshake. Copied once per site by the operator; rotate to
    /// instantly invalidate every outstanding copy.
    pub enroll_secret: String,
}

impl HubConfig {
    pub fn load_or_create() -> Result<Self, String> {
        let path = config_path();
        Self::load_or_create_at(&path)
    }

    pub fn load_or_create_at(path: &Path) -> Result<Self, String> {
        if let Some(v) = load_json(path)? {
            let bind_ip = v
                .get("bind_ip")
                .and_then(|x| x.as_str())
                .unwrap_or(&std::net::Ipv4Addr::UNSPECIFIED.to_string())
                .to_string();
            let port = v
                .get("port")
                .and_then(|x| x.as_i64())
                .unwrap_or(DEFAULT_PORT as i64) as u16;
            let dashboard_key = match v.get("dashboard_key").and_then(|x| x.as_str()) {
                Some(k) if !k.is_empty() => k.to_string(),
                _ => {
                    return Err(
                        "config.json has empty dashboard_key; delete it to regenerate".into(),
                    )
                }
            };
            let created_at = v.get("created_at").and_then(|x| x.as_i64()).unwrap_or(0) as u64;
            let advertised_url = v
                .get("advertised_url")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty());
            let mut cfg = HubConfig {
                bind_ip,
                port,
                dashboard_key,
                created_at,
                advertised_url,
                enroll_secret: v
                    .get("enroll_secret")
                    .and_then(|x| x.as_str())
                    .filter(|s| s.len() == 64)
                    .unwrap_or_default()
                    .to_string(),
            };
            // Pre-v0.9 configs carry no enrollment secret; backfill one so
            // `wtf enroll-secret` works without deleting the config. The
            // persist is best-effort: the hub still serves if it fails.
            if cfg.enroll_secret.is_empty() {
                cfg.enroll_secret = rand::key_hex();
                let _ = Self::save_at(path, &cfg);
            }
            return Ok(cfg);
        }
        let cfg = HubConfig {
            bind_ip: std::net::Ipv4Addr::UNSPECIFIED.to_string(),
            port: DEFAULT_PORT,
            dashboard_key: rand::key_hex(),
            created_at: crate::util::now_secs(),
            advertised_url: None,
            enroll_secret: rand::key_hex(),
        };
        Self::save_at(path, &cfg)?;
        Ok(cfg)
    }

    /// Persist every config field (0600, atomic). Single write path for
    /// create / advertised-url / rotate so fields never drift apart.
    fn save_at(path: &Path, cfg: &HubConfig) -> Result<(), String> {
        let mut fields = vec![
            ("bind_ip", Value::from(cfg.bind_ip.as_str())),
            ("port", Value::from(cfg.port as i64)),
            ("dashboard_key", Value::from(cfg.dashboard_key.as_str())),
            ("created_at", Value::from(cfg.created_at as i64)),
        ];
        if let Some(u) = &cfg.advertised_url {
            fields.push(("advertised_url", Value::from(u.as_str())));
        }
        fields.push(("enroll_secret", Value::from(cfg.enroll_secret.as_str())));
        save_json(path, &Value::obj(fields), 0o600)
    }

    pub fn url(&self) -> String {
        format!("http://{}:{}", self.bind_ip, self.port)
    }

    /// URL handed out to joining devices. An explicitly advertised URL (set
    /// via `wtf url`, e.g. an overlay IP or a public https endpoint) wins over
    /// the auto-detected LAN address.
    pub fn lan_url(&self) -> String {
        if let Some(u) = &self.advertised_url {
            return u.clone();
        }
        let ip = if self.bind_ip == std::net::Ipv4Addr::UNSPECIFIED.to_string() {
            crate::util::lan_ip()
        } else {
            self.bind_ip.clone()
        };
        format!("http://{ip}:{}", self.port)
    }

    /// Set or clear the advertised URL, preserving all other config fields.
    pub fn set_advertised_url_at(path: &Path, url: Option<String>) -> Result<HubConfig, String> {
        let mut cfg = HubConfig::load_or_create_at(path)?;
        cfg.advertised_url = match url {
            Some(u) => {
                let u = u.trim().trim_end_matches('/').to_string();
                if !u.starts_with("http://") && !u.starts_with("https://") {
                    return Err("advertised url must start with http:// or https://".into());
                }
                Some(u)
            }
            None => None,
        };
        Self::save_at(path, &cfg)?;
        Ok(cfg)
    }

    pub fn set_advertised_url(url: Option<String>) -> Result<HubConfig, String> {
        Self::set_advertised_url_at(&config_path(), url)
    }

    /// Mint a fresh site enrollment secret, instantly invalidating every
    /// outstanding copy. Returns the new secret.
    pub fn rotate_enroll_secret() -> Result<String, String> {
        Self::rotate_enroll_secret_at(&config_path())
    }

    pub fn rotate_enroll_secret_at(path: &Path) -> Result<String, String> {
        let mut cfg = HubConfig::load_or_create_at(path)?;
        cfg.enroll_secret = rand::key_hex();
        Self::save_at(path, &cfg)?;
        Ok(cfg.enroll_secret)
    }
}

/// A provisioned device credential. `secret` is hex; it is key material and
/// never leaves the hub except in the one-time `key issue` output.
#[derive(Clone, Debug)]
pub struct DeviceRecord {
    pub name: String,
    pub secret: String,
    pub created_at: u64,
    pub revoked: bool,
}

pub struct KeyStore {
    path: PathBuf,
    pub records: Vec<DeviceRecord>,
}

impl KeyStore {
    pub fn load() -> Result<Self, String> {
        Self::load_at(&keys_path())
    }

    pub fn load_at(path: &Path) -> Result<Self, String> {
        let records = match load_json(path)? {
            None => Vec::new(),
            Some(v) => match v.get("devices").and_then(|d| d.as_arr()) {
                Some(arr) => arr
                    .iter()
                    .filter_map(|d| {
                        Some(DeviceRecord {
                            name: d.get("name")?.as_str()?.to_string(),
                            secret: d.get("secret")?.as_str()?.to_string(),
                            created_at: d.get("created_at").and_then(|x| x.as_i64()).unwrap_or(0)
                                as u64,
                            revoked: d.get("revoked").and_then(|x| x.as_bool()).unwrap_or(false),
                        })
                    })
                    .collect(),
                None => return Err(format!("{}: missing devices array", path.display())),
            },
        };
        Ok(KeyStore {
            path: path.to_path_buf(),
            records,
        })
    }

    pub fn persist(&self) -> Result<(), String> {
        let devices: Vec<Value> = self
            .records
            .iter()
            .map(|r| {
                Value::obj(vec![
                    ("name", Value::from(r.name.as_str())),
                    ("secret", Value::from(r.secret.as_str())),
                    ("created_at", Value::from(r.created_at as i64)),
                    ("revoked", Value::from(r.revoked)),
                ])
            })
            .collect();
        let v = Value::obj(vec![("devices", Value::Arr(devices))]);
        save_json(&self.path, &v, 0o600)
    }

    pub fn find_active(&self, name: &str) -> Option<&DeviceRecord> {
        self.records.iter().find(|r| r.name == name && !r.revoked)
    }

    /// Generate a new device secret. Refuses duplicate active names.
    pub fn issue(&mut self, name: &str) -> Result<String, String> {
        if !valid_name(name) {
            return Err("device name must match [A-Za-z0-9._-]{1,64}".into());
        }
        if self.find_active(name).is_some() {
            return Err(format!(
                "device '{name}' already exists (revoke it first to rotate)"
            ));
        }
        let secret = rand::key_hex();
        self.records.retain(|r| !(r.name == name && r.revoked));
        self.records.push(DeviceRecord {
            name: name.to_string(),
            secret: secret.clone(),
            created_at: crate::util::now_secs(),
            revoked: false,
        });
        self.persist()?;
        Ok(secret)
    }

    pub fn revoke(&mut self, name: &str) -> Result<bool, String> {
        let mut found = false;
        for r in self
            .records
            .iter_mut()
            .filter(|r| r.name == name && !r.revoked)
        {
            r.revoked = true;
            found = true;
        }
        if found {
            self.persist()?;
        }
        Ok(found)
    }
}

/// A one-time enrollment token record. Only the SHA-256 hash of the token is
/// stored; the plaintext crosses the hub once (`enroll-token` output) and the
/// wire once (`POST /api/v1/enroll`).
#[derive(Clone, Debug)]
pub struct EnrollToken {
    pub name: String,
    pub token_hash: String,
    pub expires_at: u64,
    pub used: bool,
}

/// Why a token failed to redeem. Callers map every variant to the same generic
/// refusal — never leak which check failed (except `Store`, an operator-side
/// outage, which gets a 5xx so the device knows to retry later).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TokenError {
    Unknown,
    Expired,
    Used,
    BadToken,
    Store,
}

/// Store of pending/used enrollment tokens at `enroll_tokens.json` (0600),
/// next to the keystore it feeds. The hub-side answer to operator copy-paste:
/// `wtf enroll-token` mints, `wtf enroll` redeems, expiry + single-use bound
/// the blast radius of a leaked token.
#[derive(Default)]
pub struct EnrollTokenStore {
    path: PathBuf,
    records: Vec<EnrollToken>,
}

impl EnrollTokenStore {
    pub fn load() -> Result<Self, String> {
        Self::load_at(&enroll_tokens_path())
    }

    pub fn load_at(path: &Path) -> Result<Self, String> {
        let records = match load_json(path)? {
            None => Vec::new(),
            Some(v) => match v.get("tokens").and_then(|d| d.as_arr()) {
                Some(arr) => arr
                    .iter()
                    .filter_map(|d| {
                        Some(EnrollToken {
                            name: d.get("name")?.as_str()?.to_string(),
                            token_hash: d.get("token_hash")?.as_str()?.to_string(),
                            expires_at: d.get("expires_at").and_then(|x| x.as_i64()).unwrap_or(0)
                                as u64,
                            used: d.get("used").and_then(|x| x.as_bool()).unwrap_or(false),
                        })
                    })
                    .collect(),
                None => return Err(format!("{}: missing tokens array", path.display())),
            },
        };
        Ok(EnrollTokenStore {
            path: path.to_path_buf(),
            records,
        })
    }

    fn persist(&self) -> Result<(), String> {
        let tokens: Vec<Value> = self
            .records
            .iter()
            .map(|r| {
                Value::obj(vec![
                    ("name", Value::from(r.name.as_str())),
                    ("token_hash", Value::from(r.token_hash.as_str())),
                    ("expires_at", Value::from(r.expires_at as i64)),
                    ("used", Value::from(r.used)),
                ])
            })
            .collect();
        let v = Value::obj(vec![("tokens", Value::Arr(tokens))]);
        save_json(&self.path, &v, 0o600)
    }

    /// Mint a token for `name`; the plaintext is returned once and only its
    /// hash is stored. Reissuing for the same name supersedes any pending
    /// token for that name — an old token can never be resurrected.
    pub fn issue(&mut self, name: &str, ttl_secs: u64) -> Result<String, String> {
        if !valid_name(name) {
            return Err("device name must match [A-Za-z0-9._-]{1,64}".into());
        }
        if ttl_secs == 0 || ttl_secs > 86_400 {
            return Err("ttl must be 1..=86400 seconds".into());
        }
        let now = crate::util::now_secs();
        // Reissue supersedes any prior record for this name; lazily prune
        // other names' expired-and-unused records.
        self.records.retain(|r| {
            if r.name == name {
                false
            } else {
                r.used || r.expires_at > now
            }
        });
        let token = rand::key_hex();
        self.records.push(EnrollToken {
            name: name.to_string(),
            token_hash: crate::sha256::hexdigest(token.as_bytes()),
            expires_at: now + ttl_secs,
            used: false,
        });
        self.persist()?;
        Ok(token)
    }

    /// Redeem `token` for `name`. Burns the record on success; failed attempts
    /// do NOT burn (a typo must not brick the token) — the hub-side rate
    /// limiter is the anti-guessing control.
    pub fn consume(&mut self, name: &str, token: &str) -> Result<(), TokenError> {
        let now = crate::util::now_secs();
        let idx = match self.records.iter().position(|r| r.name == name) {
            Some(i) => i,
            None => return Err(TokenError::Unknown),
        };
        let rec = &mut self.records[idx];
        if rec.used {
            return Err(TokenError::Used);
        }
        if rec.expires_at <= now {
            return Err(TokenError::Expired);
        }
        let expect = crate::sha256::hexdigest(token.as_bytes());
        if !crate::util::ct_eq_str(&rec.token_hash, &expect) {
            return Err(TokenError::BadToken);
        }
        rec.used = true;
        if self.persist().is_err() {
            self.records[idx].used = false;
            return Err(TokenError::Store);
        }
        Ok(())
    }

    /// Drop pending (unused) tokens for `name` — `wtf enroll-token revoke`.
    pub fn revoke(&mut self, name: &str) -> bool {
        let before = self.records.len();
        self.records.retain(|r| !(r.name == name && !r.used));
        if self.records.len() != before {
            self.persist().ok();
            true
        } else {
            false
        }
    }
}

pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
}

/// Bridge (agent-side) configuration. Env vars win over bridge.json, which
/// lets the PQC secrets bundle deliver key material via env without touching
/// the config file.
#[derive(Clone, Debug)]
pub struct BridgeConfig {
    pub hub_url: String,
    pub device_name: String,
    pub device_key: String,
}

impl BridgeConfig {
    pub fn load() -> Result<Self, String> {
        Self::load_at(&bridge_path())
    }

    pub fn load_at(path: &Path) -> Result<Self, String> {
        let file = load_json(path)?;
        let get = |k: &str| -> Option<String> {
            file.as_ref()
                .and_then(|v| v.get(k))
                .and_then(|x| x.as_str())
                .map(|s| s.to_string())
        };
        let env = |k: &str| std::env::var(k).ok().filter(|s| !s.trim().is_empty());
        let hub_url = env("WTF_HUB_URL").or_else(|| get("hub_url"));
        let device_name = env("WTF_DEVICE_NAME").or_else(|| get("device_name"));
        let device_key = env("WTF_DEVICE_KEY").or_else(|| get("device_key"));
        let missing: Vec<&str> = [
            ("hub_url", hub_url.is_none()),
            ("device_name", device_name.is_none()),
            ("device_key", device_key.is_none()),
        ]
        .iter()
        .filter(|(_, m)| *m)
        .map(|(n, _)| *n)
        .collect();
        if !missing.is_empty() {
            return Err(format!(
                "bridge config incomplete; missing {} (env WTF_HUB_URL/WTF_DEVICE_NAME/WTF_DEVICE_KEY or '{}')",
                missing.join(", "),
                path.display()
            ));
        }
        let cfg = BridgeConfig {
            hub_url: hub_url.unwrap().trim_end_matches('/').to_string(),
            device_name: device_name.unwrap(),
            device_key: device_key.unwrap(),
        };
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn validate(&self) -> Result<(), String> {
        // http:// covers LAN and encrypted overlay networks (WireGuard et al);
        // https:// covers deployments behind a TLS-terminating proxy. Request
        // authenticity comes from the HMAC signature in both cases.
        if !self.hub_url.starts_with("http://") && !self.hub_url.starts_with("https://") {
            return Err(
                "hub_url must start with http:// (LAN/overlay) or https:// (proxied)".into(),
            );
        }
        if !valid_name(&self.device_name) {
            return Err("device_name must match [A-Za-z0-9._-]{1,64}".into());
        }
        if crate::util::hex_decode(&self.device_key)
            .map(|b| b.len())
            .unwrap_or(0)
            != 32
        {
            return Err("device_key must be 64 hex chars (256 bits)".into());
        }
        Ok(())
    }

    pub fn save(&self) -> Result<(), String> {
        Self::save_at(&bridge_path(), self)
    }

    pub fn save_at(path: &Path, cfg: &BridgeConfig) -> Result<(), String> {
        let v = Value::obj(vec![
            ("hub_url", Value::from(cfg.hub_url.as_str())),
            ("device_name", Value::from(cfg.device_name.as_str())),
            ("device_key", Value::from(cfg.device_key.as_str())),
        ]);
        save_json(path, &v, 0o600)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enroll_secret_generated_and_rotates() {
        let path = temp_dir("pskcfg").join("config.json");
        let cfg = HubConfig::load_or_create_at(&path).expect("create");
        assert_eq!(cfg.enroll_secret.len(), 64);
        let fresh = HubConfig::rotate_enroll_secret_at(&path).expect("rotate");
        assert_eq!(fresh.len(), 64);
        let reloaded = HubConfig::load_or_create_at(&path).expect("reload");
        assert_eq!(reloaded.enroll_secret, fresh);
        assert_ne!(reloaded.enroll_secret, cfg.enroll_secret);
    }

    #[test]
    fn enroll_secret_backfills_older_configs() {
        let dir = temp_dir("pskupg");
        let path = dir.join("config.json");
        // A v0.8-era config carries no enroll_secret field.
        let legacy = Value::obj(vec![
            ("bind_ip", Value::from("127.0.0.1")),
            ("port", Value::from(DEFAULT_PORT as i64)),
            ("dashboard_key", Value::from("k".repeat(64).as_str())),
            ("created_at", Value::from(1i64)),
        ]);
        save_json(&path, &legacy, 0o600).expect("write legacy config");
        let cfg = HubConfig::load_or_create_at(&path).expect("load legacy");
        assert_eq!(cfg.enroll_secret.len(), 64);
        // The backfill persists: a second load returns the same secret.
        let again = HubConfig::load_or_create_at(&path).expect("reload");
        assert_eq!(again.enroll_secret, cfg.enroll_secret);
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "wtf-test-{tag}-{}-{}",
            std::process::id(),
            crate::rand::hex(6)
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn keystore_issue_revoke_rotate() {
        let d = temp_dir("keystore");
        let mut ks = KeyStore::load_at(&d.join("keys.json")).unwrap();
        assert!(ks.records.is_empty());
        let s1 = ks.issue("laptop").unwrap();
        assert_eq!(s1.len(), 64);
        assert!(ks.issue("laptop").is_err());
        assert!(ks.issue("bad name!").is_err());
        assert!(ks.revoke("laptop").unwrap());
        assert!(ks.find_active("laptop").is_none());
        // rotation after revoke is allowed
        let s2 = ks.issue("laptop").unwrap();
        assert_ne!(s1, s2);
        // persists across reload; re-issue purges the revoked record
        let ks2 = KeyStore::load_at(&d.join("keys.json")).unwrap();
        assert_eq!(ks2.records.len(), 1);
        assert!(ks2.records.iter().all(|r| r.name == "laptop" && !r.revoked));
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn hub_config_generated_once() {
        let d = temp_dir("hubcfg");
        let p = d.join("config.json");
        let c1 = HubConfig::load_or_create_at(&p).unwrap();
        let c2 = HubConfig::load_or_create_at(&p).unwrap();
        assert_eq!(c1.dashboard_key, c2.dashboard_key);
        assert_eq!(c1.port, DEFAULT_PORT);
        let meta = std::fs::metadata(&p).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn hub_config_advertised_url_roundtrip() {
        let d = temp_dir("advertised");
        let p = d.join("config.json");
        let c0 = HubConfig::load_or_create_at(&p).unwrap();
        assert!(c0.advertised_url.is_none());
        let c1 =
            HubConfig::set_advertised_url_at(&p, Some("http://hub.tailnet:7800".into())).unwrap();
        assert_eq!(
            c1.advertised_url.as_deref(),
            Some("http://hub.tailnet:7800")
        );
        // Persists, preserves other fields, and wins over lan_url().
        let c2 = HubConfig::load_or_create_at(&p).unwrap();
        assert_eq!(
            c2.advertised_url.as_deref(),
            Some("http://hub.tailnet:7800")
        );
        assert_eq!(c2.dashboard_key, c0.dashboard_key);
        assert_eq!(c2.lan_url(), "http://hub.tailnet:7800");
        // Trailing slash trimmed; clearing works; bad scheme refused.
        let c3 =
            HubConfig::set_advertised_url_at(&p, Some("https://hub.example.com/".into())).unwrap();
        assert_eq!(
            c3.advertised_url.as_deref(),
            Some("https://hub.example.com")
        );
        assert!(HubConfig::set_advertised_url_at(&p, Some("ftp://x".into())).is_err());
        let c4 = HubConfig::set_advertised_url_at(&p, None).unwrap();
        assert!(c4.advertised_url.is_none());
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn enroll_token_lifecycle() {
        let d = temp_dir("entoken");
        let mut ts = EnrollTokenStore::load_at(&d.join("tokens.json")).unwrap();
        let t = ts.issue("box1", 600).unwrap();
        assert_eq!(t.len(), 64);
        assert!(ts.issue("bad name!", 600).is_err());
        assert!(ts.issue("box1", 0).is_err());
        assert!(ts.issue("box1", 86_401).is_err());
        // Failures do not burn: wrong token, unknown name — then the real
        // token still redeems exactly once.
        assert_eq!(
            ts.consume("box1", &"0".repeat(64)),
            Err(TokenError::BadToken)
        );
        assert_eq!(ts.consume("ghost", &t), Err(TokenError::Unknown));
        assert!(ts.consume("box1", &t).is_ok());
        assert_eq!(ts.consume("box1", &t), Err(TokenError::Used));
        // The burned flag persists across reload.
        let mut ts2 = EnrollTokenStore::load_at(&d.join("tokens.json")).unwrap();
        assert_eq!(ts2.consume("box1", &t), Err(TokenError::Used));
        // Expiry: deterministic via a crafted record (no sleeping).
        let mut ts3 = EnrollTokenStore::load_at(&d.join("tokens.json")).unwrap();
        let t3 = ts3.issue("box3", 600).unwrap();
        let i3 = ts3.records.iter().position(|r| r.name == "box3").unwrap();
        ts3.records[i3].expires_at = 1;
        assert_eq!(ts3.consume("box3", &t3), Err(TokenError::Expired));
        // Reissue supersedes the pending token for the same name.
        let mut ts4 = EnrollTokenStore::load_at(&d.join("tokens.json")).unwrap();
        let t4 = ts4.issue("box4", 600).unwrap();
        let _t4b = ts4.issue("box4", 600).unwrap();
        assert_eq!(ts4.consume("box4", &t4), Err(TokenError::BadToken));
        // Revoke drops pending tokens.
        assert!(ts4.revoke("box4"));
        assert_eq!(ts4.consume("box4", "x"), Err(TokenError::Unknown));
        assert!(!ts4.revoke("nobody"));
        // File mode is operator-only.
        let meta = std::fs::metadata(d.join("tokens.json")).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn bridge_config_validation_and_save() {
        let d = temp_dir("bridge");
        let p = d.join("bridge.json");
        let good = BridgeConfig {
            hub_url: format!("http://{}:{}", std::net::Ipv4Addr::LOCALHOST, 7800),
            device_name: "box1".into(),
            device_key: crate::rand::key_hex(),
        };
        assert!(good.validate().is_ok());
        BridgeConfig::save_at(&p, &good).unwrap();
        let meta = std::fs::metadata(&p).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        // https is now accepted (TLS-terminating proxy / cloud deployment).
        let tls = BridgeConfig {
            hub_url: "https://hub.example.com".into(),
            ..good.clone()
        };
        assert!(tls.validate().is_ok());
        let bad = BridgeConfig {
            hub_url: "ftp://example.invalid".into(),
            ..good.clone()
        };
        assert!(bad.validate().is_err());
        let bad2 = BridgeConfig {
            device_key: "tooshort".into(),
            ..good
        };
        assert!(bad2.validate().is_err());
        std::fs::remove_dir_all(&d).ok();
    }
}
