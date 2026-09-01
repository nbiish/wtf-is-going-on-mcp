# TASK 2026-09-01 — SESSION WRAP-UP: bring-up + port + docs complete; repo → refinement phase

Closes the day's arc across `TASK.2026-08-31-2.md`, `TASK.2026-09-01.machine2-bringup.md`,
`TASK.2026-09-01.skills-v0.14.0.md`, and `TASK.2026-09-01.docs-current-state.md`.

## What this session accomplished (final state)

### 1. WTF MCP shipped v0.13.x → v0.14.2 (both fleet hubs live on the merged tree)
- **v0.13.0** orchestrator contract in MCP `initialize` + repo-chat discovery.
- **v0.13.1** `env_report`/`env_probe` cross-machine capability discovery.
- **v0.13.2/0.13.3** federation log-echo fixes (windows-agent; merged both ways).
- **v0.14.0** (mac-agent): dashboard **SESSIONS card** (`/api/v1/state` carries a
  `sessions` array — id/name/repo/members/msg_count; clicking a chat opens its
  viewer) + **per-chat executor** `chat_run`/`chat_sessions` — tasks dispatched to
  a repo chat execute in ONE persistent tmux session `wtf-chat-<slug>` through the
  **omp → hermes → fcc-claude fallback chain** (first installed + exit-0 wins,
  trace names the lane; E2E receipt CHAT-RUN-E2E-OK). 20 MCP tools total.
- **v0.14.1/0.14.2** (windows-agent): dashboard capability self-discovery from the
  `/w/<token>` path; push-receipt echo kill.
- All gates green throughout (102 lib + 13 e2e).

### 2. Singular model system — live on BOTH machines
Every agent harness (omp, hermes, fcc-claude, real `ollama` CLI) routes through
**local-router `local-router/fallback-models` :11434**. Receipts: OMP/HERMES/
FCC-ROUTER-OK (mac); OMP3/HERMES3/FCC3/OLLAMA3-OK (windows; NDJSON stream bug
fixed there en route). Rule: a lane failing is a router troubleshooting item.

### 3. Windows port COMPLETE (PQC envelope lane)
17 provider keys mac → windows inside an ML-KEM-768-sealed, ML-DSA-65-signed
envelope (recipient fingerprint verified pre-decapsulate). Cross-platform
`pqc-secrets envelope export|import` shipped in the local-router py engine
(a6154e0) to unblock the leg; signature layout reverse-verified against the Rust
engine (17/17 roundtrip). windows-1's engine build predates signature-verify —
next pull picks it up.

### 4. Federation — restored + verified bidirectional
Hub restarts had emptied the mac hub's peer table (windows→mac push survived;
mac→windows push was dead). Repaired: windows-1 delivered its site secret over
the E2E ops chat and installed a Windows portproxy (host `:7800` → WSL);
`wtf federate add windows` + hub restart; canary event verified crossing both
directions (670 mac-origin events on the windows hub).

### 5. Docs + skills current everywhere
- `llms.txt` (both repos): release lines, fleet state, harness integration,
  executor + singular-model operator preferences.
- `AGENTS.md` (wtf repo): v0.14.0 block, SINGULAR MODEL SYSTEM block,
  Federation LIVE + Machine-2 RESOLVED/UPDATE, NEXT FOCUS → refinement.
- Skills `wtf-agent-hub` + `wtf-observability` synced and mirrored
  byte-identical into **ainish-coder** and **local-router** repos + the binary
  embed (`wtf skill print` diff-verified).
- Durable COMMS ledgers + task files on both repos carry every unit.

## OPEN — REFINEMENT PHASE (next work, operator directs)
- **R1 — identity registry persistence**: the registry is in-memory only; every
  hub restart clears it and members must re-join before auto-seals flow again.
  Persist (0600 file under `$WTF_HOME`) or rehydrate from session members.
- **R2 — windows-1 durable COMMS ledger append** for its side of today
  (v0.14.1/0.14.2 fixes, portproxy, docs pulls) — pending its next bridge run.
- **R3 — general wtf-MCP refinement** per operator direction.

## Classification: Confidential. No secrets in this file.
