//! Keccak-f[1600] permutation and FIPS 202 SHA-3 / SHAKE.
//!
//! In-tree on `std` only (zero-dependency constraint). The permutation is
//! the load-bearing core under SHA3-256/512, SHAKE128/256, AES-GCM's GHASH
//! is separate, and ML-KEM-768 (FIPS 203 uses SHA3/SHAKE extensively).
//! Validated against FIPS 202 known-answer test vectors below.

/// Lane count for Keccak-f[1600]: 25 lanes of 64 bits.
const ROUNDS: usize = 24;

/// Rotation offsets r[x][y] for the rho step, indexed [x][y]
/// (lane (x, y) at bit position x + 5y).
const RHO: [[u32; 5]; 5] = [
    [0, 36, 3, 41, 18],
    [1, 44, 10, 45, 2],
    [62, 6, 43, 15, 61],
    [28, 55, 25, 21, 56],
    [27, 20, 39, 8, 14],
];

/// Round constants RC[i] for iota.
const RC: [u64; ROUNDS] = [
    0x0000000000000001, 0x0000000000008082, 0x800000000000808a, 0x8000000080008000,
    0x000000000000808b, 0x0000000080000001, 0x8000000080008081, 0x8000000000008009,
    0x000000000000008a, 0x0000000000000088, 0x0000000080008009, 0x000000008000000a,
    0x000000008000808b, 0x800000000000008b, 0x8000000000008089, 0x8000000000008003,
    0x8000000000008002, 0x8000000000000080, 0x000000000000800a, 0x800000008000000a,
    0x8000000080008081, 0x8000000000008080, 0x0000000080000001, 0x8000000080008008,
];

/// One Keccak-f[1600] permutation of the 25-lane state (in place).
pub fn keccak_f1600(state: &mut [u64; 25]) {
    for rc in RC.iter().take(ROUNDS) {
        theta(state);
        rho_pi(state);
        chi(state);
        state[0] ^= rc;
    }
}

/// theta: column parities mixed across rows.
fn theta(state: &mut [u64; 25]) {
    let mut c = [0u64; 5];
    for x in 0..5 {
        c[x] = state[x] ^ state[x + 5] ^ state[x + 10] ^ state[x + 15] ^ state[x + 20];
    }
    for x in 0..5 {
        let d = c[(x + 4) % 5] ^ c[(x + 1) % 5].rotate_left(1);
        for y in 0..5 {
            state[x + 5 * y] ^= d;
        }
    }
}

/// rho + pi folded: rotate each lane, then place it at its pi position.
fn rho_pi(state: &mut [u64; 25]) {
    let mut next = [0u64; 25];
    for x in 0..5 {
        for y in 0..5 {
            next[y + 5 * ((2 * x + 3 * y) % 5)] = state[x + 5 * y].rotate_left(RHO[x][y]);
        }
    }
    *state = next;
}

/// chi: nonlinear row step.
fn chi(state: &mut [u64; 25]) {
    for y in 0..5 {
        let row = [
            state[5 * y],
            state[5 * y + 1],
            state[5 * y + 2],
            state[5 * y + 3],
            state[5 * y + 4],
        ];
        for x in 0..5 {
            state[5 * y + x] = row[x] ^ (!row[(x + 1) % 5] & row[(x + 2) % 5]);
        }
    }
}

// ---------- sponge ----------

/// Rate in bytes (FIPS 202): SHA3-224:144, SHA3-256:136, SHA3-384:104,
/// SHA3-512:72, SHAKE128:168, SHAKE256:136.
const fn rate_bytes(strength: u32) -> usize {
    match strength {
        128 => 168, // SHAKE128: rate 1344 bits
        224 => 144,
        256 => 136, // SHA3-256 / SHAKE256: rate 1088 bits
        384 => 104,
        512 => 72,  // SHA3-512: rate 576 bits
        _ => unreachable!(),
    }
}

/// Sponge state shared by all SHA3/SHAKE modes.
pub struct Keccak {
    state: [u64; 25],
    rate: usize,      // bytes absorbed per permutation
    pad: u8,          // domain suffix: 0x06 SHA3, 0x1f SHAKE
    pos: usize,       // bytes absorbed into the current block
    squeeze: bool,    // true once in squeezing phase
}

impl Keccak {
    fn new(rate: usize, pad: u8) -> Keccak {
        Keccak { state: [0u64; 25], rate, pad, pos: 0, squeeze: false }
    }

    /// SHA3-256 (FIPS 202): 1088-bit rate, 0x06 domain suffix.
    pub fn sha3_256() -> Keccak {
        Keccak::new(rate_bytes(256), 0x06)
    }

    /// SHA3-512 (FIPS 202): 576-bit rate, 0x06 domain suffix.
    pub fn sha3_512() -> Keccak {
        Keccak::new(rate_bytes(512), 0x06)
    }

    /// SHAKE128 (FIPS 202): 1344-bit rate, 0x1f domain suffix.
    pub fn shake128() -> Keccak {
        Keccak::new(rate_bytes(128), 0x1f)
    }

    /// SHAKE256 (FIPS 202): 512-bit rate, 0x1f domain suffix.
    pub fn shake256() -> Keccak {
        Keccak::new(rate_bytes(256), 0x1f)
    }

    pub fn update(&mut self, mut data: &[u8]) {
        debug_assert!(!self.squeeze, "update after finalize");
        while !data.is_empty() {
            let take = (self.rate - self.pos).min(data.len());
            for (i, b) in data[..take].iter().enumerate() {
                let idx = self.pos + i;
                self.state[idx / 8] ^= (*b as u64) << (8 * (idx % 8));
            }
            self.pos += take;
            data = &data[take..];
            if self.pos == self.rate {
                keccak_f1600(&mut self.state);
                self.pos = 0;
            }
        }
    }

    fn finalize_absorb(&mut self) {
        // Domain/pad byte at position pos; final pad bit at rate-1.
        let idx = self.pos;
        self.state[idx / 8] ^= (self.pad as u64) << (8 * (idx % 8));
        self.state[(self.rate - 1) / 8] ^= 0x80u64 << (8 * ((self.rate - 1) % 8));
        keccak_f1600(&mut self.state);
        self.pos = 0;
        self.squeeze = true;
    }

    /// Squeeze `out.len()` bytes (SHAKE) or exactly once for fixed-size (SHA3).
    pub fn squeeze_into(&mut self, out: &mut [u8]) {
        if !self.squeeze {
            self.finalize_absorb();
        }
        let mut done = 0;
        while done < out.len() {
            if self.pos == self.rate {
                keccak_f1600(&mut self.state);
                self.pos = 0;
            }
            let take = (self.rate - self.pos).min(out.len() - done);
            for i in 0..take {
                let idx = self.pos + i;
                out[done + i] = (self.state[idx / 8] >> (8 * (idx % 8))) as u8;
            }
            self.pos += take;
            done += take;
        }
    }
}

/// One-shot SHA3-256.
pub fn sha3_256(data: &[u8]) -> [u8; 32] {
    let mut k = Keccak::sha3_256();
    k.update(data);
    let mut out = [0u8; 32];
    k.squeeze_into(&mut out);
    out
}

/// One-shot SHA3-512.
pub fn sha3_512(data: &[u8]) -> [u8; 64] {
    let mut k = Keccak::sha3_512();
    k.update(data);
    let mut out = [0u8; 64];
    k.squeeze_into(&mut out);
    out
}

/// One-shot SHAKE256 with arbitrary output length.
pub fn shake256(data: &[u8], out_len: usize) -> Vec<u8> {
    let mut k = Keccak::shake256();
    k.update(data);
    let mut out = vec![0u8; out_len];
    k.squeeze_into(&mut out);
    out
}

/// One-shot SHAKE128 with arbitrary output length.
pub fn shake128(data: &[u8], out_len: usize) -> Vec<u8> {
    let mut k = Keccak::shake128();
    k.update(data);
    let mut out = vec![0u8; out_len];
    k.squeeze_into(&mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::hex_encode;

    /// FIPS 202 Appendix A / NIST "SHA3-256 msg of len 0" KAT.
    #[test]
    fn fips202_sha3_256_empty() {
        assert_eq!(
            hex_encode(&sha3_256(b"")),
            "a7ffc6f8bf1ed76651c14756a061d662f580ff4de43b49fa82d80a4b80f8434a"
        );
    }

    /// NIST KAT: SHA3-256 of the two-byte message 0x1e 0xc7 (verified
    /// against an independent implementation).
    #[test]
    fn fips202_sha3_256_kat2() {
        let msg = [0x1eu8, 0xc7];
        assert_eq!(
            hex_encode(&sha3_256(&msg)),
            "634166abd89a336a7b98a36b7f14258ce1083e4b7327765dd38cac1bb64378ed"
        );
    }

    /// FIPS 202 Appendix A: SHA3-256("abc").
    #[test]
    fn fips202_sha3_256_abc() {
        assert_eq!(
            hex_encode(&sha3_256(b"abc")),
            "3a985da74fe225b2045c172d6bd390bd855f086e3e9d525b46bfe24511431532"
        );
    }

    /// FIPS 202 Appendix A: SHA3-512("abc").
    #[test]
    fn fips202_sha3_512_abc() {
        assert_eq!(
            hex_encode(&sha3_512(b"abc")),
            "b751850b1a57168a5693cd924b6b096e08f621827444f70d884f5d0240d2712e10e116e9192af3c91a7ec57647e3934057340b4cf408d5a56592f8274eec53f0"
        );
    }

    /// FIPS 202 Appendix A: SHAKE128("abc", 32).
    #[test]
    fn fips202_shake128_abc() {
        assert_eq!(
            hex_encode(&shake128(b"abc", 32)),
            "5881092dd818bf5cf8a3ddb793fbcba74097d5c526a6d35f97b83351940f2cc8"
        );
    }

    /// SHAKE256("abc", 32) — verified against an independent
    /// implementation.
    #[test]
    fn fips202_shake256_abc() {
        assert_eq!(
            hex_encode(&shake256(b"abc", 32)),
            "483366601360a8771c6863080cc4114d8db44530f8f1e1ee4f94ea37e78b5739"
        );
    }

    /// Absorb across block boundaries must match one-shot for long input
    /// (rate 136 for SHA3-256: 135/136/137-byte spans checked).
    #[test]
    fn incremental_matches_oneshot() {
        let data: Vec<u8> = (0u8..=255).cycle().take(1000).collect();
        for n in [135usize, 136, 137, 271, 272, 273, 999, 1000] {
            let mut k = Keccak::sha3_256();
            k.update(&data[..n / 2]);
            k.update(&data[n / 2..n]);
            let mut out = [0u8; 32];
            k.squeeze_into(&mut out);
            assert_eq!(out, sha3_256(&data[..n]), "n={n}");
        }
    }

    /// SHAKE output length independence: longer prefix must match short.
    #[test]
    fn shake_length_prefix_consistency() {
        let short = shake256(b"consistency", 32);
        let long = shake256(b"consistency", 96);
        assert_eq!(&short[..], &long[..32]);
    }
}
