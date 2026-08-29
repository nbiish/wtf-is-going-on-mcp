# wtf-is-going-on-mcp

One command answers the eternal question across every machine on your LAN:
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

## Quickstart — hub machine

```
cargo build --release
./target/release/wtf serve              # binds all interfaces, port 7800
./target/release/wtf key issue laptop   # prints the one-time device key
```

`wtf serve` prints the dashboard URL including the `?k=` key and opens your
browser. Useful flags: `--bind IP:PORT` (e.g. loopback only),
`--no-open`. If the hub is reached over an overlay or the internet, tell
joiners where to find it: `wtf url http://OVERLAY-IP:7800` (or the public
`https://` URL).

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

## Quickstart — machine 2 (agent-driven)

With your SSH key on the hub machine, one command enrolls the box: it runs
`wtf key issue` remotely, receives the one-time secret inside the ssh
channel, and verifies the credentials against the hub:

```
git clone <this repo> && cd wtf-is-going-on-mcp
cargo build --release
./target/release/wtf join you@HUB-LAN-IP --name laptop
```

Prefer the manual equivalent? Issue the key on the hub
(`wtf key issue laptop`) and run `setup` with the printed secret — both
paths write the same `bridge.json` (0600) and verify credentials with a
signed request. Then wire the bridge into your MCP client:

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

Env-var delivery (skips `bridge.json`; recommended for secret managers):

```
WTF_HUB_URL=http://HUB-LAN-IP:7800
WTF_DEVICE_NAME=laptop
WTF_DEVICE_KEY=<64 hex chars>
```

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

## CLI

```
wtf serve [--bind IP:PORT] [--no-open]
wtf key issue [--json] <name> | key list | key revoke <name>
wtf url [URL | clear]   # URL handed to joining devices (overlay/https aware)
wtf setup --url URL --name NAME --key KEY
wtf join user@hub [--name NAME] [--url URL]   # self-enroll over ssh
wtf agent        # MCP stdio server — what your MCP client launches
wtf status       # plain-text hub state (same formatter as the tool)
```

## Development

```
cargo test              # 44 unit tests + e2e (two-machine flow, join-style enroll)
cargo build --release   # lto, panic=abort, overflow checks
```

`tests/e2e.rs` starts a real hub on an ephemeral port, enrolls a device,
launches the real bridge binary, and drives it over stdio — the same flow
you will run across two physical machines. See `AGENTS.md` (workflow rules)
and `llms.txt` (DOX contracts) before editing.
