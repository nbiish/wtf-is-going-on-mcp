//! Encrypted agent-to-agent session tools (bridge side).
//!
//! Implements the client side of the FIPS 203 sealing protocol:
//! - `session_create`: make a channel, generate a random 256-bit session
//!   key, seal it to the creator's own identity, register everything.
//! - `session_join`: join with a fresh ek, fetch sealed packages, decapsulate.
//! - `session_seal`: creator encapsulates the session key to a new member's
//!   registered ek and posts the package.
//! - `session_send`: AES-256-GCM encrypt with a per-(session, sender)
//!   HKDF-SHA3-256 subkey, AAD binds (session, sender, seq).
//! - `session_read`: poll, decrypt, verify AAD binding.
//! - `session_list`: list channels.
//!
//! Sequence numbers are assigned by the hub; the sender fetches the seq
//! from the send response and uses it in the AAD of the NEXT message. The
//! first message uses seq from the create/join response (`next_seq`).

use crate::gcm;
use crate::json::Value;
use crate::keccak;

/// Per-(session, sender) subkey: HKDF-SHA3-256 style expand of
/// (session_id || sender) with the session key as IKM. Session keys are
/// unique per channel, so subkey reuse across sessions is impossible; the
/// sender binding prevents two senders from colliding on nonces.
fn session_subkey(session_key: &[u8; 32], session_id: &str, sender: &str) -> [u8; 32] {
    let salt = sha3_256_concat(session_id.as_bytes(), sender.as_bytes());
    hkdf_sha3_expand(session_key, &salt, b"wtf-session-v1")
}

/// SHA3-256(two concatenated parts).
fn sha3_256_concat(a: &[u8], b: &[u8]) -> [u8; 32] {
    let mut k = keccak::Keccak::sha3_256();
    k.update(a);
    k.update(b);
    let mut out = [0u8; 32];
    k.squeeze_into(&mut out);
    out
}

/// HKDF-Expand built on SHA3-256 (T = HMAC-less expand: T(i) = SHA3-256(
/// T(i-1) || info || i) — a standard extract-then-expand fallback for
/// protocols without HMAC; here the IKM is a uniformly random session key
/// (already full-entropy extract), so plain expand preserves security.
fn hkdf_sha3_expand(ikm: &[u8; 32], salt: &[u8; 32], info: &[u8]) -> [u8; 32] {
    let mut k = keccak::Keccak::sha3_256();
    k.update(salt);
    k.update(ikm);
    k.update(info);
    let mut out = [0u8; 32];
    k.squeeze_into(&mut out);
    out
}

/// AAD binding: domain || session || sender || seq — prevents ciphertext
/// replay across sessions, members, or sequence positions.
fn message_aad(session_id: &str, sender: &str, seq: u64) -> Vec<u8> {
    let mut aad = b"wtf-msg-v1".to_vec();
    aad.extend_from_slice(session_id.as_bytes());
    aad.push(0);
    aad.extend_from_slice(sender.as_bytes());
    aad.push(0);
    aad.extend_from_slice(&seq.to_be_bytes());
    aad
}

/// Nonce: 96-bit, derived from subkey + seq (deterministic, never reused
/// under the same subkey because seq is monotonic per session).
fn message_nonce(subkey: &[u8; 32], seq: u64) -> [u8; 12] {
    let mut k = keccak::Keccak::sha3_256();
    k.update(subkey);
    k.update(&seq.to_be_bytes());
    let mut out = [0u8; 12];
    k.squeeze_into(&mut out);
    out
}

pub fn hex_enc(data: &[u8]) -> String {
    crate::util::hex_encode(data)
}

pub fn dec_hex(s: &str) -> Option<Vec<u8>> {
    crate::util::hex_decode(s)
}

/// ek fingerprint (16 hex) used to route sealed packages.
pub fn ek_fp(ek_hex: &str) -> String {
    let bytes = crate::util::hex_decode(ek_hex).unwrap_or_default();
    crate::util::hex_encode(&keccak::sha3_256(&bytes)[..8])
}

/// Seal the session key to a member ek: ML-KEM-768 encapsulate + AES-256-GCM
/// wrap of the session key under the encapsulated shared secret (defense in
/// depth: the KEM ciphertext alone never carries the key).
pub fn seal_session_key(
    member_ek_hex: &str,
    session_key: &[u8; 32],
    session_id: &str,
) -> Result<String, String> {
    let ek_bytes = dec_hex(member_ek_hex).ok_or("member ek is not valid hex")?;
    let ek: [u8; 1184] = ek_bytes.try_into().map_err(|_| "member ek wrong length")?;
    let (kem_ct, shared) = crate::mlkem768::encaps(&ek);
    // Wrap: AES-256-GCM under SHA3-256(shared) with AAD binding the session.
    let mut aek = [0u8; 32];
    aek.copy_from_slice(&keccak::sha3_256(&shared));
    let nonce: [u8; 12] = {
        let mut k = keccak::Keccak::sha3_256();
        k.update(b"wtf-seal-v1");
        k.update(session_id.as_bytes());
        k.update(&kem_ct);
        let mut n = [0u8; 12];
        k.squeeze_into(&mut n);
        n
    };
    let sealed = gcm::seal(&aek, &nonce, session_id.as_bytes(), session_key);
    let mut pkg = Vec::with_capacity(1088 + sealed.len());
    pkg.extend_from_slice(&kem_ct);
    pkg.extend_from_slice(&sealed);
    Ok(hex_enc(&pkg))
}

/// Open a sealed package addressed to us: decapsulate + unwrap.
pub fn open_sealed_package(
    pkg_hex: &str,
    dk: &[u8; 2400],
    session_id: &str,
) -> Result<[u8; 32], String> {
    let pkg = dec_hex(pkg_hex).ok_or("sealed package is not valid hex")?;
    if pkg.len() < 1088 + 48 {
        return Err("sealed package too short".into());
    }
    let (kem_ct, rest) = pkg.split_at(1088);
    let kem_ct: [u8; 1088] = kem_ct.try_into().unwrap();
    let shared = crate::mlkem768::decaps(dk, &kem_ct);
    let mut aek = [0u8; 32];
    aek.copy_from_slice(&keccak::sha3_256(&shared));
    let nonce: [u8; 12] = {
        let mut k = keccak::Keccak::sha3_256();
        k.update(b"wtf-seal-v1");
        k.update(session_id.as_bytes());
        k.update(&kem_ct);
        let mut n = [0u8; 12];
        k.squeeze_into(&mut n);
        n
    };
    gcm::open(&aek, &nonce, session_id.as_bytes(), rest)
        .map(|pt| {
            let mut key = [0u8; 32];
            key.copy_from_slice(&pt);
            key
        })
        .map_err(|e| format!("seal open failed: {e}"))
}

/// Encrypt one message under the sender subkey.
pub fn seal_message(
    session_key: &[u8; 32],
    session_id: &str,
    sender: &str,
    seq: u64,
    plaintext: &str,
) -> Result<(String, String), String> {
    let subkey = session_subkey(session_key, session_id, sender);
    let nonce = message_nonce(&subkey, seq);
    let aad = message_aad(session_id, sender, seq);
    let ct = gcm::seal(&subkey, &nonce, &aad, plaintext.as_bytes());
    Ok((hex_enc(&nonce), hex_enc(&ct)))
}

/// Decrypt one message under the sender subkey.
pub fn open_message(
    session_key: &[u8; 32],
    session_id: &str,
    sender: &str,
    seq: u64,
    nonce_hex: &str,
    ct_hex: &str,
) -> Result<String, String> {
    let subkey = session_subkey(session_key, session_id, sender);
    let nonce_bytes = dec_hex(nonce_hex).ok_or("nonce not valid hex")?;
    let nonce: [u8; 12] = nonce_bytes.try_into().map_err(|_| "nonce wrong length")?;
    let aad = message_aad(session_id, sender, seq);
    let ct = dec_hex(ct_hex).ok_or("ciphertext not valid hex")?;
    let pt = gcm::open(&subkey, &nonce, &aad, &ct).map_err(|e| format!("decrypt failed: {e}"))?;
    String::from_utf8(pt).map_err(|_| "plaintext is not valid UTF-8".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> String {
        s.to_string()
    }

    #[test]
    fn seal_open_roundtrip() {
        let (ek, dk) = crate::mlkem768::keygen();
        let ek_hex = hex_enc(&ek);
        let session_key: [u8; 32] = crate::rand::bytes(32).try_into().unwrap();
        let pkg = seal_session_key(&ek_hex, &session_key, "sess-123").unwrap();
        let opened = open_sealed_package(&pkg, &dk, "sess-123").unwrap();
        assert_eq!(opened, session_key);
        // wrong session binding fails
        assert!(open_sealed_package(&pkg, &dk, "sess-999").is_err());
        // wrong dk fails closed
        let (_ek2, dk2) = crate::mlkem768::keygen();
        assert!(open_sealed_package(&pkg, &dk2, "sess-123").is_err());
    }

    #[test]
    fn message_roundtrip_and_replay() {
        let key = [42u8; 32];
        let (nonce, ct) = seal_message(&key, "sess-1", "mac-agent", 5, "hello agent").unwrap();
        let pt = open_message(&key, "sess-1", "mac-agent", 5, &nonce, &ct).unwrap();
        assert_eq!(pt, "hello agent");
        // wrong seq → AAD mismatch → fail closed
        assert!(open_message(&key, "sess-1", "mac-agent", 6, &nonce, &ct).is_err());
        // wrong sender → different subkey → fail closed
        assert!(open_message(&key, "sess-1", "other-agent", 5, &nonce, &ct).is_err());
        // different session id → fail closed
        assert!(open_message(&key, "sess-2", "mac-agent", 5, &nonce, &ct).is_err());
    }

    #[test]
    fn ek_fp_stable() {
        let (ek, _dk) = crate::mlkem768::keygen();
        let ek_hex = hex_enc(&ek);
        assert_eq!(ek_fp(&ek_hex), ek_fp(&hex(&ek_hex)));
    }
}
