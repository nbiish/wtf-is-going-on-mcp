# FLEET — machines, hubs, model system

Verified: 2026-09-01 (windows side re-verified after mac pull; mac side per mac-agent seq 17-20)

## Machines

| Machine | Host | Hub | wtf | Router | Notes |
|---|---|---|---|---|---|
| windows (WSL2) | Windows host LAN `192.168.1.248` (WSL NAT `172.30.170.141` behind portproxy :7800) | `hub-2538554f` :7800, bind 0.0.0.0 | v0.15.0+ (opener-driven viewer, lifecycle tool, quiet logs) | local-router v0.6.4 on 127.0.0.1:11434, loopback | single bridge; omp/hermes/fcc + ollama CLI all route through it |
| mac | LAN `192.168.1.68` | `hub-799c0c4c` :7800 | v0.14.2 merged tree | local-router :11434 (state per mac-agent; always-route shim fix awaits pull of `fix/shim-always-route`) | hub restarts clear in-memory identity registry (R1) |

## Federation

- Peer edge: mac ↔ windows via site-secret handshake; windows side delivered
  via E2E ops chat (seq 12), portproxy + firewall installed on the host.
- Push parity verified both directions (canary #8560; pull <1-2 min each way).

## Model system (singular)

- ALL harnesses on BOTH machines → `local-router/fallback-models` @ :11434.
- mac chain: 21 steps (mac catalog). windows chain: 16/21 resolvable (5
  catalog ids absent); key-gated steps verified.
- ollama CLI: hardened shim (`fix/shim-always-route`) probes the router on
  EVERY invocation, auto-starts it detached if down, points OLLAMA_HOST at
  it, and treats `ollama serve` as "real ollama = backend on 11435".

## Known debt / open items

- R1: hub identity registry is in-memory; restarts force member re-joins.
- Router NDJSON newline fix is local to windows (`src/index.ts` in
  local-router); mac needs to pull it — ollama CLI NDJSON streams break
  without it.
- `pqc-secrets pack` replaces the bundle (not merge-safe) — upstream fix owed.
