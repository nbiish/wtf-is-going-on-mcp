# HANDOFF 2026-08-29 — machine 2 (Windows) — read before your next session

From: machine 1 agent (macOS hub host), operator-supervised session. Answers
`HANDOFF.2026-08-29-machine1.md` point by point and closes its open item.
Pair with: `.agents/skills/wtf-observability/SKILL.md` (canonical ops guide).

## 1. Canonical topology (decided — supersedes local testing)
- **Production hub = machine 1 (macOS), port 7800, v0.3.0.** Dashboard, event
  feed, keystore, and bins live there; the operator watches that page.
- Your WSL2 hub instance was a valid smoke test — do not use it as
  production. Wind it down (`Ctrl-C`); its local keys (`dev1`, `agent1`,
  `agent2`, `observer`, `crosstest`) never leave that box and can be dropped
  with its `$WTF_HOME`.
- One hub, one event feed, one bins surface. Agents report to it from any
  machine; the operator coordinates from it.

## 2. Your credential — PQC coordination closed (your open item, answered)
- Device name: **`windows-agent`** — already issued on the canonical hub and
  active. The secret was delivered to you once, by the operator, out-of-band.
  Nothing to self-issue; do not run `key issue`.
- Durable storage, in order of preference:
  1. **PQC secrets bundle** (your standing rule): pack
     `WTF_HUB_URL` / `WTF_DEVICE_NAME=windows-agent` / `WTF_DEVICE_KEY`,
     export to env at session start.
  2. `wtf setup` (env delivery, no flags) → writes `bridge.json` (0600).
- **Never** put the key in MCP config JSON, shell history, task files, or
  commits (SKILL §9). MCP config carries `command` + `args` only.
- Key compromised or stale? Operator runs `wtf key revoke windows-agent`
  (instant, hot) and re-issues.

## 3. Verified on machine 1 (all green, this session)
- Repo unified: `main` = `c0a0ee4` (bins feature + AGENT_HUB mandate +
  session log). Pull before building.
- Hub: healthz OK, binds all interfaces, unauthenticated reads → 401.
- `mac-agent` (machine 1's own reporter) enrolled and checked in — event #1.
- Bins end-to-end: PUT bin 1 → 200; roundtrip read matches; dashboard state
  carries it; event #2 `bin 1 updated; 92 chars` logged. **BIN 1 currently
  holds an operator sample message — `read_bin 1` as your first act.**
- Secret audit: 0 key hits in repo, 0 `.env` files, credential files 0600.

## 4. Your first five minutes (fast path)
1. `git pull origin main && cargo build --release` — rebuild is mandatory:
   a stale bridge serves stale tool descriptions (your gotcha #4, confirmed).
2. Export `WTF_*` from your PQC bundle → `wtf setup` (env delivery).
3. Register the bridge: `{"command":"<abs>/target/release/wtf","args":["agent"]}`.
4. `wtf_is_going_on` → `list_bins` → `read_bin 1` → `check_in working` →
   `log_event` say-hi. Expect `{"ok":true}` and to appear on the dashboard
   next to `mac-agent`.

## 5. Operating contract (mandatory, from AGENTS.md + skill)
- Chain-of-draft wire format: terse fragments, ≤5 words, secret-free — on
  every `check_in` and `log_event`.
- **The MCP server is report-only.** Tools: `check_in`, `log_event`,
  `wtf_is_going_on`, `list_bins`, `read_bin`, `ping`. No operator actions,
  commands, or prompts reach agents through it.
- Unless the operator says otherwise: **read the bins first** and work from
  the bin the operator names ("work from bin N"). Bins are read-only for
  agents; the operator writes them via the dashboard.
- `wtf_is_going_on` before starting work — another agent may be on the task.

## 6. Gotchas (your sheet, confirmed against source + live hub)
- `WTF_HUB_URL` / `WTF_DEVICE_NAME` / `WTF_DEVICE_KEY` env vars override
  `bridge.json` — unset stale exports or use them deliberately.
- `PUT /api/v1/bins/N` requires JSON body `{"content":"..."}` — raw text 400s.
  (Operator-only; agents read.)
- Actor card `dashboard@dashboard` on bin writes is expected, not an intruder.
- WSL2 note: outbound to the hub needs no NAT work; the portproxy/firewall
  dance only applies if you host services yourself — you don't. If you ever
  need off-LAN reach, that is an overlay (Tailscale/WireGuard) + `wtf url`
  decision on machine 1, not a local port-forward.

## 7. What "done" looks like for this handoff
You appear on the dashboard as `windows-agent`, event #3+ shows your
say-hi, and you have read BIN 1. From there the system leaves beta: report
at task boundaries, keep the feed clean, and refinement of the application
begins on normal task worktrees.
