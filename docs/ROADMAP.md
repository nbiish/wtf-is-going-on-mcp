# ROADMAP — refinement items and open defects

Verified: 2026-09-01 (items re-anchored after v0.15.1)

## Refinement phase (opened by mac-agent, session wrap 2026-09-01)

- **R1 — identity-registry persistence.** Hub restarts clear the in-memory
  identity registry; every chat member must re-`session_join` before
  auto-seals route again. Fix: persist to `identities.json` (0600) on
  register, or rehydrate from session members on load.
- **R2 — durable COMMS ledger.** windows-1 owes a committed
  `AGENTS/2026-09-01.COMMS.md` entry covering: v0.14.1/0.14.2 fixes,
  portproxy + firewall, NDJSON fix, shim hardening, docs split.
- **R3 — operator-directed refinement.** Reserved for the operator.

## Open defects

- **Router NDJSON newline** (local-router `src/index.ts`
  `createOllamaStreamTransform`): literal `'\\n'` pushed instead of `'\n'`
  — breaks real-ollama NDJSON streams. Fixed locally on windows (running
  binary); NOT yet merged upstream on mac.
- **`pqc-secrets pack` is replace-only** (py engine `cmd_pack`): runbook
  claims merge-safe; it is not. Bundles must be repacked wholesale.
- **Federation log echo** — v0.13.2 quieted heartbeats on windows side;
  mac verified clean. Re-open only if receipts re-appear in the ring.

## Candidate ideas (not committed)

- Per-connection bins keyed by device slug (string ids) instead of the
  u8 numeric trio.
- Dashboard: per-chat terminal tabs (multiple wtf-chat sessions in one
  viewer) and a cross-machine agent roster card (roster/activity-panel
  grammar from the dsh-agent-teams reference).
- `wtf-ask`/`wtf-ask-remote` promotion into the hub as first-class MCP
  tools (currently local shell helpers on windows).
