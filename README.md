# wtf-live

**One localhost page for every agent session on every machine.**

`wtf-live` is a small Rust binary (std only, no crates) that reads live state from
**Herdr** and **Orca** and renders it plus a shared pastebin at
<http://127.0.0.1:7800>. It replaces the previous `wtf` hub — the MCP bridge,
federation, encrypted session channels, device keys and signed API are gone on
purpose. What remains is a viewer and a clipboard.

## What it shows

| Column | Source | Meaning |
| --- | --- | --- |
| Machines | Herdr + Orca | local box, saved `herdr machine` profiles, paired Orca servers |
| Sessions | Herdr panes/agents, Orca worktrees/terminals | one card per agent or live shell |
| Pastebins | `~/.wtf-live/bins/` | durable text shared by humans and agents |

Each card carries the agent kind, lifecycle state (`working`, `blocked`, `done`,
`idle`, `shell`), the workspace, the working directory, the last output lines,
and a copyable handle (`term_…` for Orca, `w1:p1` for Herdr).

The page polls `/api/snapshot` every 3 seconds; press `p`-adjacent controls to
pause, `/` to filter, `Esc` to close a paste. No external assets, no CDN.

## Build and run

```bash
cargo build --release
./target/release/wtf-live            # serve on 127.0.0.1:7800 and open a browser
```

Flags and environment:

```text
wtf-live serve [--port N] [--bind IP] [--data-dir DIR] [--no-open] [--allow-remote]
WTF_LIVE_HOME     state directory (default ~/.wtf-live)
WTF_LIVE_MACHINE  display name for this box (default: hostname -s)
ORCA_BIN          explicit path to the orca CLI
HERDR_BIN         explicit path to the herdr CLI
```

The hub auto-detects the CLIs: `/Applications/Orca.app/Contents/Resources/bin/orca`
(or `orca` on PATH) and `herdr` (Homebrew or `~/.local/bin`). If a CLI is
missing the page says so instead of failing.

`--bind` refuses non-loopback addresses unless `--allow-remote` is passed: the
hub has no authentication.

## Pastebins

Three ways in, one store out:

- the **+ paste** button on the page;
- `wtf-live bin put --file notes.md --title "deploy plan"`;
- `curl -X POST localhost:7800/api/bins -d '{"title":"…","body":"…"}'`.

Read them back with `wtf-live bin ls`, `wtf-live bin get <id>`, or the page.
Deletion is `wtf-live bin rm <id>` or the viewer's delete button. Bodies are
capped at 256 KiB, bins at 400, and every write is atomic.

Cross-machine sharing: run the hub where the other machine can reach it
(`WTF_LIVE_HOME` stays local), bind a LAN or Tailscale address with
`--allow-remote`, or keep it loopback and tunnel:

```bash
ssh -L 7800:127.0.0.1:7800 you@workbox     # then open localhost:7800 on the laptop
```

## Cross-machine agent state

Both tools keep machine ownership on the machine that runs the agent, so the
hub reads whatever the local CLIs can see:

- **Herdr** — `herdr status`, `workspace list`, `pane list`, `agent list`, and
  `pane read` for previews, plus every enabled `herdr machine` profile through
  the `--machine` prefix. Herdr 0.9 forwards pane/agent API calls over SSH, so a
  saved machine's panes appear next to local ones.
- **Orca** — `orca worktree ps` (agents, states, previews) and
  `orca terminal list`, plus saved remote runtimes from `orca environment list`
  queried with `--environment`.

Requirements on the source side:

```bash
herdr                     # or: herdr server   (the CLI needs a running server)
herdr machine add workbox # one-time, interactive; needs SSH access
orca serve                # headless Orca runtime + pairing link (optional)
orca environment add --name workbox --pairing-code 'orca://pair?...'
```

Known limit (Herdr 0.9.x): the Herdr *agent CLI* still works within one server;
cross-machine pane APIs need an updated remote Herdr, and `machine add` is
interactive. Orca's `--environment` path is live but slow links show up as a
`down` machine row rather than blocking the page.

## HTTP API

| Method | Path | Purpose |
| --- | --- | --- |
| GET | `/` | the page |
| GET | `/api/snapshot?fresh=1` | machines, sessions, source health, counts |
| GET | `/api/bins` | bin summaries |
| POST | `/api/bins` | `{"title","body","machine"}` → created bin |
| GET | `/api/bins/<id>` | full bin |
| DELETE | `/api/bins/<id>` | delete |
| GET | `/healthz` | `{"ok":true}` |

## Storage

One directory, one file per paste:

```text
~/.wtf-live/bins/<id>.json     # {"id","title","body","machine","created_at",…}
```

Nothing else is written. The old hub's state (`~/.config/wtf-mcp`) is archived
under `~/.wtf-live/legacy/` and no longer read.

## Security model

- Loopback bind by default; non-loopback requires an explicit flag.
- No secrets, no accounts, no signing: the hub is a read-only aggregation plus
  a local pastebin. Do not expose it to an untrusted network.
- CLI calls use argument vectors (never a shell), a hard timeout, and are
  killed on overrun; output is trimmed to 600 characters per session.
- The page ships a restrictive CSP (`default-src 'none'`), and all rendered
  text is HTML-escaped in the client.
- Paste bodies are size-capped and ids are `[A-Za-z0-9_-]`, so no path traversal.

## Tests

```bash
cargo test          # JSON, HTTP, bins roundtrip, text hygiene
wtf-live doctor     # what the CLIs report right now
wtf-live snapshot   # one-shot text view
```
