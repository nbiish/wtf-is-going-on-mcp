# FLEET — machines, hubs, model system

Verified: 2026-11-22 (Mac hub, Windows client, WSL client).

## Machines

| Machine | Host | Role | wtf | Notes |
|---|---|---|---|---|
| mac `in-DIGI-nous` | LAN `192.168.1.68` | Hub `hub-19188245` on `0.0.0.0:7800` | v0.20.0 once this branch is installed; the running hub on 2026-11-22 morning was v0.19.0 | LaunchAgent `dev.wtf.hub`. Devices `mac`, `windows`, `wsl`. Dashboard `/w/<capability>`. |
| windows `PEOPLEOFTHEDAWN` | `peopleofthedawn.local` ssh alias `windows`, user `kenwa` | Enrolled device `windows`. Nothing listens on Windows `:7800` | v0.19.0 at `C:\Users\kenwa\bin\wtf.exe` until this branch is copied | Bridge points at `http://192.168.1.68:7800`. |
| WSL on the same Windows host | ssh alias `wsl`, user `nanoboozhoo`, port `2222` | Enrolled device `wsl` | bridge file present; `wtf` was not on PATH | No Herdr, no Orca, no wtf skill installed in that home. |

## Federation

- One hub, on the Mac. `wtf federate list` on 2026-11-22 reported no peer hubs.
- Windows and WSL are devices of `hub-19188245`, not a second server.
- SSH aliases used for transport (not stored as keys): `windows` → `kenwa@peopleofthedawn.local:22`, `wsl` → `nanoboozhoo@peopleofthedawn.local:2222`. The OpenSSH hop to Windows did not negotiate a post-quantum key exchange.

## Model system (singular universal 11434 hub)

- ALL harnesses on ALL machines (Cursor, Warp, VS Code, Codex, Trae, Mini-SWE, OhMyPy, FreeClaudeCode, edge devices) → `local-router/fallback-models` @ :11434.
- `local-router` serves as the universal, PQC-secure single-configuration source for Ollama (`/api`), OpenAI (`/v1`), and Anthropic (`/v1`) formats, proxying the real Ollama daemon on port 11435.
- Dynamic routing: `local-router` evaluates required context ($T_{\text{input}} + T_{\text{output}}$) and multimodal images for every inference call, bypassing models lacking context or vision capability, and retries through the eligible chain for 3 full rounds before terminal failure.
- mac chain: 21 steps (mac catalog). windows chain: 16/21 resolvable (5
  catalog ids absent); key-gated steps verified.
- ollama CLI: hardened shim (`fix/shim-always-route`) probes the router on
  EVERY invocation, auto-starts it detached if down, points OLLAMA_HOST at
  it, and treats `ollama serve` as "real ollama = backend on 11435".

## Known debt / open items

- `pqc-secrets pack` replaces the bundle (not merge-safe) — upstream fix owed.
