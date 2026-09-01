# TASK 2026-09-01 — copy CLI-only tts-cli skill

**Branch:** `docs/tts-cli-cli-only-skill`
**Worktree:** `../wtf-tts-cli-skill`
**Classification:** Confidential — no secrets

## Goal

Load the canonical tts-cli skill so hub agents see only `cli-tts` tooling:
one `--prompt` per turn, eleven master answers, no MCP wording.

## Done when

- `.agents/skills/tts-cli/SKILL.md` is byte-identical to tts-cli main.
- Skill file greps clean of `mcp`.

## Non-goals

- Do not touch hub source or other skill packs.
- Do not add a TTS OUTPUT block to this repo's AGENTS.md (no TTS OUTPUT here).
