# HANDOFF — machine 1 → machine 2 (2026-08-29)

Read `.agents/skills/wtf-agent-hub/SKILL.md` and the `COMMS` section and
`WTF HUB` block of `AGENTS.md` first. This file is the live coordination sheet between the
two machines: append your section, do not rewrite history.

## State on machine 1 (hub host, WSL2 Ubuntu)
- Repo: `github.com/nbiish/wtf-is-going-on-mcp`. `main` = `c8ad094` (includes
  machine 2's bins feature `b0086e8` + AGENT_HUB docs).
- Hub: **wtf v0.3.0 LIVE**, port 7800, bound to all interfaces (verified with
  `ss`). Log: `/tmp/wtf-hub.log`. NOTE: the startup line prints the dashboard
  key — never paste logs into chat, issues, or commits.
- Dashboard key: `$WTF_HOME/config.json` field `dashboard_key` (default
  `~/.config/wtf-mcp/config.json`). Never commit, never echo.
- Dashboard: `http://<machine1-host>:7800/?k=<dashboard key>`

## What machine 1 verified (all green)
- Chain-of-draft mandate ships in-protocol: bridge `initialize` carries an
  `instructions` field and every tool description mandates it. Any MCP
  harness gets the rule without loading the skill. (Requires rebuilt binary.)
- `AGENTS.md` `AGENT_HUB` section: setup, reporting contract, lift-ready
  plain-text system prompt.
- Bins end-to-end against the live hub: operator `PUT /api/v1/bins/1` → agent
  `list_bins` → `read_bin` → live signed `check_in`. Bins PUT also emits an
  event and an actor card.
- Kill switch: `key revoke` is instant + hot; issuance hot-reloads too.

## Gotchas (learned the hard way)
1. `WTF_HUB_URL` / `WTF_DEVICE_NAME` / `WTF_DEVICE_KEY` env vars OVERRIDE
   `bridge.json`. During testing a stale export made the bridge auth as the
   wrong device. Unset them before trusting `bridge.json` (or use them
   deliberately — they are the secret-manager delivery path).
2. `PUT /api/v1/bins/N` needs a JSON body `{"content":"..."}` — raw text 400s.
3. Bins writes record an actor card (`dashboard@dashboard` for dashboard-key
   writes) — expected v0.3.0 behavior, not an intruder.
4. A bridge must be rebuilt + relaunched after pulling, or it serves stale
   tool descriptions.

## Current hub state
- Devices: `dev1`, `agent1`, `agent2`, `observer` (local test devices).
  `crosstest` was revoked after testing (revocation verified working).
- BIN 1 holds a 90-char test payload — overwrite it when you connect:
  `PUT /api/v1/bins/1?k=<key>` with `{"content":"your brief for agents"}`.
- No release/tag yet — deliberately waiting for cross-machine verification.

## Connecting machine 2 (fast path)
1. `git pull origin main && cargo build --release` (optionally `install -m
   755 target/release/wtf ~/.local/bin/wtf`).
2. Enroll: `wtf join <user>@<machine1-host> --name <your-box>` (needs your
   ssh key authorized on machine 1 + `wtf` on PATH there) — or ask machine 1
   to run `wtf key issue --json <name>` and hand you the one-time secret
   inside an encrypted channel.
3. Register the bridge with your MCP harness: `command: wtf`, `args:
   ["agent"]` (exact JSON in README/skill).
4. `check_in` chain-of-draft, `read_bin` BIN 1, say hi with `log_event`.
5. Connection refused from outside? WSL2 NAT: Windows-side
   `netsh interface portproxy add v4tov4 listenport=7800 connectport=7800
   connectaddress=<machine1-wsl-ip>` + firewall rule — or join both machines
   to an overlay (Tailscale/WireGuard) and machine 1 runs
   `wtf url http://<overlay-ip>:7800` before issuing your key.

## Coordination & security — OPEN (PQC/password direction)
- Current lane: per-device HMAC-SHA256 keys (32-byte, kernel CSPRNG, printed
  once); transport is plain HTTP on LAN, `https://` via TLS proxy, overlay
  recommended off-LAN. The signature never sends the secret.
- Your PQC-secrets + password system is welcome. The designed injection
  point is credential DELIVERY: end with `WTF_HUB_URL` / `WTF_DEVICE_NAME` /
  `WTF_DEVICE_KEY` set (or `bridge.json` written 0600) and no hub code needs
  to change.
- Want a stronger lane (PQC-wrapped enrollment, password-gated `key issue`,
  machine-bootstrap trust)? Propose it in
  `.agents/handoffs/HANDOFF.2026-08-29-machine2.md` on this branch. We
  converge on the design before touching `auth.rs`/`api.rs`. Fail closed;
  no secrets in repo, logs, or dashboard text.
- Open questions to settle together: does enrollment itself get a PQC or
  password gate? How does a brand-new machine bootstrap trust for that gate
  without a human-carried secret getting weaker than what it protects?

---

## Correction (2026-08-30)

- `AGENT_HUB` was replaced by the `COMMS` protocol + `WTF HUB` block in
  `AGENTS.md`; agent-facing skill is `wtf-agent-hub` (sessions era).

---

## Addendum (2026-08-31, machine 2) — v0.7.0 COMMS channels + enrollment still pending

Shipped on main (this branch, merged): **wtf v0.7.0 — encrypted COMMS
ledger channels.** Structured agent-to-agent entries (git-ledger event
vocabulary + repo/branch scope) carried inside the existing encrypted
session transport: `comms_post` / `comms_read` (16 MCP tools now).
Same guarantees as sessions: ML-KEM-768 sealed keys, GCM ciphertext on
wire and at rest, hub stores ciphertext only. Docs updated in AGENTS.md,
llms.txt, skill §7, README. Full record: `.agents/tasks/TASK.2026-08-31.md`.

Machine-1 actions:
1. `git pull origin main && cargo build --release` and restart the
   mac-agent bridge. **The HUB needs no upgrade** — comms ride the
   existing `/api/v1/sessions*` routes; only bridges grow the two tools.
2. COMMS etiquette for mac-agent: skill §7 — check `comms_read` at task
   boundaries; post `handoff`/`blocked` entries; secrets between agents
   travel ONLY in session/COMMS channels (bins/events are public).
3. **Enrollment of `windows-1` is still pending on your side.** Machine 2
   has pulled, rebuilt (v0.7.0), read all ledgers, and is ready — it
   lacks only (a) this hub's URL and (b) the device key. Issue the key:
   `wtf key issue --json windows-1` (or `pqc-secrets issue wtf
   windows-1`), deliver the secret + hub URL to the operator out-of-band
   (never in repos/logs/chat tooling). Machine 2 will enroll via the
   `WTF_*` env lane and check in.
