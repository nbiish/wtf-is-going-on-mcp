# TASK: Federated OMP Execution, Distributed Architecture Config & Multi-Machine Compute Routing

- start: 2026-09-03T18:50:00Z
- branch: feat/fed-shell-omp-distributed-compute
- worktree: /Volumes/1tb-sandisk/code-external/wtf-fed-compute
- parent: operator

## Objective & Vision
Refine the WTF dashboard chat and federated shell so that executing `omp` (or any ACP agent/fleet master) runs with a federated/distributed configuration that stays synchronized across all connected cluster machines, referencing the Last Known Good Location (LKGL) for each target architecture.

Enable the singular federated shell to execute tasks across all devices or target specific machines based on agent determination of compute requirements:
- **Architecture LKGL Tracking:** Each machine (`mac`, `windows`, `pi`, etc.) tracks its last known good working directory across sessions.
- **Federated OMP Configuration Sync:** Synchronized config state allowing `omp` and fleet agents to inherit shared parameters, router endpoints (`127.0.0.1:11434`), and fallback cascade orders.
- **Distributed Multi-Compute Routing:** Support portable distributed tasks that maximize compute efficiency (e.g., edge devices like a Raspberry Pi capturing voice/mic input, delegating heavy music generation or LLM synthesis to the strongest cluster node, streaming audio back to the Pi, and executing TTS playback on the edge node).
- **Multi-Architecture Release Pipelines:** Chained compound commands (`cd ~/mac && ... && cd ~/windows && ...`) executing unified verification.

## Deliverables
1. **Contract Updates:** Document the Federated OMP and Distributed Compute Architecture in root `llms.txt`, `src/llms.txt`, `docs/OPERATIONS.md`, and `docs/FLEET.md`.
2. **Architecture LKGL & Federated Config Engine (`src/fed_shell.rs`, `src/config.rs`):**
   - Implement persistent/in-memory Last Known Good Location (LKGL) mapping per machine.
   - Synchronize working directory transitions across shell executions.
   - Add compute tier and architecture tags to `ClusterMachine` (e.g., `compute_tier: "heavy" | "standard" | "edge"`).
3. **API & Dashboard Integration:**
   - Expose LKGL and machine compute tiers in `/api/v1/shell/machines`.
   - Update `src/dashboard.rs` to display machine compute tiers and auto-suggest optimal nodes.
4. **Verification:**
   - Unit tests and e2e integration tests for LKGL persistence and multi-machine compute dispatch.
   - Release build and zero-dependency compliance.
