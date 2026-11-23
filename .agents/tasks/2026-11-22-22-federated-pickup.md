# TASK 2026-11-22 — federated pickup for goals, tasks, repos, and machine skills

- start: 2026-11-22T22:37:14Z · end: 2026-11-23T01:26:00Z
- **scope:** `src/fleet.rs` (new), `src/mcp.rs`, `src/api.rs`, `src/dashboard.rs`, `src/main.rs`, `src/config.rs`, `src/store.rs`, `src/lib.rs`, `tests/e2e.rs`, `skill/wtf-is-going-on-mcp.md`, `docs/FLEET.md`, `README.md`, `llms.txt`, `~/.cursor/mcp.json`, Windows `D:\Code\wtf-is-going-on-mcp`
- **branch:** `feat/federated-pickup` · **worktree:** `/Volumes/1tb-sandisk/code-external/wtf-federated-pickup`
- **lane:** now · **release:** v0.20.0
- **status:** done

## Objective

Consolidate scattered cross-machine skills, goals, tasks, repos, and SSH aliases into an agents-first MCP tool (`fleet_pickup`), CLI suite (`wtf goal`, `wtf alias`), and live browser dashboard card (`PICKUP`). Allow agents across Mac and Windows to immediately resume work where they left off, knowing which machines have which external tools (`browsermcp`, `agent-desktop`, `browser-skill`, `herdr`, `orca`).

## Acceptance

- [x] New module `src/fleet.rs` managing `FleetStore`, `Goal`, `SshAlias`, `probe_skills`, and keyless security checks (`forbidden_keys`).
- [x] MCP tools `fleet_pickup`, `goal_add`, `goal_done`, `alias_set` in `src/mcp.rs` (24 -> 27 tools).
- [x] REST API endpoints `/api/v1/fleet`, `/api/v1/goals`, `/api/v1/goals/done`, `/api/v1/aliases` in `src/api.rs`.
- [x] Live Dashboard `PICKUP` card rendering devices, goals, repos, SSH aliases, and external skills at `/w/<capability>`.
- [x] CLI commands `wtf goal` and `wtf alias`.
- [x] All 135 unit tests and 14 E2E tests green on Mac and Windows.
- [x] Mac release binary installed to `~/.local/bin/wtf` (`v0.20.0`) and live LaunchAgent verified.
- [x] Windows release binary compiled and installed to `C:\Users\kenwa\bin\wtf.exe` (`v0.20.0`) and verified via SSH.
- [x] Retired obsolete skills (`wtf-agent-hub`, `wtf-observability`, `wtf-mcp-mastery`) on Mac and Windows.
- [x] Deployed canonical `wtf-is-going-on-mcp` skill to `skills-to-distribute`.
- [x] Wired `wtf agent` in `~/.cursor/mcp.json`.

## Verification

- `cargo test`: 135 unit tests, 14 e2e tests passing on Mac; 135 unit tests, 13 e2e tests passing on Windows.
- `wtf alias list`: displays configured SSH aliases (`mac`, `windows`, `wsl`) without leaking private keys or paths.
- `ssh windows 'wtf.exe status'`: confirms Windows client connects to Mac hub and reads live federated state.
- `shasum -a 256`: SHA-256 `4d17a74215300def32578f5e42237808d683edaaf79ebd36ffbcb78aa2ff9718` identical across repo and distro copies.
