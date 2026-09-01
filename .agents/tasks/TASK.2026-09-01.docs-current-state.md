# TASK 2026-09-01 — addendum: docs-to-current-state + fleet merge (machine 1)

Extends `TASK.2026-09-01.machine2-bringup.md` and `TASK.2026-09-01.skills-v0.14.0.md`.

## Operator directive
Document everything accomplished in `llms.txt` (both repos) + COMMS + task files
so both repos read current; verify cross-machine chats/connections.

## Executed (mac side, this worktree's slice)
- `llms.txt`: release line → **v0.14.2** with the full version chain
  (0.13.0 orchestrator contract → 0.13.1 env discovery → 0.13.2/0.13.3
  log-echo fixes → 0.14.0 SESSIONS card + executor → 0.14.1/0.14.2 cap
  self-discovery + push-receipt echo kill) and BOTH-hubs-live note; new
  **Fleet state** bullet (repo chats, singular model system on both machines,
  skill mirrors in ainish-coder + local-router).
- `AGENTS.md`: Federation LIVE paragraph extended (second repo chat
  `local-router ops`, both hubs on merged tree); Machine-2 status — log-echo
  marked **RESOLVED** (their v0.13.2/0.13.3 fix), port-complete update block
  (17 keys via PQC envelope, 4 lanes green, next-pull note for the py
  envelope engine).
- Durable COMMS ledger entry for this unit.

## Cross-machine merge reality (recorded)
- windows-1 shipped `fix/dashboard-cap` (67aeb42) + version bumps
  (a44a399 → 0.14.1, 748fe52 → 0.14.2) while mac shipped
  `docs/skills-v0.14.0` (52321fe/616767d). Pulled + merged both ways:
  `b52d859` = the union; 102 lib + 13 e2e green; pushed. Both hubs now run
  the same merged tree — same-remote parity restored.

## Gates
- [x] Docs-only diff in this slice (llms.txt + AGENTS.md + ledger + task file).
- [x] No secrets in docs (hub names, session-id prefixes only).

## Classification: Confidential.
