# OPERATIONS — recurring runbooks

Verified: 2026-09-01 (all procedures executed at least once today on the fleet)

## 1. Hub bring-up (any machine)

1. Build: `cargo build --release`; install `target/release/wtf ~/.local/bin/wtf`.
2. `wtf serve --no-open` (state under `$WTF_HOME`, default `~/.config/wtf-mcp`).
3. Enroll peers: `wtf enroll --url http://HUB:7800 --name N --psk SECRET`
   (secret printed once by `wtf enroll-secret` on the hub; key arrives
   ML-KEM-768-sealed).
4. Federate: `wtf federate add <peer> --url http://PEER:7800 --psk <peer-secret>`.
5. Wire the bridge into harnesses: `{command: ~/.local/bin/wtf, args: [agent]}`.

## 2. Federation repair (after a hub restart wipes the peer table)

1. The restarted hub loses `federation.json` peers only if the file was empty;
   check `jq '.peers' $WTF_HOME/federation.json`.
2. If empty: the OTHER side posts its site secret through the E2E ops chat;
   run `wtf federate add <name> --url <peer-url> --psk <secret>`.
3. WSL2 note: the hub binds inside WSL; expose it with
   `netsh interface portproxy add v4tov4 listenaddress=0.0.0.0 listenport=7800
   connectaddress=<WSL-IP> connectport=7800` + firewall rule. Give peers the
   HOST LAN IP, never the WSL NAT IP.
4. Verify: signed canary event pushes both ways within the 10s anti-entropy
   sweep; `curl http://<hub>/healthz` from the peer.

## 3. Secrets / config transfer across machines

`pqc-secrets envelope export --recipient <their-recipient.pub> --in keys.env`
→ send the envelope JSON through the E2E ops chat (or any channel; it is
sealed + signed). Import:
`pqc-secrets envelope import --in envelope.json` — verifies the ML-DSA-65
signature and recipient binding BEFORE decapsulation (fails closed).
Known quirk: `pqc-secrets pack` REPLACES the bundle — always repack ALL
keys together, never "merge" by packing one key.

## 4. Router lifecycle (always-route contract)

- The ollama shim at `~/.local/bin/ollama` probes
  `http://127.0.0.1:11434/api/version` on EVERY invocation. Router down ⇒
  start detached (`local-router start`), wait ≤10s, proceed.
- `ollama serve` ⇒ real ollama = backend on 11435; router front on 11434.
- Other invocations: `OLLAMA_HOST=127.0.0.1:11434` + exec real CLI (client
  of the router's Ollama-compatible API).
- Bypass hatch: `LOCAL_ROUTER_NO_SHIM=1`.
- After changing the shim generator: `local-router route set` to regenerate.

## 5. Log hygiene

- Heartbeats and replication churn are quiet events (presence only — never
  in the ring, feed, or log).
- Ring: 1000 events in memory; file: `events.jsonl`, rotates at 10 MB.
- Dashboard shows work; troubleshooting signal lives in the ring + log.

## 6. Agent chat lifecycle

- `chat_session_lifecycle {session, action: open|close|reconnect|delete}` —
  open on work start, close/delete when done, reconnect on hung pane.
- Chat viewer (dashboard click) is opener-driven: the dashboard page owns
  poll/send/scope; the viewer window is a pure display (no inline scripts).

## 7. Singular dashboard URL & Federated Multi-Machine Shell

1. Access URL: Run `wtf dashboard-url` on the hub machine to print the authoritative singular capability link (`http://<host>:<port>/w/<capability>`). Opens universally across loopback and LAN without query-string secret leakage.
2. Embedded Chat Studio: Manage lanes and prompt SWE-bench coding fleet agents (`trae-cli`, `mini`, `omp`, `fcc`) directly from the dashboard. Dispatches execute inside isolated tmux sessions routing through `local-router:11434`.
3. Federated Shell: Virtual cluster root (`~/`) maps to connected machines (`~/mac`, `~/windows`, `~/creeper-pi`). Execute cross-machine release orchestration in a single compound command:
   `cd ~/mac/frontend && npm test && cd ~/windows/backend && cargo test`

## 8. Federated OMP & Architecture LKGL Operations

1. Federated OMP Configuration: `$WTF_HOME/fed_omp_config.json` synchronizes shared model parameters (`local-router/fallback-models`), proxy endpoint (`http://127.0.0.1:11434/v1`), and the fallback cascade (`free-claude-code → omp → trae-cli → mini` or `fleet`).
2. Architecture LKGL (Last Known Good Location): `$WTF_HOME/lkgl.json` persists each machine's active repository directory across sessions. Commands executed in `~/mac` or `~/windows` automatically anchor to that architecture's native LKGL unless explicitly redirected.
3. Multi-Architecture Execution: The federated terminal resolves the target machine and executes commands at its LKGL. Compound pipelines allow operators on any device to orchestrate cross-architecture builds, tests, and ACP fleet operations in a single prompt.

## 9. Three-Pillar Graph Intelligence Operations

1. **GitNexus (AST & Code Symbol Integrity):**
   - Index the codebase: `gitnexus analyze --index-only`.
   - Active repository metrics: 1,389 nodes, 4,782 edges, 55 clusters, 122 call processes.
   - Pre-flight blast radius assessment: query `gitnexus_context` and `gitnexus_impact` before modifying exported contracts.
   - Post-edit verification: `gitnexus_detect_changes` confirms zero unintended call-site regressions.
2. **Graphify (Multimodal & Cross-Artifact Synthesis):**
   - Synthesizes code (`src/*.rs`), PRD contracts (`llms.txt`), runbooks (`docs/*.md`), and tasks (`.agents/tasks/`).
   - Leiden community clustering maps module boundaries and multi-agent interaction topologies.
3. **Semantica (Context & Governance Layer):**
   - Records immutable agent decision trees with W3C PROV-O provenance in `AGENTS/{date}.COMMS.md`.
   - Enforces cryptographic compliance: FIPS 203 ML-KEM-768 for secrets encapsulation, FIPS 204 ML-DSA-65 for signatures, and AES-256-GCM payloads.

## 10. Universal Single-Config AI Tooling Source (Ollama :11434) & Convergence

1. **Universal Endpoint Contract:**
   - Pre-configured standard endpoint `http://127.0.0.1:11434` proxies the backend daemon on `127.0.0.1:11435`.
   - Acts as the universal, PQC-secure single-configuration source for all AI tools and agent harnesses (Cursor, Warp, VS Code, Codex, Trae, Mini-SWE, OhMyPy, FreeClaudeCode, and edge devices).
   - Unifies three protocols under one loopback port:
     - **Ollama API:** `/api/generate`, `/api/chat`, `/api/tags`
     - **OpenAI-compatible API:** `/v1/chat/completions`, `/v1/models`
     - **Anthropic-compatible API:** `/v1/messages`
2. **Federated Agent "Brains" (`local-router/fallback-models`):**
   - Evaluates token context requirements ($T_{\text{input}} + T_{\text{output}}$) and multimodal visual inputs for every inference call.
   - Dynamically bypasses models lacking sufficient context or vision capabilities across 3 retry passes before terminal failure.
3. **Long-Term Convergence:**
   - Harmonizes `local-router` inference and `wtf` multi-agent observability into a singular sovereign application runtime.
   - `local-router` handles local inference and PQC key protection for both WTF federated workflows and standalone non-WTF applications; `wtf` handles observability, paste-bins, cluster coordination, and the federated shell.

## 11. Windows Fleet Node Synchronization Runbook

1. **Pull Latest Release Commits (WSL2):**
   ```bash
   cd /mnt/d/Code/wtf-is-going-on-mcp
   git pull origin main
   cargo build --release
   target/release/wtf serve --bind 0.0.0.0:7800 --no-open
   ```
2. **Verify Portproxy (Windows PowerShell as Admin):**
   ```powershell
   netsh interface portproxy show v4tov4
   # If WSL2 IP has rotated upon reboot:
   $wsl_ip = (wsl hostname -I).Trim().Split()[0]
   netsh interface portproxy add v4tov4 listenaddress=0.0.0.0 listenport=7800 connectaddress=$wsl_ip connectport=7800
   ```
3. **Verify Federated Shell Link:**
   - On Mac dashboard `/w/<capability>` or terminal:
     `cd ~/windows && cargo test`
   - Confirms bidirectional replication and cross-architecture compound execution.



