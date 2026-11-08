//! Cryptographic randomness sourced from the kernel CSPRNG.
//!
//! Unix: reads blocking-after-seeded entropy from `/dev/urandom`.
//! Windows: `BCryptGenRandom` with the system-preferred RNG (no extra crate —
//! the import is bound directly, keeping the crate dependency-free).
//! Fails closed either way: if the kernel CSPRNG cannot be read, every caller
//! panics rather than silently degrading to weak randomness.

#[cfg(unix)]
use std::fs::File;
#[cfg(unix)]
use std::io::Read;

#[cfg(unix)]
fn read_random(buf: &mut [u8]) {
    let mut f = File::open("/dev/urandom").expect("fatal: cannot open /dev/urandom");
    f.read_exact(buf)
        .expect("fatal: short read from /dev/urandom");
}

#[cfg(windows)]
fn read_random(buf: &mut [u8]) {
    // BCryptGenRandom(hAlgorithm = NULL, pbBuffer, cbBuffer, dwFlags)
    // BCRYPT_USE_SYSTEM_PREFERRED_RNG asks for the system RNG, which needs no
    // algorithm handle and is safe for use from any thread.
    const BCRYPT_USE_SYSTEM_PREFERRED_RNG: u32 = 0x0000_0002;
    #[link(name = "bcrypt")]
    extern "system" {
        fn BCryptGenRandom(
            h_algorithm: *mut core::ffi::c_void,
            pb_buffer: *mut u8,
            cb_buffer: u32,
            dw_flags: u32,
        ) -> i32;
    }
    if buf.is_empty() {
        return;
    }
    let status = unsafe {
        BCryptGenRandom(
            core::ptr::null_mut(),
            buf.as_mut_ptr(),
            buf.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    if status != 0 {
        panic!("fatal: BCryptGenRandom failed (NTSTATUS {status:#010x})");
    }
}

#[cfg(not(any(unix, windows)))]
fn read_random(_buf: &mut [u8]) {
    panic!("fatal: no kernel CSPRNG binding for this target");
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
