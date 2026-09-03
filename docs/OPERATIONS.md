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

