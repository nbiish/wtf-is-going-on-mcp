---
name: wtf-observability
description: Expert operation of the wtf multi-agent observability hub (this repo). Use when instructed to report status to the hub, check what other agents are doing, set up or join the hub on any machine, configure the wtf MCP server in an agent harness, or debug hub connectivity. Covers zero-install binary discovery, MCP client wiring, signed curl fallback for non-MCP agents, operator CLI, and multi-machine bring-up over LAN, overlay networks, or cloud.
---

# wtf-observability — agent skill

`wtf` is a zero-dependency Rust pair in this repo: a **hub** (`wtf serve`) that
keeps the shared truth of what every agent is doing, and a **bridge**
(`wtf agent`) that is a normal MCP stdio server. Agents report what they are
working on; a browser dashboard answers *what the fuck is going on* across all
machines. Everything is in-tree (SHA-256, HMAC, JSON, HTTP, MCP) — no crates,
no network installs, no system packages.

Non-negotiables before you touch anything: never log, echo, or commit device
keys or the dashboard key; never port-forward plain HTTP to the public
internet.

## 1. Get the binary (no pre-installed tooling required)

Try these in order; stop at the first that works:

```bash
command -v wtf                                    # already installed?
cargo build --release --manifest-path /path/to/wtf-is-going-on-mcp/Cargo.toml
#   binary appears at /path/to/wtf-is-going-on-mcp/target/release/wtf
```

- The build needs only a Rust toolchain and **nothing else**: there are zero
  external crates to download, so it works offline.
- If there is no toolchain on the machine, you cannot run the bridge — but you
  can still *read* the team state through a browser or `curl` if the operator
  gives you the dashboard URL (`http://HUB:7800/?k=KEY`).
- Prefer the release binary path in MCP configs; it must be **absolute**.

## 2. Verify connectivity (30 seconds)

```bash
curl -sS http://localhost:7800/healthz
#   {"ok":true,"service":"wtf-hub","version":"0.2.0",...}
/path/to/wtf status            # signed read; needs bridge.json or env creds
```

If `wtf status` prints the agent table, your credentials work end-to-end.
An empty table is success — it means nobody has checked in yet.

## 3. Preferred: report through MCP

Point your harness at the bridge (same `mcpServers` shape used by Claude
Desktop, Cursor, Warp, and most MCP clients):

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

The bridge reads credentials from `$WTF_HOME/bridge.json` (written by `wtf
setup` or `wtf join`) or, overriding the file, from `WTF_HUB_URL`,
`WTF_DEVICE_NAME`, `WTF_DEVICE_KEY`.

Tools you will have:

| Tool | Args | When to call |
|------|------|--------------|
| `check_in` | `status` (working/blocked/done/idle), `task` (required), `details?`, `agent?` | When you start work, change direction, get blocked, or finish |
| `log_event` | `message`, `level?` (info/warn/error), `agent?` | Notable milestones, decisions, failures |
| `wtf_is_going_on` | `agent?` | Before you start: see what other agents/machines are doing |
| `ping` | — | Connectivity probe (unsigned `/healthz`) |

Smoke-test the stdio server by hand (what MCP clients do under the hood):

```bash
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
  | /path/to/wtf agent
```

## 4. No MCP harness? Signed curl fallback

Any agent with `curl` + `openssl` can report. The hub authenticates every
request with HMAC-SHA256 over the canonical string
`wtf-hmac-v1\nMETHOD\npath-and-query\nTS\nNONCE\nsha256hex(body)` (no trailing
newline); the key is the device secret's **raw 32 bytes**, so use `-macopt
hexkey:` (not `-hmac`).

Load credentials (bridge.json is single-line JSON; no jq needed):

```bash
export WTF_HOME="${WTF_HOME:-$HOME/.config/wtf-mcp}"
export WTF_HUB_URL=$(sed -n 's/.*"hub_url":"\([^"]*\)".*/\1/p' "$WTF_HOME/bridge.json")
export WTF_DEVICE_NAME=$(sed -n 's/.*"device_name":"\([^"]*\)".*/\1/p' "$WTF_HOME/bridge.json")
export WTF_DEVICE_KEY=$(sed -n 's/.*"device_key":"\([^"]*\)".*/\1/p' "$WTF_HOME/bridge.json")
```

The one function to rule them all:

```bash
wtf_call() {  # wtf_call METHOD PATH [JSON_BODY]
  local method="$1" path="$2" body="${3:-}"
  local ts nonce bh sig
  ts=$(date +%s)
  nonce=$(head -c16 /dev/urandom | od -An -tx1 | tr -d ' \n')
  bh=$(printf '%s' "$body" | openssl dgst -sha256 -hex | awk '{print $NF}')
  sig=$(printf 'wtf-hmac-v1\n%s\n%s\n%s\n%s\n%s' "$method" "$path" "$ts" "$nonce" "$bh" \
        | openssl dgst -sha256 -mac hmac -macopt "hexkey:$WTF_DEVICE_KEY" -hex \
        | awk '{print $NF}')
  if [ -n "$body" ]; then
    curl -sS -X "$method" "$WTF_HUB_URL$path" -H "Content-Type: application/json" \
      -H "X-Wtf-Device: $WTF_DEVICE_NAME" -H "X-Wtf-Timestamp: $ts" \
      -H "X-Wtf-Nonce: $nonce" -H "X-Wtf-Signature: $sig" --data "$body"
  else
    curl -sS -X "$method" "$WTF_HUB_URL$path" \
      -H "X-Wtf-Device: $WTF_DEVICE_NAME" -H "X-Wtf-Timestamp: $ts" \
      -H "X-Wtf-Nonce: $nonce" -H "X-Wtf-Signature: $sig"
  fi
}
```

Usage — exactly one call each, expect `{"ok":true,"id":N}`:

```bash
wtf_call POST /api/v1/checkin '{"status":"working","task":"refactor parser","details":"phase 2"}'
wtf_call POST /api/v1/event   '{"message":"tests green","level":"info"}'
wtf_call POST /api/v1/heartbeat
wtf_call GET  /api/v1/state | head -c 400   # full team state JSON
```

Signatures cover the exact path+query, the body bytes, and a ±300 s clock
window; every request needs a fresh hex nonce. Get any of that wrong and you
get a uniform 401 — see §8.

## 5. Agent etiquette

- **Chain-of-draft is the reporting format.** Every `check_in` and
  `log_event` MUST be terse fragments, <=5 words per fragment, no prose
  (e.g. `fixing auth replay bug; hub restarted; blocked on sshd`). The user
  reads these on the dashboard to see what the fuck is going on.
- `check_in` when you start (`working` + short task), when blocked
  (`blocked` + what you need in `details`), and when done (`done`). The
  dashboard stale-marks agents silent for a few minutes.
- `log_event` for decisions and failures worth a line in the shared feed;
  `warn`/`error` levels exist for a reason.
- Call `wtf_is_going_on` before orchestrating or duplicating work — another
  agent may already be on it.
- Keep `task`/`details` short and secret-free. The dashboard shows them to
  every machine on the network.

## 6. Operator reference (hub side)

```bash
wtf serve [--bind IP:PORT] [--no-open]     # run hub; prints dashboard URL + key
wtf url [URL | clear]                      # URL handed to joining devices
wtf key issue [--json] <name>              # provision device; key printed ONCE
wtf key list                               # devices + revoked state
wtf key revoke <name>                      # instant kill switch, no restart
wtf setup --url URL --name N --key K       # manual local enrollment
wtf join user@hub-host [--name N] [--url U]# self-enroll over ssh
```

State lives in `$WTF_HOME` (default `~/.config/wtf-mcp`): `config.json`
(includes the dashboard key and the advertised URL), `keys.json`,
`bridge.json`, `events.jsonl` (append-only source of truth; survives
restarts). Set `WTF_HOME` to a temp dir to experiment without touching the
real deployment.

## 7. Bring up a new machine

1. **Hub machine**: build (§1), `wtf serve`. Off-LAN or cloud? Also set
   `wtf url http://OVERLAY-IP:7800` (overlay) or `wtf url
   https://hub.example.com` (VM behind a TLS-terminating proxy — the only
   sanctioned use of https).
2. **Joining machine**: get the repo and build (§1), then:
   - ssh path (preferred): `wtf join you@HUB-HOST --name <device>` — the
     one-time key travels only inside the ssh channel; requires your ssh key
     on the hub host and `wtf` resolvable in the hub's PATH.
   - manual path: run `wtf key issue <name>` on the hub and `wtf setup`
     locally with the printed secret.
3. Wire the bridge into the MCP client (§3) and `check_in` (§3/§4).

Topology notes: on a LAN, plain HTTP is the design. Across machines, install
an encrypted overlay (WireGuard/Tailscale) and aim everything at the overlay
address — zero code changes, no exposed ports. WSL2 caveat: the hub inside
WSL is NAT'd; other machines need a Windows `netsh interface portproxy` rule
plus firewall allow, or an overlay client on both sides.

## 8. Troubleshooting

| Symptom | Cause → fix |
|---------|-------------|
| HTTP 401 on signed call | key revoked/wrong, clock off by >300 s (`date +%s` vs hub), or signature input mismatch (verify hexkey:, no trailing newline, exact path). Generate a fresh nonce and retry. |
| 401 with `?k=` dashboard | key missing/typo'd; the serving hub's key is in its `config.json`. |
| connection refused | hub not running or wrong port; `curl localhost:7800/healthz`. |
| "cannot bind" on serve | port taken; `--bind localhost:PORT` or another port. |
| join exit 127 | `wtf` not in the hub host's PATH — install/symlink it there. |
| join "did not return JSON" | hub's `wtf` is older than `key issue --json`; update the hub. |
| tool `isError: true` | application-level failure (bad args, hub down, revoked key); the JSON-RPC layer is fine — fix the arguments or the hub. |

## 9. Security hard lines

- Device keys and the dashboard key are printed once and stored only in
  `0600` files under `$WTF_HOME`. Never put them in shell history files,
  task notes, commits, or MCP config JSON.
- A lost/leaked device: `wtf key revoke <name>` immediately; re-enroll with a
  fresh `key issue`.
- Never expose the hub via raw public port-forwarding. Encryption above the
  LAN is the overlay's or a TLS proxy's job, never this codebase's.

## 10. Self-check before declaring victory

```bash
curl -sS "$WTF_HUB_URL/healthz"                                   # ok:true
wtf status                                                        # your agent listed
wtf_call POST /api/v1/checkin '{"status":"working","task":"self-check"}'   # ok:true
```

All three passing means: hub up, credentials valid, signature path correct,
and you are visible to every other machine. Then actually use it — check in,
do the work, log what matters, check out with `done`.
