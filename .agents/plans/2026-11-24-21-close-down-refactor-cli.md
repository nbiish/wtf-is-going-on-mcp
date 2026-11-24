# Plan — Close Down Hub/MCP Stack & Refactor `wtf` into Stateless SSH/Local CLI

- **date:** 2026-11-24
- **branch:** `refactor/stateless-ssh-cli`
- **worktree:** `/Volumes/1tb-sandisk/code-external/wtf-refactor-cli`
- **status:** done

## Objective

Decommission the long-running `wtf serve` hub daemon, MCP stdio bridge (`wtf agent`), PQC session/federation/task-dispatch/runner/router-chat stack across Mac, Windows, and WSL, clean up stale working trees and worktrees on both machines, preserve unmerged Windows work on archive branches (`v0.20.0-archive`, `archive/windows-connection-discovery-2026-11-24`, `archive/windows-wip-2026-11-03`), and refactor the codebase into a lean, zero-dependency stateless SSH/local status, goals, and pickup CLI (`v0.21.0`).

## Clarifications & Decisions

1. **Target architecture:** Stateless SSH/local status CLI — drop hub daemon, HTTP server/client, MCP bridge, PQC crypto, federation, sessions, task dispatch, runner, and router chat; keep zero external crates in `Cargo.toml`.
2. **Preserved state:** Keep `$WTF_HOME/fleet.json` (goals + keyless SSH aliases) on Mac; remove all legacy hub/bridge/key/event files across Mac, Windows, and WSL.
3. **Unmerged Windows work:** Archived to `origin` before removing `/mnt/d/Code/wtf-conn` and deleting local branches.
4. **Remote SSH probing:** Build `ssh` argv arrays explicitly (`BatchMode=yes`, `ConnectTimeout=5`, `-p <port>`, `<user>@<host>`, `--`, `hostname`) under an 8s wall-clock cap; no shell interpolation.

## Todo

- [x] Tag `origin/main` as `v0.20.0-archive` and archive Windows unmerged work (`wtf-conn`, `wip/operator-preserve-2026-11-03`)
- [x] Stop Mac LaunchAgent (`dev.wtf.hub`), remove legacy binaries, clean Mac/Windows/WSL runtime leftovers, strip `wtf` from `~/.cursor/mcp.json` and `~/.claude.json`, remove Windows `:7800` portproxy
- [x] Clean Mac and Windows/WSL git checkouts to `origin/main`
- [x] Remove legacy modules/tests/docs and implement `config`, `fleet`, `probe`, `main`, `tests/cli.rs`
- [x] Update `skill/wtf-is-going-on-mcp.md`, `README.md`, and DOX (`llms.txt`, `src/llms.txt`, `docs/llms.txt`, `docs/FLEET.md`)
- [x] Run `cargo test` and `cargo build --release`, install `~/.local/bin/wtf`, verify live CLI, commit, push, and open PR
