# wtf-is-going-on-mcp

One command answers the eternal question across every machine you operate:
**what the fuck is going on?**

`wtf` is a zero-dependency Rust pair:

- **Hub** (`wtf serve`) — an HTTP API plus a live browser dashboard. Agents
  report what they are doing; you watch it in one page.
- **Bridge** (`wtf agent`) — an MCP stdio server that any MCP client launches.
  It exposes reporting and collaboration tools (`check_in`, `log_event`,
  `wtf_is_going_on`, `read_bin`, `write_bin`, `list_bins`, `ping`,
  `hub_info`) and forwards everything to the hub over HMAC-signed requests.

Everything — SHA-256, HMAC-SHA256, JSON, HTTP/1.1 server + client, and the
MCP bridge — is implemented in this repo on the Rust standard library only.
**Zero external crates.** The full security surface is readable in an
afternoon.

## Architecture

```
hub machine                              agent machines
┌────────────────────────────┐           ┌─────────────────────────┐
│ wtf serve                  │           │ MCP client (agent, IDE) │
│  ├─ HTTP API  /api/v1/*    │◄──HMAC────┤   └─ wtf agent          │
│  ├─ SSE stream  /stream    │  signed   │      (MCP stdio)        │
│  └─ dashboard   /?k=…      │  requests └─────────────────────────┘
│ state: events.jsonl (SSOT) │
└────────────────────────────┘
```

- The hub keeps an append-only event log (`events.jsonl`) as the source of
  truth; agent status is rebuilt by replaying it on startup. Restarts lose
  nothing.
- Browsers subscribe to state changes over Server-Sent Events; the hub pushes
  a fresh snapshot whenever the generation counter moves.

## Requirements

- Linux, macOS, or any Unix with `/dev/urandom` (WSL2 works; see
  [Troubleshooting](#troubleshooting) for the NAT caveat).
- A Rust toolchain (stable, edition 2021) — the **only** build prerequisite.
  No crates are downloaded; the build is fully offline.
- Reporting agents need one of: an MCP client (for `wtf agent`), or just
  `curl` + `openssl` (signed fallback — see the agent skill below).
- Viewers need only a browser.

## Build and install

```bash
git clone <this repo> && cd wtf-is-going-on-mcp
cargo build --release
```

The single binary is `target/release/wtf`. Installing it on `PATH` is
optional but recommended — a joining machine's `wtf join` and the hub-side
`key issue` both assume `wtf` is resolvable:

```bash
install -m 755 target/release/wtf ~/.local/bin/wtf
```

## Running the hub

```
wtf serve                    # first run creates the config; binds all interfaces, port 7800
```

First run generates `$WTF_HOME/config.json` (dashboard key included) and
prints the hub and dashboard URLs — the dashboard URL carries the `?k=`
key. Useful flags: `--bind IP:PORT` (e.g. loopback only), `--no-open` (do
not launch a browser), and `WTF_HOME=/some/dir` to relocate all state.

Run it persistently (it is a plain foreground process):

```bash
setsid wtf serve > "$WTF_HOME/serve.log" 2>&1 < /dev/null &
```

Stop with Ctrl-C or `pkill -f "wtf serve"`. Restarting is lossless: the
event log is replayed, keys and config reload, and agents reappear with
their last reported status (stale-marked until they check in again). If the
hub is reached over an overlay or the internet, tell joiners where to find
it: `wtf url http://OVERLAY-IP:7800` (or the public `https://` URL).

## Using the dashboard

Open the printed URL: `http://HUB:7800/?k=<dashboard key>`. `wtf serve`
reprints it on every start, and on the hub machine you can always get the
exact clickable link (localhost + LAN) with:

```
wtf dashboard-url
```

The key lives in `$WTF_HOME/config.json` (0600); it is never exposed over
MCP — agents only see the hub address via the `hub_info` tool.

- Each reporting agent renders as a card: `● name [status]` with its
current task, details, and last-seen age (`●` fresh, `○` stale after a
few minutes of silence).
- The event feed lists everything agents have logged, newest first.
- The **COPY-PASTE BINS** section holds three persistent bins shared by
  humans *and* agents. Paste any content (a spec, logs, a URL list) into a
  bin, hit Save, then tell any agent on any machine: *“work from bin 2”* —
  it fetches it with the `read_bin` MCP tool. Agents can write back:
  anything an agent saves via `write_bin` (device-signed PUT) lands in the
  bin with the device name as last writer, so any other agent — on any
  machine or harness — can read it. Bins survive hub restarts; every save
  lands in the event feed, so the other dashboards see it live.
- The page live-updates over Server-Sent Events (`/stream`) — no refresh,
no external assets, works offline.

## Security model

- **Keys**: 256-bit secrets from the kernel CSPRNG (`/dev/urandom`), stored
  `0600` in a `0700` directory. Device keys are printed exactly once by
  `wtf key issue`.
- **Request auth**: every agent request carries `X-Wtf-Device`,
  `X-Wtf-Timestamp`, `X-Wtf-Nonce`, `X-Wtf-Signature`, where the signature is
  `hex(hmac_sha256(secret, "wtf-hmac-v1\nMETHOD\npath-and-query\nts\nnonce\nsha256hex(body)")))`.
  The body hash binds the payload; the secret never crosses the wire.
- **Replay protection**: ±300 s timestamp skew plus a per-device nonce cache.
- **Dashboard auth**: a separate dashboard key gates `/` and `/stream` via
  `?k=`. `/api/v1/state` accepts either the dashboard key or device auth.
- **Constant-time** comparison for every secret check. Auth failures are
  uniform 401s that do not reveal which factor failed.
- **Enrollment tokens**: one-time, hashed at rest, expiring, revocable.
  Redemption (`POST /api/v1/enroll`) is rate-limited and every refusal is
  a uniform 403; the token burns only on success, so a typo does not
  brick it.
- **Signed-handshake enrollment (v0.9.0)**: the hub holds ONE site
  `enroll_secret` (256-bit hex, 0600). A joiner proves possession with an
  HMAC over (name, its ML-KEM-768 encapsulation key, timestamp, nonce) —
  the secret never crosses the wire — plus ±300 s skew and a replay
  cache, and receives its device key **ML-KEM-768-sealed** (FIPS 203,
  AES-256-GCM): the key never crosses in plaintext. Rotate the secret
  (`wtf enroll-secret --rotate`) to invalidate every outstanding copy.
- **Transport topologies**: on a trusted LAN, plain HTTP is fine. Across
  machines or off-LAN, run an encrypted overlay (WireGuard/Tailscale) and point
  the bridge at the overlay address — no code changes; the HMAC signature
  still authenticates every request. Behind a TLS-terminating proxy,
  `https://` hub URLs are accepted. Raw port-forwarding plain HTTP to the
  public internet remains unsupported; TLS is the proxy's job, never
  hand-rolled here.
- **PQC key delivery**: since v0.9.0, signed-handshake enrollment delivers
  the device key ML-KEM-768-sealed to the joiner's encapsulation key
  (FIPS 203 / AES-256-GCM; SP 800-38D) — it is unwrapped only in memory.
  Credentials can also ride env vars (`WTF_HUB_URL`, `WTF_DEVICE_NAME`,
  `WTF_DEVICE_KEY`), the path a PQC secrets bundle uses. The in-tree
  ML-DSA-65 identity is the documented future upgrade for handshake
  signing (today's proof is HMAC-SHA256, the same standard-transport lane
  as request auth).

## Deployment topologies

- **LAN** (default): hub binds all interfaces on port 7800; `key issue`
  auto-detects the LAN address.
- **Overlay** (recommended off-LAN): install WireGuard/Tailscale on every
  machine, then `wtf url http://OVERLAY-IP:7800` on the hub so joining
  devices receive the overlay address. No exposed ports; the network is the
  tunnel.
- **Cloud**: run the hub on a VM, front it with a TLS-terminating proxy
  (Caddy/nginx + Let's Encrypt), then `wtf url https://hub.example.com`.
  The bridge accepts `https://`; TLS itself is always the proxy's job.

## Onboarding a machine

### Agent-driven (recommended)

With your SSH key authorized on the hub machine, one command enrolls a
box: it runs `wtf key issue` remotely, receives the one-time secret inside
the ssh channel, writes `bridge.json` locally, and verifies the
credentials with a signed request:

```
git clone <this repo> && cd wtf-is-going-on-mcp
cargo build --release
./target/release/wtf join you@HUB-HOST --name laptop   # add --url to override the hub address
```

### Signed handshake (recommended: one secret per site)

The hub auto-generates a single site enrollment secret; the operator
prints it once and copies it to each joining machine — no ssh, no
per-device hand-copied key. The joiner proves possession of the secret
via HMAC (the secret never crosses the wire) and receives its device key
ML-KEM-768-sealed to its own encapsulation key, unwrapped only in
memory:

```
# on the hub (prints the secret + the ready-made join command)
./target/release/wtf enroll-secret            # or --json; --rotate invalidates all copies
# on the joining machine
./target/release/wtf enroll --url http://HUB-LAN-IP:7800 --name laptop --psk <SECRET>
```

Wrong secret, stale clock (>±300 s), replayed handshake, tampered
encapsulation key, and rotated-out copies all get the same uniform 403,
under the same global rate cap (20 attempts per 5 minutes). Hub and
joiner clocks must agree within ±5 minutes.

### Enrollment token (no ssh, single-use token)

The hub operator mints a one-time token; the joining machine redeems it
and receives its device key over that single call — no ssh access to the
hub, no secret copied by hand. The token is a 256-bit secret, stored
hashed (0600), expires on its own (`--ttl` seconds, default 600), can be
dropped early with `enroll-token revoke`, and burns on redemption:

```
# on the hub
./target/release/wtf enroll-token laptop                # prints the token ONCE (or --json)
# on the joining machine
./target/release/wtf enroll --url http://HUB-LAN-IP:7800 --name laptop --token <TOKEN>
```

Wrong, unknown, truncated, expired, and reused tokens all get the same
uniform 403; a typo does not burn the token, and a global cap (20 attempts
per 5 minutes) blunts online guessing.

### Manual

Issue the key on the hub, then configure the joining machine with it:

```
# on the hub
./target/release/wtf key issue laptop                  # prints the 64-hex device key ONCE
# on the joining machine
./target/release/wtf setup --url http://HUB-LAN-IP:7800 --name laptop --key <DEVICE_KEY>
```

All three paths end the same way: `bridge.json` (0600) exists and a signed
round-trip against the hub has succeeded. For automation,
`wtf key issue --json <name>` prints one machine-readable line:
`{"hub_url":…,"device":…,"key":…}`.

### Operator bin courier (no enrollment needed)

Bins are not just for agents: with the dashboard key you can copy/paste
content between machines and agents *before* any enrollment exists — the
same channel the dashboard uses. On any machine that has a `wtf` binary
(an empty `$WTF_HOME` is fine), run:

```
# paste content in from anywhere
WTF_DASHBOARD_KEY=<key> wtf bin put 1 "paste me into the other agent" --url http://HUB-LAN-IP:7800
# pull it back byte-exact on the other machine (pipe-friendly, no added newline)
WTF_DASHBOARD_KEY=<key> wtf bin get 1 --url http://HUB-LAN-IP:7800
WTF_DASHBOARD_KEY=<key> wtf bin ls  --url http://HUB-LAN-IP:7800
```

`put` also takes `--file F` or `-` (stdin); `get -o FILE` saves to a file
instead of printing. Hub URL and key also resolve from
`bridge.json`/`config.json` when those exist, so on an already-set-up
machine plain `wtf bin get 1` works. Prefer the `WTF_DASHBOARD_KEY` env
var over `--k` (argv can leak via shell history). The hub records
`dashboard` as the last writer; enrolled agents read the same bins via
`read_bin`. Never put secrets in a bin — the courier is for specs,
prompts, logs, URLs, and setup payloads.

## Connect any agent (any harness)

Any MCP-speaking agent — Claude Desktop, Cursor, Warp, Codex, a CI bot, or
a harness you wrote last night — points at this repo the same way. Agents
should read **`.agents/skills/wtf-agent-hub/SKILL.md`** (also mirrored to
sibling repos) — the canonical operating guide, written so an agent with
nothing but a terminal can go from clone to reporting in minutes. It
covers MCP wiring, etiquette, bin-based collaboration, the operator CLI,
topologies, troubleshooting, security rules, and a signed `curl` +
`openssl` fallback for environments without an MCP harness.

MCP harnesses (Claude Desktop, Cursor, Warp, and most clients) register
the bridge with the standard shape:

```json
{
  "mcpServers": {
    "wtf": {
      "command": "/absolute/path/to/target/release/wtf",
      "args": ["agent"]
    }
  }
}
```

Credentials come from `bridge.json` (written by `wtf join`/`wtf setup`);
env vars (`WTF_HUB_URL`, `WTF_DEVICE_NAME`, `WTF_DEVICE_KEY`) override it
and are the recommended delivery path for secret managers:

```
WTF_HUB_URL=http://HUB-LAN-IP:7800
WTF_DEVICE_NAME=laptop
WTF_DEVICE_KEY=<64 hex chars>
```

### Delivery via the PQC secrets lane

When your stack ships device keys through `pqc-secrets` (FIPS 203/204/205
bundle), a packed `WTF_<NAME>_SECRET` env var unwraps into the three
`WTF_*` vars without the key ever touching disk in plaintext:

```bash
export WTF_HUB_URL=http://HUB-LAN-IP:7800
export WTF_DEVICE_NAME=laptop
eval "$(pqc-secrets export | grep '^export WTF_LAPTOP_SECRET=')"
# then launch the MCP client as usual; the bridge reads WTF_* first
```

### Distribute the skill anywhere

The portable skill ships inside the binary. From any machine that has
`wtf`, drop the operating guide into any repo, project, or harness
workspace:

```bash
wtf skill install --dir /path/to/any/project   # -> <project>/.agents/skills/wtf-agent-hub/SKILL.md
wtf skill print                                # raw SKILL.md to stdout
```

Installs are idempotent (identical copies are a no-op); overwrite a drifted
file with `--force`. Point agents at `.agents/skills/wtf-agent-hub/SKILL.md`
and they know how to connect, report, and collaborate.

### Collaboration across agents and harnesses

Bins are the collaboration surface: one agent writes findings with
`write_bin`, tells the operator (or a peer via an event), and any other
agent — any machine, any harness — picks it up with `read_bin`. Typical
flows:

- **Operator → agent**: paste a spec on the dashboard, say *“work from
  bin N”*; the agent reads it before starting.
- **Agent → agent**: agent A writes its findings/context to a bin and
  logs `context in bin 2` to the event feed; agent B on another machine
  reads the bin and continues the task.
- **Agent → operator**: an agent drops a long report in a bin instead of
  flooding the event feed; the operator reads it on the dashboard.

## Agent etiquette

- **Chain-of-draft is the reporting format.** Every `check_in` and
`log_event` MUST be terse fragments, <=5 words per fragment, no prose
(e.g. `fixing auth replay bug; hub restarted; blocked on sshd`). The user
reads these on the dashboard to see what the fuck is going on.
- `check_in` when you start (`working` + short task), when blocked
(`blocked` + what you need), and when done (`done`).
- `log_event` for milestones and failures; `warn`/`error` levels exist for
a reason.
- `wtf_is_going_on` before starting work — another agent may already be on
it.
- When the operator says *“work from bin N”*, call `read_bin` with that N
before starting; bins hold pasted specs, logs, or whatever the operator
queued for you.
- **Bin hygiene**: `write_bin` replaces the whole bin (last writer wins) —
read it first, write whole-purposeful content, and log an event so peers
know the bin changed. Never put secrets in a bin; every device on the hub
can read them, and bins persist to disk.
- The bridge heartbeats automatically every 60 s while running. Keep
`task`/`details` short and secret-free; the whole network can read them.
## Encrypted agent-to-agent sessions

Dedicated private chat channels between agents on any machine or harness.
The hub stores only ML-KEM-768 sealed key packages and AES-256-GCM
ciphertext — it relays bytes and cannot read a single message.

- **Crypto (all in-tree, FIPS-aligned):** the creator holds a random
  256-bit session key and seals it to every member's ML-KEM-768 identity
  (FIPS 203). Messages are AES-256-GCM (SP 800-38D) with per-(session,
  sender) subkeys; the AEAD's AAD binds the session id, sender, and the
  hub-assigned monotonic sequence number, so replaying a ciphertext in any
  other slot fails closed. The hub stores session keys only in sealed form
  (`sessions.json`, 0600) and message content only as ciphertext.
- **Identity:** each bridge auto-generates an ML-KEM-768 keypair at
  `$WTF_HOME/identity.json` (0600) on first session use and registers the
  public half with the hub.
- **MCP tools:** `session_create`, `session_list`, `session_join`,
  `session_seal`, `session_send`, `session_read` — see
  `.agents/skills/wtf-agent-hub/SKILL.md` §6 for the flow.
- **COMMS ledger channels:** `comms_post` / `comms_read` add structure on
  top of the same encryption: small JSON envelopes (a ledger event type —
  `checkin`, `update`, `intent-merge`, `checkout`, `blocked`, `announce`,
  `handoff` — plus an optional `repo/branch` scope) carried as ordinary
  encrypted session messages and rendered as ledger lines. This is the
  fast cross-repo / cross-machine form of the `AGENTS/{date}.COMMS.md`
  protocol — and the ONLY surface where secrets may travel between agents
  (bins and the event feed are public).
- **Validation:** all NIST ACVP keygen/encapsulation/decapsulation KATs
  pass byte-exact, and the implementation cross-validates against
  pyca/cryptography (OpenSSL) and kyber-py.


## MCP tools

| Tool | Args | Purpose |
|------|------|---------|
| `check_in` | `status` (working/blocked/done/idle), `task`, `details?`, `agent?` | Report current status; shown as an agent card |
| `log_event` | `message`, `level?` (info/warn/error), `agent?` | Append to the shared event feed |
| `wtf_is_going_on` | `agent?` | Text snapshot of all agents + recent events |
| `read_bin` | `bin` (1-3) | Fetch bin content (use when told “work from bin N” or picking up a peer's handoff) |
| `write_bin` | `bin` (1-3), `content` | Publish content to a bin for other agents/machines (device-signed, attributed to your device) |
| `list_bins` | — | List bins with sizes and last-writer metadata |
| `ping` | — | Hub connectivity probe (unsigned `/healthz`) |
| `hub_info` | — | Which hub is this bridge connected to (URL, device, version); never exposes the dashboard key |
| `session_create` | `name` | Create an encrypted agent-to-agent channel (ML-KEM-768 sealed keys; hub stores ciphertext only) |
| `session_list` | — | List encrypted channels with member/message counts |
| `session_join` | `session` | Join with your ML-KEM-768 identity; decapsulates the sealed session key |
| `session_seal` | `session`, `member` | Creator: seal the session key to a member's identity |
| `session_send` | `session`, `message` | Send an encrypted message (AAD binds session/sender/seq) |
| `session_read` | `session`, `after?` | Read + decrypt new messages |
| `comms_post` | `session`, `event`, `note`, `scope?` | Post an encrypted COMMS ledger entry (cross-repo/cross-machine agent coordination; secrets allowed — e2e encrypted, hub stores ciphertext only) |
| `comms_read` | `session`, `after?`, `event?` | Read + decrypt new COMMS entries as ledger lines |

Tool failures (bad args, hub down, revoked key) are returned as
`isError: true` results, never as MCP protocol errors.

## HTTP API

| Route | Auth | Purpose |
|-------|------|---------|
| `GET /healthz` | none | Connectivity probe (version, uptime) |
| `GET /?k=KEY` | dashboard key | Dashboard page |
| `GET /stream?k=KEY` | dashboard key (device auth ok) | SSE state stream |
| `GET /api/v1/state` | dashboard key or device auth | Full state JSON (includes `bins`) |
| `GET /api/v1/bins` | dashboard key or device auth | All three paste-bins |
| `GET /api/v1/bins/N` | dashboard key or device auth | One paste-bin (N = 1-3) |
| `PUT /api/v1/bins/N` | dashboard key or device auth | Write a paste-bin: `{"content":"…"}` (max 65,536 chars; oversize is rejected, not truncated) |
| `POST /api/v1/checkin` | device auth | Upsert agent status |
| `POST /api/v1/event` | device auth | Append event |
| `POST /api/v1/heartbeat` | device auth | Liveness touch |
| `POST /api/v1/enroll` | none (token- or proof-gated, rate-limited) | Two modes: `{"name":…,"token":…}` → one-time `{"hub_url":…,"device":…,"key":…}`; or signed handshake `{"name":…,"ek":…,"ts":…,"nonce":…,"proof":…}` → `{"hub_url":…,"device":…,"ek_fp":…,"sealed":…}` with the device key ML-KEM-768-sealed, never plaintext |

Limits: 32 KiB head, 1 MiB body, 100 headers, 15 s read/write timeouts.
`Transfer-Encoding` requests are rejected `501` by design.

## Storage

All state lives in `$WTF_HOME` (default `~/.config/wtf-mcp`):

- `config.json` — hub bind address, port, dashboard key, optional advertised URL, site enroll secret (0600)
- `keys.json` — device records (0600)
- `bridge.json` — agent-side hub URL + credentials (0600)
- `enroll_tokens.json` — pending one-time enrollment tokens, hashed (0600)
- `bins.json` — shared paste-bins, content included (0600)
- `events.jsonl` — append-only log, rotates to `events.jsonl.old` at 10 MB
- `sessions.json` — session channels: members, ML-KEM-768 sealed key packages, ciphertext ring per channel — never plaintext (0600)
- `identity.json` — this bridge's ML-KEM-768 identity keypair (0600)
- `session_keys.json` — recovered session keys for joined channels (0600)

`wtf key revoke <name>` instantly disables a device; the hub picks up
issuance/revocation without a restart.

## Key management

```
wtf key issue [--json] <name>   # provision; secret printed exactly once
wtf key list                    # devices with created/revoked state
wtf key revoke <name>           # instant kill switch — no hub restart
```

- The hub hot-reloads the keystore: newly issued devices can check in
immediately, and revocation disables a device at once.
- Rotation = `revoke` + fresh `issue` with the same name (the old secret
stops working the moment it is revoked).
- If a device's key leaks, revoke first, investigate second.

## Upgrading

```
git pull && cargo build --release
# restart the hub (Ctrl-C / pkill, then wtf serve again) and re-launch agents
```

Config, keys, and history persist across upgrades; nothing is lost on
restart.

## CLI

```
wtf serve [--bind IP:PORT] [--no-open]
wtf key issue [--json] <name> | key list | key revoke <name>
wtf url [URL | clear]   # URL handed to joining devices (overlay/https aware)
wtf setup --url URL --name NAME --key KEY
wtf join user@hub [--name NAME] [--url URL]   # self-enroll over ssh
wtf enroll-token <name> [--ttl SECS] [--json] | enroll-token revoke <name>  # one-time token (hub side)
wtf enroll-secret [--rotate] [--json]           # site enrollment secret (hub side; rotate kills copies)
wtf enroll --url URL --name NAME --token TOKEN  # redeem a token to enroll this machine
wtf enroll --url URL --name NAME --psk SECRET   # signed-handshake enroll (key arrives sealed)
wtf bin ls [--url U] [--k K] [--json]           # operator courier: list paste-bins (dashboard key)
wtf bin get N [-o FILE] [--url U] [--k K]       # operator courier: print bin N raw (copy/paste channel)
wtf bin put N (TEXT | --file F | -) [--url U] [--k K] [--json]  # operator courier: write bin N
wtf agent        # MCP stdio server — what your MCP client launches
wtf status       # plain-text hub state (same formatter as the tool)
wtf dashboard-url # clickable dashboard URL (hub machine; never over MCP)
wtf skill install [--dir DIR] [--force] | skill print  # distribute the hub skill anywhere
wtf version
```

## Troubleshooting

| Symptom | Cause → fix |
|---------|-------------|
| HTTP 401 on a signed call | key revoked/wrong, clock off by >300 s, or signature input mismatch (exact path+query, raw 32-byte key as hex). Fresh nonce and retry. |
| Dashboard 401 | append `?k=<dashboard key>` — or run `wtf dashboard-url` on the hub machine for the exact link. |
| Connection refused | hub not running or wrong port — `curl http://localhost:7800/healthz`. |
| "cannot bind" on serve | port taken: `--bind IP:PORT` or free the port. |
| `wtf join` exit 127 | `wtf` is not on the hub host's PATH — install it there (see Build and install). |
| `wtf join` "did not return JSON" | the hub's `wtf` predates `key issue --json`; update the hub. |
| Other machines cannot reach a WSL2 hub | WSL is NAT'd: forward the port on the Windows host (`netsh interface portproxy add v4tov4 listenport=7800 connectport=7800 connectaddress=<WSL-IP>`) plus a firewall allow rule — or join both sides to an overlay. |

## Development

```
cargo test              # 91 unit tests + 11 e2e tests (real hub + real bridge over stdio)
cargo build --release   # lto, panic=abort, overflow checks
```

`tests/e2e.rs` starts a real hub on an ephemeral port, enrolls devices,
launches the real bridge binary, and drives it over stdio — the same flow
you will run across physical machines. For agents working *in this repo*:
read `AGENTS.md` (workflow rules), `llms.txt` (DOX contracts), and
`.agents/skills/wtf-observability/SKILL.md` (operating guide) before
editing.
