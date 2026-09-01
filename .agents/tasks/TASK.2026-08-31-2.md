# TASK — hub federation + capability-path dashboard (2026-08-31, machine 1)

Operator directive: a hub on EVERY machine; the ledger replicates across all
of them ("Anishinaabe updates on one immediately updates all three"); each
machine serves its dashboard on a LOOPBACK-ONLY endpoint hidden behind a
high-entropy capability path ("hard to hash out"); one machine runs agents
across many repos, with agents from other machines connecting to the same
surface; operator/agents print the URL via CLI or MCP (`wtf dashboard-url`,
`hub_info`).

Reverses the documented ONE-hub-per-fleet invariant (AGENTS.md REPO_STATE,
handoff sheets): machine-2's hub STAYS UP as a peer; windows-1 enrolls on
its LOCAL hub; the mac PSK handoff is moot (still fine if ever needed).
Comms announcement #6 posted to mac-win-pipeline.

## Approach (converged with operator in-session)

Full-mesh push replication over the existing HMAC-SHA256 request lane
(standard-transport, same as agent auth — transport crypto, not secrets-at-rest;
the ledger is a public-surface event log by design, secrets forbidden).
- Each hub gets a stable name + peer table in config (`federation.json` 0600).
- Every event carries an origin hub id; replication is push-on-append plus
  anti-entropy pull on a 30 s cadence; generation-cursor pull (hub-assigned
  monotonic per hub) + (origin, event_id) dedupe keep the mesh loop-free
  without global clocks or CRDTs — append-only feed, last-writer-wins on
  agent cards, consistent with the existing single-node model.
- Dashboard: bind loopback-only by default; root served only at
  `/w/<64-hex capability token>`; wrong/absent token → uniform 404 (same
  page as any unknown path, no oracle). API stays on its signed routes;
  `/healthz` stays open (no state).
- Multi-repo agents: optional `repo` field on checkin/event, surfaced in
  state + dashboard grouping (machine → agent → repos). One bridge per
  machine per repo or per agent identity is documented, not enforced.

## Shipped

- `src/federation.rs` (new): `FedConfig` (hub `name` minted on first
  serve, stable; peer table with per-peer `{name, url, device, device_key}`
  — the credential THE PEER issued — in `federation.json`, 0600, atomic),
  dashboard capability token (`dashboard_capability`, 0600, 64-hex,
  auto-minted, corrupt file regenerates), push-envelope shape validation.
- `src/store.rs`: events gain `origin` + `origin_id` + `repo`;
  `Store::set_origin_name` stamps local events; `ingest` dedupes on
  (origin, origin_id), assigns local ids, last-writer-wins agent cards by
  ts; cursors `max_origin_id` / `events_since`; replay keeps old events
  first-class.
- `src/api.rs`: `POST /api/v1/fed/push` (device-authed, `fed-*`
  credential required, ingest-deduped, ingress event validation), `GET
  /api/v1/fed/pull?origin=&after=` (cursor pull for anti-entropy), `GET
  /api/v1/fed/peers` (real fed identity for link-time adoption). Dashboard
  page served ONLY at `/w/<capability>`; loopback hubs gate on the token
  alone, LAN hubs also accept `?k=`; uniform 404 on wrong/absent token.
  `checkin`/`event` accept optional `repo`; state JSON agents carry
  `repo` + `origin`.
- `src/replicate.rs` (new): per-peer replicator thread — push-on-append
  (generation-triggered) + 10 s anti-entropy sweep over the standard HMAC
  device lane; throttled warns; never crashes the hub.
- `src/main.rs`: serve mints/stamps the fed name, spawns replication when
  peers exist, prints the capability dashboard link on loopback hubs;
  `wtf federate add <name> --url U --psk S [--as DEV]` (PSK handshake as
  `fed-<hub-name>`, adopts the peer's REAL fed identity via signed
  `/api/v1/fed/peers`, verifies with an anti-entropy round-trip) /
  `federate list` / `federate remove`; `wtf dashboard-url` prints the
  localhost capability URL.
- `src/mcp.rs`: `check_in`/`log_event` accept `repo` (default = bridge
  cwd basename, `WTF_REPO` overrides); `hub_info` points operators at
  `wtf dashboard-url` for the capability link.
- `src/dashboard.rs`: agents group by origin hub (chips), repo chips per
  agent/event; auth via `?cap=` on loopback, `?k=` elsewhere.
- Docs: README (federation section, dashboard section, security bullets,
  CLI + API tables, troubleshooting), root + src `llms.txt`, AGENTS.md
  REPO_STATE, SKILL.md reporting contract (multi-repo note; mirror sync
  owed post-merge). Version 0.10.0 -> 0.11.0.

## Gates

- [x] cargo test: 100 unit + 12 e2e green (new
  `federation_two_hub_end_to_end`: two real hubs on pinned ports, one
  `federate add`, events checked in on both hubs appear on both ledgers
  origin-tagged; `federation.json` asserted 0600; capability dashboard
  200 / wrong-token 404 / absent-path 404).
- [x] cargo build --release clean (wtf 0.11.0).
- [x] Secret grep of diff: see verification phase (runs pre-merge).

## Notes

- Loopback default changes behavior for LAN dashboards; operator-approved.
- Debugging notes: (1) the dashboard route needed explicit `/w/` dispatch —
  exact-match `GET /` never saw capability paths; (2) `federate add` must
  adopt the peer's REAL fed identity (its minted `hub-<hex>` name), not the
  operator's label — pull cursors address the origin name; (3) fed push/pull
  authz is on the CALLER's `fed-*` credential, not per-origin device
  coupling — one credential serves both directions and the caller != origin
  case (pulling a peer's events) is the common path; (4) e2e must pin hub
  ports because `federation.json` records the peer URL at add time.
- Mesh verified by hand beyond the e2e: two hubs converge both ways in
  <3 s; single-warn on peer-down, no crash; dedupe holds under restart.
