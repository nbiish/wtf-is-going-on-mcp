# MEMORY — wtf-is-going-on-mcp (curated digest)

One entry per durable fact. Format: `- [YYYY-MM-DD] <fact> *(source: <server/tool>)*` — newest first per section. Contracts live in the `llms.txt` chain, never duplicated here. Exports beside this file are machine snapshots; this file is the curated layer agents iterate on.

## Facts

- [2026-09-10] Memory CLI tools memorix (1.3.0), engram (1.20.0), and mcp-server-memory are installed and active on this node (WSL ~/.local/bin). *(source: engram, memorix CLI probe, this session)*
- [2026-09-10] wtf-is-going-on-mcp is at release v0.16.0 with zero external crates, router-chat-only inference (:11434/v1), and singular wtf-mcp-mastery skill. *(source: repo inspection, this session)*

## Decisions

- [2026-09-10] Retired legacy graph-intelligence substrate and skill in favor of the three-MCP repo-memory substrate (memorix, reference_memory, engram) and .agents/memories/ layer. *(source: operator directive, this session)*
- [2026-09-10] All memory operations across agent harnesses synchronize durable knowledge via .agents/memories/ (MEMORY.md + exports snapshots). *(source: operator directive, this session)*

## Gotchas

- [2026-09-10] Engram `export`/`import` snapshots include soft-deleted observations — scrub exports for retired memories before committing. *(source: engram v1.20.0 export)*
- [2026-09-10] Memorix binds to the git root of the executing directory; run memorix from repo root or pass project context explicitly. *(source: memorix 1.3.0)*
