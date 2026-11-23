---
plan_format: ainish/plan/v1
slug: federated-pickup
date: 2026-11-22
status: complete
branch: feat/federated-pickup
worktree: /Volumes/1tb-sandisk/code-external/wtf-federated-pickup
task_record: .agents/tasks/2026-11-22-22-federated-pickup.md
operator: question-me
---

# Plan: Federated pickup for goals, tasks, repos, and machine skills

## 1. Problem

Agents cannot pick up cross-machine work from one MCP call. Goals, tasks, repos, SSH reachability, and the skills that can use a machine's apps and logins are scattered across retired skill files, SSH config, Herdr, and Orca. The hub already records presence and a task board, and a browser can view it, but nothing returns the pickup set an agent needs to continue on the machine that actually has the checkout, the login, or the app.

## 2. Anchor Facts

- Mac hub `hub-19188245` v0.19.0 is the live server: LaunchAgent `dev.wtf.hub` pid 1228, `wtf serve --bind 0.0.0.0:7800`, healthz `ok:true` version `0.19.0` (upgraded to `0.20.0`). Installed `~/.local/bin/wtf`. Established from `wtf --help`, `curl /healthz`, `shasum`, `launchctl print`.
- Windows `PEOPLEOFTHEDAWN` / `kenwa` is an enrolled device, not a second hub. `C:\Users\kenwa\bin\wtf.exe` was v0.19.0, upgraded to `v0.20.0`. `bridge.json` points at `http://192.168.1.68:7800` device `windows`. `wtf status` from Windows reads the Mac hub. Established from `ssh windows`.
- WSL user `nanoboozhoo` has a bridge for device `wsl` aimed at the same hub. Established from `ssh wsl`.
- Enrolled devices: `mac`, `windows`, `wsl`, all active. `wtf federate list` reports no peer hubs. Established from `wtf status`, `wtf tasks`, `wtf federate list`.
- MCP surface in this tree expanded from 24 tools to 27 tools (`fleet_pickup`, `goal_add`, `goal_done`, `alias_set`).
- Canonical skill is `skill/wtf-is-going-on-mcp.md` (sha256 `4d17a74215300def32578f5e42237808d683edaaf79ebd36ffbcb78aa2ff9718`), distributed byte-identical to `~/code/ainish-coder/skills-to-distribute/skills/wtf-is-going-on-mcp/SKILL.md` and `.agents/skills/wtf-is-going-on-mcp/SKILL.md`.
- Legacy skills (`wtf-agent-hub`, `wtf-observability`, `wtf-mcp-mastery`) were retired from Mac and Windows.
- `~/.cursor/mcp.json` wires `wtf` (`/Users/nbiish/.local/bin/wtf agent`).

## 3. Plan

### Goals

- One MCP tool returns the pickup set: enrolled machines, open and recent tasks, repos attached to those records, SSH aliases (name, user, host, port), and which external skills each machine has reported.
- The operator's browser shows that same set on the existing dashboard card (`PICKUP`).
- The singular skill tells an agent to dispatch work to the machine that has the app or the login, and names BrowserMCP, browser-skill, and agent-desktop as external tools on that machine.
- After the skill text changes, the copy in `~/code/ainish-coder/skills-to-distribute/skills/wtf-is-going-on-mcp/` matches it byte for byte.

### Non-goals

- Embedding BrowserMCP, browser-skill, or agent-desktop inside the Rust binary. They stay separate programs on the machine that holds the login.
- Returning private keys, IdentityFile paths, dashboard capability tokens, or browser cookies from any tool.
- A second hub, a second dashboard origin, or a public HTTP listener.

### Constraints and invariants

- Zero crates.io dependencies. `[dependencies]` stays empty.
- The hub does not spawn a browser, a desktop agent, or an SSH session. `task_dispatch` stays pull-based.
- Device keys and the capability token never enter the tool result, the ledger, or the plan.

### Implemented architecture

The Mac hub remains the singular record. Each machine reports a skill catalog through the env report (`herdr`, `orca`, `browsermcp`, `agent-desktop`, `browser-skill`). A read-only pickup tool joins devices, tasks, repos stored on those records, the reported catalog, and an operator-owned alias table (`$WTF_HOME/fleet.json`). The alias table stores SSH config aliases (`windows`, `wsl`, `mac`), not key material. The dashboard card renders the same JSON the tool returns.

### Interfaces and data

- MCP tool `fleet_pickup`. Result text is JSON: `hub`, `devices[]`, `tasks[]`, `repos[]`, `ssh_aliases[]`, `skills[]`.
- MCP tools `goal_add`, `goal_done`, `alias_set`.
- `ssh_aliases[]` fields: `alias`, `user`, `host`, `port`.
- `skills[]` fields: `device`, `name`, `present`.
- Dashboard: "PICKUP · goals, repos, machines" card at `/w/<capability>`.

## 4. Clarifications

### 4.1 Extend the existing hub
- Selected: Extend this hub.

### 4.2 External apps stay external
- Selected: External, per machine, named in the skill catalog.

### 4.3 SSH info is aliases only
- Selected: Alias table only.

### 4.4 Distribution waits for a real skill change
- Selected: Copy after the text changes.

## 5. Todos

- [x] Extend `wtf` with `src/fleet.rs` (`FleetStore`, `Goal`, `SshAlias`, `probe_skills`, `skills_from_report`, security `forbidden_keys` check).
- [x] Expose new MCP tools: `fleet_pickup`, `goal_add`, `goal_done`, `alias_set` in `src/mcp.rs`.
- [x] Expose HTTP API endpoints: `/api/v1/fleet`, `/api/v1/goals`, `/api/v1/goals/done`, `/api/v1/aliases` in `src/api.rs`.
- [x] Update Browser Dashboard with `"PICKUP · goals, repos, machines"` card in `src/dashboard.rs`.
- [x] Update CLI commands `wtf goal` and `wtf alias` in `src/main.rs`.
- [x] Update canonical skill `skill/wtf-is-going-on-mcp.md` and distribute to `~/code/ainish-coder/skills-to-distribute/`.
- [x] Update `docs/FLEET.md`, `README.md`, `llms.txt`.
- [x] Compile release binary on Mac, install to `~/.local/bin/wtf`, verify `dev.wtf.hub` running `v0.20.0`.
- [x] Compile release binary on Windows, install to `C:\Users\kenwa\bin\wtf.exe`, verify `v0.20.0`.
- [x] Retire obsolete skills on Windows and Mac; install canonical `wtf-is-going-on-mcp`.
- [x] Wire `wtf` into `~/.cursor/mcp.json`.

## 6. Verification

- `cargo test --offline`: 135 unit tests and 14 E2E tests green on Mac and Windows.
- `wtf alias list` and `wtf goal list`: verified on Mac and Windows via SSH.
- `ssh windows 'wtf.exe status'`: successfully queries Mac hub from Windows.
- `shasum -a 256` of canonical skill matches distribution copy (`4d17a742...`).

## 7. Open Decisions

- Dashboard shape: Option 1 chosen and implemented (added pickup card to the current `/w/<capability>` page while preserving Studio chat pane and Federated Shell).
