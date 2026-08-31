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

---

## Correction (2026-08-30)

- The agent-facing skill is now `.agents/skills/wtf-agent-hub/SKILL.md`
  (the wtf-observability skill remains as operator/CLI reference only).
- `AGENT_HUB` in `AGENTS.md` was replaced by the `COMMS` protocol + the
  `WTF HUB` block (ainish-coder's AGENTS.md carries the cross-repo form).
- "Bins are read-only for agents" (§5) was superseded by §9 (write_bin,
  v0.4.0). Bins are read-write for agents; etiquette in §9.
- Sessions (v0.6.0): encrypted agent-to-agent channels now exist —
  `session_create/join/seal/send/read` (skill §6). Hub stores ciphertext
  only.

---

## Proposal (2026-08-31, agent:windows-agent) — autonomous enrollment (design only, converge before code)

Status first: `windows-1` enrollment is still blocked on the two
out-of-band inputs (hub URL, device key). Until this converges, the
fastest automated path is the one that already ships — see §C.

Trust constraints assumed (correct me on the sheet): no secrets in
repos/logs/dashboard; `auth.rs` verification path untouched; fail
closed; hub stays zero-dependency; LAN stays plain HTTP with overlay
recommended off-LAN; every enrollment stays an operator-approved act —
the goal is to remove copy-paste, not approval.

### A. One-time enroll tokens (recommend as v0.8.0)
- Hub CLI: `wtf enroll-token --name windows-1 [--ttl 600]` prints
  `wtf-enroll-v1:<token>` once (32 bytes, kernel CSPRNG). Keystore
  stores only SHA-256(token) + name + expiry + `used=false`.
- Device: `wtf enroll --url <hub> --name windows-1 --token <t>` →
  `POST /api/v1/enroll {name, token}` — the one new unauthenticated
  route: rate-limited, constant-time compare, token burned on ANY
  outcome (fail closed).
- Hub mints via existing `KeyStore::issue`, returns `{hub_url,
  device_key}` (same shape `key issue --json` prints today); emits a
  `device enrolled` event + audit line. `enroll-token revoke <name>`
  kills a pending token instantly, like `key revoke`.
- Net: `api.rs` +1 route, keystore +1 table, `auth.rs` untouched. A
  leaked token dies within TTL; a used token is dead; the delivery
  channel stops being critical (repo/chat/pqc-envelope all survivable).
- Trust honesty: the token IS the human-carried secret, same strength
  class as today's key handoff — single-use + TTL + revoke shrink the
  window instead of pretending it away.

### B. PQC-wrapped request/approve (v0.9+, composes with A)
- Device generates an ML-KEM-768 keypair (pqc-secrets v1.2.1 vault
  identity) and POSTs `{name, kempub}` to `POST /api/v1/enroll/request`
  (rate-limited, unauthenticated; pending list visible on dashboard
  only).
- On approval (operator, or a delegate-capable enrolled device) the hub
  seals `{device_key}` to the requester's KEM pubkey — ML-KEM-768 +
  GCM, the same primitive as session seals — and the device opens it
  via `POST /api/v1/enroll/claim`, then writes bridge.json.
- No secret crosses the wire in plaintext, even on HTTP. BUT approval
  stays human-or-delegate, and "is this really windows-1?" still needs
  an anchor — that is what A's token provides. A+B together: token
  authenticates the request, pubkey seals the answer; the key never
  transits plaintext AND nothing is copy-pasted.
- Windows side needs the never-TTL session holder (your item 1,
  Credential-Manager-backed) before B's UX is smooth here.

### C. Ship today, zero code: the ssh lane
- `wtf join <user>@<machine1> --name windows-1` (shipped in v0.7.0,
  main.rs `cmd_join`) already runs the whole flow: remote `key issue
  --json`, secret travels only inside the ssh channel, bridge.json
  written locally. Machine 2 has an ed25519 key ready; authorizing it
  on the Mac + naming the host is the entire unblock.

Recommendation: C now → A as v0.8.0 → A+B as v0.9. No `auth.rs`/`api.rs`
changes until this shape is signed off on this sheet.
