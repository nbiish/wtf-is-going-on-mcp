//! Transport authentication: HMAC-SHA256 request signing with replay
//! protection. Standard-crypto-for-transport lane per AGENTS.md (no
//! secrets-at-rest operations happen here).
//!
//! Canonical string (documented in README):
//!   wtf-hmac-v1 \n METHOD \n path-and-query \n timestamp \n nonce \n sha256hex(body)
//! signature = hex(hmac_sha256(device_secret, canonical))
//!
//! Headers: x-wtf-device / x-wtf-timestamp / x-wtf-nonce / x-wtf-signature.
//! Verification rejects |now - ts| > SKEW_SECS and any repeated
//! (device, nonce) within the replay window. Secret material never crosses
//! the wire; comparisons are constant-time.

use crate::config::KeyStore;
use crate::util::{ct_eq_str, now_secs};
use std::collections::HashMap;

pub const ALGO_TAG: &str = "wtf-hmac-v1";
pub const SKEW_SECS: u64 = 300;
pub const HDR_DEVICE: &str = "x-wtf-device";
pub const HDR_TS: &str = "x-wtf-timestamp";
pub const HDR_NONCE: &str = "x-wtf-nonce";
pub const HDR_SIG: &str = "x-wtf-signature";

#[derive(Debug, Clone)]
pub enum AuthError {
    Missing(&'static str),
    Malformed(&'static str),
    UnknownDevice,
    BadTimestamp,
    Replay,
    BadSignature,
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthError::Missing(h) => write!(f, "missing header {h}"),
            AuthError::Malformed(h) => write!(f, "malformed header {h}"),
            AuthError::UnknownDevice => write!(f, "unknown device"),
            AuthError::BadTimestamp => write!(f, "timestamp outside allowed skew"),
            AuthError::Replay => write!(f, "nonce replay detected"),
            AuthError::BadSignature => write!(f, "signature mismatch"),
        }
    }
}

pub fn string_to_sign(
    method: &str,
    path_and_query: &str,
    ts: u64,
    nonce: &str,
    body: &[u8],
) -> String {
    format!(
        "{ALGO_TAG}\n{method}\n{path_and_query}\n{ts}\n{nonce}\n{}",
        crate::sha256::hexdigest(body)
    )
}

pub fn sign(
    secret_hex: &str,
    method: &str,
    path_and_query: &str,
    ts: u64,
    nonce: &str,
    body: &[u8],
) -> Option<String> {
    let key = crate::util::hex_decode(secret_hex)?;
    Some(crate::hmac::hmac_sha256_hex(
        &key,
        string_to_sign(method, path_and_query, ts, nonce, body).as_bytes(),
    ))
}

#[derive(Debug, Clone)]
pub struct AuthHeaders {
    pub device: String,
    pub ts: u64,
    pub nonce: String,
    pub sig: String,
}

pub fn extract(headers: &[(String, String)]) -> Result<AuthHeaders, AuthError> {
    let get = |name: &'static str| -> Result<&str, AuthError> {
        headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
            .ok_or(AuthError::Missing(name))
    };
    let device = get(HDR_DEVICE)?;
    if device.is_empty() || device.len() > 64 {
        return Err(AuthError::Malformed(HDR_DEVICE));
    }
    let ts_raw = get(HDR_TS)?;
    let ts: u64 = ts_raw.parse().map_err(|_| AuthError::Malformed(HDR_TS))?;
    let nonce = get(HDR_NONCE)?;
    if nonce.is_empty() || nonce.len() > 128 || crate::util::hex_decode(nonce).is_none() {
        return Err(AuthError::Malformed(HDR_NONCE));
    }
    let sig = get(HDR_SIG)?;
    if sig.len() != 64 || crate::util::hex_decode(sig).is_none() {
        return Err(AuthError::Malformed(HDR_SIG));
    }
    Ok(AuthHeaders {
        device: device.to_string(),
        ts,
        nonce: nonce.to_string(),
        sig: sig.to_string(),
    })
}

/// Nonce replay cache: (device:nonce) -> expiry. Prunes opportunistically.
#[derive(Default)]
pub struct NonceCache {
    map: HashMap<String, u64>,
}

impl NonceCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// true = first use (accepted), false = replay.
    pub fn check_and_insert(&mut self, key: &str, expires_at: u64) -> bool {
        if self.map.len() > 4096 {
            let now = now_secs();
            self.map.retain(|_, exp| *exp > now);
        }
        if self.map.contains_key(key) {
            return false;
        }
        self.map.insert(key.to_string(), expires_at);
        true
    }
}

/// Verify signed headers; returns the authenticated device name.
pub fn verify(
    h: AuthHeaders,
    keys: &KeyStore,
    nonces: &mut NonceCache,
    method: &str,
    path_and_query: &str,
    body: &[u8],
) -> Result<String, AuthError> {
    let now = now_secs();
    // Device lookup first: an unknown-device miss must not consume the nonce,
    // so callers can reload the keystore and retry with the same headers.
    let record = keys
        .find_active(&h.device)
        .ok_or(AuthError::UnknownDevice)?;
    if h.ts.abs_diff(now) > SKEW_SECS {
        return Err(AuthError::BadTimestamp);
    }
    let nonce_key = format!("{}:{}", h.device, h.nonce);
    if !nonces.check_and_insert(&nonce_key, now + SKEW_SECS + 60) {
        return Err(AuthError::Replay);
    }
    let expected = sign(&record.secret, method, path_and_query, h.ts, &h.nonce, body)
        .ok_or(AuthError::BadSignature)?;
    if !ct_eq_str(&expected, &h.sig) {
        return Err(AuthError::BadSignature);
    }
    Ok(h.device)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::KeyStore;
    use std::path::Path;

    fn keystore_with_device(tmp: &Path, name: &str) -> (KeyStore, String) {
        let path = tmp.join("keys.json");
        let mut ks = KeyStore::load_at(&path).unwrap();
        let secret = ks.issue(name).unwrap();
        (ks, secret)
    }

    fn tmpdir() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "wtf-auth-{}-{}",
            std::process::id(),
            crate::rand::hex(6)
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn authz(
        secret: &str,
        device: &str,
        body: &[u8],
        ts: u64,
        nonce: &str,
    ) -> Vec<(String, String)> {
        let sig = sign(secret, "POST", "/api/v1/checkin", ts, nonce, body).unwrap();
        vec![
            (HDR_DEVICE.into(), device.into()),
            (HDR_TS.into(), ts.to_string()),
            (HDR_NONCE.into(), nonce.into()),
            (HDR_SIG.into(), sig),
        ]
    }

    #[test]
    fn sign_verify_roundtrip() {
        let tmp = tmpdir();
        let (ks, secret) = keystore_with_device(&tmp, "box1");
        let body = br#"{"status":"working"}"#;
        let ts = now_secs();
        let headers = authz(&secret, "box1", body, ts, "aabb");
        let h = extract(&headers).unwrap();
        let mut nonces = NonceCache::new();
        let dev = verify(h, &ks, &mut nonces, "POST", "/api/v1/checkin", body).unwrap();
        assert_eq!(dev, "box1");
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn tampered_body_rejected() {
        let tmp = tmpdir();
        let (ks, secret) = keystore_with_device(&tmp, "box1");
        let ts = now_secs();
        let headers = authz(&secret, "box1", b"body-a", ts, "cc");
        let h = extract(&headers).unwrap();
        let mut nonces = NonceCache::new();
        assert!(matches!(
            verify(h, &ks, &mut nonces, "POST", "/api/v1/checkin", b"body-b"),
            Err(AuthError::BadSignature)
        ));
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn replay_rejected() {
        let tmp = tmpdir();
        let (ks, secret) = keystore_with_device(&tmp, "box1");
        let ts = now_secs();
        let headers = authz(&secret, "box1", b"x", ts, "dd");
        let mut nonces = NonceCache::new();
        let h = extract(&headers).unwrap();
        verify(h.clone(), &ks, &mut nonces, "POST", "/api/v1/checkin", b"x").unwrap();
        assert!(matches!(
            verify(h, &ks, &mut nonces, "POST", "/api/v1/checkin", b"x"),
            Err(AuthError::Replay)
        ));
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn stale_timestamp_rejected() {
        let tmp = tmpdir();
        let (ks, secret) = keystore_with_device(&tmp, "box1");
        let ts = now_secs() - SKEW_SECS - 10;
        let headers = authz(&secret, "box1", b"x", ts, "ee");
        let h = extract(&headers).unwrap();
        let mut nonces = NonceCache::new();
        assert!(matches!(
            verify(h, &ks, &mut nonces, "POST", "/api/v1/checkin", b"x"),
            Err(AuthError::BadTimestamp)
        ));
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn unknown_device_and_missing_headers() {
        let tmp = tmpdir();
        let (ks, _secret) = keystore_with_device(&tmp, "box1");
        let ts = now_secs();
        let headers = authz(&crate::rand::key_hex().as_str(), "ghost", b"x", ts, "ff");
        let h = extract(&headers).unwrap();
        let mut nonces = NonceCache::new();
        assert!(matches!(
            verify(h, &ks, &mut nonces, "POST", "/api/v1/checkin", b"x"),
            Err(AuthError::UnknownDevice)
        ));
        assert!(matches!(extract(&[]), Err(AuthError::Missing(_))));
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn canonical_string_shape() {
        let s = string_to_sign("POST", "/x", 1, "ab", b"hi");
        let expect = format!(
            "{ALGO_TAG}\nPOST\n/x\n1\nab\n{}",
            crate::sha256::hexdigest(b"hi")
        );
        assert_eq!(s, expect);
    }
}
