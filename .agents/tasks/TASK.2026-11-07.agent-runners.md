# TASK 2026-11-07 — agent runners (Orca first, Herdr fallback)

- start: 2026-11-07T23:40:00Z · end: 2026-11-08T01:32:00Z
- **scope:** `src/runner.rs` (new), `src/tasks.rs`, `src/api.rs`, `src/mcp.rs`, `src/lib.rs`, `tests/e2e.rs`, root + `src/` `llms.txt`, `README.md`, `.agents/skills/wtf-is-going-on-mcp/SKILL.md`
- **branch:** `feat/runner-orca-herdr` · **worktree:** `/mnt/d/Code/wtf-runner`
- **lane:** now · **release:** v0.19.0
- **status:** merged (remote proof outstanding — see Blockers)

## Objective

A dispatched task should start an agent on the target machine. Orca (`orca-ide`
here) is the preferred runtime because it supervises workers and reports through
its own Run; Herdr is the fallback because it is installed far more widely and
keeps agents alive across reconnects. Operator directive 2026-11-07, both
decisions confirmed: Orca-first preference, explicit runner honored exactly.

## Acceptance

- [x] `runner` field on the task record + `run_handle` for the launched agent.
- [x] `runner: auto` probes Orca then Herdr and reports which and why.
- [x] Explicit `runner: orca|herdr` honored exactly; absent runtime fails closed with its install step.
- [x] argv built as an array, unit-tested, specs never passed through a shell.
- [x] Remote placement expressed as a paired Orca environment (`--on`) or a saved Herdr machine (`--machine`).
- [x] `task_run` + `runner_probe` MCP tools; `GET /tasks/runners`; `POST /tasks/{id}/run`.
- [x] Only the claiming device may launch; terminal tasks refuse; failed launch keeps the claim.
- [x] 132 unit + 14 e2e green; live binary reports both runners and prefers Orca.
- [ ] **Remote launch proven against a second machine** — blocked, not done.

## Defects found by exercising the real runtime

1. `task_run` re-ran `device_auth`, consuming the request nonce twice → replay guard 401. Device is now threaded in.
2. Orca `worker-start` refuses to create a terminal without `--agent` (`agent_unconfigured`). An agent id is now mandatory, default `codex`, overridable.
3. The launch outlived the client's 10 s read timeout, reporting a false failure while the agent had in fact started. `api_post_with_timeout` gives startup calls 240 s.

A fourth real signal, not a code defect: Orca in this WSL environment blocks on
`agent-hooks-review-prompt` at startup. The code handled it correctly — claim
kept, error surfaced verbatim — and the skill now documents the fix.

## compat:

Additive. 22 → 24 MCP tools; new `runner` (defaults `auto`) and `run_handle`
(defaults empty) fields on task records — an older record loads with those
defaults; new `/api/v1/tasks/{runners,{id}/run}` routes. An older bridge against
a v0.19.0 hub simply never calls them. Rollback: revert the merge; records with
the new fields still deserialize in v0.18.0 because unknown keys are ignored by
`from_json`.

## budget:

No model or CI spend. Local verification: `cargo test` (~4 s), release build
(~13 s), two loopback launches against the real Orca binary and a fake runtime.

## Blockers

**The mac at `192.168.1.68` is ARP-visible but every TCP port times out** (22,
7800, 445). No connection is possible from here, so the remote half of this
feature is code-complete and locally proven but unproven against a live peer.
Additional prerequisites on the target, neither doable from this machine:

- Orca remote needs `orca serve` running there **and** a pairing code redeemed
  here with `orca environment add --name <n> --pairing-code 'orca://pair?...'`.
- Herdr remote needs SSH reachable, then `herdr machine add user@host --label <n>`.

## Next step

Once the mac answers, redeem one of the two pairings above and run a single
end-to-end dispatch → claim → `task_run` → `task_report` against it; the local
proof already covers everything except the transport and the pairing.
