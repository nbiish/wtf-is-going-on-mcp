# TASK: Singular Secure Dashboard URL with Chat & Federated Multi-Machine Shell

## Branch & Worktree
- Branch: `feat/hub-dashboard-federated-shell`
- Worktree: `/Volumes/1tb-sandisk/code-external/wtf-dashboard-federated-shell`

## Objectives
1. Refine dashboard URL to a singular, highly secure capability endpoint (`/w/<capability>`) valid across all topologies (loopback, LAN, advertised URL).
2. Embed the User Chat & SWE-bench Agent Orchestration Studio directly into the singular dashboard page.
3. Build the Federated Shell module (`fed_shell.rs`) presenting a virtual `~/` root composed of cluster machines (`~/mac`, `~/windows`, etc.) with cross-machine compound script execution in one prompt.
4. Update DOX contracts (`llms.txt`, `src/llms.txt`, `docs/OPERATIONS.md`) and verify zero test regressions across all 103 unit and 13 e2e tests.
