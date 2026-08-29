//! HMAC-SHA256 (RFC 2104), implemented in-tree on `std` only.
//!
//! Validated against RFC 4231 test vectors below.

use crate::sha256::{Sha256, sha256};

const BLOCK: usize = 64;

pub fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut k = [0u8; BLOCK];
    if key.len() > BLOCK {
        k[..32].copy_from_slice(&sha256(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }

    let mut ipad = [0x36u8; BLOCK];
    let mut opad = [0x5cu8; BLOCK];
    for i in 0..BLOCK {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }

    let mut inner = Sha256::new();
    inner.update(&ipad);
    inner.update(message);
    let inner_hash = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(&opad);
    outer.update(&inner_hash);
    outer.finalize()
}

pub fn hmac_sha256_hex(key: &[u8], message: &[u8]) -> String {
    crate::util::hex_encode(&hmac_sha256(key, message))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        crate::util::hex_decode(s).unwrap()
    }

    #[test]
    fn rfc4231_case1() {
        // TC1: key = 0x0b x 20, data = "Hi There"
        let key = hex("0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b");
        assert_eq!(
            hmac_sha256_hex(&key, b"Hi There"),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }

    #[test]
    fn rfc4231_case2() {
        // TC2: key = "Jefe", data = "what do ya want for nothing?"
        assert_eq!(
            hmac_sha256_hex(b"Jefe", b"what do ya want for nothing?"),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn rfc4231_case6_oversize_key() {
        // TC6: 131-byte key exercises the hash-the-key-first path.
        let key = vec![0xaau8; 131];
        assert_eq!(
            hmac_sha256_hex(&key, b"Test Using Larger Than Block-Size Key - Hash Key First"),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn empty_key_and_message() {
        // TC with empty key/message sanity: differs from non-empty inputs.
        let e = hmac_sha256(b"", b"");
        assert_eq!(e.len(), 32);
        assert_ne!(e, hmac_sha256(b"x", b""));
    }
}
