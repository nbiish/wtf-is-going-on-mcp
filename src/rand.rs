//! Cryptographic randomness sourced from the kernel CSPRNG.
//!
//! Reads blocking-after-seeded entropy via `/dev/urandom` (Linux). Fails
//! closed: if the kernel CSPRNG cannot be read, every caller panics rather
//! than silently degrading to weak randomness.

use std::fs::File;
use std::io::Read;

fn read_random(buf: &mut [u8]) {
    let mut f = File::open("/dev/urandom").expect("fatal: cannot open /dev/urandom");
    f.read_exact(buf)
        .expect("fatal: short read from /dev/urandom");
}

/// Fill `buf` with kernel CSPRNG bytes.
pub fn fill(buf: &mut [u8]) {
    read_random(buf);
}

/// `n` random bytes.
pub fn bytes(n: usize) -> Vec<u8> {
    let mut v = vec![0u8; n];
    fill(&mut v);
    v
}

/// `n` random bytes, hex-encoded (length 2n).
pub fn hex(n: usize) -> String {
    crate::util::hex_encode(&bytes(n))
}

/// Random u64.
pub fn u64() -> u64 {
    let mut b = [0u8; 8];
    fill(&mut b);
    u64::from_be_bytes(b)
}

/// A 256-bit key as 64 lowercase hex chars.
pub fn key_hex() -> String {
    hex(32)
}

/// A 128-bit nonce as 32 lowercase hex chars.
pub fn nonce_hex() -> String {
    hex(16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_and_uniqueness() {
        assert_eq!(hex(32).len(), 64);
        assert_eq!(bytes(0).len(), 0);
        let a = key_hex();
        let b = key_hex();
        assert_ne!(a, b);
        assert_ne!(u64(), u64());
    }
}
