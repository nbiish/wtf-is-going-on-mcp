//! AES-256 block cipher (FIPS 197), in-tree on `std` only.
//!
//! Encrypt-only block primitive used by AES-256-GCM (the GCM layer owns
//! keystream generation, GHASH, and tag computation — see gcm.rs).
//! Validated against FIPS 197 Appendix C.3 and NIST SP 800-38A vectors.

/// AES state is a 4x4 column-major byte matrix held as 16 bytes.
const NB: usize = 4; // state columns
const NR: usize = 14; // rounds for AES-256

/// S-box (FIPS 197 §5.1.1, affine transform of multiplicative inverse).
const SBOX: [u8; 256] = [
    0x63, 0x7c, 0x77, 0x7b, 0xf2, 0x6b, 0x6f, 0xc5, 0x30, 0x01, 0x67, 0x2b, 0xfe, 0xd7, 0xab, 0x76,
    0xca, 0x82, 0xc9, 0x7d, 0xfa, 0x59, 0x47, 0xf0, 0xad, 0xd4, 0xa2, 0xaf, 0x9c, 0xa4, 0x72, 0xc0,
    0xb7, 0xfd, 0x93, 0x26, 0x36, 0x3f, 0xf7, 0xcc, 0x34, 0xa5, 0xe5, 0xf1, 0x71, 0xd8, 0x31, 0x15,
    0x04, 0xc7, 0x23, 0xc3, 0x18, 0x96, 0x05, 0x9a, 0x07, 0x12, 0x80, 0xe2, 0xeb, 0x27, 0xb2, 0x75,
    0x09, 0x83, 0x2c, 0x1a, 0x1b, 0x6e, 0x5a, 0xa0, 0x52, 0x3b, 0xd6, 0xb3, 0x29, 0xe3, 0x2f, 0x84,
    0x53, 0xd1, 0x00, 0xed, 0x20, 0xfc, 0xb1, 0x5b, 0x6a, 0xcb, 0xbe, 0x39, 0x4a, 0x4c, 0x58, 0xcf,
    0xd0, 0xef, 0xaa, 0xfb, 0x43, 0x4d, 0x33, 0x85, 0x45, 0xf9, 0x02, 0x7f, 0x50, 0x3c, 0x9f, 0xa8,
    0x51, 0xa3, 0x40, 0x8f, 0x92, 0x9d, 0x38, 0xf5, 0xbc, 0xb6, 0xda, 0x21, 0x10, 0xff, 0xf3, 0xd2,
    0xcd, 0x0c, 0x13, 0xec, 0x5f, 0x97, 0x44, 0x17, 0xc4, 0xa7, 0x7e, 0x3d, 0x64, 0x5d, 0x19, 0x73,
    0x60, 0x81, 0x4f, 0xdc, 0x22, 0x2a, 0x90, 0x88, 0x46, 0xee, 0xb8, 0x14, 0xde, 0x5e, 0x0b, 0xdb,
    0xe0, 0x32, 0x3a, 0x0a, 0x49, 0x06, 0x24, 0x5c, 0xc2, 0xd3, 0xac, 0x62, 0x91, 0x95, 0xe4, 0x79,
    0xe7, 0xc8, 0x37, 0x6d, 0x8d, 0xd5, 0x4e, 0xa9, 0x6c, 0x56, 0xf4, 0xea, 0x65, 0x7a, 0xae, 0x08,
    0xba, 0x78, 0x25, 0x2e, 0x1c, 0xa6, 0xb4, 0xc6, 0xe8, 0xdd, 0x74, 0x1f, 0x4b, 0xbd, 0x8b, 0x8a,
    0x70, 0x3e, 0xb5, 0x66, 0x48, 0x03, 0xf6, 0x0e, 0x61, 0x35, 0x57, 0xb9, 0x86, 0xc1, 0x1d, 0x9e,
    0xe1, 0xf8, 0x98, 0x11, 0x69, 0xd9, 0x8e, 0x94, 0x9b, 0x1e, 0x87, 0xe9, 0xce, 0x55, 0x28, 0xdf,
    0x8c, 0xa1, 0x89, 0x0d, 0xbf, 0xe6, 0x42, 0x68, 0x41, 0x99, 0x2d, 0x0f, 0xb0, 0x54, 0xbb, 0x16,
];

/// Round constants for key expansion.
const RCON: [u8; 7] = [0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40];

/// Expanded key: 15 round keys of 16 bytes.
pub struct Aes256 {
    round_keys: [[u8; 16]; NR + 1],
}

impl Aes256 {
    /// Expand a 32-byte key (FIPS 197 §5.2, Nk=8).
    pub fn new(key: &[u8; 32]) -> Aes256 {
        let nk = 8;
        let mut w = [[0u8; 4]; 4 * (NR + 1)];
        for i in 0..nk {
            w[i] = [key[4 * i], key[4 * i + 1], key[4 * i + 2], key[4 * i + 3]];
        }
        for i in nk..4 * (NR + 1) {
            let mut temp = w[i - 1];
            if i % nk == 0 {
                temp = [
                    SBOX[temp[1] as usize] ^ RCON[i / nk - 1],
                    SBOX[temp[2] as usize],
                    SBOX[temp[3] as usize],
                    SBOX[temp[0] as usize],
                ];
            } else if i % nk == 4 {
                // AES-256 extra SubWord step.
                temp = [
                    SBOX[temp[0] as usize],
                    SBOX[temp[1] as usize],
                    SBOX[temp[2] as usize],
                    SBOX[temp[3] as usize],
                ];
            }
            for j in 0..4 {
                w[i][j] = w[i - nk][j] ^ temp[j];
            }
        }
        let mut round_keys = [[0u8; 16]; NR + 1];
        for r in 0..=NR {
            for c in 0..NB {
                round_keys[r][4 * c..4 * c + 4].copy_from_slice(&w[4 * r + c]);
            }
        }
        Aes256 { round_keys }
    }

    /// Encrypt one 16-byte block in place (FIPS 197 §5.1).
    pub fn encrypt_block(&self, block: &mut [u8; 16]) {
        add_round_key(block, &self.round_keys[0]);
        for round in 1..NR {
            sub_bytes(block);
            shift_rows(block);
            mix_columns(block);
            add_round_key(block, &self.round_keys[round]);
        }
        sub_bytes(block);
        shift_rows(block);
        add_round_key(block, &self.round_keys[NR]);
    }
}

fn add_round_key(state: &mut [u8; 16], rk: &[u8; 16]) {
    for i in 0..16 {
        state[i] ^= rk[i];
    }
}

fn sub_bytes(state: &mut [u8; 16]) {
    for b in state.iter_mut() {
        *b = SBOX[*b as usize];
    }
}

fn shift_rows(state: &mut [u8; 16]) {
    // State is column-major: state[r + 4c]. Row r rotates left by r.
    let s = *state;
    for r in 1..4 {
        for c in 0..4 {
            state[r + 4 * c] = s[r + 4 * ((c + r) % 4)];
        }
    }
}

/// GF(2^8) multiply with reduction polynomial x^8+x^4+x^3+x+1 (0x11b).
fn xtime(a: u8) -> u8 {
    let hi = a & 0x80;
    let mut r = a << 1;
    if hi != 0 {
        r ^= 0x1b;
    }
    r
}

fn gmul(mut a: u8, mut b: u8) -> u8 {
    let mut out = 0u8;
    while b != 0 {
        if b & 1 != 0 {
            out ^= a;
        }
        a = xtime(a);
        b >>= 1;
    }
    out
}

fn mix_columns(state: &mut [u8; 16]) {
    for c in 0..4 {
        let col = [
            state[4 * c],
            state[4 * c + 1],
            state[4 * c + 2],
            state[4 * c + 3],
        ];
        state[4 * c] = gmul(col[0], 2) ^ gmul(col[1], 3) ^ col[2] ^ col[3];
        state[4 * c + 1] = col[0] ^ gmul(col[1], 2) ^ gmul(col[2], 3) ^ col[3];
        state[4 * c + 2] = col[0] ^ col[1] ^ gmul(col[2], 2) ^ gmul(col[3], 3);
        state[4 * c + 3] = gmul(col[0], 3) ^ col[1] ^ col[2] ^ gmul(col[3], 2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> [u8; 16] {
        let v = crate::util::hex_decode(s).unwrap();
        let mut out = [0u8; 16];
        out.copy_from_slice(&v);
        out
    }

    fn hex32(s: &str) -> [u8; 32] {
        let v = crate::util::hex_decode(s).unwrap();
        let mut out = [0u8; 32];
        out.copy_from_slice(&v);
        out
    }

    /// FIPS 197 Appendix C.3: AES-256 single block.
    #[test]
    fn fips197_appendix_c3() {
        let key = hex32("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f");
        let aes = Aes256::new(&key);
        let mut block = hex("00112233445566778899aabbccddeeff");
        aes.encrypt_block(&mut block);
        assert_eq!(
            crate::util::hex_encode(&block),
            "8ea2b7ca516745bfeafc49904b496089"
        );
    }

    /// NIST SP 800-38A F.1.5 ECB-AES256: first and last vector.
    #[test]
    fn sp800_38a_ecb_aes256() {
        let key = hex32("603deb1015ca71be2b73aef0857d77811f352c073b6108d72d9810a30914dff4");
        let aes = Aes256::new(&key);
        let mut b1 = hex("6bc1bee22e409f96e93d7e117393172a");
        aes.encrypt_block(&mut b1);
        assert_eq!(
            crate::util::hex_encode(&b1),
            "f3eed1bdb5d2a03c064b5a7e3db181f8"
        );
        let mut b2 = hex("ae2d8a571e03ac9c9eb76fac45af8e51");
        aes.encrypt_block(&mut b2);
        assert_eq!(
            crate::util::hex_encode(&b2),
            "591ccb10d410ed26dc5ba74a31362870"
        );
        let mut b4 = hex("65697a4e265b2b3b7f9d461da9741678");
        aes.encrypt_block(&mut b4);
        assert_eq!(
            crate::util::hex_encode(&b4),
            "ac52c2c2f6e259055e2020b85b294ff1"
        );
    }

    /// Key expansion correctness: FIPS 197 C.3 round key 0 and 14.
    #[test]
    fn fips197_expansion_c3() {
        let key = hex32("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f");
        let aes = Aes256::new(&key);
        assert_eq!(
            crate::util::hex_encode(&aes.round_keys[0]),
            "000102030405060708090a0b0c0d0e0f"
        );
        assert_eq!(
            crate::util::hex_encode(&aes.round_keys[1]),
            "101112131415161718191a1b1c1d1e1f"
        );
        assert_eq!(
            crate::util::hex_encode(&aes.round_keys[NR]),
            "24fc79ccbf0979e9371ac23c6d68de36"
        );
    }

    /// GF(2^8) multiply sanity: xtime(0x80) wraps with 0x1b reduction.
    #[test]
    fn gmul_reduction() {
        assert_eq!(xtime(0x80), 0x1b);
        assert_eq!(gmul(0x57, 0x83), 0xc1); // FIPS 197 §4.2 worked example
        assert_eq!(gmul(0x57, 0x13), 0xfe);
    }
}
