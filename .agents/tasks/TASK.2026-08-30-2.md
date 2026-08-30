# TASK 2026-08-30 (2) — encrypted agent-to-agent session channels (v0.6.0)

Goal: dedicated private agent chats; hub stores ciphertext only.

- Operator green light after pqc skill finalization. ####
- Sealing lane: user chose in-tree FIPS 203. ####
- Worktree feat/agent-sessions from main. ####
- In-tree crypto: Keccak/FIPS 202. ####
- AES-256/FIPS 197, GCM/SP 800-38D. ####
- ML-KEM-768/FIPS 203 + NTT tables. ####
- Debugged: rate fn, pad byte, zetas, gamma. ####
- Debugged: SampleNTT has no post-NTT. ####
- Debugged: encaps ss = K, no KDF. ####
- ACVP KATs byte-exact: keygen/encaps/decaps. ####
- Cross-validated: pyca + kyber-py. ####
- Hub: sessions store + identity registry. ####
- API: create/join/seal/seals/send/recv. ####
- Bridge: 6 MCP session tools (14 total). ####
- Identity: identity.json 0600 auto-gen. ####
- e2e: two-agent flow, ciphertext-only hub. ####
- Live: prod hub 0.6.0, smoke test green. ####
- Tamper: GCM auth fails closed. ####
- Docs: skill/README/AGENTS/DOX updated. ####

####

## Deliverables

### 1. Crypto modules (all std-only, zero crates)
- `keccak.rs` — FIPS 202 SHA3-256/512, SHAKE128/256; FIPS 202 KATs.
- `aes.rs` — FIPS 197 AES-256; C.3 + SP 800-38A vectors.
- `gcm.rs` — SP 800-38D; McGrew-Viega appendix B vectors; ct-time tag cmp.
- `mlkem768.rs` + `ntt_tables.rs` — FIPS 203; Kyber NTT (Montgomery zetas,
  k=1 ascending; basemul zetas[64±i]; poly_tomont f=2^32%q; invntt f=1441).
  SampleNTT = raw rejection samples (NO post-NTT). Encaps returns K = G's
  first half (no KDF). ACVP `tests/vectors/*.json` byte-exact.

### 2. Sessions layer
- `sessions.rs` (hub): members, sealed key pkgs, seq ring, `sessions.json`
  0600; fail-closed caps (64 sessions / 16 members / 200 msgs each).
- `api.rs`: `/api/v1/identity`, `/api/v1/devices`, `/api/v1/sessions{,/{id}
  /{join,seal,seals,send,recv}}`; member-gated writes; dashboard-or-device
  reads; identity auto-registered on join.
- `identity.rs` (bridge): `$WTF_HOME/identity.json` 0600, auto-gen,
  corrupt-fails-closed, purge = rotate.
- `session_crypto.rs` (bridge): ML-KEM-768 encaps of session key per member
  (GCM wrap under SHA3-256(shared)); per-(session, sender) subkeys; AAD =
  (domain, session, sender, seq); nonce = SHA3-256(subkey‖seq) truncated.
- `mcp.rs`: session_create/list/join/seal/send/read (14 tools total);
  local key cache `session_keys.json` 0600.

### 3. Verification
- 82 unit + 7 e2e green (release build, lto, panic=abort).
- e2e: two bridges create/join/seal/send/read through real hub; hub disk +
  API verified ciphertext-only; tampered ct fails GCM auth.
- Live production hub (port 7800): upgraded 0.5.0 → 0.6.0, identity
  register + session create + encrypted send + recv + local decrypt +
  tamper-fails-closed, all via signed-curl fallback as mac-agent.
- Interop: pyca/cryptography (OpenSSL) and kyber-py both encapsulate
  against our ek; our decaps recovers both secrets; our encaps output is
  byte-identical to kyber-py's for the same (ek, m).

### 4. Security notes
- Hub cannot read sessions: key material exists only sealed (ML-KEM-768 to
  member eks) + in member `$WTF_HOME` (0600).
- Revoked device loses hub access (instant revocation) but NOT past
  ciphertext (stored ciphertext stays encrypted; forward secrecy is NOT a
  property of this design — recreate sessions after member revocation).
- No secrets in repo/logs/tasks; session keys and identity keys stay in
  0600 files under `$WTF_HOME`.

---

## Wrap-up (2026-08-30, later session)

- Merge executed: feat/agent-sessions → main (7555c6e); operator's pqc skill
  updates + COMMS protocol merged (ae63929); conflicts resolved preserving
  operator structure + sessions bullet.
- Worktrees/branches cleaned: only main; remote task branches deleted;
  TASK.2026-08-30.md restored from the orphaned hub-upgrade branch.
- PQC integration: wtf-agent-hub distributed via ainish-coder --rules PQC
  block; PQC shortcut (`issue wtf`) restored in skill §2; pqc-secrets §5.9a
  documents the app-level key-sealing blueprint from this work (ACVP +
  pyca + kyber-py validated).
- Docs: src/llms.txt module coverage complete (client/hmac/rand/util/lib
  added; mlkem768 invariants documented); root llms.txt gained a
  Distributed Skills index; legacy wtf-observability skill carries a
  status pointer to wtf-agent-hub; handoff docs corrected in place.
- English scan: 0 CJK/cyrillic/mojibake hits in wtf repo docs+source.
- Final sweep: 14 MCP tools + serverInfo name (15 string hits = false
  positive); long-hex literals confined to #[cfg(test)] KAT vectors.

## Session close-out (handoff to next chat)

- Session work complete; `main` = 320386a, pushed, clean tree, no worktrees.
- Production hub: wtf 0.6.0 live on port 7800 (supervised `wtf-hub`,
  restart=on-failure), smoke-tested through session flow.
- Interop proof chain: ACVP KATs byte-exact + pyca/OpenSSL + kyber-py.
- Next-session pointers:
  - Open design: PQC/password enrollment gate for `key issue` (converge
    before touching auth.rs/api.rs per operating contract).
  - Candidate follow-ups: dashboard sessions panel, `session_read` long
    polling via SSE, channel key rotation without losing history,
    `windows-agent` first handshake on the production hub (still pending).
  - ainish-coder side: `--rules` deploys all three skills; pqc-secrets
    §5.9a key-sealing pattern added; vault identity-source matrix is
    operator-maintained — re-sync when vault v1.2.x realities change.
