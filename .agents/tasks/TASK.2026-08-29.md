# TASK 2026-08-29 — hub-mvp

Branch: feat/hub-mvp
Worktree: ../hub-mvp
PRD anchor: llms.txt (root) + operator brief (chat)

## Draft
- Read operator contracts. Done.
- Product: multi-agent activity hub.
- Repo name answers question. Yes.
- Hub: HTTP API + dashboard.
- Bridge: MCP stdio, per machine.
- Zero external crates. Std only.
- Own SHA-256, HMAC, JSON, HTTP.
- Transport auth: HMAC-SHA256.
- Headers: device, ts, nonce, sig.
- Replay guard: skew + nonce cache.
- Keys: urandom, 256-bit, 0600.
- Dashboard key gates browser UI.
- State: agents, events, JSONL.
- SSE stream via generation poll.
- Dashboard: embedded, no CDNs.
- Tools: check_in, log_event,
  wtf_is_going_on, ping.
- CLI: serve, key, setup, agent,
  status, help.
- E2E test simulates machine 2.
- Gates: fmt, clippy, test, build.
- PQC lane untouched: no secrets ops.
- No merge without operator confirm.
- rustfmt/clippy absent. Skipped.
- Tests 45 pass. Release ok.
- Audit clean. No secrets.
- DOX indexed. README done.
- Live verify: hub on 7899.
- healthz ok. Unauth 401.
- Signed checkin accepted live.
- Dashboard key gate verified.
- Teardown clean. MVP verified.

####
