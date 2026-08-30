//! AES-256-GCM authenticated encryption (NIST SP 800-38D), in-tree.
//!
//! CTR-mode keystream from the AES-256 block cipher plus GHASH over
//! GF(2^128) for the tag. Deterministic test vectors come from the
//! original McGrew/Viega GCM spec (appendix B); tamper/roundtrip tests
//! cover the rest. Security notes:
//! - 96-bit nonces only (SP 800-38D §8); never reuse a key+nonce pair.
//! - Tag compare must be constant-time (ct_eq).

use crate::aes::Aes256;
use crate::util::ct_eq;

/// GHASH multiplication in GF(2^128), polynomial x^128 + x^7 + x^2 + x + 1
/// (GCM's MSB-first bit-string convention).
fn ghash_mul(x: &[u8; 16], y: &[u8; 16]) -> [u8; 16] {
    let mut z = [0u8; 16];
    let mut v = *y;
    for i in 0..128 {
        let bit = (x[i / 8] >> (7 - (i % 8))) & 1;
        if bit == 1 {
            for b in 0..16 {
                z[b] ^= v[b];
            }
        }
        // v >>= 1 with reduction when the LSB (x^0 coefficient) shifts out.
        let lsb = v[15] & 1;
        for b in (1..16).rev() {
            v[b] = (v[b] >> 1) | (v[b - 1] << 7);
        }
        v[0] >>= 1;
        if lsb == 1 {
            v[0] ^= 0xe1;
        }
    }
    z
}

/// Increment the rightmost 32 bits of the block (SP 800-38D §6.2).
fn inc32(block: &mut [u8; 16]) {
    let ctr = u32::from_be_bytes([block[12], block[13], block[14], block[15]]).wrapping_add(1);
    block[12..16].copy_from_slice(&ctr.to_be_bytes());
}

/// XOR src into dst in place.
fn xor_slice(dst: &mut [u8], src: &[u8]) {
    for (d, s) in dst.iter_mut().zip(src.iter()) {
        *d ^= s;
    }
}

#[derive(Debug)]
pub struct GcmError(pub &'static str);

impl std::fmt::Display for GcmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// GHASH over AAD and ciphertext with the length block appended.
fn ghash_aad_ct(h: &[u8; 16], aad: &[u8], ct: &[u8]) -> [u8; 16] {
    let mut s = [0u8; 16];
    let mut absorb = |s: &mut [u8; 16], data: &[u8]| {
        for block in data.chunks(16) {
            let mut b = [0u8; 16];
            b[..block.len()].copy_from_slice(block);
            xor_slice(s, &b);
            *s = ghash_mul(s, h);
        }
    };
    absorb(&mut s, aad);
    absorb(&mut s, ct);
    let mut len_block = [0u8; 16];
    len_block[..8].copy_from_slice(&((aad.len() as u64) * 8).to_be_bytes());
    len_block[8..].copy_from_slice(&((ct.len() as u64) * 8).to_be_bytes());
    xor_slice(&mut s, &len_block);
    ghash_mul(&s, h)
}

/// Encrypt plaintext; returns ciphertext || tag (16-byte tag appended).
pub fn seal(key: &[u8; 32], nonce: &[u8; 12], aad: &[u8], plaintext: &[u8]) -> Vec<u8> {
    let aes = Aes256::new(key);

    // H = E(K, 0^128); J0 = IV || 0^31 || 1 for a 96-bit IV.
    let mut h = [0u8; 16];
    aes.encrypt_block(&mut h);
    let mut j0 = [0u8; 16];
    j0[..12].copy_from_slice(nonce);
    j0[15] = 1;
    let mut ek_j0 = j0;
    aes.encrypt_block(&mut ek_j0);

    // CTR keystream from inc32(J0): ct = pt ^ E(K, counter).
    let mut ct = vec![0u8; plaintext.len()];
    let mut counter = j0;
    for (chunk_start, pt_chunk) in plaintext.chunks(16).enumerate() {
        inc32(&mut counter);
        let mut stream = counter;
        aes.encrypt_block(&mut stream);
        xor_slice(&mut stream, pt_chunk);
        ct[chunk_start * 16..chunk_start * 16 + pt_chunk.len()]
            .copy_from_slice(&stream[..pt_chunk.len()]);
    }

    // Tag = GHASH_H(AAD, CT, lens) ^ E(K, J0).
    let mut tag = ghash_aad_ct(&h, aad, &ct);
    xor_slice(&mut tag, &ek_j0);

    let mut out = ct;
    out.extend_from_slice(&tag);
    out
}

/// Open ciphertext || tag; returns plaintext or auth failure. Empty
/// plaintext is legal (AAD-only).
pub fn open(
    key: &[u8; 32],
    nonce: &[u8; 12],
    aad: &[u8],
    ct_and_tag: &[u8],
) -> Result<Vec<u8>, GcmError> {
    if ct_and_tag.len() < 16 {
        return Err(GcmError("ciphertext shorter than tag"));
    }
    let (ct, tag) = ct_and_tag.split_at(ct_and_tag.len() - 16);
    let aes = Aes256::new(key);

    let mut h = [0u8; 16];
    aes.encrypt_block(&mut h);
    let mut j0 = [0u8; 16];
    j0[..12].copy_from_slice(nonce);
    j0[15] = 1;
    let mut ek_j0 = j0;
    aes.encrypt_block(&mut ek_j0);

    // Verify FIRST: tag = GHASH_H(aad, received ct, lens) ^ E(K, J0).
    // GHASH runs over the ciphertext bytes as received — never re-encrypt
    // first (that would hash the plaintext instead).
    let mut expected_tag = ghash_aad_ct(&h, aad, ct);
    xor_slice(&mut expected_tag, &ek_j0);
    if !ct_eq(tag, &expected_tag) {
        return Err(GcmError("authentication failed"));
    }

    // Decrypt: pt = ct ^ CTR keystream (same keystream as encryption).
    let mut pt = vec![0u8; ct.len()];
    let mut counter = j0;
    for (chunk_start, ct_chunk) in ct.chunks(16).enumerate() {
        inc32(&mut counter);
        let mut stream = counter;
        aes.encrypt_block(&mut stream);
        xor_slice(&mut stream, ct_chunk);
        pt[chunk_start * 16..chunk_start * 16 + ct_chunk.len()]
            .copy_from_slice(&stream[..ct_chunk.len()]);
    }
    Ok(pt)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k32(s: &str) -> [u8; 32] {
        let v = crate::util::hex_decode(s).unwrap();
        let mut k = [0u8; 32];
        k.copy_from_slice(&v);
        k
    }

    fn n12(s: &str) -> [u8; 12] {
        let v = crate::util::hex_decode(s).unwrap();
        let mut n = [0u8; 12];
        n.copy_from_slice(&v);
        n
    }

    /// Test Case 1 (McGrew-Viega GCM spec Appendix B): empty inputs.
    #[test]
    fn mv_case1_empty() {
        let key = k32("0000000000000000000000000000000000000000000000000000000000000000");
        let nonce = n12("000000000000000000000000");
        let out = seal(&key, &nonce, b"", b"");
        assert_eq!(crate::util::hex_encode(&out), "530f8afbc74536b9a963b4f1c4cb738b");
    }

    /// Test Case 2: one zero block, empty AAD.
    #[test]
    fn mv_case2_single_block() {
        let key = k32("0000000000000000000000000000000000000000000000000000000000000000");
        let nonce = n12("000000000000000000000000");
        let pt = [0u8; 16];
        let out = seal(&key, &nonce, b"", &pt);
        assert_eq!(
            crate::util::hex_encode(&out),
            "cea7403d4d606b6e074ec5d3baf39d18d0d1c8a799996bf0265b98b5d48ab919"
        );
    }

    /// Test Case 3: 4-block PT + 20-byte AAD, matching key/IV.
    #[test]
    fn mv_case3_multi_block() {
        let key = k32("feffe9928665731c6d6a8f9467308308feffe9928665731c6d6a8f9467308308");
        let nonce = n12("cafebabefacedbaddecaf888");
        let pt = crate::util::hex_decode(concat!(
            "d9313225f88406e5a55909c5aff5269a86a7a9531534f7da2e4c303d8a318a72",
            "1c3c0c95956809532fcf0e2449a6b525b16aedf5aa0de657ba637b391aafd255",
        ))
        .unwrap();
        let aad = crate::util::hex_decode("feedfacedeadbeeffeedfacedeadbeefabaddad2").unwrap();
        let out = seal(&key, &nonce, &aad, &pt);
        assert_eq!(
            crate::util::hex_encode(&out),
            "522dc1f099567d07f47f37a32a84427d643a8cdcbfe5c0c97598a2bd2555d1aa"
                .to_owned()
                + "8cb08e48590dbb3da7b08b1056828838c5f61e6393ba7a0abcc9f662898015a"
                + "d2df7cd675b4f09163b41ebf980a7f638"
        );
    }

    /// AAD-only sealing roundtrips; wrong AAD fails auth (covers the
    /// CAVP AAD-only shape without hardcoding an unverifiable constant).
    #[test]
    fn aad_only_auth() {
        let key = k32("0000000000000000000000000000000000000000000000000000000000000000");
        let nonce = n12("000000000000000000000000");
        let aad = [0u8; 16];
        let out = seal(&key, &nonce, &aad, b"");
        assert_eq!(out.len(), 16);
        assert!(open(&key, &nonce, &aad, &out).is_ok());
        assert!(open(&key, &nonce, b"", &out).is_err());
    }

    /// Roundtrip: non-trivial key/nonce, 100-byte plaintext with AAD.
    #[test]
    fn seal_open_roundtrip() {
        let key = k32("ffeeddccbbaa99887766554433221100ffeeddccbbaa99887766554433221100");
        let nonce = n12("00112233445566778899aabb");
        let aad = b"session header v1";
        let pt: Vec<u8> = (0..100u16).map(|i| (i * 7 % 251) as u8).collect();
        let sealed = seal(&key, &nonce, aad, &pt);
        let opened = open(&key, &nonce, aad, &sealed).unwrap();
        assert_eq!(opened, pt);
        assert_eq!(sealed.len(), pt.len() + 16);

        // Bit flips break auth (tamper detection in ct and in tag).
        let mut tampered = sealed.clone();
        tampered[0] ^= 1;
        assert!(open(&key, &nonce, aad, &tampered).is_err());
        let mut tampered_tag = sealed.clone();
        let last = tampered_tag.len() - 1;
        tampered_tag[last] ^= 1;
        assert!(open(&key, &nonce, aad, &tampered_tag).is_err());
    }

    /// Truncated input errors cleanly, never panics.
    #[test]
    fn open_rejects_short_input() {
        let key = k32("0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20");
        let nonce = n12("090909090909090909090909");
        assert!(open(&key, &nonce, b"", &[0u8; 8]).is_err());
    }

    /// GHASH multiplication sanity: H*0 = 0, 1*H = H (GCM bit order).
    #[test]
    fn ghash_mul_identity() {
        let h = [0x12u8; 16];
        let zero = [0u8; 16];
        let one = {
            let mut b = [0u8; 16];
            b[0] = 0x80;
            b
        };
        assert_eq!(ghash_mul(&zero, &h), zero);
        assert_eq!(ghash_mul(&one, &h), h);
    }
}
