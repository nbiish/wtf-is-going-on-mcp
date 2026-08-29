# TASK 2026-08-29 (7) — session log: bins release + cross-machine connect

Goal: record hub-bins session; coordinate with remote agent.

- Merged hub-bins-connect into main. ####
- b0086e8 pushed to origin. ####
- Worktree + branch cleaned up. ####
- Hub v0.3.0 live, port 7800. ####
- Healthz OK, binds all interfaces. ####
- Cross-machine test attempted hub-side. ####
- Pi mDNS resolve failed (NXDOMAIN). ####
- Direct-IP ssh cancelled by operator. ####
- Pi was wrong guess (ssh config). ####
- Operator: other machine is Windows. ####
- machine-2 issued, then revoked. ####
- windows-agent issued; active. ####
- Keystore is $WTF_HOME/keys.json. ####
- Credential: MCP env or PQC. ####
- Handoff JSON delivered to operator. ####
- Fetched remote: origin/main advanced. ####
- Remote tip c8ad094 (AGENT_HUB docs). ####
- Staged ../wtf-remote-review, unmerged. ####
- Awaiting operator go before pull. ####
- Next: confirm windows-agent handshake. ####

####

## Deliverables — session state record

### 1. Git / release state
- `feat/hub-bins-connect` (3 commits: 5c378f9 task, 3f6c6c4 bins feature, 219059a dox) merged into `main` as **b0086e8**, pushed to `origin/main`, then worktree `../hub-bins-connect` removed and branch deleted (cleanup verified: only main worktree, only main branch, clean tree).
- **Local `main` = b0086e8.** **`origin/main` = c8ad094** (remote agent pushed 2 commits: f8f06ce docs(agents) + merge c8ad094). Local main is 0 ahead / 2 behind — fast-forward integration, no conflicts expected.
- Active worktrees (this session's staging):
  - `../wtf-remote-review` → branch `chore/remote-agent-review` @ c8ad094 (tracks origin/main; **read/inspect only, not merged**)
  - `../wtf-session-docs` → branch `docs/session-log-cross-machine` @ b0086e8 (this file)
- Task-file namespace collision noted: remote agent used `TASK.2026-08-29-6.md`; this session uses `-7` to avoid conflict.

### 2. Remote agent's pushed work (c8ad094) — review notes
- `AGENTS.md`: new `<AGENT_HUB>` section — wtf hub reporting made mandatory for every agent; setup via skill → `wtf join`/`wtf setup` → MCP bridge (`args: ["agent"]`); reporting contract = chain-of-draft wire format; `check_in` (working/blocked/done), `log_event`, `wtf_is_going_on` before starting, `list_bins`/`read_bin` before planning; includes lift-ready plain-text system prompt for harnesses lacking the file; frontmatter description extended.
- Their task file records gates (test, build, scan) and a "hub rebuild 0.3.0, restart" step — **not verifiable from this machine**; production hub here shows continuous uptime since 17:43 local (no restart observed). Integration should not assume hub state changed.

### 3. Hub production state (this Mac)
- Process: `~/.local/bin/wtf serve`, v0.3.0, port 7800, binds `*:7800` (verified via lsof), `WTF_HOME=$HOME/.config/wtf-mcp` (0700).
- Files: `config.json` (hub settings), `keys.json` (device keystore), `events.jsonl` (empty — no authenticated events yet), `serve.log` (dashboard URL line).
- Devices: `machine-2` (revoked — superseded), `windows-agent` (active). Secret material lives only in `keys.json` (0600) and the one-time issue output; never in repo, logs, or task files.

### 4. Cross-machine connectivity (what was verified)
- Hub reachable on LAN path: healthz OK via LAN address (v0.3.0).
- Inbound auth enforced: unauthenticated `/api/v1/state` → 401 (rejected auth is not logged — silent from hub side).
- 60s hub-side watch: zero devices connected, zero events at time of watch.
- ssh paths to the Pi abandoned: mDNS NXDOMAIN Mac→Pi; direct-IP attempt cancelled by operator (irrelevant — target machine is Windows).
- **Pending:** windows-agent's first authenticated handshake (will append to `events.jsonl` / appear on dashboard).

### 5. Windows machine handoff (delivered to operator)
- Build: clone → `cargo build --release` → `target\release\wtf.exe`.
- Credential carriers (either): MCP config `env` block (`WTF_HUB_URL`, `WTF_DEVICE_NAME=windows-agent`, `WTF_DEVICE_KEY`) or PQC secrets bundle → env (`wtf setup` reads env).
- Non-MCP agents: signed-request fallback per `.agents/skills/wtf-observability/SKILL.md`.
- Oversight: dashboard at `http://<HUB-IP>:7800/?k=<dashboard-key>` (URL line in `serve.log`) — devices, live events, BIN 1–3; works from any machine's browser; web-only Claude instances covered via bins + operator relay.

### 6. Coordination plan (awaiting operator)
1. Operator says go → fast-forward local `main` to `origin/main` (c8ad094).
2. Merge `docs/session-log-cross-machine` (this file) → `main` after user confirms; then mandatory cleanup of both worktrees/branches (review worktree may instead re-track for further remote pushes — operator's call).
3. Confirm windows-agent handshake on dashboard / `events.jsonl`.
4. Refresh `../wtf-remote-review` (`git pull`) if the remote agent pushes more while we coordinate.
