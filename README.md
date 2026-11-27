# wtf

Stateless, zero-dependency Rust CLI for checking local + SSH fleet status, tracking open goals, and printing cross-machine pickup snapshots.

> **Legacy hub/MCP stack retired:** The `wtf serve` HTTP hub, `wtf agent` MCP stdio bridge, PQC session channels, hub federation, task dispatch, and dashboard server have been closed down and preserved at git tag `v0.20.0-archive`. Cross-machine coordination now uses standard OpenSSH (`ssh` / `scp`) directly alongside Herdr/Orca.

## Zero external dependencies

Everything (`json`, `rand`, `config`, `fleet`, `probe`, `util`) is implemented in-tree on the Rust standard library (`[dependencies]` is empty).

## Build and install

```bash
cargo build --release
install -m 755 target/release/wtf ~/.local/bin/wtf
```

On Windows (from a repo checkout):

```powershell
cargo build --release
copy target\release\wtf.exe %USERPROFILE%\bin\wtf.exe
```

## CLI reference

```text
wtf [status] [--local] [--json]               probe local machine + SSH aliases + open goals
wtf pickup [--json]                           offline pickup snapshot (local, goals, SSH targets)
wtf goal [list] [--all] [--json]              list open goals (or all with --all)
wtf goal add <title> [--detail D] [--repo R] [--target H]
wtf goal done <id> | wtf goal rm <id>         mark a goal done or delete it
wtf alias [list] [--json]                     list keyless SSH aliases
wtf alias set <alias> <user> <host> [--port N]
wtf alias rm <alias>                          remove an SSH alias
wtf skill print | install [--dir DIR] [--force]
wtf version | wtf help
```

## SSH & SCP workflow

Register targets once (no keys or `IdentityFile` paths are ever stored):

```bash
wtf alias set windows kenwa peopleofthedawn.local --port 22
wtf alias set wsl nanoboozhoo peopleofthedawn.local --port 2222
```

Then check reachability across all machines in parallel or move files directly:

```bash
wtf status                                    # local + parallel BatchMode SSH probes
ssh windows 'hostname'                        # run a remote command
scp ./handoff.md wsl:/tmp/                    # copy files
```

## Storage (`$WTF_HOME/fleet.json`)

All state lives in `$WTF_HOME/fleet.json` (default `~/.config/wtf-mcp/fleet.json`, mode `0600` in a `0700` directory on Unix). It stores only:

- `goals`: `{id, title, detail, repo, target, status, created_at, updated_at}`
- `aliases`: `{alias, user, host, port}`

`valid_name`, `valid_host`, and `forbidden_keys` reject paths, spaces, leading dashes, and secret-like field names.

## Development

```bash
cargo test              # unit + CLI integration tests
cargo build --release   # lto, panic=abort, overflow checks
```
