# MAC-TO-WINDOWS.COMMS.md — three-agent federated append test

Contract: three agent CLIs (OhMyPy `omp`, Hermes, FreeClaudeCode
`fcc-claude`) each append ONE identified line to this file, commanded
from the Mac (or any repo-chat handshake). Each append is verifiable
via `git diff` + the `wtf-is-going-on-mcp` federated chat.

Format (append-only, one line per CLI):
`- <ISO-8601> <cli-name>@<machine>: <one-line result>`

---
- 2026-09-01T17:45:32Z omp@mac: headless append verified via omp CLI
- 2026-09-01T17:45:58Z hermes@mac: headless append verified via hermes CLI
- 2026-09-01T17:53:24Z fcc-claude@mac: headless append verified via fcc-claude through tmux freeclaude-wtf-mcp
