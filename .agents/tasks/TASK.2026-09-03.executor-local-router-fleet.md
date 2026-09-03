# Task: Executor Local-Router Fleet & Hub Upgrades
Date: 2026-09-03
Branch: feat/executor-local-router-fleet
Worktree: /Volumes/1tb-sandisk/code-external/wtf-fleet

## Objective
Fix agent executor CLI invocations (trae-cli, mini, omp, fcc-claude) to route properly through local-router on port 11434, implement identity-registry persistence (R1 from ROADMAP.md), rebuild release binary, restart wtf hub daemon to v0.15.0+, and verify live execution.

## Changes
- src/executor.rs: fix trae-cli flags and prompt positioning
- src/api.rs & src/main.rs: implement identity-registry persistence to identities.json (0600)
- docs/ROADMAP.md & docs/FLEET.md: update state
- build release binary and restart hub daemon
- verify chat_run execution end-to-end
