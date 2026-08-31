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
