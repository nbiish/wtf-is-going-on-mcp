# TASK 2026-11-07 — federated task dispatch (cross-machine work lane)

- **scope:** `src/tasks.rs` (new), `src/api.rs`, `src/mcp.rs`, `src/store.rs`, `src/dashboard.rs`, `src/main.rs`, `src/config.rs`, `src/lib.rs`, `tests/e2e.rs`, root + `src/` `llms.txt`, `README.md`, `.agents/skills/wtf-is-going-on-mcp/SKILL.md`
- **branch:** `feat/ssh-task-dispatch` · **worktree:** `/mnt/d/Code/wtf-task-dispatch`
- **lane:** now · **release:** v0.18.0
- **status:** complete, verified, merged

## Objective

Let one agent designate work for a paired machine and get the result back, without the hub ever spawning a harness or driving an ssh session. Work is addressed (machine / compute tier / architecture), claimed by an agent already running on the target, executed with that machine's own tooling and credentials, and reported back with a deliverable. A task may bind one federated chat so the operator follows the conversation and the machine result on the same handle.

## Acceptance

- [x] `task_dispatch` / `task_claim` / `task_report` / `task_status` MCP tools.
- [x] `/api/v1/tasks*` routes with device auth and per-device ownership.
- [x] Durable store (`tasks.json`, 0600, atomic, replayed on boot).
- [x] Dashboard TASKS card + `wtf tasks` operator CLI from any machine.
- [x] Pull-based with lease expiry; no hub-side spawn or ssh exec.
- [x] 117 unit + 13 e2e tests green; live two-device smoke test proved the loop.

## compat:

Additive only — four new MCP tools (17 → 22), new `/api/v1/tasks*` routes, a new `tasks` object in the state payload, a new `tasks.json` in `$WTF_HOME`, and a new `wtf tasks` subcommand. No existing tool, route, or field changed shape. An older bridge against a v0.18.0 hub simply sees `tasks: {total: 0, tasks: []}` in the state payload and ignores it; a v0.18.0 bridge against an older hub gets a 404 from the task routes, which surfaces as a normal `isError` result. Rollback: revert the merge; `tasks.json` is inert to earlier versions and can be deleted safely.

## budget:

No model or CI spend. Verification was local: `cargo test` (~8 s), a release build (~11 s), and a two-device loopback smoke test (sub-second).

## Result

`src/tasks.rs` (new, ~700 lines incl. 11 unit tests) owns the record and the transition table. `decode` in `src/mcp.rs` now accepts every 2xx and surfaces the hub's `error` text — both found by the smoke test, not by inspection. Dashboard renders per-state counts plus one row per item with result/error inline. Skill doc updated in this repo and in the ainish-coder source it distributes from.

## Next step

Give a worker agent a polling loop (`task_claim` on an interval) so a paired machine picks up work without an agent already being asked to look — today a queued task is a correct resting state, and that only becomes useful when something is reliably polling.
