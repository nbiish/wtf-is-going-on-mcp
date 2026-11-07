# MEMORY — wtf-is-going-on-mcp (curated digest)

One entry per durable fact. Format: `- [YYYY-MM-DD] <fact> *(source: <server/tool>)*` — newest first per section. Contracts live in the `llms.txt` chain, never duplicated here. Exports beside this file are machine snapshots; this file is the curated layer agents iterate on.

## Facts

- [2026-11-07] wtf-is-going-on-mcp is at release v0.18.0 with zero external crates, 22 MCP tools, and federated task dispatch (`src/tasks.rs`, `/api/v1/tasks*`, `wtf tasks`). *(source: repo inspection, this session)*
- [2026-09-10] Memory CLI tools memorix (1.3.0), engram (1.20.0), and mcp-server-memory are installed and active on this node (WSL ~/.local/bin). *(source: engram, memorix CLI probe, this session)*
- [2026-09-10] wtf-is-going-on-mcp is at release v0.16.0 with zero external crates, router-chat-only inference (:11434/v1), and singular wtf-mcp-mastery skill. *(source: repo inspection, this session)*

## Decisions

- [2026-11-07] Task dispatch is pull-based with 15-minute claim leases: the hub queues an addressed record and an agent on the target claims it. The hub never spawns a harness or an ssh session, preserving the v0.16.0 harness-free contract. *(source: operator directive, this session)*
- [2026-09-10] Retired legacy graph-intelligence substrate and skill in favor of the three-MCP repo-memory substrate (memorix, reference_memory, engram) and .agents/memories/ layer. *(source: operator directive, this session)*
- [2026-09-10] All memory operations across agent harnesses synchronize durable knowledge via .agents/memories/ (MEMORY.md + exports snapshots). *(source: operator directive, this session)*

## Gotchas

- [2026-11-07] A route under `/api/v1/<thing>/` is swallowed by that resource's `{id}` fallthrough unless the path parser exempts it — `POST /api/v1/tasks/claim` was read as task id `claim`. Register fixed sub-paths in the route table AND exempt them in the parser. *(source: this session, `task_path`)*
- [2026-11-07] The MCP bridge decoded only HTTP 200, so a 201 Created surfaced as "hub returned HTTP 201". Any 2xx now decodes, and error bodies are surfaced so the reason reaches the agent. *(source: this session, smoke test)*
- [2026-11-07] The shipped skill documented port 47820 and an automatic port fallback; the real default is 7800 and no fallback exists. Corrected in `.agents/skills/wtf-is-going-on-mcp/SKILL.md` (canonical source: `~/code-external/ainish-coder/.agents/skills/`). *(source: this session, config.rs)*
- [2026-09-10] Engram `export`/`import` snapshots include soft-deleted observations — scrub exports for retired memories before committing. *(source: engram v1.20.0 export)*
- [2026-09-10] Memorix binds to the git root of the executing directory; run memorix from repo root or pass project context explicitly. *(source: memorix 1.3.0)*
