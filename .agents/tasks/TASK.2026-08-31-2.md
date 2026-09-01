# TASK — hub federation + capability-path dashboard (2026-08-31, machine 1)

Operator directive: a hub on EVERY machine; the ledger replicates across all
of them ("Anishinaabe updates on one immediately updates all three"); each
machine serves its dashboard on a LOOPBACK-ONLY endpoint hidden behind a
high-entropy capability path ("hard to hash out"); one machine runs agents
across many repos, with agents from other machines connecting to the same
surface; operator/agents print the URL via CLI or MCP (`wtf dashboard-url`,
`hub_info`).

Reverses the documented ONE-hub-per-fleet invariant (AGENTS.md REPO_STATE,
handoff sheets): machine-2's hub STAYS UP as a peer; windows-1 enrolls on
its LOCAL hub; the mac PSK handoff is moot (still fine if ever needed).
Comms announcement #6 posted to mac-win-pipeline.

## Approach (converged with operator in-session)

Full-mesh push replication over the existing HMAC-SHA256 request lane
(standard-transport, same as agent auth — transport crypto, not secrets-at-rest;
the ledger is a public-surface event log by design, secrets forbidden).
- Each hub gets a stable name + peer table in config (`federation.json` 0600).
- Every event carries an origin hub id; replication is push-on-append plus
  anti-entropy pull on a 30 s cadence; generation-cursor pull (hub-assigned
  monotonic per hub) + (origin, event_id) dedupe keep the mesh loop-free
  without global clocks or CRDTs — append-only feed, last-writer-wins on
  agent cards, consistent with the existing single-node model.
- Dashboard: bind loopback-only by default; root served only at
  `/w/<64-hex capability token>`; wrong/absent token → uniform 404 (same
  page as any unknown path, no oracle). API stays on its signed routes;
  `/healthz` stays open (no state).
- Multi-repo agents: optional `repo` field on checkin/event, surfaced in
  state + dashboard grouping (machine → agent → repos). One bridge per
  machine per repo or per agent identity is documented, not enforced.

## Shipped

- `src/federation.rs` (new): `FedConfig` (hub `name` minted on first
  serve, stable; peer table with per-peer `{name, url, device, device_key}`
  — the credential THE PEER issued — in `federation.json`, 0600, atomic),
  dashboard capability token (`dashboard_capability`, 0600, 64-hex,
  auto-minted, corrupt file regenerates), push-envelope shape validation.
- `src/store.rs`: events gain `origin` + `origin_id` + `repo`;
  `Store::set_origin_name` stamps local events; `ingest` dedupes on
  (origin, origin_id), assigns local ids, last-writer-wins agent cards by
  ts; cursors `max_origin_id` / `events_since`; replay keeps old events
  first-class.
- `src/api.rs`: `POST /api/v1/fed/push` (device-authed, `fed-*`
  credential required, ingest-deduped, ingress event validation), `GET
  /api/v1/fed/pull?origin=&after=` (cursor pull for anti-entropy), `GET
  /api/v1/fed/peers` (real fed identity for link-time adoption). Dashboard
  page served ONLY at `/w/<capability>`; loopback hubs gate on the token
  alone, LAN hubs also accept `?k=`; uniform 404 on wrong/absent token.
  `checkin`/`event` accept optional `repo`; state JSON agents carry
  `repo` + `origin`.
- `src/replicate.rs` (new): per-peer replicator thread — push-on-append
  (generation-triggered) + 10 s anti-entropy sweep over the standard HMAC
  device lane; throttled warns; never crashes the hub.
- `src/main.rs`: serve mints/stamps the fed name, spawns replication when
  peers exist, prints the capability dashboard link on loopback hubs;
  `wtf federate add <name> --url U --psk S [--as DEV]` (PSK handshake as
  `fed-<hub-name>`, adopts the peer's REAL fed identity via signed
  `/api/v1/fed/peers`, verifies with an anti-entropy round-trip) /
  `federate list` / `federate remove`; `wtf dashboard-url` prints the
  localhost capability URL.
- `src/mcp.rs`: `check_in`/`log_event` accept `repo` (default = bridge
  cwd basename, `WTF_REPO` overrides); `hub_info` points operators at
  `wtf dashboard-url` for the capability link.
- `src/dashboard.rs`: agents group by origin hub (chips), repo chips per
  agent/event; auth via `?cap=` on loopback, `?k=` elsewhere.
- Docs: README (federation section, dashboard section, security bullets,
  CLI + API tables, troubleshooting), root + src `llms.txt`, AGENTS.md
  REPO_STATE, SKILL.md reporting contract (multi-repo note; mirror sync
  owed post-merge). Version 0.10.0 -> 0.11.0.

## Gates

- [x] cargo test: 100 unit + 12 e2e green (new
  `federation_two_hub_end_to_end`: two real hubs on pinned ports, one
  `federate add`, events checked in on both hubs appear on both ledgers
  origin-tagged; `federation.json` asserted 0600; capability dashboard
  200 / wrong-token 404 / absent-path 404).
- [x] cargo build --release clean (wtf 0.11.0).
- [x] Secret grep of diff: see verification phase (runs pre-merge).

## Notes

- Loopback default changes behavior for LAN dashboards; operator-approved.
- Debugging notes: (1) the dashboard route needed explicit `/w/` dispatch —
  exact-match `GET /` never saw capability paths; (2) `federate add` must
  adopt the peer's REAL fed identity (its minted `hub-<hex>` name), not the
  operator's label — pull cursors address the origin name; (3) fed push/pull
  authz is on the CALLER's `fed-*` credential, not per-origin device
  coupling — one credential serves both directions and the caller != origin
  case (pulling a peer's events) is the common path; (4) e2e must pin hub
  ports because `federation.json` records the peer URL at add time.
- Mesh verified by hand beyond the e2e: two hubs converge both ways in
  <3 s; single-warn on peer-down, no crash; dedupe holds under restart.


---

# TASK — session pairing keys + repo-tagged chats (2026-08-31, machine 1)

Operator directive: a hard-to-guess pairing key for the federated chat
system, copyable to the other machine/agent or redeemable via CLI; MCP
tooling that lists agent chats WITH their paired repo so agents can pick
the right chat instantly.

## Shipped

- `sessions.rs`: `pairing_hash` (SHA-256 of the 256-bit pairing key; the
  key itself never touches the hub) + `repo` per session;
  `create` mints the key and returns it once; `check_pairing`
  constant-time; `join_or_refresh` (pairing path: admit + ek refresh on
  identity rotation); `set_repo`.
- `api.rs`: create returns `{...pairing_key}` once; join accepts
  `pairing` (wrong key = uniform 403; valid key joins even when the
  membership edge would block, refreshing ek); response carries
  `pairing_ok`.
- `mcp.rs`: `session_create` takes `repo`, surfaces the pairing key, and
  persists it locally (`session_keys.json` `pairings`, 0600) so the
  operator can re-print it; `session_join` takes `pairing`;
  `auto_seal_members` (key-holder seals to any member lacking a package;
  hooked into send/read); `session_read` recovers the key from seals;
  `session_list` shows repo + pairing status.
- `main.rs`: `wtf sessions` — operator chat list (id, name, repo,
  members, msgs) with local pairing keys re-printed on the creator
  machine; dashboard-key gated.
- Docs: root + src llms.txt, README (tools table, pairing paragraph,
  CLI row), SKILL.md §6 (pairing flow, auto-seal, manual fallback).
  Version 0.11.0 -> 0.12.0.

## Gates

- [x] cargo test: 100 unit + 13 e2e green (new
  `session_pairing_key_end_to_end`: repo-tagged create → pairing key
  surfaced once → wrong key uniform reject → pairing join → creator
  auto-seal → cross-agent message read → repo visible in session_list).
- [x] cargo build --release clean (wtf 0.12.0).
- [x] Secret/banned-algo/CJK greps: clean (run pre-merge).

## Post-merge bring-up record (machine 1, 2026-08-31)

Both releases merged to main (v0.11.0 `595f05c`, v0.12.0 `b9e357b`),
pushed, tagged `v0.12.0`. Mac hub live on 0.12.0 (state intact across
restart; capability URL stable). Worktrees + branches cleaned. SKILL.md
mirror synced to ainish-coder (byte-identical, pushed).

### Cross-machine bring-up sequence (the 1-2-3)

Prerequisite on machine 2 first: `git pull origin main && cargo build
--release && install -m755 target/release/wtf ~/.local/bin/wtf`, then
restart its hub + bridges (a stale bridge serves stale tools).

1. **Enroll windows-1 on the Mac hub** — operator runs `wtf
   enroll-secret` on the Mac and hands the printed command to machine 2:
   `wtf enroll --url http://192.168.1.68:7800 --name windows-1 --psk
   <secret>`. The secret never crosses the wire (HMAC proof); clocks
   must agree within ±5 min; `wtf enroll-secret --rotate` kills every
   outstanding copy.
2. **Federate the two hubs** — machine 2: `wtf enroll-secret` (its own
   site secret); Mac: `wtf federate add win --url http://<win-ip>:7800
   --psk <win site secret>`, then restart the Mac hub. Both ledgers
   replicate both ways (push-on-append + 10 s anti-entropy; dedupe on
   origin+origin_id).
3. **Connect the agents** — an agent creates a repo-tagged chat:
   `session_create {name, repo}` (pairing key returned once; also
   re-printable with `wtf sessions` on the creator machine). Joiner:
   `session_join {session, pairing}` — auto-seal delivers the session
   key; no manual seal round-trip. Verify: `wtf status` on both machines
   shows both agents; `session_list` shows the chat under its repo.

### Same-remote verification (both agents, after step 3)

Each agent: `git rev-parse HEAD` on main must match on both machines
(`b9e357b` or later); `wtf status` shows the other agent present;
`wtf_is_going_on` returns both; a `comms_post`/`comms_read` round-trip
in the shared chat works; `session_list` shows the same chats + repos on
both sides. Any mismatch = stale pull/build — re-run the prerequisite.

### Verification receipt (machine 1, live)

- `/healthz` → 0.12.0; `wtf sessions` lists existing chats with repos;
  `wtf dashboard-url` prints the loopback capability URL (unchanged
  across restarts); 100 unit + 13 e2e green on main.

---

# VERIFIED — cross-machine handshake complete (2026-09-01, machine 1)

The 1-2-3 sequence ran green end to end, driven by windows-1 from its
side:

1. **Enroll**: windows-1 enrolled on BOTH hubs (`windows-1` device on
   the Mac hub; `windows-1` on its own local hub).
2. **Federate**: windows-1 federated its hub `hub-2538554f` with the Mac
   hub `hub-799c0c4c` from its side — replication confirmed LIVE both
   directions (federation events every 10 s on both ledgers; my hub had
   zero local peer config and the mesh still formed — the pull side
   works via the fed device windows-1 enrolled on my hub).
3. **Connect**: windows-1 created the repo-tagged chat
   `a305c8ea… 'wtf-is-going-on-mcp'` and left the pairing key with the
   operator. mac-agent joined (ek registered). windows-1's handshake
   send triggered the AUTO-SEAL; mac-agent's first `session_read`
   recovered the session key and decrypted all three of windows-1's
   messages. mac-agent replied (seq 4, encrypted) and posted a COMMS
   checkin (#5).

## Verification battery (all green)

- Encrypted round-trip: send → read-back, sender-bound (seq), no
  plaintext on the hub.
- COMMS ledger envelopes over the chat: `handoff` from windows-1,
  `checkin` from mac-agent.
- Same-remote: `origin/main` @ `79dd4d5` contains windows-1's `784886c`
  + bring-up docs (`ee84db1`, `0a192a3`); both heads match.
- Federation: bidirectional event flow (`+N event(s) from hub-…` on both
  sides, 0 duplicates).
- `wtf sessions` + `session_list` parity: chat shows repo
  `wtf-is-going-on-mcp`, 2 members, pairing:key.
- Dashboard: both agents visible with repo chips + origin hub tags.

## Next lanes (operator-set priorities)

1. **Dashboard CHATS view**: chat blocks in the AGENTS section — per
   chat: name, repo chip, member pills, msg counts, pairing state. The
   hub stores ciphertext only, so the dashboard shows metadata + live
   counts (SSE), never message content; content review stays in agent
   CLIs (`session_read`) and `wtf sessions`.
2. **Autonomous task handoff (agent-CLI fallback chain)**: agents
   receive tasks in the repo chat and execute them via their own agent
   CLI (non-interactive ACP tooling), reporting progress back into the
   same chat. OPERATOR PRE-AUTHORIZATION: agents directed to this MCP
   are trusted to work autonomously — no per-task permission checks.
   **Fallback priority (operator-set):** (1) OhMyPy CLI, (2) Hermes
   CLI, (3) FreeClaudeCode — the free Claude Code server + Claude
   system; when neither OhMyPy nor Hermes is installed, start the
   FreeClaudeCode server and run Claude through it. FreeClaudeCode
   ALWAYS runs inside a named tmux session
   `freeclaude-<repo-or-task-slug>` (identifiable process, reattachable;
   report session name + PID in task notes). Never block a task on a
   missing CLI brand; record which CLI ran the task.
   **Install surface (operator directive 2026-09-01):** collect the
   install paths so a machine with NONE of these can be brought up
   easily — OhMyPy: `bun install -g oh-my-pi` (binary `omp`); Hermes:
   rides the user's ACP harness config (e.g. acp-hermes); FreeClaudeCode:
   per-task tmux start. One-command environment: `ainish-coder --rules
   <repo>` deploys AGENTS.md + COMMS protocol + all skill packs. Documented
   in SKILL.md §3 (Agent CLIs — install + fallback).
3. **Zero-config join (user directive 2026-09-01)**: the user should be
   able to hand ANY agent on ANY machine (a) the skill file (ships in
   the binary; `wtf skill install`) and (b) one highly secure federated
   key, and that agent connects autonomously — no manual MCP config, no
   technical steps. Advanced users get full control; non-technical users
   get a two-artifact setup. Design sketch (converge before code):
   the skill file carries self-configuration instructions an agent can
   follow from a single key input; candidate UX = `wtf join-key <key>`
   (one command: resolves hub URL embedded in the key or prompts once,
   enrolls via the PSK handshake, writes bridge.json, wires the MCP
   client entry if the harness config is discoverable, verifies with a
   signed round-trip) — everything in-tree per the hard-code directive.


---

# TASK — env_report / env_probe cross-machine capability discovery (2026-09-01, v0.13.1)

Operator directive: verify through MCP calls that remote machines have
the agent CLIs set up; report models/tooling cross-machine so the user
can have one agent configure the federated system on another (e.g.
claude compute) — securely, with keys ported only by explicit user
action, never auto-scanned.

## Shipped

- `api.rs`: `POST /api/v1/env` (device-auth bridge self-report: CLI
  presence/versions/os/arch; 8 KiB cap; ring of 64; credentials
  explicitly out of scope) + `GET /api/v1/env` (device-auth: all
  devices' reports).
- `mcp.rs`: `env_report` (probe own machine: omp/hermes versions +
  paths, freeclaude tmux sessions, os/arch; never touches key material)
  + `env_probe` (all devices' reports) — tools 16 -> 18.
- SKILL.md §3: cross-machine capability discovery note.
- Gates: 100 unit + 13 e2e green (tools count assertion 16 -> 18).

## Security posture

- The report contains PRESENCE + VERSIONS ONLY. API keys, model
  credentials, and config files are never read, never transmitted,
  never stored — they live in the user's env / PQC bundle.
- Configuration porting (keys to claude compute etc.) remains an
  explicit, operator-driven action through the PQC envelope path, not
  an env-probe feature.

---

# TASK — MAC-TO-WINDOWS.COMMS.md three-agent verification + FCC live check (2026-09-01)

Operator final-test goal: a durable `MAC-TO-WINDOWS.COMMS.md` proving all
three agent CLIs (OhMyPy, Hermes, FreeClaudeCode) append to a shared
cross-machine text file, commanded from this machine (or any repo chat
handshake — the repo 'smart-contract' of federated work).

## FCC live check (this machine, first leg)

- `fcc-server` started in NAMED tmux session `freeclaude-wtf-mcp`
  (PID 30349) — `/health` → `{"status":"healthy"}`.
- `fcc-claude -p "..."` reaches the server; upstream returned 401 (the
  claude-code CLI behind it has no valid auth on this box right now).
  CHAIN MECHANICS VERIFIED (server-in-tmux healthy, client reaches
  server); upstream auth is operator-configured per the keys posture
  (user watches keys/limits; keys port only via PQC envelope on explicit
  operator action).
- Rule now in SKILL.md/AGENTS.md/llms.txt: FreeClaudeCode ALWAYS runs in
  a named tmux session `freeclaude-<slug>`.

## Three-agent append plan (next chat / windows-1 joint task)

1. Ensure `omp`, `hermes` on the executing machine (env_probe first;
   install paths in SKILL.md §3). FCC: tmux-named server + `fcc-claude`.
2. Create `MAC-TO-WINDOWS.COMMS.md` in the repo root (or the federated
   chat defines its path), with a header explaining the three-agent
   append contract.
3. Each CLI appends one identified line:
   - `omp -p "append '<line>' to MAC-TO-WINDOWS.COMMS.md"` (headless)
   - hermes equivalent via its ACP CLI
   - `fcc-claude -p ...` through the tmux-named server
4. mac-agent (this machine) commands; windows-1 executes on its machine;
   both verify via `git diff`/`session_read` + the COMMS chat.
5. Durable record: the file itself + this task + COMMS ledger entries.
