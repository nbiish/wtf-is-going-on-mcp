#[test]
fn keygen_matches_pyca_from_seed() {
    let seed = std::fs::read("/tmp/xcheck_dkseed").unwrap();
    let ek_ref = std::fs::read("/tmp/xcheck_ek").unwrap();
    let mut d = [0u8; 32];
    d.copy_from_slice(&seed[..32]);
    let mut z = [0u8; 32];
    z.copy_from_slice(&seed[32..]);
    let (ek, _dk) = wtf::mlkem768::keygen_det(&d, &z);
    assert_eq!(ek.to_vec(), ek_ref, "keygen_det(d,z) must match final FIPS 203 reference (pyca)");
}
