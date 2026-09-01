# TASK — machine-2 bring-up + federation connect (2026-09-01, machine 2)

Operator directive: pull all repo content, review llms.txt / COMMS /
task records, get machine-2 updated and connected to the federated
agent chat system.

## Shipped (ops record; no code changes)

- main @ `784886c` (v0.12.0) pulled from origin; release rebuilt,
  installed to `~/.local/bin/wtf` (PATH).
- Local hub restarted on 0.12.0 via persistent process manager
  (`wtf serve --no-open`; loopback bind, capability dashboard).
- `windows-1` enrolled on the LOCAL hub via v0.9.0 signed handshake
  (a stale same-name registration from the earlier partial setup was
  revoked first; hub origin `hub-2538554f`).
- omp harness MCP wiring: `~/.omp/agent/mcp.json` → stdio
  `/home/<user>/.local/bin/wtf agent` (16 tools verified via stdio
  initialize + tools/list).
- Operator handed the mac site secret in-session: `wtf federate add
  mac --url http://<mac-lan>:7800 --psk <secret>` (enrolls this hub on
  the mac hub as `fed-hub-2538554f`, adopts peer identity
  `hub-799c0c4c`); hub restarted to spawn the replicator.
- `windows-1` also enrolled on the MAC hub (same PSK; sessions are
  hub-local, so the mac hub is the shared chat surface).
- Shared repo-tagged chat created on the mac hub:
  `wtf-is-going-on-mcp` (`a305c8ea6934b65b5531d631410b81cc`, repo
  wtf-is-going-on-mcp); pairing key handed to the operator for
  mac-agent; announce sent (seq 1).
- Reporting proven via signed curl fallback + MCP stdio: checkin /
  event / heartbeat / state / sessions all green against BOTH hubs.

## Verification

- Federation: mac hub feed carries origin `hub-2538554f` events incl.
  windows-1's repo-tagged bring-up event; win hub carries 33 mac-origin
  events. Both directions converge (push-on-append + anti-entropy).
- Capability dashboard: `/w/<token>` 200, wrong token uniform 404,
  old LAN key 401 after rotation. `windows-1` card visible on both
  hubs. session_list parity on the mac hub.

## Incident (self-caught, mitigated)

- During dashboard verification, `wtf dashboard-url` output (incl. the
  LAN dashboard key) landed in a tool transcript. Mitigated: hub
  stopped, capability token + `dashboard_key` regenerated in place
  (python, 0600, no echo), hub restarted; old key verified 401, new
  token 200; device lane unaffected. Rule confirmed the hard way:
  capture capability URLs shape-only.

## Gates

- [x] No code changes; docs-only task file + COMMS ledger + AGENTS.md
  frontmatter fix (stale "One hub per fleet" summary → per-machine
  federated hubs, matching REPO_STATE v0.11.0).
- [x] Secret grep of diff: clean (no secrets in tasks, ledger, commits).

## Followups

- mac-agent: join `wtf-is-going-on-mcp` session with the pairing key
  (operator holds it); `comms_post`/`comms_read` from there.
- Optional Windows portproxy if mac→win push latency ever matters
  (anti-entropy converges ≤10 s without it).
