# TASK 2026-11-24 — close down hub/MCP stack and refactor into stateless CLI

- start: 2026-11-24T21:50:21Z · end: 2026-11-24T21:56:00Z
- **scope:** Mac (`~/.local/bin/wtf*`, `~/.config/wtf-mcp/`, LaunchAgent `dev.wtf.hub`, `~/.cursor/mcp.json`, `~/.claude.json`), Windows (`C:\Users\kenwa\bin\wtf.exe`, `netsh portproxy`, `D:\Code\wtf-is-going-on-mcp`, `D:\Code\wtf-conn`), WSL (`~/.config/wtf-mcp/`), and refactored codebase (`Cargo.toml`, `src/*`, `tests/cli.rs`, `skill/`, `README.md`, DOX)
- **branch:** `refactor/stateless-ssh-cli` · **worktree:** `/Volumes/1tb-sandisk/code-external/wtf-refactor-cli`
- **release:** v0.21.0
- **status:** done

## Objective

Retire the hub/MCP/PQC/federation server stack across Mac, Windows, and WSL in favor of plain OpenSSH (`ssh`/`scp`) + a stateless zero-dependency Rust CLI (`wtf status`, `wtf pickup`, `wtf goal`, `wtf alias`, `wtf skill`).

## Acceptance

- [x] Archive tag `v0.20.0-archive` and branches `archive/windows-connection-discovery-2026-11-24`, `archive/windows-wip-2026-11-03` pushed to `origin`
- [x] Mac `dev.wtf.hub` LaunchAgent stopped/removed, port 7800 free, MCP entries stripped from `~/.cursor/mcp.json` and `~/.claude.json`
- [x] Windows `wtf.exe`, `bridge.json`, `:7800` portproxy removed; WSL `~/.config/wtf-mcp` removed
- [x] Both Mac and Windows/WSL git checkouts synced clean to `origin/main`
- [x] Lean CLI modules (`config`, `fleet`, `probe`, `util`, `json`, `rand`, `main`) implemented with zero external crates
- [x] `cargo test` + `cargo build --release` green, installed to `~/.local/bin/wtf`, live `wtf status`/`wtf pickup` verified
- [x] DOX pass complete and PR opened against `main`

## Verification

- `cargo test`: 21 unit tests + 3 CLI integration tests (`tests/cli.rs`) passed; 0 warnings.
- `cargo build --release`: succeeded cleanly; installed to `/Users/nbiish/.local/bin/wtf` (`wtf 0.21.0`).
- Live `/Users/nbiish/.local/bin/wtf status`: probed local Mac (`herdr`, `orca`, `browser-skill`) and all three preserved SSH aliases (`mac`, `windows`, `wsl`) `[up]`.
- `lsof -iTCP:7800 -sTCP:LISTEN` on Mac: empty; `netsh interface portproxy show all` on Windows: empty.
- `git status -sb` on Mac `main` and Windows `/mnt/d/Code/wtf-is-going-on-mcp`: clean `## main...origin/main`.
