# ROADMAP — refinement items and open defects

Verified: 2026-09-03 (items updated for v0.15.0+ executor and R1 persistence; R3-R5 synced with root llms.txt)

## Refinement phase (opened by mac-agent, session wrap 2026-09-01)

- **R1 — identity-registry persistence [RESOLVED 2026-09-03].** Hub persists
  identities to `identities.json` (0600) on registration and session join,
  and rehydrates on load from persistence + session members.
- **R2 — durable COMMS ledger.** windows-1 owes a committed
  `AGENTS/2026-09-01.COMMS.md` entry covering: v0.14.1/0.14.2 fixes,
  portproxy + firewall, NDJSON fix, shim hardening, docs split.
- **R3 — singular dashboard URL & federated multi-machine shell [RESOLVED 2026-09-03].** Singular `/w/<capability>` endpoint across loopback, LAN, and remote; embedded Chat & Agent Orchestration Studio paired with the virtual `~/` multi-machine shell (`fed_shell.rs`).
- **R4 — federated OMP config, architecture LKGL & distributed compute [RESOLVED 2026-09-03].** Synchronized `fed_omp_config.json`, per-machine LKGL mapping (`lkgl.json`), compute-tier tagging, cross-machine routing.
- **R5 — legacy `?k=` dashboard-key retirement [RESOLVED 2026-09-04].** Retired `dash_ok` in `api.rs`, removed non-loopback 401 hint page for uniform-404 security, deleted the third `wtf dashboard-url` link, and enforced singular capability routing (`/w/<capability>`).

## Open defects

- **Router NDJSON newline [RESOLVED].** Verified upstream in local-router
  `src/index.ts` `createOllamaStreamTransform`: proper newline streaming active.
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
