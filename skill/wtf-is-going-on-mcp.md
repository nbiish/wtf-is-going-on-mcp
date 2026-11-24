---
name: wtf-is-going-on-mcp
description: Stateless zero-dependency CLI for cross-machine pickup, SSH host reachability checks, and goals. Use `ssh`/`scp` directly for remote commands and file transfers; use Herdr/Orca for agent sessions.
whenToUse: Use when starting or resuming work across machines (`wtf status`, `wtf pickup`), tracking short goals (`wtf goal`), or managing keyless SSH aliases (`wtf alias`).
---

# wtf

**Plain `ssh`/`scp` moves files and runs remote commands. Herdr/Orca run agent sessions. `wtf` gives you one stateless status and pickup CLI.**

The legacy hub daemon (`wtf serve`), MCP stdio bridge (`wtf agent`), PQC session server, federation, and task-dispatch stack were retired at tag `v0.20.0-archive`.

## Commands

```bash
wtf status [--local] [--json]   # probe local machine, git repo, PATH tools, and SSH aliases
wtf pickup [--json]             # offline pickup snapshot: local state, open goals, ssh/scp targets
wtf goal [list] [--all] [--json]
wtf goal add "<title>" [--detail "<text>"] [--repo <name>] [--target <host>]
wtf goal done <id> | wtf goal rm <id>
wtf alias [list] [--json]
wtf alias set <alias> <user> <host> [--port <n>]
wtf alias rm <alias>
wtf skill print | install [--dir <dir>] [--force]
```

## Cross-machine workflow (no daemon, no MCP)

1. Resume work: run `wtf pickup` (offline) or `wtf status` (includes live `BatchMode=yes` SSH probes).
2. Run remote work directly over SSH:
   ```bash
   ssh <alias> '<command>'
   ```
3. Move files directly with `scp`:
   ```bash
   scp ./handoff.md <alias>:/tmp/
   scp <alias>:/tmp/result.txt .
   ```
4. Manage agent sessions on each machine with `herdr` or `orca` directly.

## Security rules

- `$WTF_HOME/fleet.json` (default `~/.config/wtf-mcp/fleet.json`, mode `0600`) stores **only** goals and `{alias, user, host, port}`.
- Never store private keys, `IdentityFile` paths, tokens, or secrets in `fleet.json`.
- SSH probes run via explicit argv arrays (`ssh -o BatchMode=yes -o ConnectTimeout=5 -p <port> <user>@<host> -- hostname`) under an 8s timeout; never shell-interpolated.
