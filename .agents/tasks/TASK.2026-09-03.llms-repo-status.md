# Task: Synchronize llms.txt with Current Repo Status
Date: 2026-09-03
Branch: docs/llms-repo-status
Worktree: /Volumes/1tb-sandisk/code-external/wtf-llms-status

## Objective
Update llms.txt and src/llms.txt to accurately reflect current repo status:
- Fallback execution cascade: free-claude-code → omp → trae-cli → mini (or agent: fleet/auto)
- Local-router dynamic context sizing and multimodal fallback routing
- Resolution of R1 identity-registry persistence ($WTF_HOME/identities.json 0600)
- Current release: v0.15.0 with 21 MCP tools (including chat_session_lifecycle)
- Verification gates: 103 unit tests and 13 e2e tests passing
- Deployed curated skill set: 12 skills deployed by ainish-coder --rules (llm-security removed)
