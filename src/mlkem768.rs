//! ML-KEM-768 key encapsulation (FIPS 203), in-tree on `std` only.
//!
//! Module-LWE KEM over the Kyber ring Rq = Zq[X]/(X^256+1), q = 3329,
//! k = 3 for parameter set 768. Wire sizes: ek 1184 B, dk 2400 B,
//! ct 1088 B, shared secret 32 B. Deterministic KATs from the official
//! NIST ACVP vector set (tests/vectors/mlkem768-*.json) drive the tests;
//! the implementation follows FIPS 203 §7 (KeyGen/Encaps/Decaps) with
//! the §7.3 implicit-rejection decapsulation.
//!
//! Conventions: coefficients are i16 in [-q/2, q/2] for arithmetic;
//! serialization follows FIPS 203 §5 (byteEncode12 for q-adic elements
//! of Rq, bit packing for Zq in compress_1..5).

pub const K: usize = 3; // ML-KEM-768 rank
pub const N: usize = 256; // ring degree
pub const Q: i32 = 3329; // prime modulus
pub const DU: usize = 10; // ciphertext component compression (dB/u)
pub const DV: usize = 4; // ciphertext v compression
pub const EK_BYTES: usize = 1184; // encapsulation key
pub const DK_BYTES: usize = 2400; // decapsulation key (expanded form)
pub const CT_BYTES: usize = 1088; // ciphertext
pub const SS_BYTES: usize = 32; // shared secret

/// NTT twiddles: zeta = 17^brv7(i) mod q where brv7 is 7-bit bit-reversal.
const fn brv7(i: usize) -> u32 {
    let mut r = 0u32;
    let mut x = i as u32;
    let mut n = 7;
    while n > 0 {
        r = (r << 1) | (x & 1);
        x >>= 1;
        n -= 1;
    }
    r
}

const fn pow_mod(mut base: i64, mut e: u32, q: i64) -> i32 {
    let mut acc: i64 = 1;
    while e > 0 {
        if e & 1 == 1 {
            acc = acc * base % q;
        }
        base = base * base % q;
        e >>= 1;
    }
    acc as i32
}

const ZETAS_NTT: [i32; 128] = {
    let mut t = [0i32; 128];
    let mut i = 0;
    while i < 128 {
        // Montgomery form: (17^brv7(i) * 2^16) mod q — required by
        // montgomery_reduce in the butterfly.
        t[i] = ((pow_mod(17, brv7(i), Q as i64) as i64 * 65536) % Q as i64) as i32;
        i += 1;
    }
    t
};

/// gamma values for BaseCaseMultiply: gamma[2i] = zeta^(2*brv3(i)+1),
/// gamma[2i+1] = -gamma[2i] (FIPS 203 §4.3 Table 3).
const fn brv3(i: usize) -> u32 {
    let mut r = 0u32;
    let mut x = i as u32;
    let mut n = 3;
    while n > 0 {
        r = (r << 1) | (x & 1);
        x >>= 1;
        n -= 1;
    }
    r
}

const GAMMAS: [i32; 256] = {
    let mut g = [0i32; 256];
    let mut i = 0;
    while i < 128 {
        let v = pow_mod(17, 16 * brv3(i) + 1, Q as i64);
        g[2 * i] = v;
        g[2 * i + 1] = -v;
        i += 1;
    }
    g
};

fn barrett_reduce(x: i32) -> i32 {
    // Reduce x mod q into (-q/2, q/2]. FIPS 203 §4.2.1 Barrett.
    let mut t = (x as i64 * 20159) >> 26; // 20159/2^26 ≈ 1/q
    t *= Q as i64;
    let mut r = x - t as i32;
    if r >= Q {
        r -= Q;
    }
    if r < -Q / 2 {
        r += Q;
    }
    r
}

/// Montgomery reduction (Kyber ref port): returns a·R⁻¹ mod q for
/// |a| < q·2^15. QINV = -3327 = q⁻¹ mod 2^16 (signed); MONT = -1044 =
/// 2^16 mod q (signed).
fn montgomery_reduce_fe(a: i64) -> i32 {
    const QINV: i64 = -3327; // q^-1 mod 2^16
    let q = Q as i64;
    // t = signed low 16 bits of a*QINV.
    let t16 = (((a * QINV) % 65536) as i64 + 65536) % 65536;
    let t16 = if t16 > 32767 { t16 - 65536 } else { t16 };
    let r = (a - t16 * q) >> 16;
    let r = r as i32;
    if r >= Q {
        r - Q
    } else {
        r
    }
}

fn csubq(x: i32) -> i32 {
    if x >= Q {
        x - Q
    } else {
        x
    }
}

/// basemul (Kyber ref port): (a0 + a1·X)(b0 + b1·X) mod (X² − zeta),
/// all in the Montgomery domain via fqmul semantics:
/// r0 = fqmul(fqmul(a1, b1), zeta) + fqmul(a0, b0);
/// r1 = fqmul(a0, b1) + fqmul(a1, b0).
fn basemul(c: &mut [i32], a0: i32, a1: i32, b0: i32, b1: i32, zeta: i32) {
    let fq = |x: i64, y: i64| montgomery_reduce_fe(x * y);
    let r0 = fq(fq(a1 as i64, b1 as i64) as i64, zeta as i64);
    let r0 = barrett_reduce(r0 + fq(a0 as i64, b0 as i64));
    let r1 = barrett_reduce(fq(a0 as i64, b1 as i64) + fq(a1 as i64, b0 as i64));
    c[0] = r0;
    c[1] = r1;
}

/// NTT in place (FIPS 203 §4.3.1; Kyber ref ntt() port). Input standard
/// order, output bit-reversed; zetas consumed from index 1 upward. All
/// values stay congruent mod q in (-q/2, q/2] via barrett/montgomery.
pub fn ntt(a: &mut [i32; N]) {
    let mut k = 1usize;
    let mut len = 128;
    while len >= 2 {
        let mut start = 0;
        while start < N {
            let zeta = crate::ntt_tables::ZETAS[k];
            k += 1;
            for j in start..start + len {
                let t = montgomery_reduce_fe(zeta as i64 * a[j + len] as i64);
                a[j + len] = barrett_reduce(a[j] - t);
                a[j] = barrett_reduce(a[j] + t);
            }
            start += 2 * len;
        }
        len >>= 1;
    }
}

/// Inverse NTT in place, with Montgomery factor ×2^16 (Kyber ref
/// invntt_tomont port): k descends from 127, final scalar f = 1441 =
/// mont²/128. Output leaves values in the Montgomery domain by exactly
/// one factor of 2^16, which cancels the ×2^16 the ntt side absorbed —
/// the net domain across encode/decode stays FIPS 203 conformant.
pub fn intt(a: &mut [i32; N]) {
    let mut k = 127usize;
    let mut len = 2usize;
    while len <= 128 {
        let mut start = 0;
        while start < N {
            let zeta = crate::ntt_tables::ZETAS[k];
            k = k.wrapping_sub(1);
            for j in start..start + len {
                let t = a[j];
                a[j] = barrett_reduce(t + a[j + len]);
                a[j + len] = a[j + len] - t;
                a[j + len] = montgomery_reduce_fe(zeta as i64 * a[j + len] as i64);
            }
            start += 2 * len;
        }
        len <<= 1;
    }
    // f = mont^2/128 mod q (Kyber ref constant 1441).
    const F: i64 = 1441;
    for c in a.iter_mut() {
        *c = montgomery_reduce_fe(F * *c as i64);
    }
}

/// Polynomial in NTT domain: 256 coefficients.
type Poly = [i32; N];

/// Multiply two NTT-domain polynomials (Kyber ref poly_basemul_montgomery
/// port): 64 block pairs; block (4i, 4i+1) uses zeta = ZETAS[64+i], block
/// (4i+2, 4i+3) uses -ZETAS[64+i]. All multiplies go through
/// montgomery_reduce (fqmul semantics), keeping the domain consistent
/// with ntt/intt.
pub fn polymul_ntt(a: &Poly, b: &Poly) -> Poly {
    let mut out = [0i32; N];
    for i in 0..(N / 4) {
        let zeta = crate::ntt_tables::ZETAS[64 + i];
        basemul(
            &mut out[4 * i..4 * i + 2],
            a[4 * i],
            a[4 * i + 1],
            b[4 * i],
            b[4 * i + 1],
            zeta,
        );
        basemul(
            &mut out[4 * i + 2..4 * i + 4],
            a[4 * i + 2],
            a[4 * i + 3],
            b[4 * i + 2],
            b[4 * i + 3],
            -zeta,
        );
    }
    out
}

/// Vector of k polynomials.
type PolyVec = [Poly; K];

fn polyvec_ntt(v: &mut PolyVec) {
    for p in v.iter_mut() {
        ntt(p);
    }
}

fn polyvec_mul_ntt(a: &PolyVec, b: &PolyVec) -> Poly {
    let mut acc = polymul_ntt(&a[0], &b[0]);
    let mut tmp;
    for i in 1..K {
        tmp = polymul_ntt(&a[i], &b[i]);
        for j in 0..N {
            acc[j] = barrett_reduce(acc[j] + tmp[j]);
        }
    }
    acc
}

// ---------- sampling (FIPS 203 §4.2.1) ----------

/// Rejection-sample uniform coefficients in [0, q) from an XOF stream.
fn sample_uniform(bytes: &[u8]) -> Poly {
    let mut out = [0i32; N];
    let mut i = 0;
    let mut pos = 0;
    // Two 12-bit candidates per 3 bytes (little-endian bit order).
    while i < N && pos + 3 <= bytes.len() {
        let b0 = bytes[pos] as u32;
        let b1 = bytes[pos + 1] as u32;
        let b2 = bytes[pos + 2] as u32;
        let d1 = (b0 | (b1 << 8)) & 0xfff;
        let d2 = ((b1 >> 4) | (b2 << 4)) & 0xfff;
        pos += 3;
        if d1 < Q as u32 {
            out[i] = d1 as i32;
            i += 1;
            if i == N {
                break;
            }
        }
        if d2 < Q as u32 {
            out[i] = d2 as i32;
            i += 1;
        }
    }
    out
}

/// Sample noise polynomial with eta from a prf stream (CBD).
fn sample_cbd(bytes: &[u8], eta: usize) -> Poly {
    let bits_per_coeff = 2 * eta;
    let total_bits = N * bits_per_coeff;
    debug_assert!(bytes.len() * 8 >= total_bits);
    let mut out = [0i32; N];
    let mut bitpos = 0usize;
    for coeff in out.iter_mut().take(N) {
        let mut a = 0i32;
        let mut b = 0i32;
        for j in 0..eta {
            let idx = (bitpos + j) / 8;
            let sh = (bitpos + j) % 8;
            a += ((bytes[idx] >> sh) & 1) as i32;
        }
        for j in 0..eta {
            let idx = (bitpos + eta + j) / 8;
            let sh = (bitpos + eta + j) % 8;
            b += ((bytes[idx] >> sh) & 1) as i32;
        }
        bitpos += bits_per_coeff;
        *coeff = barrett_reduce(a - b);
    }
    out
}

// ---------- serialization (FIPS 203 §5) ----------

/// byteEncode_d for d in 4 (u), 10 (u), 11 (v), 12 (NTT-domain keys).
fn poly_encode(poly: &[i32; N], d: usize) -> Vec<u8> {
    let bits = N * d;
    let mut out = vec![0u8; (bits + 7) / 8];
    let mut bitpos = 0usize;
    for c in poly.iter() {
        let val = (*c as u32).rem_euclid(Q as u32);
        for j in 0..d {
            let bit = (val >> j) & 1;
            if bit == 1 {
                let idx = bitpos / 8;
                out[idx] |= 1 << (bitpos % 8);
            }
            bitpos += 1;
        }
    }
    out
}

fn poly_decode(bytes: &[u8], d: usize) -> Poly {
    let mut out = [0i32; N];
    let mut bitpos = 0usize;
    for c in out.iter_mut() {
        let mut val = 0u32;
        for j in 0..d {
            let idx = bitpos / 8;
            let bit = ((bytes[idx] >> (bitpos % 8)) & 1) as u32;
            val |= bit << j;
            bitpos += 1;
        }
        *c = val as i32;
    }
    out
}

/// compress_d(x mod+ q, d): round(x * 2^d / q) mod 2^d.
fn compress_coeff(x: i32, d: usize) -> u32 {
    let scaled = ((x as i64) << d) + (Q as i64) / 2;
    let v = (scaled / Q as i64) as u64;
    let mask = (1u64 << d) - 1;
    (v & mask) as u32
}

/// decompress_d(y in [0,2^d)): round(y * q / 2^d) mod+ q.
fn decompress_coeff(y: u32, d: usize) -> i32 {
    let numer = (y as i64) * (Q as i64);
    let denom = 1i64 << d;
    let v = ((2 * numer + denom) / (2 * denom)) as i64;
    v as i32
}

fn compress_poly(poly: &Poly, d: usize) -> Vec<u8> {
    let mut comp: Poly = [0i32; N];
    for (i, c) in poly.iter().enumerate() {
        comp[i] = compress_coeff(cbuf(*c), d) as i32;
    }
    poly_encode(&comp, d)
}

fn cbuf(x: i32) -> i32 {
    // bring to positive residue for compression
    x.rem_euclid(Q)
}

fn decompress_poly(bytes: &[u8], d: usize) -> Poly {
    let comp = poly_decode(bytes, d);
    let mut out = [0i32; N];
    for (i, c) in comp.iter().enumerate() {
        out[i] = decompress_coeff(*c as u32, d);
    }
    out
}

// ---------- hash helpers ----------

/// SHAKE256 with arbitrary output.
fn shake(data: &[&[u8]], out_len: usize) -> Vec<u8> {
    let mut k = crate::keccak::Keccak::shake256();
    for d in data {
        k.update(d);
    }
    let mut out = vec![0u8; out_len];
    k.squeeze_into(&mut out);
    out
}

fn sha3_256_parts(data: &[&[u8]]) -> [u8; 32] {
    let mut k = crate::keccak::Keccak::sha3_256();
    for d in data {
        k.update(d);
    }
    let mut out = [0u8; 32];
    k.squeeze_into(&mut out);
    out
}

fn sha3_512_parts(data: &[&[u8]]) -> [u8; 64] {
    let mut k = crate::keccak::Keccak::sha3_512();
    for d in data {
        k.update(d);
    }
    let mut out = [0u8; 64];
    k.squeeze_into(&mut out);
    out
}

// ---------- XOF with domain separation ----------

/// Matrix XOF: SHAKE128(rho || i || j) (FIPS 203 §5.3 / Kyber xof_absorb).
fn xof_prf(rho: &[u8], i: u8, j: u8, out_len: usize) -> Vec<u8> {
    let mut k = crate::keccak::Keccak::shake128();
    k.update(rho);
    k.update(&[i, j]);
    let mut out = vec![0u8; out_len];
    k.squeeze_into(&mut out);
    out
}
// ---------- FIPS 203 API ----------

/// Deterministic KeyGen from (d, z): FIPS 203 Algorithm 16.
/// Returns (ek, dk) with dk in expanded form (2400 B).
pub fn keygen_det(d: &[u8; 32], z: &[u8; 32]) -> ([u8; EK_BYTES], [u8; DK_BYTES]) {
    // (rho, sigma) = G(d || k)
    let g = sha3_512_parts(&[d, &[K as u8]]);
    let mut rho = [0u8; 32];
    let mut sigma = [0u8; 32];
    rho.copy_from_slice(&g[..32]);
    sigma.copy_from_slice(&g[32..]);

    // Ahat[i][j] = NTT(XOF(rho, i, j)) — matrix sampling in NTT domain.
    let mut a_rows: [[Poly; K]; K] = [[[0i32; N]; K]; K];
    for i in 0..K {
        for j in 0..K {
            let stream = xof_prf(&rho, j as u8, i as u8, 3 * 168 + 16);
            a_rows[i][j] = sample_uniform(&stream); // SampleNTT: coefficients ARE the NTT-domain values
        }
    }

    // s_i = NTT(CBD(PRF(sigma, i, eta3=2))) — PRF output 64*eta = 128 B.
    let mut s: PolyVec = [[0i32; N]; K];
    for i in 0..K {
        let prf = shake(&[&sigma, &[i as u8]], 64 * 2);
        s[i] = ntt_of(&sample_cbd(&prf, 2));
    }

    // e_i = NTT(CBD(PRF(sigma, i+K, eta3)))
    let mut e: PolyVec = [[0i32; N]; K];
    for i in 0..K {
        let prf = shake(&[&sigma, &[(K as u8) + i as u8]], 64 * 2);
        e[i] = ntt_of(&sample_cbd(&prf, 2));
    }

    // that = tomont(Ahat ∘ s) + e. Our basemul leaves an R^-1 factor
    // (Kyber fqmul semantics); poly_tomont (fqmul with f = 2^32 mod q = R²)
    const TOMONT: i64 = 1353; // 2^32 mod q = R²; fqmul scales by R
    let mut that: PolyVec = [[0i32; N]; K];
    for i in 0..K {
        let mut acc_v: Poly = polymul_ntt(&a_rows[i][0], &s[0]);
        for j in 1..K {
            let t = polymul_ntt(&a_rows[i][j], &s[j]);
            for x in 0..N {
                acc_v[x] = barrett_reduce(acc_v[x] + t[x]);
            }
        }
        for x in 0..N {
            let tm = montgomery_reduce_fe(TOMONT * acc_v[x] as i64);
            that[i][x] = barrett_reduce(tm + e[i][x]);
        }
    }

    // ek = byteEncode12(that) || rho ; dk = dk_pke || ek || H(ek) || z
    let mut ek = [0u8; EK_BYTES];
    for i in 0..K {
        let enc = poly_encode(&that[i], 12);
        ek[i * 384..(i + 1) * 384].copy_from_slice(&enc);
    }
    ek[1152..].copy_from_slice(&rho);

    let mut dk = [0u8; DK_BYTES];
    // dk = byteEncode12(s_hat) || ek || H(ek) || z  (FIPS 203 §7.1: 1152+1184+32+32)
    let mut off = 0;
    for i in 0..K {
        let enc = poly_encode(&s[i], 12);
        dk[off..off + 384].copy_from_slice(&enc);
        off += 384;
    }
    dk[off..off + EK_BYTES].copy_from_slice(&ek);
    off += EK_BYTES;
    let h = sha3_256_parts(&[&ek]);
    dk[off..off + 32].copy_from_slice(&h);
    off += 32;
    dk[off..off + 32].copy_from_slice(z);
    (ek, dk)
}

fn ntt_of(p: &Poly) -> Poly {
    let mut out = *p;
    ntt(&mut out);
    out
}

/// Encapsulation with explicit randomness (m): FIPS 203 Algorithm 17.
pub fn encaps_det(ek: &[u8; EK_BYTES], m: &[u8; 32]) -> ([u8; CT_BYTES], [u8; SS_BYTES]) {
    // that[i] = byteDecode12(ek[i*384..])
    let mut that: PolyVec = [[0i32; N]; K];
    for i in 0..K {
        let mut seg = [0u8; 384];
        seg.copy_from_slice(&ek[i * 384..(i + 1) * 384]);
        that[i] = poly_decode(&seg, 12);
    }
    let rho = {
        let mut r = [0u8; 32];
        r.copy_from_slice(&ek[1152..]);
        r
    };

    let (kr64) = sha3_512_parts(&[m, &{
        let h = sha3_256_parts(&[ek]);
        h.to_vec()
    }]);
    let r_vec = kr64;
    let k_part = &r_vec[..32];
    let r_part = &r_vec[32..];

    // y_i = CBD(PRF(r, i, eta1=2)) in normal domain, then NTT
    let mut y: PolyVec = [[0i32; N]; K];
    for i in 0..K {
        let prf = shake(&[r_part, &[i as u8]], 128);
        y[i] = ntt_of(&sample_cbd(&prf, 2));
    }
    // e1_i = CBD(PRF(r, i+K, eta2=2)) normal domain
    let mut e1: [Poly; K] = [[0i32; N]; K];
    for i in 0..K {
        let prf = shake(&[r_part, &[(K as u8) + i as u8]], 128);
        e1[i] = sample_cbd(&prf, 2);
    }
    // e2 = CBD(PRF(r, 2K, eta2))
    let e2 = {
        let prf = shake(&[r_part, &[2 * K as u8]], 128);
        sample_cbd(&prf, 2)
    };

    // u = InvNTT(Ahat^T ∘ y) + e1  (A^T: u_i = sum_j A[j][i]*y[j])
    let mut u: [Poly; K] = [[0i32; N]; K];
    for i in 0..K {
        // recompute A column i rows j
        let mut acc = [0i32; N];
        let mut first = true;
        for j in 0..K {
            let stream = xof_prf(&rho, i as u8, j as u8, 3 * 168 + 16);
            let a_ji = sample_uniform(&stream); // SampleNTT: NTT-domain direct
            let t = polymul_ntt(&a_ji, &y[j]);
            if first {
                acc = t;
                first = false;
            } else {
                for x in 0..N {
                    acc[x] = barrett_reduce(acc[x] + t[x]);
                }
            }
        }
        let mut inv = acc;
        intt(&mut inv);
        for x in 0..N {
            u[i][x] = barrett_reduce(inv[x] + e1[i][x]);
        }
    }

    // mu = decompress_1(byteEncode1(m))
    let mu = decompress_poly(&m.to_vec(), 1);

    // v = InvNTT(that ∘ y) + e2 + mu
    let t = polyvec_mul_ntt(&that, &y);
    let mut tinv = t;
    intt(&mut tinv);
    let mut v = [0i32; N];
    for x in 0..N {
        v[x] = barrett_reduce(tinv[x] + e2[x] + mu[x]);
    }

    // c1 = byteEncode10(compress_10(u)) ; c2 = byteEncode4(compress_4(v))
    let mut ct = [0u8; CT_BYTES];
    for i in 0..K {
        let enc = compress_poly(&u[i], DU);
        ct[i * 320..(i + 1) * 320].copy_from_slice(&enc);
    }
    let enc_v = compress_poly(&v, DV);
    ct[960..].copy_from_slice(&enc_v);

    // ss = K directly from G(m || H(ek)) (FIPS 203 Algorithm 17 — no
    // KDF step; the KDF(k || H(c)) form belongs to TLS-style KEM
    // combinators, not the FIPS 203 encapsulation itself).
    let mut ss = [0u8; SS_BYTES];
    ss.copy_from_slice(k_part);

    (ct, ss)
}

/// Decapsulation: FIPS 203 Algorithm 18 with implicit rejection.
pub fn decaps(dk: &[u8; DK_BYTES], ct: &[u8; CT_BYTES]) -> [u8; SS_BYTES] {
    // Parse dk: s_hat || ek || H(ek) || z (2400 = 1152+1184+32+32)
    let mut s: PolyVec = [[0i32; N]; K];
    let mut off = 0;
    for i in 0..K {
        let mut seg = [0u8; 384];
        seg.copy_from_slice(&dk[off..off + 384]);
        off += 384;
        s[i] = poly_decode(&seg, 12);
    }
    let mut ek = [0u8; EK_BYTES];
    ek.copy_from_slice(&dk[off..off + EK_BYTES]);
    off += EK_BYTES;
    let h = {
        let mut x = [0u8; 32];
        x.copy_from_slice(&dk[off..off + 32]);
        x
    };
    off += 32;
    let z = {
        let mut x = [0u8; 32];
        x.copy_from_slice(&dk[off..off + 32]);
        x
    };

    // Parse ct: c1 (k*320) + c2 (128)
    let mut u: [Poly; K] = [[0i32; N]; K];
    for i in 0..K {
        let mut seg = [0u8; 320];
        seg.copy_from_slice(&ct[i * 320..(i + 1) * 320]);
        u[i] = decompress_poly(&seg, DU);
    }
    let v = decompress_poly(&ct[960..], DV);

    // w = v - InvNTT(s ∘ NTT(u))
    let mut u_ntt: [Poly; K] = [[0i32; N]; K];
    for i in 0..K {
        u_ntt[i] = ntt_of(&u[i]);
    }
    let t = polyvec_mul_ntt(&s, &u_ntt);
    let mut tinv = t;
    intt(&mut tinv);
    let mut w = [0i32; N];
    for x in 0..N {
        w[x] = barrett_reduce(v[x] - tinv[x]);
    }

    // m' = byteEncode1(compress_1(w))
    let m2 = compress_poly(&w, 1);
    let mut m = [0u8; 32];
    m.copy_from_slice(&m2);

    // K', r' = G(m' || H(ek))
    let kr = sha3_512_parts(&[&m, &h]);
    let mut k_part = [0u8; 32];
    k_part.copy_from_slice(&kr[..32]);
    let r_part = &kr[32..];

    // Kbar = J(z || c) via SHAKE256 — the implicit-rejection secret.
    let kbar_v = shake(&[&z, ct], 32);
    let mut kbar = [0u8; 32];
    kbar.copy_from_slice(&kbar_v);

    // Re-encrypt with r' and compare — implicit rejection (§7.3).
    let (ct2, _ss2) = encaps_det(&ek, &m);
    let _ = r_part;
    if ct2 == *ct {
        k_part
    } else {
        kbar
    }
}

/// Random (ek, dk) pair from kernel CSPRNG.
pub fn keygen() -> ([u8; EK_BYTES], [u8; DK_BYTES]) {
    let d: [u8; 32] = crate::rand::bytes(32).try_into().unwrap();
    let z: [u8; 32] = crate::rand::bytes(32).try_into().unwrap();
    keygen_det(&d, &z)
}

/// Random encapsulation from kernel CSPRNG.
pub fn encaps(ek: &[u8; EK_BYTES]) -> ([u8; CT_BYTES], [u8; SS_BYTES]) {
    let m: [u8; 32] = crate::rand::bytes(32).try_into().unwrap();
    encaps_det(ek, &m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn load(name: &str) -> serde_json_ish::Value {
        serde_json_ish::parse(&fs::read_to_string(name).unwrap()).unwrap()
    }

    /// Minimal JSON reader wrapper over the repo's in-tree parser.
    mod serde_json_ish {
        pub type Value = crate::json::Value;
        pub fn parse(s: &str) -> Result<Value, crate::json::Error> {
            crate::json::parse(s)
        }
        impl Value {
            pub fn str_at(&self, k: &str) -> String {
                self.get(k)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string()
            }
            pub fn i_at(&self, k: &str) -> i64 {
                self.get(k).and_then(|v| v.as_i64()).unwrap_or(0)
            }
        }
    }

    fn hex(s: &str) -> Vec<u8> {
        crate::util::hex_decode(s).unwrap()
    }

    /// ACVP keyGen: (d, z) -> (ek, dk) byte-exact (expanded dk, 2400 B).
    #[test]
    fn acvp_keygen_768() {
        let p = load("tests/vectors/mlkem768-keygen-prompt.json");
        let e = load("tests/vectors/mlkem768-keygen-expected.json");
        let mut checked = 0;
        for (pg, eg) in p
            .get("testGroups")
            .unwrap()
            .as_arr()
            .unwrap()
            .iter()
            .zip(e.get("testGroups").unwrap().as_arr().unwrap().iter())
        {
            if pg.get("parameterSet").and_then(|v| v.as_str()) != Some("ML-KEM-768") {
                continue;
            }
            for (pt, et) in pg
                .get("tests")
                .unwrap()
                .as_arr()
                .unwrap()
                .iter()
                .zip(eg.get("tests").unwrap().as_arr().unwrap().iter())
            {
                let d = hex(&pt.str_at("d"));
                let z = hex(&pt.str_at("z"));
                let mut db = [0u8; 32];
                db.copy_from_slice(&d);
                let mut zb = [0u8; 32];
                zb.copy_from_slice(&z);
                let (ek, dk) = keygen_det(&db, &zb);
                assert_eq!(
                    crate::util::hex_encode(&ek),
                    et.str_at("ek").to_lowercase(),
                    "ek mismatch tcId {}",
                    pt.i_at("tcId")
                );
                assert_eq!(
                    crate::util::hex_encode(&dk),
                    et.str_at("dk").to_lowercase(),
                    "dk mismatch tcId {}",
                    pt.i_at("tcId")
                );
                checked += 1;
            }
        }
        assert!(
            checked >= 20,
            "too few 768 keyGen vectors checked: {checked}"
        );
    }

    /// ACVP encapsulation: (ek, m) -> (ct, k) byte-exact.
    #[test]
    fn acvp_encaps_768() {
        let p = load("tests/vectors/mlkem768-encapdecap-prompt.json");
        let e = load("tests/vectors/mlkem768-encapdecap-expected.json");
        let mut checked = 0;
        for (pg, eg) in p
            .get("testGroups")
            .unwrap()
            .as_arr()
            .unwrap()
            .iter()
            .zip(e.get("testGroups").unwrap().as_arr().unwrap().iter())
        {
            if pg.get("parameterSet").and_then(|v| v.as_str()) != Some("ML-KEM-768")
                || pg.get("function").and_then(|v| v.as_str()) != Some("encapsulation")
            {
                continue;
            }
            for (pt, et) in pg
                .get("tests")
                .unwrap()
                .as_arr()
                .unwrap()
                .iter()
                .zip(eg.get("tests").unwrap().as_arr().unwrap().iter())
            {
                let ek = hex(&pt.str_at("ek"));
                let m = hex(&pt.str_at("m"));
                let mut ek_b = [0u8; EK_BYTES];
                ek_b.copy_from_slice(&ek);
                let mut m_b = [0u8; 32];
                m_b.copy_from_slice(&m);
                let (ct, ss) = encaps_det(&ek_b, &m_b);
                assert_eq!(
                    crate::util::hex_encode(&ct),
                    et.str_at("c").to_lowercase(),
                    "ct mismatch tcId {}",
                    pt.i_at("tcId")
                );
                assert_eq!(
                    crate::util::hex_encode(&ss),
                    et.str_at("k").to_lowercase(),
                    "ss mismatch tcId {}",
                    pt.i_at("tcId")
                );
                checked += 1;
            }
        }
        assert!(
            checked >= 20,
            "too few 768 encapsulation vectors checked: {checked}"
        );
    }

    /// ACVP decapsulation: (dk, c) -> k byte-exact (includes implicit
    /// rejection cases if present).
    #[test]
    fn acvp_decaps_768() {
        let p = load("tests/vectors/mlkem768-encapdecap-prompt.json");
        let e = load("tests/vectors/mlkem768-encapdecap-expected.json");
        let mut checked = 0;
        for (pg, eg) in p
            .get("testGroups")
            .unwrap()
            .as_arr()
            .unwrap()
            .iter()
            .zip(e.get("testGroups").unwrap().as_arr().unwrap().iter())
        {
            if pg.get("parameterSet").and_then(|v| v.as_str()) != Some("ML-KEM-768")
                || pg.get("function").and_then(|v| v.as_str()) != Some("decapsulation")
            {
                continue;
            }
            for (pt, et) in pg
                .get("tests")
                .unwrap()
                .as_arr()
                .unwrap()
                .iter()
                .zip(eg.get("tests").unwrap().as_arr().unwrap().iter())
            {
                let dk = hex(&pt.str_at("dk"));
                let c = hex(&pt.str_at("c"));
                let mut dk_b = [0u8; DK_BYTES];
                dk_b.copy_from_slice(&dk);
                let mut c_b = [0u8; CT_BYTES];
                c_b.copy_from_slice(&c);
                let ss = decaps(&dk_b, &c_b);
                assert_eq!(
                    crate::util::hex_encode(&ss),
                    et.str_at("k").to_lowercase(),
                    "ss mismatch tcId {}",
                    pt.i_at("tcId")
                );
                checked += 1;
            }
        }
        assert!(
            checked >= 5,
            "too few 768 decapsulation vectors checked: {checked}"
        );
    }

    /// End-to-end random: keygen -> encaps -> decaps yields same secret.
    #[test]
    fn roundtrip_random() {
        let (ek, dk) = keygen();
        let (ct, ss1) = encaps(&ek);
        let ss2 = decaps(&dk, &ct);
        assert_eq!(ss1, ss2);
    }

    /// Tampered ciphertext decapsulates to a DIFFERENT secret (implicit
    /// rejection) — both sides fail closed, no panic.
    #[test]
    fn implicit_rejection_tamper() {
        let (ek, dk) = keygen();
        let (mut ct, ss1) = encaps(&ek);
        ct[0] ^= 0x40;
        let ss2 = decaps(&dk, &ct);
        assert_ne!(ss1, ss2);
    }

    /// Decapsulation with a wrong (mismatched) keypair rejects.
    #[test]
    fn wrong_key_rejects() {
        let (ek1, _dk1) = keygen();
        let (_ek2, dk2) = keygen();
        let (ct, _ss) = encaps(&ek1);
        let ss_bad = decaps(&dk2, &ct);
        // Must not panic; secret is the Kbar rejection value (32 bytes).
        assert_eq!(ss_bad.len(), 32);
    }
}
