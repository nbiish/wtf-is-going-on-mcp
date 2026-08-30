# TASK 2026-08-30 — session log: pull machine-2 work, upgrade hub to v0.5.0

Goal: sync remote changes; execute assigned machine-1 action chain.

- Fetched origin: main d28b762 → 11dd683. ####
- Fast-forward, zero conflicts. ####
- New remote work reviewed. ####
- Bins read: BIN 1 sample only. ####
- Production hub was v0.3.0. ####
- Security notice: revocation lag bug. ####
- Rebuilt release on this Mac. ####
- Installed binary to ~/.local/bin. ####
- Old hub stopped cleanly. ####
- Relaunched hub under supervisor. ####
- healthz v0.5.0 local + LAN. ####
- Bridge tool surface: 8 tools. ####
- write_bin, hub_info present. ####
- Tests: 48 unit + 6 e2e green. ####
- check_in + log_event live. ####
- mac-agent visible on dashboard. ####
- No tags yet — operator call. ####

####

## Deliverables — session state record

### 1. Remote work pulled (machine 2's commits, all merged to main)
- `d4727b4` fix(security): instant revocation — keystore reload per request
  (bug: revoked devices kept authenticating until hub restart; fix: reload
  on every authenticated request; e2e `revocation_is_instant`).
- `a3ae798` feat: `pqc-secrets gen` command + keygen overwrite guard
  (agent-side tooling; no hub change).
- `3fd39e3` feat: `write_bin` MCP tool (0.4.0) — agents publish to shared
  bins, device-signed, attributed to device name; supersedes the
  "bins read-only" wording in older handoffs (etiquette in machine-2
  handoff §9).
- `11dd683` feat: `hub_info` tool, `wtf dashboard-url`, `wtf skill
  install` distribution, dashboard polish (0.5.0).

### 2. Action chain executed (per handoff addenda)
1. Pulled `main` → `11dd683` (fast-forward from `d28b762`).
2. `cargo build --release` → wtf 0.5.0; installed to `~/.local/bin/wtf`.
3. Stopped stale hub (pid 59887, v0.3.0, uptime since Aug 29 17:43);
   relaunched as supervised process `wtf-hub` (`wtf serve --no-open`,
   `WTF_HOME=$HOME/.config/wtf-mcp`, bind 0.0.0.0:7800 per config.json).
4. Verified: healthz `{"ok":true,"version":"0.5.0"}` on localhost AND LAN
   interface; bridge smoke test lists all 8 tools incl. `write_bin`,
   `hub_info`; `initialize` carries chain-of-draft instructions.
5. `cargo test`: 48 unit + 6 e2e green (matches machine 2's matrix).
6. Signed check-in + event via fallback lane; `mac-agent` status
   `working`, task "sync+upgrade hub v0.5.0" on the dashboard.

### 3. Security posture after upgrade
- Instant revocation now live on the production hub (fix `d4727b4`).
- Keystore: `mac-agent` active; `machine-2` revoked (unchanged).
- No secrets in repo, logs, task files. Bridge creds remain only in
  0600 `$WTF_HOME` files / PQC bundle lane.
- BIN 1 still holds the old operator sample; bins 2-3 empty. Operator
  may overwrite via dashboard (or ask an agent; writes are attributed).

### 4. Open items (operator / next session)
- No release tag yet — cross-machine verification now effectively done
  (both machines green on 0.5.0); tagging is the operator's call.
- Windows-side machine-2 connectivity was never observed from THIS hub
  (their §8 verified against their local hub only); first authenticated
  `windows-agent` handshake on the production hub is still pending.
- PQC/password enrollment-gate design question remains open by contract
  (converge before touching `auth.rs`/`api.rs`).
- Old handoff wording "bins are read-only for agents" in
  `HANDOFF.2026-08-29-machine1.md` §"What machine 1 verified" predates
  v0.4.0; machine-2's handoff §9 supersedes it. Left as history per the
  "append, do not rewrite" rule.

---
