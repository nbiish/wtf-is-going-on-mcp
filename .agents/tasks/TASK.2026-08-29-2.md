# TASK 2026-08-29 (2) — Overlay + join

Goal: agents self-enroll off-LAN.
- Worktree feat/overlay-join. ####
- validate(): allow https:// too. ####
- HubConfig advertised_url field. ####
- lan_url prefers advertised URL. ####
- wtf url get/set/clear. ####
- key issue --json one-liner. ####
- wtf join user@host over ssh. ####
- Sanitize name (shell safety). ####
- Reuse setup core in join. ####
- Unit tests: validate, url. ####
- e2e: key --json checkin. ####
- Gates: test, build, scan. ####
- DOX: topologies, join docs. ####
- Live verify: temp home, 7899. ####
- No sshd: stub ssh shim. ####
- Atomic commits. Ask merge. ####
- Tests: 47 green. Release ok. ####
- Audit clean. No secrets. ####
- DOX: topologies, join docs. ####
- Live: url+join+status pass. ####
- Shim proved ssh join flow. ####
