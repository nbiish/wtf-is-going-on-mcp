//! Bridge-side session identity: a per-device ML-KEM-768 keypair stored in
//! `$WTF_HOME/identity.json` (0600). The public encapsulation key is
//! registered with the hub; the decapsulation key never leaves the machine.
//!
//! The session key for a channel is random per session; each member
//! receives it inside an ML-KEM-768 encapsulation sealed to their public
//! key. Messages are AES-256-GCM with per-sender HKDF-SHA3-derived
//! subkeys, keyed by session id + sender, so senders never reuse nonces
//! under the same key material.

use crate::config;
use crate::json::Value;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

pub const EK_HEX: usize = 2368; // 1184-byte encapsulation key
pub const DK_HEX: usize = 4800; // 2400-byte decapsulation key (expanded)

#[derive(Clone, Debug)]
pub struct Identity {
    pub ek: [u8; 1184],
    pub dk: [u8; 2400],
}

pub fn identity_path() -> PathBuf {
    config::home().join("identity.json")
}

/// Load the identity, generating + persisting a fresh keypair on first
/// use. Fails closed if the file exists but is corrupt (never overwrite).
pub fn load_or_create() -> Result<Identity, String> {
    let path = identity_path();
    load_or_create_at(&path)
}

pub fn load_or_create_at(path: &PathBuf) -> Result<Identity, String> {
    if let Some(v) = config::load_json(path)? {
        let ek_hex = v
            .get("ek")
            .and_then(|x| x.as_str())
            .ok_or_else(|| format!("{}: missing 'ek'", path.display()))?;
        let dk_hex = v
            .get("dk")
            .and_then(|x| x.as_str())
            .ok_or_else(|| format!("{}: missing 'dk'", path.display()))?;
        let ek = parse_key::<1184>(ek_hex, path)?;
        let dk = parse_key::<2400>(dk_hex, path)?;
        return Ok(Identity { ek, dk });
    }
    let (ek, dk) = crate::mlkem768::keygen();
    let v = Value::obj(vec![
        ("ek", Value::from(crate::util::hex_encode(&ek))),
        ("dk", Value::from(crate::util::hex_encode(&dk))),
        ("created_at", Value::from(crate::util::now_secs() as i64)),
    ]);
    config::save_json(path, &v, 0o600)?;
    Ok(Identity { ek, dk })
}

/// Delete the identity (key rotation support); next use generates fresh.
pub fn purge() -> Result<(), String> {
    let p = identity_path();
    match std::fs::remove_file(&p) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("{}: {e}", p.display())),
    }
}

fn parse_key<const N: usize>(hex: &str, path: &PathBuf) -> Result<[u8; N], String> {
    let bytes = crate::util::hex_decode(hex)
        .ok_or_else(|| format!("{}: key is not valid hex", path.display()))?;
    <[u8; N]>::try_from(bytes.as_slice())
        .map_err(|_| format!("{}: key has wrong length ({} bytes, want {N})", path.display(), bytes.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_id(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "wtf-identity-{tag}-{}-{}",
            std::process::id(),
            crate::rand::hex(6)
        ));
        std::fs::create_dir_all(&d).unwrap();
        d.join("identity.json")
    }

    #[test]
    fn generate_once_reload_same() {
        let p = temp_id("gen");
        let id1 = load_or_create_at(&p).unwrap();
        let id2 = load_or_create_at(&p).unwrap();
        assert_eq!(id1.ek, id2.ek);
        assert_eq!(id1.dk, id2.dk);
        // 0600
        let meta = std::fs::metadata(&p).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }

    #[test]
    fn purge_then_regenerate() {
        let p = temp_id("purge");
        let id1 = load_or_create_at(&p).unwrap();
        purge_at(&p).unwrap();
        let id2 = load_or_create_at(&p).unwrap();
        assert_ne!(id1.ek, id2.ek);
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }

    #[test]
    fn corrupt_file_fails_closed() {
        let p = temp_id("corrupt");
        std::fs::write(&p, "not json").unwrap();
        assert!(load_or_create_at(&p).is_err());
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }
}

/// Purge at an explicit path (test seam).
pub fn purge_at(path: &PathBuf) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}
