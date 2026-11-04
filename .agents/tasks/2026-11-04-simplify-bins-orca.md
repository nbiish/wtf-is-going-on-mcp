# TASK simplify-bins-orca (2026-11-04)

- lane: now
- scope: Remove the paste-bin subsystem end-to-end; consolidate three wtf skills into one Herdr-first skill distributed from ainish-coder; simplify the dashboard.
- branch: refactor/wtf-simplify-bins-orca
- worktree: ../wtf-simplify (removed after merge)
- status: done — merged to main (6a972ad), v0.17.0 rebuilt and smoke-tested

## Result

- `src/bins.rs` (442 lines) deleted; `/api/v1/bins*` routes + handlers gone (uniform 404 verified live).
- MCP tools 20 → 17: `read_bin`, `write_bin`, `list_bins` removed.
- `wtf bin ls|get|put` CLI removed; dashboard paste-bin grid, scope tabs, and ~180 lines of JS removed; `bins`/`bin_scopes` dropped from state JSON; `bins.json` no longer written.
- One skill: `.agents/skills/wtf-is-going-on-mcp/SKILL.md`, canonical in `~/code-external/ainish-coder`, byte-identical here, embedded via `include_str!`. Replaced `wtf-mcp-mastery` + `wtf-agent-hub` + `wtf-observability`.
- Herdr-first: Herdr owns agent sessions; wtf owns the record and encrypted lanes.
- Gates: 106 unit + 12 e2e green, clippy clean (17 pre-existing warnings, unchanged from baseline).
- Net: 16 files changed, 99 insertions, 2107 deletions.

## Follow-ups

- `wip/operator-preserve-2026-11-03` (fc3e2f0) holds operator WIP that must be rebased onto v0.17.0 — CompletionsConfig + hermes/pi switching, dashboard config modal, router pills. Its bin-federation half is obsolete.
- Dashboard simplification landed as bin removal only; the broader "much more simplified" rework is unstarted.
