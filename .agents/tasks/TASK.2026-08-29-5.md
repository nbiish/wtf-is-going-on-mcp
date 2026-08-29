# TASK 2026-08-29 — hub bins + connect

- Branch: feat/hub-bins-connect
- Goal: paste-bins; cross-machine connect.
- Read: llms.txt, src/llms.txt, SKILL.md.
- Design: 3 bins, hub-persisted.
- Storage: $WTF_HOME/bins.json, 0600.
- API: GET /api/v1/bins.
- API: GET/PUT /api/v1/bins/N.
- Auth: dashboard key or device.
- PUT body: {"content": str}.
- Reject >65536 chars, fail closed.
- Bin update logs hub event.
- State JSON gains "bins".
- MCP tools: read_bin, list_bins.
- Bridge: api_put helper added.
- Dashboard: BIN 1/2/3 section.
- Textareas, save, copy buttons.
- Dirty guard; SSE rerender skips dirty.
- e2e: bins via dash key + bridge.
- Unit: persistence, size, ids.
- Docs: README, llms.txt chain, SKILL.
- Version: 0.2.0 → 0.3.0.
- Gates: cargo test, build --release.
- Verify: live hub, curl smoke.
- Merge: ask operator first.
- Cleanup: worktree + branch after merge.

####
