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
