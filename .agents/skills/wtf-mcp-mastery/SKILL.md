---
name: wtf-mcp-mastery
description: The singular mastery skill for the wtf federated observability hub + MCP bridge (wtf-is-going-on-mcp). Covers connecting any AI agent harness or MCP client (Claude Desktop, Cursor, Warp, Codex, OpenCode, Cline, dsh, CI bots), zero-config join and PQC enrollment (FIPS 203 ML-KEM-768 burn-on-handshake), the singular capability dashboard (/w/<cap>), federated encrypted chat lanes + COMMS ledger + scoped bins, the paired Federated Multi-Machine Shell (~/ virtual cluster root with LKGL), and ROUTER CHAT — OpenAI-compatible chat completions against the operator-run local-router (127.0.0.1:11434/v1) with live model discovery that prefers the llama.cpp qwen3 abliterated tier. wtf NEVER spawns local-router, fcc-server, or fcc-dsh and launches no agent-harness CLIs.
whenToUse: Use when setting up, joining, operating, or debugging the wtf hub/MCP on any machine; when wiring the wtf MCP server into an agent harness; when chatting with the local-router brain from wtf (MCP chat tool, dashboard studio, or signed curl); when coordinating agents through federated chat lanes, bins, or the COMMS ledger; when using the federated multi-machine shell.
---

# wtf-mcp-mastery — the one skill for the wtf MCP

`wtf` is a zero-dependency Rust pair in the wtf-is-going-on-mcp repo: a **hub**
(`wtf serve`) that keeps the shared truth of what every agent is doing across
all machines, and an **MCP stdio bridge** (`wtf agent`) any harness can mount.
Everything is in-tree (SHA-256, HMAC, ML-KEM-768, AES-GCM, JSON, HTTP, MCP) —
no external crates, no network installs.

**Invariants (operator directives 2026-09-04 / 2026-09-07):**
- wtf **NEVER spawns, stops, restarts, or supervises `local-router`, `fcc-server`, or `fcc-dsh`** — not as a process, not via `ensure`, not in the background. That infrastructure is installed and run by the operator; wtf only PASSIVELY probes `127.0.0.1:11434`.
- wtf **launches no agent-harness CLIs**. Cross-machine agent work happens through each machine's own tooling and configuration — wtf orchestrates, observes, and chats with the router brain.

Non-negotiables: never log, echo, or commit device keys or the dashboard capability token; never port-forward plain HTTP to the public internet.

## 1. Get the binary + join (zero-config)

```bash
command -v wtf                                    # already installed?
cargo build --release --manifest-path /path/to/wtf-is-going-on-mcp/Cargo.toml
# binary: /path/to/wtf-is-going-on-mcp/target/release/wtf (absolute path in MCP configs)

curl -sS http://HUB:7800/healthz                  # {"ok":true,"service":"wtf-hub","version":"..."}
wtf dashboard-url                                 # singular capability link: http://<host>:7800/w/<64-hex>
wtf join user@hub-host [--name N]                 # self-enroll over ssh
wtf enroll-token <name> [--ttl SECS]              # mint one-time token (burned on use)
wtf enroll --url http://<hub>:7800 --name N --psk <hub's wtf enroll-secret>
wtf setup --url URL --name N --key K              # manual enrollment with an issued key
```

Enrollment is a ONE-TIME handshake (hashed TTL tokens or ML-KEM-768-sealed PSK
delivery); identities persist in `$WTF_HOME/identities.json` (0600) and
rehydrate across restarts — requiring re-enrollment is a bug. Keys print ONCE.

## 2. Wire the MCP server (any harness)

```json
{ "mcpServers": { "wtf": { "command": "/absolute/path/to/target/release/wtf", "args": ["agent"] } } }
```

Credentials come from `$WTF_HOME/bridge.json` (written by `wtf setup`/`join`/
`enroll`) or env `WTF_HUB_URL` / `WTF_DEVICE_NAME` / `WTF_DEVICE_KEY`.

## 3. Tools (v0.16.0 — 20)

| Tool | Args | When |
|------|------|------|
| `check_in` | `status`, `task`, `details?`, `agent?`, `repo?` | Lifecycle: working/blocked/done/idle |
| `log_event` | `message`, `level?`, `agent?`, `repo?` | Milestones, decisions, failures |
| `wtf_is_going_on` | `agent?` | Orientation: who is doing what, everywhere |
| `read_bin` / `write_bin` / `list_bins` | `bin` (1-3), `content?`, `scope?` | Scoped paste-bins: user / `chat:<id>` / `machine:<name>` |
| `hub_info` | — | Hub URL + device + version, no secrets |
| `env_report` / `env_probe` | — | This machine's os/arch/wtf version; all devices' reports |
| `ping` | — | Unsigned `/healthz` probe |
| `session_create` | `name`, `repo?` | Federated encrypted lane; pairing key minted once |
| `session_list` / `session_join` | `repo?` / `session`, `pairing` | Discover + join lanes |
| `session_seal` / `session_send` / `session_read` | — | ML-KEM-768-sealed member messaging |
| `comms_post` / `comms_read` | `session`, `event`, `note`, `scope?` | Structured COMMS ledger (checkin/update/intent-merge/checkout/handoff) |
| **`chat`** | `prompt`, `model?`, `system?` | **One OpenAI completion via the operator-run local-router** |
| `router_status` | — | Passive :11434 probe (alive, version, models) |

## 4. ROUTER CHAT — the core capability

The machine running wtf talks to the operator-run local-router's
OpenAI-compatible endpoint (`http://127.0.0.1:11434/v1`):

- **MCP:** `chat {prompt, model?, system?}` → one completion, `model:` named in the reply.
- **Dashboard:** the Studio chat pane POSTs `/api/v1/chat/completions {prompt, model?, system?}` (capability-gated) and renders the reply.
- **Signed curl:** `wtf_call POST /api/v1/chat/completions '{"prompt":"..."}'` (helper below).

**Model discovery (v0.15.4+, unchanged):** when `model` is omitted, the hub
queries the router's live `/v1/models` and prefers **llama.cpp qwen3
abliterated** → any qwen3 → any llama.cpp build → `local-router/fallback-models`.

**Fail-closed contract:** if :11434 is down, chat fails with
`local-router offline on 127.0.0.1:11434 — start it yourself (operator-run);
wtf never spawns it`. Starting the router is ALWAYS the operator's move.

## 5. Federated chat lanes, COMMS + bins (cross-machine coordination)

1. **One lane = one project** (`session_create {name, repo}`); members join
   from any machine with the pairing key; all traffic is E2E-encrypted
   (ML-KEM-768 seals, AES-256-GCM messages, hub stores ciphertext only).
2. Agent-to-agent flow: post progress via `session_send`/`comms_post`; peers
   poll `session_read`/`comms_read` and execute handed-off work with **their
   own machine's tooling** — wtf never runs it for them.
3. Bins are scoped couriers (user / `chat:<id>` / `machine:<name>`), 64 KiB
   each, rejected (never truncated) when oversize.
4. COMMS ledger vocabulary: `checkin → update → intent-merge → checkout`
   (+ `handoff`, `blocked`, `announce`); chain-of-draft, terse, no secrets.

## 6. Federated Multi-Machine Shell

Virtual `~/` cluster root (`~/mac`, `~/windows`, …) in the dashboard's right
pane: LKGL anchoring (`$WTF_HOME/lkgl.json`), compute tiers, and compound
cross-machine prompts (`cd ~/mac/frontend && npm test && cd ~/windows &&
cargo test`). Execution goes through the machines' own shells over the
federation — again, no harness launching, no router spawning.

## 7. No MCP harness? Signed curl fallback

HMAC-SHA256 over `wtf-hmac-v1\nMETHOD\npath-and-query\nTS\nNONCE\nsha256hex(body)`
(no trailing newline); key = device secret's raw 32 bytes (`-macopt hexkey:`).
Load creds from `$WTF_HOME/bridge.json` and use one helper:

```bash
wtf_call() { local m=$1 p=$2 b=${3:-}; local ts n bh sig
  ts=$(date +%s); n=$(head -c16 /dev/urandom | od -An -tx1 | tr -d ' \n')
  bh=$(printf '%s' "$b" | openssl dgst -sha256 -hex | awk '{print $NF}')
  sig=$(printf 'wtf-hmac-v1\n%s\n%s\n%s\n%s\n%s' "$m" "$p" "$ts" "$n" "$bh" \
        | openssl dgst -sha256 -mac hmac -macopt "hexkey:$WTF_DEVICE_KEY" -hex | awk '{print $NF}')
  if [ -n "$b" ]; then curl -sS -X "$m" "$WTF_HUB_URL$p" -H "Content-Type: application/json" \
    -H "X-Wtf-Device: $WTF_DEVICE_NAME" -H "X-Wtf-Timestamp: $ts" -H "X-Wtf-Nonce: $n" \
    -H "X-Wtf-Signature: $sig" --data "$b"; else curl -sS "$WTF_HUB_URL$p" \
    -H "X-Wtf-Device: $WTF_DEVICE_NAME" -H "X-Wtf-Timestamp: $ts" -H "X-Wtf-Nonce: $n" \
    -H "X-Wtf-Signature: $sig"; fi; }
# example: router chat
wtf_call POST /api/v1/chat/completions '{"prompt":"summarize the hub state"}'
```

## 8. Operator CLI + dashboard

`wtf serve [--bind IP:PORT] [--no-open]` · `wtf dashboard-url` · `wtf url` ·
`wtf key issue|list|revoke` · `wtf enroll-token` / `wtf enroll --psk` /
`wtf enroll-secret [--rotate]` · `wtf setup` / `wtf join` · `wtf status` ·
`wtf bin ls|get|put` · `wtf skill install|print` · `wtf federate add|list|remove`.
Dashboard: single `/w/<capability>` page (uniform-404 security), Studio chat
pane (router chat), lane viewer with bins, federated shell, live agent cards
grouped by origin hub, passive router status pill.

## 9. Troubleshooting

| Symptom | Fix |
|---|---|
| chat fails "local-router offline" | Operator starts the router (`local-router start`); wtf never will. |
| HTTP 401 on signed call | key revoked/wrong, clock off by >300 s, signature input mismatch |
| HTTP 404 on `/` or `/w/...` | capability token missing/wrong — `wtf dashboard-url` on the hub |
| connection refused | hub not running: `wtf serve` on the hub machine |
| bin content too large | bins cap at 64 KiB; split or shrink |
| model:id in reply looks wrong | check the router's `/v1/models`; pass explicit `model` to override discovery |

## 10. Self-check

```bash
curl -sS "$WTF_HUB_URL/healthz"          # ok:true, version
wtf status                               # your agent listed
wtf_call POST /api/v1/chat/completions '{"prompt":"ping"}'   # ok:true, model named
```
