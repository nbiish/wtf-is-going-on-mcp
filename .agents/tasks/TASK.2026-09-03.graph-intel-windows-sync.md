# TASK: Graph Intelligence Integration, Universal 11434 Local-Router Single-Config Convergence, and Windows Fleet Sync

- date: 2026-09-03
- branch: docs/graph-intel-windows-sync
- worktree: /Volumes/1tb-sandisk/code-external/wtf-graph-sync
- parent: operator

## OBJECTIVES & SCOPE
1. **Graph Intelligence Analysis & PRD Reflection**:
   - Ingest and document the Three-Pillar Graph Intelligence Layer (GitNexus AST call-graph substrate, Graphify multimodal synthesis, Semantica PROV-O context and governance) into `llms.txt`, `src/llms.txt`, and `docs/OPERATIONS.md`.
   - Record AST metrics: 1,389 nodes, 4,782 edges, 55 community clusters, 122 call flows across `fed_shell.rs`, `mcp.rs`, `api.rs`, and `dashboard.rs`.
2. **Universal Single-Config AI Tooling Source (Ollama Port 11434)**:
   - Formulate and document the convergence architecture: `local-router` on loopback `127.0.0.1:11434` (proxying real daemon on `11435`) as the PQC-secure single-configuration source for all AI tools, agent harnesses (Cursor, Warp, VS Code, Codex, Trae, Mini, OMP, FCC), and federated agent brains (`local-router/fallback-models`).
   - Eliminate per-tool API key management by unifying Ollama (`/api`), OpenAI (`/v1`), and Anthropic (`/v1`) protocols under one loopback proxy with dynamic token context matching and multimodal failover.
3. **Windows Machine Fleet Synchronization**:
   - Push all verified commits to `origin/main` (completed: `408ec44`).
   - Document exact operational commands to pull, compile, and run on Windows WSL2 (`hub-2538554f`), verifying bidirectional federation and cross-architecture shell execution.
