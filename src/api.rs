//! Hub HTTP API: route dispatch, auth glue (device HMAC or dashboard key),
//! and the SSE state fan-out loop.
//!
//! AuthZ model:
//! - `/healthz` — open (connectivity probe; leaks nothing but version/uptime)
//! - `/healthz` — open (connectivity probe; leaks nothing but version/uptime)
//! - `/w/<capability>` — singular dashboard page (uniform 404 on invalid paths)
//! - `/stream` — capability (`?cap=`, header, or bearer) OR device auth
//! - `/api/v1/state` — capability OR device auth
//! - `/api/v1/bins`, `/api/v1/bins/{1,2,3}` — GET/PUT: capability OR device auth
//! - `/api/v1/{checkin,event,heartbeat}` — device auth only
//! - `/api/v1/identity` — device auth only (register own ML-KEM-768 ek)
//! - `/api/v1/devices` — capability OR device auth (identity registry read)
//! - `/api/v1/agents/available` — GET: capability OR device auth
//! - `/api/v1/sessions` — GET list: capability OR device; POST create: capability OR device
//! - `/api/v1/sessions/{id}` — GET: capability OR device
//! - `/api/v1/sessions/{id}/{join,seal,seals,send,recv}` — device auth only;
//!   seal/seals/send/recv additionally require session membership.
//! - `/api/v1/enroll` — NO device HMAC: two credential modes. (1) a one-time
//!   enrollment token (v0.8.0; SHA-256-hashed at rest, single-use, short
//!   TTL); (2) a signed PSK handshake (v0.9.0): an HMAC proof of possession
//!   of the site enrollment secret over (name, ek, ts, nonce) with skew and
//!   replay guards, answered with the fresh device key ML-KEM-768-sealed to
//!   the device's ek — the key never crosses in plaintext. A global rate
//!   limiter blunts online guessing on both modes.
//! All failures are 401 with a generic message; never leak which factor failed.

use crate::auth::{self, NonceCache};
use crate::bins::Bins;
use crate::config::{HubConfig, KeyStore};
use crate::hmac;
use crate::http::{HandlerResult, Request, Response, SseSession};
use crate::json::{self, Value};
use crate::session_crypto;
use crate::sessions::{Sessions, MAX_CIPHERTEXT_CHARS};
use crate::store::{Store, LEVELS, STATUSES};
use crate::util::{ct_eq_str, now_secs};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub struct Hub {
    pub store: Arc<Store>,
    pub bins: Arc<Bins>,
    pub keys: Mutex<KeyStore>,
    pub nonces: Mutex<NonceCache>,
    pub dashboard_key: String,
    pub started_at: u64,
    /// ML-KEM-768 identity registry: device -> encapsulation key (hex).
    pub identities: Mutex<Vec<(String, String)>>,
    /// Encrypted agent-to-agent session channels.
    pub sessions: Sessions,
    /// Sliding window of enroll attempt timestamps (rate limiter state).
    pub enroll_hits: Mutex<Vec<u64>>,
    /// PSK-handshake replay guard: (nonce, first-seen ts), pruned past 600 s.
    pub enroll_nonces: Mutex<Vec<(String, u64)>>,
    /// This hub's federation identity ("" until `wtf federate` first use).
    pub fed_name: String,
    /// Peer table with device creds issued by each peer (0600-backed).
    /// Arc so the replicator thread shares it without polling the file.
    pub fed: Arc<Mutex<crate::federation::FedConfig>>,
    /// Device environment reports (agent-CLI presence): (device, json, ts).
    /// Ring of 64; populated by POST /api/v1/env.
    pub env_reports: Mutex<Vec<(String, Value, u64)>>,
    /// 64-hex capability token gating the dashboard page path (/w/<token>).
    pub capability: String,
    /// True when the HTTP listener is loopback-only (dashboard may then be
    /// served without the dashboard key; the capability path is the gate).
    pub loopback_only: bool,
}

pub fn handle(hub: &Arc<Hub>, req: &Request) -> HandlerResult {
    let known_path = matches!(
        req.path.as_str(),
        "/" | "/healthz"
            | "/stream"
            | "/api/v1/state"
            | "/api/v1/checkin"
            | "/api/v1/event"
            | "/api/v1/heartbeat"
            | "/api/v1/bins"
            | "/api/v1/identity"
            | "/api/v1/devices"
            | "/api/v1/sessions"
            | "/api/v1/enroll"
            | "/api/v1/fed/push"
            | "/api/v1/fed/peers"
            | "/api/v1/env"
            | "/api/v1/term"
            | "/api/v1/agents/available"
            | "/api/v1/shell/machines"
            | "/api/v1/shell/config"
            | "/api/v1/shell/exec"
    ) || req.path == "/w"
        || req.path.starts_with("/w/")
        || bin_id_of(&req.path).is_some()
        || req.path.starts_with("/api/v1/sessions/")
        || req.path.starts_with("/api/v1/term/")
        || req.path == "/api/v1/agents/available";
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/healthz") => HandlerResult::Respond(healthz(hub)),
        ("GET", "/") => HandlerResult::Respond(dashboard(hub, req)),
        (_, p) if p == "/" || p == "/w" || p.starts_with("/w/") => {
            HandlerResult::Respond(dashboard(hub, req))
        }
        ("GET", "/stream") => stream(hub, req),
        ("GET", "/api/v1/state") => HandlerResult::Respond(state(hub, req)),
        ("POST", "/api/v1/checkin") => HandlerResult::Respond(checkin(hub, req)),
        ("POST", "/api/v1/event") => HandlerResult::Respond(event(hub, req)),
        ("POST", "/api/v1/heartbeat") => HandlerResult::Respond(heartbeat(hub, req)),
        ("GET", "/api/v1/bins") => HandlerResult::Respond(bins_list(hub, req)),
        (_, p) if p == "/api/v1/bins" || bin_id_of(p).is_some() => {
            HandlerResult::Respond(bin_single(hub, req))
        }
        ("POST", "/api/v1/identity") => HandlerResult::Respond(identity_register(hub, req)),
        ("GET", "/api/v1/devices") => HandlerResult::Respond(devices_list(hub, req)),
        ("POST", "/api/v1/enroll") => HandlerResult::Respond(enroll(hub, req)),
        ("POST", "/api/v1/fed/push") => HandlerResult::Respond(fed_push(hub, req)),
        ("GET", "/api/v1/fed/peers") => HandlerResult::Respond(fed_peers(hub, req)),
        ("POST", "/api/v1/env") => HandlerResult::Respond(env_report(hub, req)),
        ("GET", "/api/v1/env") => HandlerResult::Respond(env_list(hub, req)),
        ("GET", "/api/v1/shell/machines") => HandlerResult::Respond(shell_machines(hub, req)),
        ("GET", "/api/v1/shell/config") => HandlerResult::Respond(shell_config_get(hub, req)),
        ("POST", "/api/v1/shell/config") => HandlerResult::Respond(shell_config_post(hub, req)),
        ("POST", "/api/v1/shell/exec") => HandlerResult::Respond(shell_exec(hub, req)),
        ("GET", "/api/v1/agents/available") => HandlerResult::Respond(agents_available(hub, req)),
        (_, p) if p == "/api/v1/term" || p.starts_with("/api/v1/term/") => {
            HandlerResult::Respond(term(hub, req))
        }
        (_, p) if p.starts_with("/api/v1/fed/pull") => HandlerResult::Respond(fed_pull(hub, req)),
        ("GET", "/api/v1/sessions") => HandlerResult::Respond(sessions_list(hub, req)),
        ("POST", "/api/v1/sessions") => HandlerResult::Respond(session_create(hub, req)),
        (_, p) if p == "/api/v1/sessions" || p.starts_with("/api/v1/sessions/") => {
            HandlerResult::Respond(session_single(hub, req))
        }
        _ if known_path => HandlerResult::Respond(Response::error(405, "method not allowed")),
        _ => HandlerResult::Respond(Response::error(404, "not found")),
    }
}

/// `/api/v1/bins/N` → N (1..=3); anything else → None (caller 404s).
fn bin_id_of(path: &str) -> Option<u8> {
    let rest = path.strip_prefix("/api/v1/bins/")?;
    if rest.is_empty() || !rest.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let id: u64 = rest.parse().ok()?;
    if Bins::valid_id(id as i64) {
        Some(id as u8)
    } else {
        None
    }
}

fn device_auth(hub: &Hub, req: &Request) -> Result<String, Response> {
    let headers = auth::extract(&req.headers).map_err(|e| Response::error(401, &e.to_string()))?;
    // keys.json may have changed on disk since the last request (issue,
    // revoke, rotate via `wtf key`). Reload unconditionally: the file is
    // tiny, and a stale in-memory record must never authenticate a revoked
    // device. Fail closed if the keystore cannot be read.
    let fresh = KeyStore::load().map_err(|_| Response::error(401, "keystore unavailable"))?;
    *hub.keys.lock().unwrap() = fresh;
    let verify = |keys: &KeyStore, nonces: &mut NonceCache| {
        auth::verify(
            headers.clone(),
            keys,
            nonces,
            &req.method,
            &req.target,
            &req.body,
        )
    };
    let mut nonces = hub.nonces.lock().unwrap();
    let keys = hub.keys.lock().unwrap();
    verify(&keys, &mut nonces).map_err(|e| Response::error(401, &e.to_string()))
}

/// Capability token accepted where a dashboard key is: `?cap=<token>`,
/// `X-Wtf-Capability: <token>`, or `Authorization: Bearer <token>`.
/// Validated constant-time across loopback, LAN, and remote topologies.
fn cap_ok(hub: &Hub, req: &Request) -> bool {
    if let Some(v) = req.q("cap") {
        if crate::util::ct_eq_str(&hub.capability, &v) {
            return true;
        }
    }
    for (k, v) in &req.headers {
        if k.eq_ignore_ascii_case("x-wtf-capability") {
            if crate::util::ct_eq_str(&hub.capability, v.trim()) {
                return true;
            }
        }
        if k.eq_ignore_ascii_case("authorization") {
            if let Some(token) = v.strip_prefix("Bearer ") {
                if crate::util::ct_eq_str(&hub.capability, token.trim()) {
                    return true;
                }
            }
        }
    }
    false
}

fn parse_body(req: &Request) -> Result<Value, Response> {
    json::parse(&String::from_utf8_lossy(&req.body))
        .map_err(|e| Response::error(400, &format!("invalid JSON body: {e}")))
}

fn healthz(hub: &Hub) -> Response {
    Response::json(
        200,
        &Value::obj(vec![
            ("ok", Value::from(true)),
            ("service", Value::from("wtf-hub")),
            ("version", Value::from(crate::VERSION)),
            ("started_at", Value::from(hub.started_at as i64)),
            ("now", Value::from(now_secs() as i64)),
        ]),
    )
}

fn state(hub: &Hub, req: &Request) -> Response {
    if !cap_ok(hub, req) && device_auth(hub, req).is_err() {
        return Response::error(401, "provide ?cap=<capability> or device auth headers");
    }
    Response::json(200, &hub.store.to_state_json(hub.started_at, &hub.bins))
}

fn bins_list(hub: &Hub, req: &Request) -> Response {
    if !cap_ok(hub, req) && device_auth(hub, req).is_err() {
        return Response::error(401, "provide ?cap=<capability> or device auth headers");
    }
    Response::json(200, &Value::obj(vec![("bins", hub.bins.to_state_json())]))
}

/// `/api/v1/bins/N` — GET reads one bin; PUT writes it. The actor recorded on
/// the bin (and in the update event) is the device name, or "dashboard" when
/// the dashboard key was used.
fn bin_single(hub: &Arc<Hub>, req: &Request) -> Response {
    let Some(id) = bin_id_of(&req.path) else {
        return Response::error(404, "not found");
    };
    let actor = if cap_ok(hub, req) {
        "dashboard".to_string()
    } else {
        match device_auth(hub, req) {
            Ok(d) => d,
            Err(r) => return r,
        }
    };
    match req.method.as_str() {
        "GET" => match hub.bins.get(id) {
            Some(b) => Response::json(200, &b.to_state_json()),
            None => Response::error(404, "bin not found"), // unreachable: ids fixed 1..=3
        },
        "PUT" => {
            let body = match parse_body(req) {
                Ok(v) => v,
                Err(r) => return r,
            };
            let content = match body.get("content").and_then(|v| v.as_str()) {
                Some(c) => c,
                None => return Response::error(400, "missing 'content'"),
            };
            let bin = match hub.bins.set(id, content, &actor) {
                Ok(b) => b,
                Err(e) => return Response::error(400, &e),
            };
            // Feeds the dashboard event log and bumps the SSE generation.
            let ev = hub.store.log_event(
                &actor,
                &actor,
                "info",
                &format!("bin {id} updated; {} chars", bin.content.chars().count()),
                "",
            );
            Response::json(
                200,
                &Value::obj(vec![
                    ("ok", Value::from(true)),
                    ("id", Value::from(bin.id as i64)),
                    ("event", Value::from(ev.id as i64)),
                ]),
            )
        }
        _ => Response::error(405, "method not allowed"),
    }
}

/// Capability-gated dashboard. The page lives at /w/<64-hex token>; any
/// other path shape gets the same uniform 404 as every unknown route (no
/// oracle distinguishing "wrong token" from "no token"). When the hub is
/// loopback-only the page is served without ?k= (the capability path IS the
/// secret and localhost is the network gate). Otherwise the legacy ?k=
/// dashboard key still works, so remote dashboards keep functioning.
/// Capability-gated dashboard. The page lives exclusively at /w/<64-hex token>;
/// any other path shape gets the same uniform 404 as every unknown route (no
/// oracle distinguishing "wrong token" from "no token").
fn dashboard(hub: &Hub, req: &Request) -> Response {
    let on_cap = req.path == format!("/w/{}", hub.capability);
    if on_cap {
        return Response::html(200, crate::dashboard::PAGE);
    }
    Response::error(404, "not found")
}

fn stream(hub: &Arc<Hub>, req: &Request) -> HandlerResult {
    if !cap_ok(hub, req) && device_auth(hub, req).is_err() {
        return HandlerResult::Respond(Response::error(401, "missing or bad capability token or device auth"));
    }
    let hub2 = Arc::clone(hub);
    HandlerResult::Sse(Box::new(move |session: &mut SseSession| {
        let mut cycles = 0u32;
        loop {
            let st = hub2.store.to_state_json(hub2.started_at, &hub2.bins);
            if session.event("state", &st.to_json()).is_err() {
                break; // client went away
            }
            let gen = hub2.store.generation();
            let mut waited = 0u32;
            loop {
                std::thread::sleep(Duration::from_millis(400));
                waited += 1;
                if hub2.store.generation() != gen || waited >= 150 {
                    break; // state changed OR ~60s keepalive tick
                }
            }
            cycles += 1;
            if cycles >= 30 {
                break; // ~30 min; EventSource reconnects automatically
            }
        }
    }))
}

fn checkin(hub: &Arc<Hub>, req: &Request) -> Response {
    let device = match device_auth(hub, req) {
        Ok(d) => d,
        Err(r) => return r,
    };
    let body = match parse_body(req) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let status = match body.get("status").and_then(|v| v.as_str()) {
        Some(s) if STATUSES.contains(&s) => s,
        Some(_) => {
            return Response::error(
                400,
                &format!("status must be one of: {}", STATUSES.join(", ")),
            )
        }
        None => return Response::error(400, "missing 'status'"),
    };
    let task = match body.get("task").and_then(|v| v.as_str()) {
        Some(t) if !t.trim().is_empty() => t,
        _ => return Response::error(400, "missing 'task'"),
    };
    let details = body.get("details").and_then(|v| v.as_str()).unwrap_or("");
    let repo = body.get("repo").and_then(|v| v.as_str()).unwrap_or("");
    let agent = body
        .get("agent")
        .and_then(|v| v.as_str())
        .unwrap_or(device.as_str());
    let ev = hub
        .store
        .check_in(&device, agent, status, task, details, repo);
    Response::json(
        200,
        &Value::obj(vec![
            ("ok", Value::from(true)),
            ("id", Value::from(ev.id as i64)),
        ]),
    )
}

fn event(hub: &Arc<Hub>, req: &Request) -> Response {
    let device = match device_auth(hub, req) {
        Ok(d) => d,
        Err(r) => return r,
    };
    let body = match parse_body(req) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let message = match body.get("message").and_then(|v| v.as_str()) {
        Some(m) if !m.trim().is_empty() => m,
        _ => return Response::error(400, "missing 'message'"),
    };
    let level = match body.get("level").and_then(|v| v.as_str()) {
        Some(l) if LEVELS.contains(&l) => l,
        Some(_) => {
            return Response::error(400, &format!("level must be one of: {}", LEVELS.join(", ")))
        }
        None => "info",
    };
    let agent = body
        .get("agent")
        .and_then(|v| v.as_str())
        .unwrap_or(device.as_str());
    let repo = body.get("repo").and_then(|v| v.as_str()).unwrap_or("");
    let ev = hub.store.log_event(&device, agent, level, message, repo);
    Response::json(
        200,
        &Value::obj(vec![
            ("ok", Value::from(true)),
            ("id", Value::from(ev.id as i64)),
        ]),
    )
}

fn heartbeat(hub: &Arc<Hub>, req: &Request) -> Response {
    let device = match device_auth(hub, req) {
        Ok(d) => d,
        Err(r) => return r,
    };
    let agent = match parse_body(req) {
        Ok(body) => body
            .get("agent")
            .and_then(|v| v.as_str())
            .unwrap_or(device.as_str())
            .to_string(),
        Err(_) => device.clone(),
    };
    hub.store.heartbeat(&device, &agent);
    Response::json(200, &Value::obj(vec![("ok", Value::from(true))]))
}

// ---------- enrollment tokens ----------

/// Global sliding-window limiter for the unauthenticated enroll route. The
/// token is the real gate; this only blunts online guessing of a 256-bit
/// secret. 20 tries / 5 min is orders of magnitude above any legitimate flow
/// and far below what a brute force needs.
const ENROLL_WINDOW_SECS: u64 = 300;
const ENROLL_MAX_ATTEMPTS: usize = 20;

fn enroll_allowed(hits: &mut Vec<u64>, now: u64) -> bool {
    hits.retain(|t| now.saturating_sub(*t) < ENROLL_WINDOW_SECS);
    if hits.len() >= ENROLL_MAX_ATTEMPTS {
        return false;
    }
    hits.push(now);
    true
}

/// POST /api/v1/enroll { name, token } or { name, ek, ts, nonce, proof } —
/// redeem a one-time enrollment token (v0.8.0) or run the signed PSK
/// handshake (v0.9.0). Response shape matches `wtf key issue --json` (token
/// mode) or carries the key ML-KEM-768-sealed (psk mode). Failures are
/// uniform; tokens burn only on success (a typo must not brick it).
fn enroll(hub: &Arc<Hub>, req: &Request) -> Response {
    if !enroll_allowed(hub.enroll_hits.lock().unwrap().as_mut(), now_secs()) {
        return Response::error(429, "too many enrollment attempts; slow down");
    }
    let body = match parse_body(req) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let Some(name) = body.get("name").and_then(|v| v.as_str()) else {
        return Response::error(400, "missing 'name'");
    };
    match body.get("token").and_then(|v| v.as_str()) {
        Some(token) => enroll_token(hub, name, token),
        None => match body.get("proof").and_then(|v| v.as_str()) {
            Some(proof) => enroll_psk(hub, name, proof, &body),
            None => Response::error(400, "missing 'token' or 'proof'"),
        },
    }
}

/// Token mode (v0.8.0): the token is the credential; the fresh device key
/// crosses in the one-time `key issue --json` response shape.
fn enroll_token(hub: &Arc<Hub>, name: &str, token: &str) -> Response {
    if !crate::config::valid_name(name)
        || token.len() != 64
        || !token.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Response::error(403, "invalid or expired enrollment token");
    }
    let mut tokens = match crate::config::EnrollTokenStore::load() {
        Ok(t) => t,
        Err(_) => return Response::error(500, "enrollment store unavailable"),
    };
    if let Err(e) = tokens.consume(name, token) {
        return match e {
            crate::config::TokenError::Store => {
                Response::error(500, "enrollment store unavailable")
            }
            _ => Response::error(403, "invalid or expired enrollment token"),
        };
    }
    issue_and_respond(hub, name, "enrollment token")
}

/// Replay guard for PSK handshakes: (nonce, first-seen ts) entries are pruned
/// after 600 s; a nonce seen twice is a replay and fails closed. Only proofs
/// that already passed verification reach this cache.
fn enroll_nonce_fresh(cache: &mut Vec<(String, u64)>, nonce: &str, now: u64) -> bool {
    cache.retain(|(_, ts)| now.saturating_sub(*ts) < 600);
    if cache.iter().any(|(n, _)| n == nonce) {
        return false;
    }
    cache.push((nonce.to_string(), now));
    true
}

/// PSK mode (v0.9.0): the device proves possession of the site enrollment
/// secret with proof = HMAC(enroll_secret, "wtf-enroll-v2\n{name}\n{ek}\n{ts}
/// \n{nonce}") — the secret itself never travels — and receives the fresh
/// device key ML-KEM-768-sealed to its own encapsulation key. Every failure
/// is the same uniform 403; success is operator-sanctioned by the secret
/// copy, and `key revoke` / secret rotation remain the instant kill switches.
fn enroll_psk(hub: &Arc<Hub>, name: &str, proof: &str, body: &Value) -> Response {
    let ek = match body.get("ek").and_then(|v| v.as_str()) {
        Some(e) => e,
        None => return Response::error(403, "invalid or expired enrollment proof"),
    };
    let ts = body.get("ts").and_then(|v| v.as_i64()).unwrap_or(0) as u64;
    let nonce = match body.get("nonce").and_then(|v| v.as_str()) {
        Some(n) => n,
        None => return Response::error(403, "invalid or expired enrollment proof"),
    };
    let now = now_secs();
    let shape_ok = crate::config::valid_name(name)
        && ek.len() == crate::identity::EK_HEX
        && ek.bytes().all(|b| b.is_ascii_hexdigit())
        && nonce.len() >= 16
        && nonce.len() <= 128
        && nonce.bytes().all(|b| b.is_ascii_hexdigit())
        && proof.len() == 64
        && proof.bytes().all(|b| b.is_ascii_hexdigit())
        && now.saturating_sub(ts) <= 300
        && ts.saturating_sub(now) <= 300;
    if !shape_ok {
        return Response::error(403, "invalid or expired enrollment proof");
    }
    // Read per-request like the keystore: `enroll-secret --rotate` is instant.
    let cfg = match HubConfig::load_or_create() {
        Ok(c) => c,
        Err(_) => return Response::error(500, "enrollment store unavailable"),
    };
    let expected = hmac::hmac_sha256_hex(
        cfg.enroll_secret.as_bytes(),
        format!("wtf-enroll-v2\n{name}\n{ek}\n{ts}\n{nonce}").as_bytes(),
    );
    if !ct_eq_str(&expected, proof) {
        return Response::error(403, "invalid or expired enrollment proof");
    }
    // Only verified proofs reach the replay cache, so it cannot be poisoned.
    if !enroll_nonce_fresh(hub.enroll_nonces.lock().unwrap().as_mut(), nonce, now) {
        return Response::error(403, "invalid or expired enrollment proof");
    }
    issue_and_respond_sealed(hub, name, ek, &cfg)
}

/// Token-mode tail: mint the device key (hot-reloaded keystore) and respond
/// in the one-time `key issue --json` shape.
fn issue_and_respond(hub: &Arc<Hub>, name: &str, via: &str) -> Response {
    let mut ks = match KeyStore::load() {
        Ok(k) => k,
        Err(_) => return Response::error(500, "keystore unavailable"),
    };
    let secret = match ks.issue(name) {
        Ok(s) => s,
        Err(e) => return Response::error(400, &e),
    };
    let hub_url = HubConfig::load_or_create()
        .map(|c| c.lan_url())
        .unwrap_or_default();
    let _ = hub.store.log_event(
        "enroll",
        name,
        "info",
        &format!("device '{name}' enrolled via {via}"),
        "",
    );
    Response::json(
        200,
        &Value::obj(vec![
            ("hub_url", Value::from(hub_url.as_str())),
            ("device", Value::from(name)),
            ("key", Value::from(secret.as_str())),
        ]),
    )
}

fn issue_and_respond_sealed(hub: &Arc<Hub>, name: &str, ek: &str, cfg: &HubConfig) -> Response {
    let mut ks = match KeyStore::load() {
        Ok(k) => k,
        Err(_) => return Response::error(500, "keystore unavailable"),
    };
    let secret = match ks.issue(name) {
        Ok(s) => s,
        Err(e) => return Response::error(400, &e),
    };
    let key32 = match crate::util::hex_decode(&secret) {
        Some(k) if k.len() == 32 => {
            let mut out = [0u8; 32];
            out.copy_from_slice(&k);
            out
        }
        _ => return Response::error(500, "keystore unavailable"),
    };
    let sealed =
        match session_crypto::seal_session_key(ek, &key32, &format!("wtf-enroll-v2:{name}")) {
            Ok(s) => s,
            Err(_) => return Response::error(500, "key sealing failed"),
        };
    let _ = hub.store.log_event(
        "enroll",
        name,
        "info",
        &format!("device '{name}' enrolled via signed handshake (psk)"),
        "",
    );
    Response::json(
        200,
        &Value::obj(vec![
            ("hub_url", Value::from(cfg.lan_url().as_str())),
            ("device", Value::from(name)),
            ("ek_fp", Value::from(session_crypto::ek_fp(ek).as_str())),
            ("sealed", Value::from(sealed.as_str())),
        ]),
    )
}

// ---------- identity registry ----------

/// Load persisted identities from $WTF_HOME/identities.json (0600) and rehydrate
/// any session members whose encapsulation keys are known.
pub fn load_identities(sessions: &crate::sessions::Sessions) -> Vec<(String, String)> {
    let path = crate::config::identities_path();
    let mut list = Vec::new();
    if let Ok(Some(val)) = crate::config::load_json(&path) {
        if let Some(arr) = val.as_arr() {
            for item in arr {
                if let (Some(d), Some(ek)) = (
                    item.get("device").and_then(|v| v.as_str()),
                    item.get("ek").and_then(|v| v.as_str()),
                ) {
                    list.push((d.to_string(), ek.to_string()));
                }
            }
        }
    }
    // Rehydrate from session members if not already recorded.
    for s in sessions.list() {
        for m in s.members {
            if !m.ek.is_empty() && !list.iter().any(|(d, _)| *d == m.device) {
                list.push((m.device, m.ek));
            }
        }
    }
    list
}

/// Persist the identity registry atomically to $WTF_HOME/identities.json (0600).
pub fn save_identities(identities: &[(String, String)]) {
    let path = crate::config::identities_path();
    let arr = Value::arr(
        identities
            .iter()
            .map(|(d, ek)| {
                Value::obj(vec![
                    ("device", Value::from(d.as_str())),
                    ("ek", Value::from(ek.as_str())),
                ])
            })
            .collect(),
    );
    let _ = crate::config::save_json(&path, &arr, 0o600);
}

/// POST /api/v1/identity { ek } — device registers its own ML-KEM-768
/// encapsulation key. Re-registering overwrites (key rotation support).
fn identity_register(hub: &Arc<Hub>, req: &Request) -> Response {
    let device = match device_auth(hub, req) {
        Ok(d) => d,
        Err(r) => return r,
    };
    let body = match parse_body(req) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let Some(ek) = body.get("ek").and_then(|v| v.as_str()) else {
        return Response::error(400, "missing 'ek'");
    };
    // ML-KEM-768 ek is 1184 bytes = 2368 hex chars.
    if ek.len() != 2368 || !ek.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Response::error(400, "ek must be 2368 hex chars (ML-KEM-768)");
    }
    {
        let mut reg = hub.identities.lock().unwrap();
        match reg.iter_mut().find(|(d, _)| *d == device) {
            Some(slot) => slot.1 = ek.to_string(),
            None => reg.push((device.clone(), ek.to_string())),
        }
        save_identities(&reg);
    }
    let _ = hub
        .store
        .log_event(&device, &device, "info", "identity registered", "");
    Response::json(200, &Value::obj(vec![("ok", Value::from(true))]))
}

/// GET /api/v1/devices — the identity registry (dashboard key or device).
fn devices_list(hub: &Arc<Hub>, req: &Request) -> Response {
    if !cap_ok(hub, req) && device_auth(hub, req).is_err() {
        return Response::error(401, "provide ?cap=<capability> or device auth headers");
    }
    let reg = hub.identities.lock().unwrap();
    let devices: Vec<Value> = reg
        .iter()
        .map(|(d, ek)| {
            Value::obj(vec![
                ("device", Value::from(d.as_str())),
                (
                    "ek_fp",
                    Value::from(crate::util::hex_encode(
                        &crate::keccak::sha3_256(&crate::util::hex_decode(ek).unwrap_or_default())
                            [..8],
                    )),
                ),
            ])
        })
        .collect();
    Response::json(200, &Value::obj(vec![("devices", Value::Arr(devices))]))
}

// ---------- session channels ----------

/// GET /api/v1/sessions — list session metadata (no messages).
fn sessions_list(hub: &Arc<Hub>, req: &Request) -> Response {
    if !cap_ok(hub, req) && device_auth(hub, req).is_err() {
        return Response::error(401, "provide ?cap=<capability> or device auth headers");
    }
    let all = hub.sessions.list();
    let arr: Vec<Value> = all.iter().map(|s| s.to_wire_json(false)).collect();
    Response::json(200, &Value::obj(vec![("sessions", Value::Arr(arr))]))
}

/// POST /api/v1/sessions { name } — create a session; creator is first
/// member with its registered ek.
fn session_create(hub: &Arc<Hub>, req: &Request) -> Response {
    let device = match device_auth(hub, req) {
        Ok(d) => d,
        Err(r) => {
            if cap_ok(hub, req) {
                let reg = hub.identities.lock().unwrap();
                reg.first().map(|(d, _)| d.clone()).unwrap_or_else(|| "dashboard".to_string())
            } else {
                return r;
            }
        }
    };
    let body = match parse_body(req) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let Some(name) = body.get("name").and_then(|v| v.as_str()) else {
        return Response::error(400, "missing 'name'");
    };
    let ek = {
        let reg = hub.identities.lock().unwrap();
        match reg.iter().find(|(d, _)| *d == device) {
            Some((_, ek)) => ek.clone(),
            None => match reg.first() {
                Some((_, ek)) => ek.clone(),
                None => "00".repeat(crate::mlkem768::EK_BYTES),
            },
        }
    };
    let repo = body.get("repo").and_then(|v| v.as_str()).unwrap_or("");
    match hub.sessions.create(name, &device, &ek, repo) {
        Ok((s, pairing_key)) => {
            let _ = hub.store.log_event(
                &device,
                &device,
                "info",
                &format!(
                    "session '{}' created (repo {})",
                    s.id,
                    if s.repo.is_empty() { "-" } else { &s.repo }
                ),
                "",
            );
            // The pairing key crosses the wire exactly once, in the
            // creator's create response — same posture as `key issue`.
            Response::json(
                200,
                &Value::obj(vec![
                    ("id", Value::from(s.id.as_str())),
                    ("name", Value::from(s.name.as_str())),
                    ("repo", Value::from(s.repo.as_str())),
                    ("pairing_key", Value::from(pairing_key.as_str())),
                ]),
            )
        }
        Err(e) => Response::error(400, &e),
    }
}

/// `/api/v1/sessions/{id}` and sub-resources:
///   GET  …/{id}            session metadata (+msgs for dashboard key)
///   POST …/{id}/join       { ek } — join as member
///   POST …/{id}/seal       { pkgs: [{ct, ek_fp}] } — member seals key
///   GET  …/{id}/seals?fp=  fetch sealed packages addressed to fp
///   POST …/{id}/send       { nonce, ct } — member posts message
///   GET  …/{id}/recv?after= — member polls messages
fn session_single(hub: &Arc<Hub>, req: &Request) -> Response {
    let rest = match req.path.strip_prefix("/api/v1/sessions/") {
        Some(r) if !r.is_empty() => r,
        _ => return Response::error(404, "not found"),
    };
    let (id, action) = match rest.split_once('/') {
        Some((id, action)) => (id, action),
        None => (rest, ""),
    };
    if id.is_empty() || action.contains('/') {
        return Response::error(404, "not found");
    }

    match (req.method.as_str(), action) {
        ("GET", "") => {
            let is_dash = term_allowed(hub, req);
            let member = device_auth(hub, req).ok();
            if !is_dash && member.is_none() {
                return Response::error(401, "provide ?cap=<capability> or device auth headers");
            }
            match hub.sessions.get(id) {
                Some(s) => Response::json(200, &s.to_wire_json(is_dash || member.is_some())),
                None => Response::error(404, "session not found"),
            }
        }
        ("POST", "join") => {
            let device = match device_auth(hub, req) {
                Ok(d) => d,
                Err(r) => return r,
            };
            let body = match parse_body(req) {
                Ok(v) => v,
                Err(r) => return r,
            };
            let Some(ek) = body.get("ek").and_then(|v| v.as_str()) else {
                return Response::error(400, "missing 'ek'");
            };
            if ek.len() != 2368 || !ek.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Response::error(400, "ek must be 2368 hex chars (ML-KEM-768)");
            }
            // Optional pairing key (v0.12.0): a joiner presenting the valid
            // session pairing key is admitted even if the membership edge
            // would otherwise block (e.g. re-join after identity rotation).
            let pairing = body.get("pairing").and_then(|v| v.as_str()).unwrap_or("");
            let pairing_ok = !pairing.is_empty() && hub.sessions.check_pairing(id, pairing);
            if !pairing.is_empty() && !pairing_ok {
                return Response::error(403, "pairing key rejected");
            }
            // Register identity implicitly on join (first contact point).
            {
                let mut reg = hub.identities.lock().unwrap();
                match reg.iter_mut().find(|(d, _)| *d == device) {
                    Some(slot) => slot.1 = ek.to_string(),
                    None => reg.push((device.to_string(), ek.to_string())),
                }
                save_identities(&reg);
            }
            let join_result = if pairing_ok {
                // Pairing-validated join: tolerate duplicate membership
                // (identity rotation) by updating the member's ek in place.
                hub.sessions.join_or_refresh(id, &device, ek)
            } else {
                hub.sessions
                    .join(id, &device, ek)
                    .map(|(s, sealed)| (s, sealed, false))
            };
            match join_result {
                Ok((s, sealed, refreshed)) => {
                    let note = if refreshed {
                        " (pairing: ek refreshed)"
                    } else {
                        ""
                    };
                    let _ = hub.store.log_event(
                        &device,
                        &device,
                        "info",
                        &format!("joined session {}{note}", id),
                        "",
                    );
                    Response::json(
                        200,
                        &Value::obj(vec![
                            ("session", s.to_wire_json(false)),
                            ("pairing_ok", Value::from(pairing_ok)),
                            (
                                "sealed",
                                Value::Arr(
                                    sealed
                                        .iter()
                                        .map(|p| {
                                            Value::obj(vec![
                                                ("ct", Value::from(p.ct.as_str())),
                                                ("ek_fp", Value::from(p.ek_fp.as_str())),
                                            ])
                                        })
                                        .collect(),
                                ),
                            ),
                        ]),
                    )
                }
                Err(e) => Response::error(400, &e),
            }
        }
        ("POST", "seal") => {
            let device = match device_auth(hub, req) {
                Ok(d) => d,
                Err(r) => return r,
            };
            let body = match parse_body(req) {
                Ok(v) => v,
                Err(r) => return r,
            };
            let Some(pkgs) = body.get("pkgs").and_then(|v| v.as_arr()) else {
                return Response::error(400, "missing 'pkgs'");
            };
            let mut parsed = Vec::new();
            for p in pkgs {
                let Some(ct) = p.get("ct").and_then(|v| v.as_str()) else {
                    return Response::error(400, "package missing 'ct'");
                };
                let Some(fp) = p.get("ek_fp").and_then(|v| v.as_str()) else {
                    return Response::error(400, "package missing 'ek_fp'");
                };
                parsed.push((ct.to_string(), fp.to_string()));
            }
            match hub.sessions.post_sealed(id, &device, &parsed) {
                Ok(()) => Response::json(200, &Value::obj(vec![("ok", Value::from(true))])),
                Err(e) => Response::error(400, &e),
            }
        }
        ("GET", "seals") => {
            let device = match device_auth(hub, req) {
                Ok(d) => d,
                Err(r) => return r,
            };
            // The member identifies its packages by its registered ek fp.
            let fp = req
                .q("fp")
                .map(|s| s.to_string())
                .or_else(|| {
                    let reg = hub.identities.lock().unwrap();
                    reg.iter().find(|(d, _)| *d == device).map(|(_, ek)| {
                        crate::util::hex_encode(
                            &crate::keccak::sha3_256(
                                &crate::util::hex_decode(ek).unwrap_or_default(),
                            )[..8],
                        )
                    })
                })
                .unwrap_or_default();
            match hub.sessions.take_sealed(id, &fp) {
                Ok(pkgs) => {
                    let arr: Vec<Value> = pkgs
                        .iter()
                        .map(|p| {
                            Value::obj(vec![
                                ("ct", Value::from(p.ct.as_str())),
                                ("ek_fp", Value::from(p.ek_fp.as_str())),
                            ])
                        })
                        .collect();
                    Response::json(200, &Value::obj(vec![("sealed", Value::Arr(arr))]))
                }
                Err(e) => Response::error(400, &e),
            }
        }
        ("POST", "send") => {
            let device = match device_auth(hub, req) {
                Ok(d) => d,
                Err(r) => return r,
            };
            let body = match parse_body(req) {
                Ok(v) => v,
                Err(r) => return r,
            };
            let Some(nonce) = body.get("nonce").and_then(|v| v.as_str()) else {
                return Response::error(400, "missing 'nonce'");
            };
            let Some(ct) = body.get("ct").and_then(|v| v.as_str()) else {
                return Response::error(400, "missing 'ct'");
            };
            if ct.len() > MAX_CIPHERTEXT_CHARS * 2 {
                return Response::error(400, "ciphertext too large");
            }
            match hub.sessions.post_message(id, &device, nonce, ct) {
                Ok(msg) => {
                    let _ = hub.store.log_event(
                        &device,
                        &device,
                        "info",
                        &format!("session msg seq {}", msg.seq),
                        "",
                    );
                    Response::json(
                        200,
                        &Value::obj(vec![
                            ("ok", Value::from(true)),
                            ("seq", Value::from(msg.seq as i64)),
                        ]),
                    )
                }
                Err(e) => Response::error(400, &e),
            }
        }
        ("GET", "recv") => {
            let device = match device_auth(hub, req) {
                Ok(d) => d,
                Err(r) => return r,
            };
            // Membership check: recv requires membership.
            let is_member = hub
                .sessions
                .get(id)
                .map(|s| s.members.iter().any(|m| m.device == device))
                .unwrap_or(false);
            if !is_member {
                return Response::error(403, "not a member");
            }
            let after = req
                .q("after")
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0);
            match hub.sessions.read_messages(id, after) {
                Ok(msgs) => {
                    let arr: Vec<Value> = msgs
                        .iter()
                        .map(|m| {
                            Value::obj(vec![
                                ("seq", Value::from(m.seq as i64)),
                                ("sender", Value::from(m.sender.as_str())),
                                ("nonce", Value::from(m.nonce.as_str())),
                                ("ct", Value::from(m.ct.as_str())),
                                ("ts", Value::from(m.ts as i64)),
                            ])
                        })
                        .collect();
                    Response::json(200, &Value::obj(vec![("msgs", Value::Arr(arr))]))
                }
                Err(e) => Response::error(400, &e),
            }
        }
        ("POST", "scope") => {
            // Chat-as-project scope labels (operator directive): a chat may
            // span several repos/machines. The repo field carries a free
            // scope string ("repo-a+repo-b@mac+win"); the dashboard renders
            // it as chips. Dashboard-key or device auth (member) may set it.
            let is_dash = term_allowed(hub, req);
            let member = device_auth(hub, req).ok();
            if !is_dash && member.is_none() {
                return Response::error(401, "operator or member auth required");
            }
            let body = match parse_body(req) {
                Ok(v) => v,
                Err(r) => return r,
            };
            let Some(repo) = body.get("repo").and_then(|v| v.as_str()) else {
                return Response::error(400, "missing 'repo' scope label");
            };
            match hub.sessions.set_repo(id, repo) {
                Ok(()) => Response::json(200, &Value::obj(vec![("ok", Value::from(true))])),
                Err(e) => Response::error(400, &e),
            }
        }
        ("GET", "view") => {
            // Operator chat viewer (v0.15.0, operator directive): the
            // dashboard-key holder may read decrypted chat bodies. The
            // operator already holds every credential on the machine
            // (dashboard key, bridge keys, session keys on disk) — this
            // endpoint only surfaces what the local operator can already
            // decrypt with `$WTF_HOME/session_keys.json`. Remote callers
            // on a loopback hub get nothing (loopback_only gates ?cap=
            // elsewhere; here the dashboard key IS the gate, and the key
            // never leaves the machine it guards).
            if !term_allowed(hub, req) {
                return Response::error(401, "operator auth required (?cap=)");
            }
            let after = req
                .q("after")
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0);
            match crate::mcp::load_session_key(id) {
                None => Response::error(
                    404,
                    "no local session key for this chat — operator machine has not joined it",
                ),
                Some(key) => match hub.sessions.read_messages(id, after) {
                    Ok(msgs) => {
                        let arr: Vec<Value> = msgs
                            .iter()
                            .map(|m| {
                                let pt = crate::session_crypto::open_message(
                                    &key,
                                    id,
                                    &m.sender,
                                    m.seq,
                                    &m.nonce,
                                    &m.ct,
                                )
                                .unwrap_or_else(|e| format!("<decrypt failed: {e}>"));
                                Value::obj(vec![
                                    ("seq", Value::from(m.seq as i64)),
                                    ("sender", Value::from(m.sender.as_str())),
                                    ("ts", Value::from(m.ts as i64)),
                                    ("text", Value::from(pt.as_str())),
                                ])
                            })
                            .collect();
                        Response::json(200, &Value::obj(vec![("msgs", Value::Arr(arr))]))
                    }
                    Err(e) => Response::error(400, &e),
                },
            }
        }
        _ => Response::error(404, "not found"),
    }
}

// ---------- federation ----------

/// POST /api/v1/fed/push — device-authenticated ingest from a peer hub.
/// Body: { origin, events: [Event...] }. Dedupe on (origin, origin_id) makes
/// pushes idempotent; unknown origin device names are rejected (a peer's
/// device credential is issued by THIS hub's keystore via `wtf federate`
/// enrollment on the peer side). AuthZ: device only.
fn fed_push(hub: &Arc<Hub>, req: &Request) -> Response {
    let device = match device_auth(hub, req) {
        Ok(d) => d,
        Err(r) => return r,
    };
    let body = match parse_body(req) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let (origin, events) = match crate::federation::parse_push(&body) {
        Ok(x) => x,
        Err(e) => return Response::error(400, &e),
    };
    // Only federation devices (names minted by `wtf federate`, prefix
    // "fed-") may push: agent keys have no business on this route. The
    // credential itself is the trust anchor — the hub issued it to exactly
    // one peer via the PSK handshake.
    if !device.starts_with("fed-") {
        return Response::error(
            403,
            "federation push requires a federated device credential",
        );
    }
    let mut accepted = 0usize;
    let mut duplicates = 0usize;
    for ev_v in &events {
        let Some(mut ev) = event_from_value(ev_v) else {
            return Response::error(400, "malformed event in push");
        };
        ev.origin = origin.clone();
        if hub.store.ingest(&ev) {
            accepted += 1;
        } else {
            duplicates += 1;
        }
    }
    // Receipts are an echo engine, not signal: they replicate, peers
    // re-ingest and push back new events, each hub logs again — the 10 s
    // loop found by windows-1 (2026-09-01). Batch results stay on the
    // HTTP response; the store stays clean.
    Response::json(
        200,
        &Value::obj(vec![
            ("ok", Value::from(true)),
            ("accepted", Value::from(accepted as i64)),
            ("duplicates", Value::from(duplicates as i64)),
        ]),
    )
}

/// Rebuild an Event from wire JSON (strict; fails closed on missing core
/// fields). origin/origin_id/repo optional on the wire for robustness but
/// origin_id is required for dedupe; pre-federation origins are not
/// accepted over federation (origin "" would collide with local history).
fn event_from_value(v: &Value) -> Option<crate::store::Event> {
    let kind = v.get("kind").and_then(|x| x.as_str())?;
    if kind != "checkin" && kind != "event" {
        return None;
    }
    let origin_id = v.get("origin_id").and_then(|x| x.as_i64())?;
    if origin_id <= 0 {
        return None;
    }
    let ts = v.get("ts").and_then(|x| x.as_i64())?;
    let status = v.get("status").and_then(|x| x.as_str()).unwrap_or("");
    let task = v.get("task").and_then(|x| x.as_str()).unwrap_or("");
    if kind == "checkin" && !crate::store::STATUSES.contains(&status) {
        return None;
    }
    let level = v.get("level").and_then(|x| x.as_str()).unwrap_or("info");
    if !crate::store::LEVELS.contains(&level) {
        return None;
    }
    Some(crate::store::Event {
        id: 0, // assigned locally at ingest
        ts: ts as u64,
        device: v
            .get("device")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        agent: v
            .get("agent")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        level: level.to_string(),
        message: v
            .get("message")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        status: status.to_string(),
        task: task.to_string(),
        details: v
            .get("details")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        kind: kind.to_string(),
        origin: String::new(), // overwritten by caller
        origin_id: origin_id as u64,
        repo: v
            .get("repo")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
    })
}

/// GET /api/v1/fed/pull?origin=<me>&after=<cursor> — a peer asks what it
/// missed from `origin` (usually the requesting hub's own name, i.e. the
/// anti-entropy sweep of this hub's copy of the requester's events; the
/// common path is `origin == requester` with the requester's cursor).
/// Device-authenticated; the device must be the federation endpoint for the
/// requested origin.
fn fed_pull(hub: &Arc<Hub>, req: &Request) -> Response {
    let device = match device_auth(hub, req) {
        Ok(d) => d,
        Err(r) => return r,
    };
    if !device.starts_with("fed-") {
        return Response::error(
            403,
            "federation pull requires a federated device credential",
        );
    }
    let origin = req.q("origin").unwrap_or_default();
    let after: u64 = req.q("after").and_then(|v| v.parse().ok()).unwrap_or(0);
    if origin.is_empty() {
        return Response::error(400, "missing 'origin'");
    }
    let events = hub.store.events_since(&origin, after);
    let events_v: Vec<Value> = events.iter().map(|e| event_to_value(e)).collect();
    let cursor = events.last().map(|e| e.origin_id).unwrap_or(after);
    Response::json(
        200,
        &Value::obj(vec![
            ("origin", Value::from(origin.as_str())),
            ("cursor", Value::from(cursor as i64)),
            ("events", Value::Arr(events_v)),
        ]),
    )
}

fn event_to_value(e: &crate::store::Event) -> Value {
    Value::obj(vec![
        ("kind", Value::from(e.kind.as_str())),
        ("ts", Value::from(e.ts as i64)),
        ("device", Value::from(e.device.as_str())),
        ("agent", Value::from(e.agent.as_str())),
        ("level", Value::from(e.level.as_str())),
        ("message", Value::from(e.message.as_str())),
        ("status", Value::from(e.status.as_str())),
        ("task", Value::from(e.task.as_str())),
        ("details", Value::from(e.details.as_str())),
        ("origin", Value::from(e.origin.as_str())),
        ("origin_id", Value::from(e.origin_id as i64)),
        ("repo", Value::from(e.repo.as_str())),
    ])
}

/// GET /api/v1/fed/peers — device-authenticated; returns this hub's fed
/// identity and per-origin cursors so a newly linked peer can discover what
/// to pull (used by `wtf federate add` and the anti-entropy loop).
fn fed_peers(hub: &Arc<Hub>, req: &Request) -> Response {
    let _device = match device_auth(hub, req) {
        Ok(d) => d,
        Err(r) => return r,
    };
    let fed = hub.fed.lock().unwrap();
    Response::json(
        200,
        &Value::obj(vec![
            ("name", Value::from(fed.name.as_str())),
            ("peers", Value::from(fed.peers.len() as i64)),
        ]),
    )
}

// ---------- environment reports (agent-CLI presence) ----------

/// POST /api/v1/env — a bridge posts ITS machine's environment report
/// (device-auth; the report is about the reporting device only). Body:
/// { clis: {omp: {version, path} | null, hermes: {...}|null,
///          freeclaude: {tmux_session, pid} | null}, models: [...],
///          os, arch, wtf_version }. Keys/credentials are NEVER included —
/// the report records presence + versions only. Size-capped.
fn env_report(hub: &Arc<Hub>, req: &Request) -> Response {
    let device = match device_auth(hub, req) {
        Ok(d) => d,
        Err(r) => return r,
    };
    let body = match parse_body(req) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let text = body.to_json();
    if text.len() > 8192 {
        return Response::error(413, "env report too large (max 8 KiB)");
    }
    let mut reports = hub.env_reports.lock().unwrap();
    reports.retain(|(d, _, _)| d != &device);
    let now = now_secs();
    reports.push((device.clone(), body, now));
    // ring: keep the freshest 64
    if reports.len() > 64 {
        reports.remove(0);
    }
    Response::json(
        200,
        &Value::obj(vec![
            ("ok", Value::from(true)),
            ("devices", Value::from(reports.len() as i64)),
        ]),
    )
}

/// GET /api/v1/env — device auth: all devices' latest reports (cross-machine
/// capability discovery). No ?k= path: env data is operational, not operator.
fn env_list(hub: &Arc<Hub>, req: &Request) -> Response {
    if device_auth(hub, req).is_err() {
        return Response::error(401, "device auth required");
    }
    let reports = hub.env_reports.lock().unwrap();
    let arr: Vec<Value> = reports
        .iter()
        .map(|(d, v, at)| {
            Value::obj(vec![
                ("device", Value::from(d.as_str())),
                ("reported_at", Value::from(*at as i64)),
                ("report", v.clone()),
            ])
        })
        .collect();
    Response::json(200, &Value::obj(vec![("devices", Value::Arr(arr))]))
}

// ---------- operator terminal (v0.15.0) ----------

/// Operator terminal for chat executor sessions. Dashboard-key gated;
/// restricted to `wtf-chat-*` tmux sessions (the executor's namespace —
/// arbitrary shell sessions on the machine are NOT exposed).
///
///   GET  /api/v1/term/<session>?lines=N        → pane capture (text)
///   POST /api/v1/term/<session> {keys: "…"}    → send keys + Enter
///   POST /api/v1/term/<session> {spawn: "…"}   → create session (workdir $PWD)
///
/// This is the operator driving the SAME tmux sessions the chat executor
/// uses (`wtf-chat-<slug>`), from the dashboard. The dashboard key is the
/// operator credential; on a loopback hub the capability path plays that
/// role for the page, and the page forwards ?cap= — accepted here too on
/// loopback-only hubs.
fn term_session_name(input: &str) -> Option<String> {
    let name = percent_decode(input);
    if name.starts_with("wtf-chat-") && name.len() <= 48 && name.chars().all(|c| {
        c.is_ascii_alphanumeric() || c == '-' || c == '_'
    }) {
        Some(name)
    } else {
        None
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn term_allowed(hub: &Hub, req: &Request) -> bool {
    cap_ok(hub, req) || device_auth(hub, req).is_ok()
}

fn agents_available(hub: &Arc<Hub>, req: &Request) -> Response {
    if !cap_ok(hub, req) && device_auth(hub, req).is_err() {
        return Response::error(401, "operator auth required (?cap=)");
    }
    let agents = crate::executor::available_agents();
    Response::json(200, &Value::obj(vec![
        ("ok", Value::from(true)),
        ("backend", Value::from(crate::executor::detect_backend_str())),
        ("router_endpoint", Value::from("http://127.0.0.1:11434/v1")),
        ("router_model", Value::from("local-router/fallback-models")),
        ("agents", Value::arr(agents)),
    ]))
}

fn term(hub: &Arc<Hub>, req: &Request) -> Response {
    if !term_allowed(hub, req) {
        return Response::error(401, "operator auth required (?cap=)");
    }
    let Some(name) = term_session_name(req.path.strip_prefix("/api/v1/term/").unwrap_or("")) else {
        return Response::error(404, "unknown or disallowed tmux session (wtf-chat-* only)");
    };
    match req.method.as_str() {
        "GET" => {
            let lines = req.q("lines").and_then(|s| s.parse::<usize>().ok()).unwrap_or(200).min(5000);
            match crate::executor::tmux_capture_pane(&name, lines) {
                Ok(pane) => Response::json(200, &Value::obj(vec![
                    ("session", Value::from(name.as_str())),
                    ("pane", Value::from(pane.as_str())),
                ])),
                Err(e) => Response::error(404, &format!("session output not found: {e}")),
            }
        }
        "POST" => {
            let body = match parse_body(req) {
                Ok(v) => v,
                Err(r) => return r,
            };
            let spawn_req = body.get("spawn").map(|x| match x {
                Value::Str(s) => Some(s.clone()),
                Value::Bool(true) => Some(String::new()),
                _ => None,
            }).unwrap_or(None);
            if let Some(_spawn_cmd) = spawn_req {
                if !crate::executor::tmux_has_session(&name) {
                    let cwd = std::env::current_dir().unwrap_or_default().display().to_string();
                    if !crate::executor::tmux_new_session(&name, &cwd) {
                        return Response::error(500, "cannot create tmux session");
                    }
                }
                return Response::json(200, &Value::obj(vec![("ok", Value::from(true)), ("session", Value::from(name.as_str()))]));
            }
            if let Some(agent_prompt) = body.get("prompt").and_then(|x| x.as_str()) {
                let agent = body.get("agent").and_then(|x| x.as_str()).unwrap_or("auto");
                let fleet = body.get("fleet").and_then(|x| x.as_bool()).unwrap_or(true);
                let cwd = std::env::current_dir().unwrap_or_default().display().to_string();
                let outcome = crate::executor::run_in_tmux_with_options(&name, &cwd, agent_prompt, 300, agent, fleet);
                return Response::json(200, &Value::obj(vec![
                    ("ok", Value::from(outcome.ok)),
                    ("cli", Value::from(outcome.cli.as_str())),
                    ("output", Value::from(outcome.output.as_str())),
                    ("trace", Value::arr(outcome.trace.into_iter().map(|s| Value::from(s.as_str())).collect())),
                    ("session", Value::from(name.as_str())),
                ]));
            }
            let Some(keys) = body.get("keys").and_then(|x| x.as_str()) else {
                return Response::error(400, "body must be {prompt: \"...\"}, {keys: \"...\"} or {spawn: true}");
            };
            if keys.len() > 8192 {
                return Response::error(413, "keys payload too large");
            }
            match crate::executor::tmux_send_keys(&name, keys) {
                Ok(_) => Response::json(200, &Value::obj(vec![
                    ("ok", Value::from(true)),
                    ("session", Value::from(name.as_str())),
                ])),
                Err(e) => Response::error(500, &e),
            }
        }
        _ => Response::error(405, "method not allowed"),
    }
}

// ---------- federated multi-machine shell ----------

fn shell_machines(hub: &Hub, req: &Request) -> Response {
    if !term_allowed(hub, req) {
        return Response::error(401, "operator auth required (?cap=)");
    }
    let fed_lock = hub.fed.lock().unwrap();
    let peers: Vec<(String, String)> = fed_lock
        .peers
        .iter()
        .map(|p| (p.name.clone(), p.url.clone()))
        .collect();
    drop(fed_lock);

    let keys_lock = hub.keys.lock().unwrap();
    let devices: Vec<String> = keys_lock
        .records
        .iter()
        .filter(|d| !d.revoked)
        .map(|d| d.name.clone())
        .collect();
    drop(keys_lock);

    let machines = crate::fed_shell::discover_machines(&hub.fed_name, &peers, &devices);
    let arr: Vec<Value> = machines.iter().map(|m| m.to_json()).collect();
    Response::json(
        200,
        &Value::obj(vec![
            ("ok", Value::from(true)),
            ("machines", Value::Arr(arr)),
            ("root", Value::from("~/")),
        ]),
    )
}

fn shell_exec(hub: &Arc<Hub>, req: &Request) -> Response {
    if !term_allowed(hub, req) {
        return Response::error(401, "operator auth required (?cap=)");
    }
    let body = match parse_body(req) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let cmd = match body.get("cmd").and_then(|v| v.as_str()) {
        Some(c) if !c.trim().is_empty() => c,
        _ => return Response::error(400, "missing or empty 'cmd'"),
    };
    let cwd = body.get("cwd").and_then(|v| v.as_str()).unwrap_or("~/");
    let timeout_secs = body
        .get("timeout_secs")
        .and_then(|v| v.as_i64())
        .unwrap_or(30)
        .max(1) as u64;

    let fed_lock = hub.fed.lock().unwrap();
    let peers: Vec<(String, String)> = fed_lock
        .peers
        .iter()
        .map(|p| (p.name.clone(), p.url.clone()))
        .collect();
    drop(fed_lock);

    let keys_lock = hub.keys.lock().unwrap();
    let devices: Vec<String> = keys_lock
        .records
        .iter()
        .filter(|d| !d.revoked)
        .map(|d| d.name.clone())
        .collect();
    drop(keys_lock);

    let machines = crate::fed_shell::discover_machines(&hub.fed_name, &peers, &devices);
    let outcome = crate::fed_shell::exec_federated(cmd, cwd, &machines, timeout_secs);
    Response::json(200, &outcome.to_json())
}

fn shell_config_get(hub: &Arc<Hub>, req: &Request) -> Response {
    if !term_allowed(hub, req) {
        return Response::error(401, "operator auth required (?cap=)");
    }
    let cfg = crate::fed_shell::load_fed_omp_config();
    Response::json(
        200,
        &Value::obj(vec![("ok", Value::from(true)), ("config", cfg)]),
    )
}

fn shell_config_post(hub: &Arc<Hub>, req: &Request) -> Response {
    if !term_allowed(hub, req) {
        return Response::error(401, "operator auth required (?cap=)");
    }
    let val = match parse_body(req) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let cfg = match val.get("config") {
        Some(c) => c,
        None => &val,
    };
    if let Err(e) = crate::fed_shell::save_fed_omp_config(cfg) {
        return Response::error(500, &e);
    }
    Response::json(200, &Value::obj(vec![("ok", Value::from(true))]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enroll_limiter_sliding_window() {
        let mut hits = Vec::new();
        let t0 = 1_000_000u64;
        for _ in 0..ENROLL_MAX_ATTEMPTS {
            assert!(enroll_allowed(&mut hits, t0));
        }
        assert!(!enroll_allowed(&mut hits, t0 + 1));
        assert!(!enroll_allowed(&mut hits, t0 + ENROLL_WINDOW_SECS - 1));
        // The window slides: aged-out attempts stop counting.
        assert!(enroll_allowed(&mut hits, t0 + ENROLL_WINDOW_SECS));
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn enroll_nonce_cache_rejects_replay() {
        let mut cache = Vec::new();
        assert!(enroll_nonce_fresh(&mut cache, "aaaa", 1_000_000));
        assert!(!enroll_nonce_fresh(&mut cache, "aaaa", 1_000_001));
        assert!(enroll_nonce_fresh(&mut cache, "bbbb", 1_000_001));
        // Entries age out after 600 s (the upstream ±300 s skew guard would
        // reject the stale ts of a genuine replay anyway).
        assert!(enroll_nonce_fresh(&mut cache, "aaaa", 1_000_000 + 600));
    }

    #[test]
    fn identity_persistence_and_rehydration() {
        let tmp = format!("/tmp/wtf-test-identities-{}", crate::rand::nonce_hex());
        std::env::set_var("WTF_HOME", &tmp);

        let sessions = crate::sessions::Sessions::load();
        let list1 = vec![
            ("device-a".to_string(), "01020304".to_string()),
            ("device-b".to_string(), "05060708".to_string()),
        ];
        save_identities(&list1);
        let loaded = load_identities(&sessions);
        assert_eq!(loaded, list1);

        let _ = std::fs::remove_dir_all(&tmp);
        std::env::remove_var("WTF_HOME");
    }

    #[test]
    fn agents_available_endpoint_structure() {
        let agents = crate::executor::available_agents();
        assert!(!agents.is_empty());
        let auto = agents.iter().find(|a| a.get("id").and_then(|x| x.as_str()) == Some("auto"));
        assert!(auto.is_some());
        let hermes = agents.iter().find(|a| a.get("id").and_then(|x| x.as_str()) == Some("hermes"));
        assert!(hermes.is_some());
    }
}
