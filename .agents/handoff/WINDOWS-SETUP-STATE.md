# WINDOWS-1 (machine-2) SETUP STATE — HANDOFF TO MAC-AGENT

Date: 2026-09-01 · Author: windows-1 · Purpose: full setup parity for the
final coordination pass. Everything here is either public config or
name-references — **no secret values** (PQC bundle names only).

## 1. Machine

- Windows 11 host + WSL2 (Ubuntu), `/mnt/d/Code/<repo>` checkout surface.
- WSL NAT IP `172.30.170.141` (internal); **Windows host LAN `192.168.1.248`**
  with `netsh portproxy v4tov4 0.0.0.0:7800 → 172.30.170.141:7800` +
  inbound firewall rule `wtf-hub-7800` — hub reachable from LAN on the host IP.
- Hub `hub-2538554f` v0.14.2, port 7800, bind 0.0.0.0, loopback_only=false.

## 2. WTF MCP — singular bridge

- ONE bridge: `~/.local/bin/wtf agent` (wtf 0.14.2). One hub: `wtf serve`.
- Registrations pointing at that bridge:
  - omp global: `~/.omp/agent/mcp.json` (see handoff/omp-mcp.json)
  - hermes: registered MCP server `wtf`
  - fcc/claude: share the same bridge via MCP config
  - repo-scoped: `local-router/.mcp.json` (fixed from mac path → local, d36424d)
- Repo-scoped `.mcp.json` = sanctioned pattern for "another agent repo
  calling the MCP": same bridge binary, repo-tagged events, one hub
  connection per machine. New machines (VM/cloud) join via the signed PSK
  handshake (secret copied once by the operator, never crosses the wire;
  device key returns ML-KEM-768-sealed).

## 3. Harnesses → local-router (all four lanes green)

- Router: local-router v0.6.4 @ http://127.0.0.1:11434 (loopback), repo
  `/mnt/d/Code/local-router` @ d36424d, Ollama-native surface (fixed NDJSON
  newline bug in `createOllamaStreamTransform` — ollama CLI works natively).
- omp: default+advisor roles `local-router/local-router/fallback-models`;
  provider entry in `~/.omp/agent/models.yml` (handoff/omp-models.yml,
  modal tokens redacted — values live in the PQC bundle only).
- hermes: provider `local-router`, base_url `http://127.0.0.1:11434`,
  default `fallback-models` (handoff/hermes-config.yaml; key_env NONE).
- fcc: `ANTHROPIC_BASE_URL=http://127.0.0.1:11434` via env.sh (sourced in
  .bashrc); receipts FCC3-OK.
- ollama CLI: `OLLAMA_HOST=http://127.0.0.1:11434` → router; OLLAMA3-OK.
- Chain: 16 of mac's 21 steps resolvable here (5 catalog ids absent); mac's
  ordering kept. Key-gated steps verified directly (moonshot/wafer/openrouter).

## 4. PQC bundle state

- 18 secrets: 15 `LOCALROUTER_*` provider keys + `MODAL_PROXY_TOKEN` parts
  + `WTF_WINDOWS_AGENT_SECRET` (bridge key, also in bridge.json 0600).
- Imported via the NEW py envelope engine (a6154e0): ML-DSA-65 sig verified,
  recipient fingerprint matched, tamper test fails closed (17/17 roundtrip).
- Engine quirk found: `pqc-secrets pack` REPLACES the bundle (runbook says
  merge-safe — it is not). Repack-all pattern required; upstream fix owed.

## 5. Divergences from mac (deliberate)

- `tool-routing.json` kept LOCAL paths (`/home/nanoboozhoo/...`); mac's
  `/Users/...` paths break services mode here.
- `endpoint-models-cache.json` + curated ids differ slightly (catalog drift).

## 6. Waiting on mac

- Run `wtf federate add windows --url http://192.168.1.248:7800 --psk
  <secret from ops seq 12>` → restores your peer table + proactive push.
- Ack receipts in ops seq 14/16 + repo chat seq 29/30.
- Pull/adopt the NDJSON newline fix for `createOllamaStreamTransform`
  (stashed here; makes real ollama CLI + all hardcoded-ollama tools work).
