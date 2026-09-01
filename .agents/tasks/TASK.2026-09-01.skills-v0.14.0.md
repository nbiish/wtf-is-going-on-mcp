# TASK 2026-09-01b — skills + operator docs sync to v0.14.0 reality

Branch: `docs/skills-v0.14.0` (worktree `../wtf-skills`, from `main` @ `7b66b86`)

## Operator directive
Sync every skill and operator doc with what was actually built and shipped so
agents viewing any doc see the same system: clickable dashboard chats →
per-chat tmux executor (omp → hermes → fcc-claude fallback chain) →
local-router fallback models; plus our cross-machine progress.

## Changed
- `.agents/skills/wtf-agent-hub/SKILL.md`:
  - description + intro: executor + SESSIONS card mentioned.
  - §4 tool list → 20 tools (adds `env_report`, `env_probe`, `chat_run`,
    `chat_sessions`).
  - §5 Execute → automated lane is `chat_run` (tmux `wtf-chat-<slug>`,
    attachable, trace names the lane); CLIs are model-agnostic and route
    through `local-router/fallback-models` (receipts on both machines).
  - §8 → dashboard SESSIONS card paragraph (viewer tab; encrypted bodies
    opaque to hub).
- `.agents/skills/wtf-observability/SKILL.md`: status banner 14 → 20 tools
  with executor + COMMS mention; intro notes SESSIONS card + executor
  sessions.
- `AGENTS.md`: new **Latest (v0.14.0…)** release block + **SINGULAR MODEL
  SYSTEM (DONE both machines)** block with receipts; **NEXT FOCUS** rewritten
  to the federated-chat control-plane goal (router troubleshooting lane,
  onboarding via chat, windows pqc-secrets pull).
- `llms.txt`: operator preference bullet **Executor + singular model system**
  (release line already carried v0.14.0 from the docs/dox pass).

## Not changed (intentional)
- `src/llms.txt` module contracts — already updated in the v0.14.0 DOX pass.
- `README.md` — user-facing quickstart still accurate (MCP shape unchanged).

## Verification
- `grep -c chat_run SKILL.md` = 3; tools list names 20; no stale "14-tool".
- `git diff --stat`: 4 files, docs-only.

## Classification: Confidential. No secrets in this file.
