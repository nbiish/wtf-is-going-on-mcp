# wtf-is-going-on-mcp

One command answers the eternal question across every machine you operate:
**what the fuck is going on?**

`wtf` is a zero-dependency Rust pair:

- **Hub** (`wtf serve`) — an HTTP API plus a live browser dashboard. Agents
  report what they are doing; you watch it in one page.
- **Bridge** (`wtf agent`) — an MCP stdio server that any MCP client launches.
  It exposes reporting tools (`check_in`, `log_event`, `wtf_is_going_on`,
  `ping`) and forwards everything to the hub over HMAC-signed requests.

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

Open the printed URL: `http://HUB:7800/?k=<dashboard key>` (the key lives
in `$WTF_HOME/config.json` and is reprinted by `wtf serve` on every start).

- Each reporting agent renders as a card: `● name [status]` with its
current task, details, and last-seen age (`●` fresh, `○` stale after a
few minutes of silence).
- The event feed lists everything agents have logged, newest first.
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
- **Transport topologies**: on a trusted LAN, plain HTTP is fine. Across
  machines or off-LAN, run an encrypted overlay (WireGuard/Tailscale) and point
  the bridge at the overlay address — no code changes; the HMAC signature
  still authenticates every request. Behind a TLS-terminating proxy,
  `https://` hub URLs are accepted. Raw port-forwarding plain HTTP to the
  public internet remains unsupported; TLS is the proxy's job, never
  hand-rolled here.
- **PQC-compatible key delivery**: credentials can be delivered via env vars
  (`WTF_HUB_URL`, `WTF_DEVICE_NAME`, `WTF_DEVICE_KEY`), which is the delivery
  path a PQC secrets bundle would use. The PQC (FIPS 203/204/205) lane is
  reserved for future secrets-at-rest features; none exist yet.

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

### Manual

Issue the key on the hub, then configure the joining machine with it:

```
# on the hub
./target/release/wtf key issue laptop                  # prints the 64-hex device key ONCE
# on the joining machine
./target/release/wtf setup --url http://HUB-LAN-IP:7800 --name laptop --key <DEVICE_KEY>
```

Both paths end the same way: `bridge.json` (0600) exists and a signed
round-trip against the hub has succeeded. For automation,
`wtf key issue --json <name>` prints one machine-readable line:
`{"hub_url":…,"device":…,"key":…}`.

## Wiring up agents

Agents should read **`.agents/skills/wtf-observability/SKILL.md`** — the
canonical operating guide, written so an agent with nothing but a terminal
can go from clone to reporting in minutes. It covers MCP wiring, etiquette,
the operator CLI, topologies, troubleshooting, security rules, and a
signed `curl` + `openssl` fallback for environments without an MCP
harness.

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

Credentials come from `bridge.json`; env vars (`WTF_HUB_URL`,
`WTF_DEVICE_NAME`, `WTF_DEVICE_KEY`) override it and are the recommended
delivery path for secret managers:

```
WTF_HUB_URL=http://HUB-LAN-IP:7800
WTF_DEVICE_NAME=laptop
WTF_DEVICE_KEY=<64 hex chars>
```

## Agent etiquette

- `check_in` when you start (`working` + short task), when blocked
(`blocked` + what you need), and when done (`done`).
- `log_event` for milestones and failures; `warn`/`error` levels exist for
a reason.
- `wtf_is_going_on` before starting work — another agent may already be on
it.
- The bridge heartbeats automatically every 60 s while running. Keep
`task`/`details` short and secret-free; the whole network can read them.

## MCP tools

| Tool | Args | Purpose |
|------|------|---------|
| `check_in` | `status` (working/blocked/done/idle), `task`, `details?`, `agent?` | Report current status; shown as an agent card |
| `log_event` | `message`, `level?` (info/warn/error), `agent?` | Append to the shared event feed |
| `wtf_is_going_on` | `agent?` | Text snapshot of all agents + recent events |
| `ping` | — | Hub connectivity probe (unsigned `/healthz`) |

Tool failures (bad args, hub down, revoked key) are returned as
`isError: true` results, never as MCP protocol errors.

## HTTP API

| Route | Auth | Purpose |
|-------|------|---------|
| `GET /healthz` | none | Connectivity probe (version, uptime) |
| `GET /?k=KEY` | dashboard key | Dashboard page |
| `GET /stream?k=KEY` | dashboard key (device auth ok) | SSE state stream |
| `GET /api/v1/state` | dashboard key or device auth | Full state JSON |
| `POST /api/v1/checkin` | device auth | Upsert agent status |
| `POST /api/v1/event` | device auth | Append event |
| `POST /api/v1/heartbeat` | device auth | Liveness touch |

Limits: 32 KiB head, 1 MiB body, 100 headers, 15 s read/write timeouts.
`Transfer-Encoding` requests are rejected `501` by design.

## Storage

All state lives in `$WTF_HOME` (default `~/.config/wtf-mcp`):

- `config.json` — hub bind address, port, dashboard key, optional advertised URL (0600)
- `keys.json` — device records (0600)
- `bridge.json` — agent-side hub URL + credentials (0600)
- `events.jsonl` — append-only log, rotates to `events.jsonl.old` at 10 MB

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
wtf agent        # MCP stdio server — what your MCP client launches
wtf status       # plain-text hub state (same formatter as the tool)
wtf version
```

## Troubleshooting

| Symptom | Cause → fix |
|---------|-------------|
| HTTP 401 on a signed call | key revoked/wrong, clock off by >300 s, or signature input mismatch (exact path+query, raw 32-byte key as hex). Fresh nonce and retry. |
| Dashboard 401 | append `?k=<dashboard key>` (see `config.json`). |
| Connection refused | hub not running or wrong port — `curl http://localhost:7800/healthz`. |
| "cannot bind" on serve | port taken: `--bind IP:PORT` or free the port. |
| `wtf join` exit 127 | `wtf` is not on the hub host's PATH — install it there (see Build and install). |
| `wtf join` "did not return JSON" | the hub's `wtf` predates `key issue --json`; update the hub. |
| Other machines cannot reach a WSL2 hub | WSL is NAT'd: forward the port on the Windows host (`netsh interface portproxy add v4tov4 listenport=7800 connectport=7800 connectaddress=<WSL-IP>`) plus a firewall allow rule — or join both sides to an overlay. |

## Development

```
cargo test              # 44 unit tests + 3 e2e tests (real hub + real bridge over stdio)
cargo build --release   # lto, panic=abort, overflow checks
```

`tests/e2e.rs` starts a real hub on an ephemeral port, enrolls devices,
launches the real bridge binary, and drives it over stdio — the same flow
you will run across physical machines. For agents working *in this repo*:
read `AGENTS.md` (workflow rules), `llms.txt` (DOX contracts), and
`.agents/skills/wtf-observability/SKILL.md` (operating guide) before
editing.
