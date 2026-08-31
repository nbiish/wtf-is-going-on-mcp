//! Hub HTTP API: route dispatch, auth glue (device HMAC or dashboard key),
//! and the SSE state fan-out loop.
//!
//! AuthZ model:
//! - `/healthz` — open (connectivity probe; leaks nothing but version/uptime)
//! - `/` and `/stream` — dashboard key via `?k=` (device auth also accepted on /stream)
//! - `/api/v1/state` — dashboard key OR device auth
//! - `/api/v1/bins`, `/api/v1/bins/{1,2,3}` — GET/PUT: dashboard key OR device auth
//! - `/api/v1/{checkin,event,heartbeat}` — device auth only
//! - `/api/v1/identity` — device auth only (register own ML-KEM-768 ek)
//! - `/api/v1/devices` — dashboard key OR device auth (identity registry read)
//! - `/api/v1/sessions` — GET list: dashboard key OR device; POST create: device only
//! - `/api/v1/sessions/{id}` — GET: dashboard key OR device
//! - `/api/v1/sessions/{id}/{join,seal,seals,send,recv}` — device auth only;
//!   seal/seals/send/recv additionally require session membership.
//! - `/api/v1/enroll` — NO HMAC: the credential is a one-time enrollment
//!   token minted hub-side by `wtf enroll-token` (SHA-256-hashed at rest,
//!   single-use, short TTL). Global rate limiter blunts online guessing.
//! All failures are 401 with a generic message; never leak which factor failed.

use crate::auth::{self, NonceCache};
use crate::bins::Bins;
use crate::config::KeyStore;
use crate::http::{HandlerResult, Request, Response, SseSession};
use crate::json::{self, Value};
use crate::sessions::{Sessions, MAX_CIPHERTEXT_CHARS};
use crate::store::{LEVELS, STATUSES, Store};
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
    ) || bin_id_of(&req.path).is_some()
        || req.path.starts_with("/api/v1/sessions/");
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/healthz") => HandlerResult::Respond(healthz(hub)),
        ("GET", "/") => HandlerResult::Respond(dashboard(hub, req)),
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
    let headers =
        auth::extract(&req.headers).map_err(|e| Response::error(401, &e.to_string()))?;
    // keys.json may have changed on disk since the last request (issue,
    // revoke, rotate via `wtf key`). Reload unconditionally: the file is
    // tiny, and a stale in-memory record must never authenticate a revoked
    // device. Fail closed if the keystore cannot be read.
    let fresh = KeyStore::load().map_err(|_| Response::error(401, "keystore unavailable"))?;
    *hub.keys.lock().unwrap() = fresh;
    let verify = |keys: &KeyStore, nonces: &mut NonceCache| {
        auth::verify(headers.clone(), keys, nonces, &req.method, &req.target, &req.body)
    };
    let mut nonces = hub.nonces.lock().unwrap();
    let keys = hub.keys.lock().unwrap();
    verify(&keys, &mut nonces).map_err(|e| Response::error(401, &e.to_string()))
}

fn dash_ok(hub: &Hub, req: &Request) -> bool {
    match req.q("k") {
        Some(k) => ct_eq_str(&hub.dashboard_key, &k),
        None => false,
    }
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
    if !dash_ok(hub, req) && device_auth(hub, req).is_err() {
        return Response::error(401, "provide ?k=<dashboard key> or device auth headers");
    }
    Response::json(200, &hub.store.to_state_json(hub.started_at, &hub.bins))
}

fn bins_list(hub: &Hub, req: &Request) -> Response {
    if !dash_ok(hub, req) && device_auth(hub, req).is_err() {
        return Response::error(401, "provide ?k=<dashboard key> or device auth headers");
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
    let actor = if dash_ok(hub, req) {
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

fn dashboard(hub: &Hub, req: &Request) -> Response {
    if !dash_ok(hub, req) {
        return Response::html(
            401,
            r#"<!doctype html><html><head><meta charset="utf-8"><title>401</title></head>
<body style="background:#0b0e14;color:#d7dde8;font-family:monospace;padding:40px">
<h1>401 — dashboard key required</h1>
<p>append <code>?k=&lt;dashboard_key&gt;</code> (printed by <code>wtf serve</code>).</p>
</body></html>"#,
        );
    }
    Response::html(200, crate::dashboard::PAGE)
}

fn stream(hub: &Arc<Hub>, req: &Request) -> HandlerResult {
    if !dash_ok(hub, req) && device_auth(hub, req).is_err() {
        return HandlerResult::Respond(Response::error(401, "missing or bad ?k= key"));
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
    let agent = body.get("agent").and_then(|v| v.as_str()).unwrap_or(device.as_str());
    let ev = hub.store.check_in(&device, agent, status, task, details);
    Response::json(
        200,
        &Value::obj(vec![("ok", Value::from(true)), ("id", Value::from(ev.id as i64))]),
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
    let agent = body.get("agent").and_then(|v| v.as_str()).unwrap_or(device.as_str());
    let ev = hub.store.log_event(&device, agent, level, message);
    Response::json(
        200,
        &Value::obj(vec![("ok", Value::from(true)), ("id", Value::from(ev.id as i64))]),
    )
}

fn heartbeat(hub: &Arc<Hub>, req: &Request) -> Response {
    let device = match device_auth(hub, req) {
        Ok(d) => d,
        Err(r) => return r,
    };
    let agent = match parse_body(req) {
        Ok(body) => body.get("agent").and_then(|v| v.as_str()).unwrap_or(device.as_str()).to_string(),
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

/// POST /api/v1/enroll { name, token } — redeem a one-time enrollment token
/// for this device's key. Response shape matches `wtf key issue --json` so
/// the device-side `wtf enroll` can share `wtf join`'s parser. Failures are
/// uniform; the token burns only on success (a typo must not brick it).
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
    let Some(token) = body.get("token").and_then(|v| v.as_str()) else {
        return Response::error(400, "missing 'token'");
    };
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
    // Token redeemed. Mint the device key exactly like `wtf key issue` does;
    // the hub's per-request keystore reload picks it up immediately.
    let mut ks = match KeyStore::load() {
        Ok(k) => k,
        Err(_) => return Response::error(500, "keystore unavailable"),
    };
    let secret = match ks.issue(name) {
        Ok(s) => s,
        Err(e) => return Response::error(400, &e),
    };
    let hub_url = crate::config::HubConfig::load_or_create()
        .map(|c| c.lan_url())
        .unwrap_or_default();
    let _ = hub
        .store
        .log_event("enroll", name, "info", &format!("device '{name}' enrolled via enrollment token"));
    Response::json(
        200,
        &Value::obj(vec![
            ("hub_url", Value::from(hub_url.as_str())),
            ("device", Value::from(name)),
            ("key", Value::from(secret.as_str())),
        ]),
    )
}

// ---------- identity registry ----------

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
    }
    let _ = hub.store.log_event(&device, &device, "info", "identity registered");
    Response::json(
        200,
        &Value::obj(vec![("ok", Value::from(true))]),
    )
}

/// GET /api/v1/devices — the identity registry (dashboard key or device).
fn devices_list(hub: &Arc<Hub>, req: &Request) -> Response {
    if !dash_ok(hub, req) && device_auth(hub, req).is_err() {
        return Response::error(401, "provide ?k=<dashboard key> or device auth headers");
    }
    let reg = hub.identities.lock().unwrap();
    let devices: Vec<Value> = reg
        .iter()
        .map(|(d, ek)| {
            Value::obj(vec![
                ("device", Value::from(d.as_str())),
                ("ek_fp", Value::from(crate::util::hex_encode(
                    &crate::keccak::sha3_256(&crate::util::hex_decode(ek).unwrap_or_default())[..8],
                ))),
            ])
        })
        .collect();
    Response::json(200, &Value::obj(vec![("devices", Value::Arr(devices))]))
}

// ---------- session channels ----------

/// GET /api/v1/sessions — list session metadata (no messages).
fn sessions_list(hub: &Arc<Hub>, req: &Request) -> Response {
    if !dash_ok(hub, req) && device_auth(hub, req).is_err() {
        return Response::error(401, "provide ?k=<dashboard key> or device auth headers");
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
        Err(r) => return r,
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
            None => {
                return Response::error(400, "register identity first (POST /api/v1/identity)")
            }
        }
    };
    match hub.sessions.create(name, &device, &ek) {
        Ok(s) => {
            let _ = hub.store.log_event(&device, &device, "info", &format!("session '{}' created", s.id));
            Response::json(200, &s.to_wire_json(false))
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
            let is_dash = dash_ok(hub, req);
            let member = device_auth(hub, req).ok();
            if !is_dash && member.is_none() {
                return Response::error(401, "provide ?k=<dashboard key> or device auth headers");
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
            // Register identity implicitly on join (first contact point).
            {
                let mut reg = hub.identities.lock().unwrap();
                match reg.iter_mut().find(|(d, _)| *d == device) {
                    Some(slot) => slot.1 = ek.to_string(),
                    None => reg.push((device.to_string(), ek.to_string())),
                }
            }
            match hub.sessions.join(id, &device, ek) {
                Ok((s, sealed)) => {
                    let _ = hub.store.log_event(&device, &device, "info", &format!("joined session {}", id));
                    Response::json(
                        200,
                        &Value::obj(vec![
                            ("session", s.to_wire_json(false)),
                            (
                                "sealed",
                                Value::Arr(
                                    sealed
                                        .iter()
                                        .map(|p| Value::obj(vec![("ct", Value::from(p.ct.as_str())), ("ek_fp", Value::from(p.ek_fp.as_str()))]))
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
                            &crate::keccak::sha3_256(&crate::util::hex_decode(ek).unwrap_or_default())[..8],
                        )
                    })
                })
                .unwrap_or_default();
            match hub.sessions.take_sealed(id, &fp) {
                Ok(pkgs) => {
                    let arr: Vec<Value> = pkgs
                        .iter()
                        .map(|p| Value::obj(vec![("ct", Value::from(p.ct.as_str())), ("ek_fp", Value::from(p.ek_fp.as_str()))]))
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
                    let _ = hub.store.log_event(&device, &device, "info", &format!("session msg seq {}", msg.seq));
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
            let after = req.q("after").and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
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
        _ => Response::error(404, "not found"),
    }
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
}
