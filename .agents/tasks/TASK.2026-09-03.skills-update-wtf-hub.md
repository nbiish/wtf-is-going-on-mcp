# TASK: Update WTF Skills with Singular Dashboard Capability & Federated Shell

- start: 2026-09-03T18:40:00Z
- branch: docs/skills-update-wtf-hub
- worktree: /Volumes/1tb-sandisk/code-external/wtf-skills-update
- upstream: ~/code/ainish-coder/.agents/skills/

## Objectives
1. Update `.agents/skills/wtf-agent-hub/SKILL.md` in this repository to reflect v0.15.1:
   - Singular unguessable capability dashboard URL (`/w/<capability>`).
   - Integrated User Chat & Agent Orchestration Studio.
   - Paired Federated Multi-Machine Shell (`~/` cluster root, machine navigation, compound command execution).
   - SWE-bench Coding Fleet (`trae-cli`, `mini`, `omp`, `fcc`) via `local-router/fallback-models` on `127.0.0.1:11434`.
2. Update `.agents/skills/wtf-observability/SKILL.md` to keep hub-operator and signed-curl documentation aligned.
3. Synchronize `wtf-agent-hub/SKILL.md` (and companion skills) to `~/code/ainish-coder/.agents/skills/` for distribution via `ainish-coder --rules`.
4. Verify tests (`cargo test`) and binary recompilation (`AGENT_SKILL` compiled-in).
