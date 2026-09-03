# FLEET — machines, hubs, model system

Verified: 2026-09-03 (mac and windows fleet re-verified)

## Machines

| Machine | Host | Hub | wtf | Router | Notes |
|---|---|---|---|---|---|
| windows (WSL2) | Windows host LAN `192.168.1.248` (WSL NAT `172.30.170.141` behind portproxy :7800) | `hub-2538554f` :7800, bind 0.0.0.0 | v0.15.0+ (opener-driven viewer, lifecycle tool, quiet logs) | local-router v0.6.4 on 127.0.0.1:11434, loopback | single bridge; omp/hermes/fcc + ollama CLI all route through it |
| mac | LAN `192.168.1.68` | `hub-799c0c4c` :7800 | v0.15.0+ (executor chat_run, R1 persistence) | local-router :11434 (all-harness fallback-models) | R1 identity persistence active; trae-cli/mini dual engine |

## Federation

- Peer edge: mac ↔ windows via site-secret handshake; windows side delivered
  via E2E ops chat (seq 12), portproxy + firewall installed on the host.
- Push parity verified both directions (canary #8560; pull <1-2 min each way).

## Model system (singular)

- ALL harnesses on BOTH machines → `local-router/fallback-models` @ :11434.
- Dynamic routing: `local-router` evaluates required context ($T_{\text{input}} + T_{\text{output}}$) and multimodal images for every inference call, bypassing models lacking context or vision capability, and retries through the eligible chain for 3 full rounds before terminal failure.
- mac chain: 21 steps (mac catalog). windows chain: 16/21 resolvable (5
  catalog ids absent); key-gated steps verified.
- ollama CLI: hardened shim (`fix/shim-always-route`) probes the router on
  EVERY invocation, auto-starts it detached if down, points OLLAMA_HOST at
  it, and treats `ollama serve` as "real ollama = backend on 11435".

## Known debt / open items

- `pqc-secrets pack` replaces the bundle (not merge-safe) — upstream fix owed.
