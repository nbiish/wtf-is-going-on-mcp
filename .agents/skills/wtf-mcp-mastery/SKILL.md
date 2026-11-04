---
name: wtf-mcp-mastery
description: The singular mastery skill for the wtf federated observability hub + MCP bridge (wtf-is-going-on-mcp). Covers connecting any AI agent harness or MCP client (Claude Desktop, Cursor, Warp, Codex, OpenCode, Cline, dsh, CI bots), zero-config join and PQC enrollment (FIPS 203 ML-KEM-768 burn-on-handshake), the singular capability dashboard (/w/<cap>) on obscure default port 47820 with auto-port fallback, agent tooling selection strictly narrowed to pi (Anthropic) and hermes (OpenAI), completions config for any OpenAI or Anthropic provider (defaulting to local-router localhost:11434), cross-machine federated pastebins (BIN 1-3) replicated across all cluster machines, 3 dedicated pastebins per agent team chat, federated encrypted chat lanes + COMMS ledger, and the paired Federated Multi-Machine Shell (~/ virtual cluster root with LKGL). wtf NEVER spawns local-router, fcc-server, or fcc-dsh and launches no agent-harness CLIs.
whenToUse: Use when setting up, joining, operating, or debugging the wtf hub/MCP on any machine; when wiring the wtf MCP server into an agent harness; when configuring agent tooling (hermes vs pi) or completions endpoints (OpenAI/Anthropic defaulting to local-router :11434); when sharing prompts or code through cross-machine federated pastebins or team chat pastebins; when coordinating agents through federated chat lanes, bins, or the COMMS ledger; when using the federated multi-machine shell.
---

# wtf-mcp-mastery — the singular skill for the wtf MCP

`wtf` is a zero-dependency Rust pair in the wtf-is-going-on-mcp repo: a **hub**
(`wtf serve`) that keeps the shared truth of what every agent is doing across
all machines, and an **MCP stdio bridge** (`wtf agent`) any harness can mount.
Everything is in-tree (SHA-256, HMAC, ML-KEM-768, AES-GCM, JSON, HTTP, MCP) —
no external crates, no network installs.

**Invariants (operator directives 2026-09-04 / 2026-09-07 / 2026-09-19):**
- wtf **NEVER spawns, stops, restarts, or supervises `local-router`, `fcc-server`, or `fcc-dsh`** — not as a process, not via `ensure`, not in the background. That infrastructure is installed and run by the operator; wtf only PASSIVELY probes `127.0.0.1:11434`.
- wtf **launches no agent-harness CLIs**. Cross-machine agent work happens through each machine's own tooling and configuration — wtf orchestrates, observes, and chats with the completions backend.
- Agent tooling is **strictly narrowed to `hermes` and `pi`**, with operator selection between them via MCP args, dashboard toggle, or `$WTF_HOME/completions.json`.
- The dashboard and hub serve on an **obscure default port `47820`** (to avoid collisions with standard tools like Gradio on 7800), with **automatic fallback** up to `+100` ports if occupied.
- Bins 1-3 on the dashboard are **cross-machine federated pastebins** replicated across all connected machines in the cluster mesh.
- Every agent team chat has **3 dedicated pastebins** (`chat:<session_id>`) for sharing code, instructions, and credentials.

Non-negotiables: never log, echo, or commit device keys or the dashboard capability token; never port-forward plain HTTP to the public internet.

## 1. Get the binary + join (zero-config)

```bash
command -v wtf                                    # already installed?
cargo build --release --manifest-path /path/to/wtf-is-going-on-mcp/Cargo.toml
# binary: /path/to/wtf-is-going-on-mcp/target/release/wtf (absolute path in MCP configs)

curl -sS http://HUB:47820/healthz                 # {"ok":true,"service":"wtf-hub","version":"..."}
wtf dashboard-url                                 # singular capability link: http://<host>:47820/w/<64-hex>
wtf join user@hub-host [--name N]                 # self-enroll over ssh
wtf enroll-token <name> [--ttl SECS]              # mint one-time token (burned on use)
wtf enroll --url http://<hub>:47820 --name N --psk <hub's wtf enroll-secret>
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

## 3. Tools (v0.16.0)

| Tool | Args | When |
|------|------|------|
| `check_in` | `status`, `task`, `details?`, `agent?`, `repo?` | Lifecycle: working/blocked/done/idle (chain-of-draft <=5 words) |
| `log_event` | `message`, `level?`, `agent?`, `repo?` | Milestones, decisions, failures (chain-of-draft <=5 words) |
| `wtf_is_going_on` | `agent?` | Orientation: who is doing what, everywhere across machines |
| `read_bin` / `write_bin` / `list_bins` | `bin` (1-3), `content?`, `scope?` | Scoped pastebins: global `user` (replicated) / `chat:<id>` (team chat) / `machine:<name>` |
| `hub_info` | — | Hub URL + device + version, no secrets |
| `env_report` / `env_probe` | — | This machine's agent tooling (`hermes`/`pi`), provider, os/arch; all devices' reports |
| `ping` | — | Unsigned `/healthz` probe |
| `session_create` | `name`, `repo?` | Federated encrypted lane; pairing key minted once |
| `session_list` / `session_join` | `repo?` / `session`, `pairing` | Discover + join lanes |
| `session_seal` / `session_send` / `session_read` | — | ML-KEM-768-sealed member messaging |
| `comms_post` / `comms_read` | `session`, `event`, `note`, `scope?` | Structured COMMS ledger (checkin/update/intent-merge/checkout/handoff) |
| **`chat`** | `prompt`, `model?`, `system?`, `agent?`, `provider?` | **Completion via configured agent (`hermes` or `pi`) and provider (OpenAI or Anthropic), defaulting to local-router :11434** |
| `router_status` | — | Passive :11434 probe (alive, version, models count) |

## 4. Tooling Agent Selection (`hermes` vs `pi`) & Completions Config

Agentic work is strictly narrowed to two tooling agents:
- **`hermes`**: OpenAI-compatible chat protocol (completions, tool call schemas, JSON output).
- **`pi`**: Anthropic-compatible messages protocol (`/v1/messages`, `x-api-key`, system prompt separation).

### Completions Configuration
Configurations persist in `$WTF_HOME/completions.json`:
```json
{
  "provider": "openai",
  "endpoint": "http://localhost:11434",
  "api_key": "",
  "model": "",
  "selected_agent": "hermes",
  "system_prompt": ""
}
```

- **Default Endpoint:** `http://localhost:11434` (the operator-run local-router Ollama/OpenAI proxy).
- **Auto-Discovery:** When `model` is blank, wtf queries the router catalog and prefers `llama.cpp qwen3 abliterated` → any `qwen3` → any `llama.cpp` build.
- **Provider Override:** You can point to official OpenAI (`https://api.openai.com/v1`), Anthropic (`https://api.anthropic.com`), or any custom endpoint with an API key.
- **Switching Agents:**
  - MCP: pass `agent: "hermes"` or `agent: "pi"` in `chat`.
  - Dashboard: click the header toggle pill (`⚡ Agent: hermes`) or open `⚙ Config`.
  - API: `POST /api/v1/completions/config` with `{"selected_agent": "pi", "provider": "anthropic"}`.

## 5. Cross-Machine Federated Pastebins & Team Chat Pastebins

### Global Cross-Machine Federated Pastebin (BIN 1-3)
- Located on the main dashboard and accessed via `read_bin`/`write_bin` with `scope: "user"` (or omitting `scope`).
- **Federated Replication:** Changes to BIN 1, BIN 2, or BIN 3 automatically replicate across all connected machines in the federated cluster using Last-Write-Wins timestamps.
- **Use Case:** Copy-paste findings, shared credentials, prompt fragments, and reference context between machines without manual file transfers.

### Dedicated Team Chat Pastebins (3 Bins per Chat)
- Every agent team chat lane (`session_create`) gets **3 dedicated pastebins** scoped to `chat:<session_id>`.
- Accessible directly in the Chat Studio pane via tabs `[Bin 1]`, `[Bin 2]`, `[Bin 3]` with instant 1-click `Copy` and `Save`.
- Agents in that chat lane read and write these bins using `read_bin {bin: 1, scope: "chat:<session_id>"}` and `write_bin {bin: 1, content: "...", scope: "chat:<session_id>"}`.

## 6. Obscure Default Port (47820) & Port Auto-Fallback

- Default listening port is **`47820`** (chosen specifically to avoid conflicts with common web frameworks, dev servers, or tools like Gradio on 7800).
- If port `47820` is already in use by another service on any machine, wtf automatically attempts consecutive ports up to `47820 + 100` (`47920`).
- The active port is clearly displayed in the server logs, CLI messages, and the dashboard header pill (`● :47820`).

## 7. Federated Multi-Machine Shell

Virtual `~/` cluster root (`~/mac`, `~/windows`, …) in the dashboard's right
pane: LKGL anchoring (`$WTF_HOME/lkgl.json`), compute tiers, and compound
cross-machine prompts (`cd ~/mac/frontend && npm test && cd ~/windows && cargo test`). Execution goes through the machines' own shells over the
federation — again, no harness launching, no router spawning.

## 8. No MCP harness? Signed curl fallback

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

# example: chat via hermes
wtf_call POST /api/v1/chat/completions '{"prompt":"summarize status","agent":"hermes"}'
```

## 9. Operator CLI + Dashboard

`wtf serve [--bind IP:PORT] [--no-open]` · `wtf dashboard-url` · `wtf url` ·
`wtf key issue|list|revoke` · `wtf enroll-token` / `wtf enroll --psk` /
`wtf enroll-secret [--rotate]` · `wtf setup` / `wtf join` · `wtf status` ·
`wtf bin ls|get|put` · `wtf skill install|print` · `wtf federate add|list|remove`.

Dashboard: single `/w/<capability>` page (uniform-404 security), Chat Studio
pane (with Hermes/Pi agent selection, completions config, and 3 team chat bins),
cross-machine federated pastebin (BIN 1-3), federated shell, live agent cards
grouped by origin hub, passive router status pill.

## 10. Troubleshooting

| Symptom | Fix |
|---|---|
| chat fails "local-router offline" | Operator starts the router (`local-router start`); wtf never will. |
| HTTP 401 on signed call | key revoked/wrong, clock off by >300 s, signature input mismatch |
| HTTP 404 on `/` or `/w/...` | capability token missing/wrong — `wtf dashboard-url` on the hub |
| connection refused | hub not running: `wtf serve` on the hub machine |
| port busy on startup | wtf automatically falls back to an open port between `47820` and `47920` |
| bin content too large | bins cap at 64 KiB; split or shrink |
| model:id in reply looks wrong | check the router's `/v1/models`; configure explicit `model` in `⚙ Config` or MCP args |
