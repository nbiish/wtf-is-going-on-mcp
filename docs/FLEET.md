# FLEET — machines and SSH aliases

## Status Summary

The legacy `wtf serve` hub, `wtf agent` MCP stdio bridge, and `:7800` federation stack have been shut down and archived at git tag `v0.20.0-archive`. Cross-machine coordination runs directly over OpenSSH (`ssh` / `scp`) and the stateless `wtf` CLI (`v0.21.0`).

## Machines & SSH Aliases

| Alias | User | Host | Port | Notes |
|---|---|---|---|---|
| `mac` | `nbiish` | `mac.attlocal.net` | 22 | Local Mac (`in-DIGI-nous`). LaunchAgent `dev.wtf.hub` unloaded and removed; `~/.local/bin/wtf` updated to stateless CLI; `"wtf"` removed from `~/.cursor/mcp.json` and `~/.claude.json`; `~/.config/wtf-mcp/fleet.json` preserved. |
| `windows` | `kenwa` | `peopleofthedawn.local` | 22 | Windows (`PEOPLEOFTHEDAWN`). Legacy `C:\Users\kenwa\bin\wtf.exe`, `bridge.json`, and `:7800` portproxy removed; `D:\Code\wtf-is-going-on-mcp` cleaned and synced to `origin/main` (unmerged work archived on `origin`). |
| `wsl` | `nanoboozhoo` | `peopleofthedawn.local` | 2222 | WSL Ubuntu on Windows host. Legacy `~/.config/wtf-mcp/` removed; `~/code-external/wtf-is-going-on-mcp` symlinked to `/mnt/d/Code/wtf-is-going-on-mcp`. |

Verified: 2026-11-24 (Mac teardown, Windows/WSL cleanup, and stateless `wtf` CLI verified).
