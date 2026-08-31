# TASK — v0.7.0 encrypted COMMS ledger channels (2026-08-31)

Machine 2 (windows). Operator directive: expand the bins/sessions concept
into a structured agent-to-agent COMMS surface modeled on the
`AGENTS/{date}.COMMS.md` protocol — fast coordination across repos,
worktrees, subagents, subtasks, and machines, without the user relaying.
Secrets only over encrypted channels (at rest + transit); agents hold the
only keys to what they may read. Document in AGENTS.md + llms.txt.

## Approach

Bridge-side protocol layer over the existing v0.6.0 session transport —
zero hub-side changes (`api.rs`/`auth.rs` untouched; cross-machine
convergence contract respected). COMMS entries are versioned JSON
envelopes (`wtf-comms-v1`: event, scope, note, ts) carried as the
plaintext of ordinary encrypted session messages; the session layer's
ML-KEM-768 sealed keys, per-(session, sender) subkeys, and
(session, sender, seq) AAD binding apply unchanged.

## Shipped

- `src/comms.rs` (new): envelope build/parse/render, fail-closed
  validation, event vocabulary = ledger lifecycle (checkin, update,
  intent-merge, checkout, blocked, announce, handoff) + unit tests.
- `src/mcp.rs`: `comms_post` / `comms_read` tools (14 → 16), reuse the
  session send/recv path; unreadable and non-envelope messages degrade
  gracefully.
- Docs: skill §7 (new COMMS protocol section; Troubleshooting → §8),
  AGENTS.md WTF-HUB block (channels + secrets-encrypted-only mandate),
  root `llms.txt` (v0.7.0, 16 tools), `src/llms.txt` (comms.rs + mcp.rs
  + session bullets), README (tools table, sessions section, storage).
- Version 0.6.0 → 0.7.0.

## Gates

- [x] cargo test: 86 unit + 8 e2e green (new `comms_channels_end_to_end`:
      join/seal handshake, both members post + read, event filter,
      after-pagination, invalid event fails closed, non-member fails
      closed, plain messages render raw, hub wire state + sessions.json
      carry no envelope plaintext).
- [x] cargo build --release clean.
- [x] Secret grep of diff: clean (public NIST KAT vectors excluded).

## Followups / notes

- Hub unchanged → machine 1 only needs to rebuild BRIDGES (mac-agent) to
  gain the comms tools; the running hub serves them as-is.
- Session seq race (two simultaneous senders on one channel) is a
  pre-existing session-layer property; comms_post surfaces it with a
  re-send hint. Candidate followup: client-side retry.
- Machine 2 still blocked on enrollment inputs (machine-1 hub URL +
  windows-1 device key) — see handoff addendum.

---

# TASK — v0.8.0 one-time enroll tokens (2026-08-31, machine 2)

Operator directive: implement the autonomous-enrollment proposal (lane A
from `HANDOFF.2026-08-29-machine2.md`). A joining device should not need
ssh access to the hub or a hand-copied device key. The converge-before-code
gate on `api.rs` was explicitly overridden by the operator for this lane;
`auth.rs` untouched. Sheet notes to machine-1 follow on both handoff
sheets.

## Shipped

- `src/config.rs`: `EnrollTokenStore` (`enroll_tokens.json`, 0600) —
  records carry name + SHA-256(token) + expiry + `used`; `issue` mints a
  64-hex kernel-CSPRNG token, supersedes prior same-name records, ttl
  1..=86400 s (default 600); `consume` burns on success only (a typo
  must not brick the token); atomic persist, `TokenError::Store` rolls
  the in-memory burn back.
- `src/api.rs`: `POST /api/v1/enroll` — global sliding-window limiter
  (20 attempts / 5 min), uniform 403 for unknown/expired/used/wrong-
  length/non-hex tokens (400 only for unparseable JSON), 500 only on
  store failures, `KeyStore::issue` on success (hot reload), enroll
  event logged, response shape identical to `key issue --json`.
- `src/main.rs`: `wtf enroll-token <name> [--ttl SECS] [--json]` (+
  `revoke` subcommand; token printed once, only the hash stored) and
  `wtf enroll --url URL --name N --token T` (redeem → `run_setup`;
  `--url` wins as the stored hub address; non-200 → generic error +
  fresh-token hint). Help updated.
- Docs: skill §2 (token enrollment path), AGENTS.md WTF-HUB setup,
  root + `src/llms.txt`, README (onboarding path, security model, API
  table, storage, CLI, test counts). Cargo.toml 0.7.0 → 0.8.0.

## Gates

- [x] cargo test: 88 unit + 9 e2e green (new `enroll_token_flow_end_to_end`:
      mint `--json` advertises the real hub URL; wrong/ghost/truncated
      tokens all 403; redeem 200 in key-issue shape; reuse 403; redeemed
      key checks in immediately). Two test-side bugs fixed during
      bring-up: a raw-string `\"` produced invalid JSON (hub rightly 400'd)
      and the truncation assert had lost its `[..32]` slice. Manual curl
      reproduction (403/200/403) proved the hub code correct first.
- [x] cargo build --release clean (wtf 0.8.0).
- [x] Secret grep of diff: clean.

## Followups / notes

- Hub-side route → machine 1 must pull, rebuild, AND restart the hub
  (bridges alone are not enough this time).
- Deviations from the pushed proposal, flagged for convergence: burn on
  success only (not on any outcome); global rate limiter (not per-name);
  bare 64-hex token (no `wtf-enroll-v1:` prefix).
- Proposal lanes B (PQC-wrapped request/approve) and C (ssh join) remain
  open; B composes on top of A.

## v0.9.0 — signed-handshake enrollment (PSK bootstrap, PQC-sealed delivery)

**Status:** code-complete, all gates green (2026-08-31).

### What shipped

- **Site enroll secret (hub):** `HubConfig.enroll_secret` — 256-bit hex,
  auto-generated on first serve (0600), backfilled into pre-0.9 configs on
  load; single `save_at` write path shared with advertised-url; minted fresh
  by `rotate_enroll_secret[_at]`.
- **Signed PSK handshake (route):** `POST /api/v1/enroll` now dispatches on
  presence of `token` (v0.8.0, unchanged) or `proof` (v0.9.0). PSK mode:
  shape checks (name / 2368-hex ek / 16..=128-hex nonce / 64-hex proof /
  ±300 s skew) → constant-time HMAC compare against
  `HMAC-SHA256(enroll_secret, "wtf-enroll-v2\n{name}\n{ek}\n{ts}\n{nonce}")`
  → nonce replay cache (`Hub.enroll_nonces`, 600 s prune, filled only after a
  valid proof) → `KeyStore::issue` → key sealed via
  `session_crypto::seal_session_key` (ML-KEM-768 + AES-256-GCM, context
  `wtf-enroll-v2:{name}`) → response `{hub_url, device, ek_fp, sealed}` — the
  secret never crosses the wire, the key never crosses in plaintext. All
  failures share one uniform 403; global limiter (20/5 min) covers both modes.
- **CLI:** `wtf enroll --psk S` (mutually exclusive with `--token`; loads the
  bridge identity, posts the transcript + proof, opens the sealed package with
  `open_sealed_package`, `run_setup`s the unwrapped key) and `wtf enroll-secret
  [--rotate] [--json]` (hub machine; rotate prints an invalidation notice).
- **Tests:** unit +3 (`enroll_secret_generated_and_rotates`,
  `enroll_secret_backfills_older_configs`, `enroll_nonce_cache_rejects_replay`);
  e2e +1 (`psk_handshake_end_to_end`: real-CLI enroll → bridge.json + agent
  check-in; wire carries `sealed`/`ek_fp` and never plaintext `key`; wrong
  secret / stale ts / tampered ek / replayed nonce all uniform 403; rotate
  kills the old copy and the fresh secret enrolls).

### Gates

- [x] cargo test: 91 unit + 10 e2e, all green.
- [x] cargo build --release clean (wtf 0.9.0).
- [x] Secret grep of diff vs 989c8f4: clean.

### Notes / deviations

- `auth.rs` untouched. api.rs convergence contract: hub-side route grew a
  dispatcher + two tails (`enroll_token` / `enroll_psk`); flagged on both
  handoff sheets per operator override.
- PQC posture: delivery is PQC (FIPS 203 / 197 / SP 800-38D); the proof is
  HMAC-SHA256 — the repo's standard-transport lane, same as request auth.
  In-tree ML-DSA-65 handshake signing is the documented future upgrade.
- Debugging note: a stray quote after a raw-string closer (invalid Rust that
  some file tools rendered as valid) cost a bisect to find; fixed byte-exactly
  via python3. File tools on this box can mis-render — verify via shell/od/git.
