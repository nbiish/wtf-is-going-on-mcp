# Task: Executor Local-Router Fleet & Hub Upgrades
Date: 2026-09-03
Branch: feat/executor-local-router-fleet
Worktree: /Volumes/1tb-sandisk/code-external/wtf-fleet

## Objective
Fix agent executor CLI invocations (trae-cli, mini, omp, fcc-claude) to route properly through local-router on port 11434, implement identity-registry persistence (R1 from ROADMAP.md), rebuild release binary, restart wtf hub daemon to v0.15.0+, and verify live execution.

## Changes
- src/executor.rs: fix trae-cli flags and prompt positioning; include mini in auto fleet fallback chain
- src/api.rs & src/main.rs: implement identity-registry persistence to identities.json (0600)
- docs/ROADMAP.md & docs/FLEET.md: update state and document R1 resolved
- .agents/skills/wtf-agent-hub/SKILL.md & AGENTS.md: document chat_run, chat_sessions, and chat_session_lifecycle
- build release binary and restart hub daemon (v0.15.0)

## Verification
- cargo test: 103 unit tests, 13 e2e tests pass
- chat_run tool verified live: executed tasks via omp and mini in persistent tmux sessions powered by local-router fallback-models on 11434
- chat_sessions and chat_session_lifecycle verified: opened, listed, and closed sessions cleanly
- healthz endpoint verified: returns v0.15.0 with ok: true

