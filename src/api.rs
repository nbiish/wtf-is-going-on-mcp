//! Hub HTTP API: route dispatch, auth glue (device HMAC or dashboard key),
//! and the SSE state fan-out loop.
//!
//! AuthZ model:
//! - `/healthz` — open (connectivity probe; leaks nothing but version/uptime)
//! - `/` and `/stream` — dashboard key via `?k=` (device auth also accepted on /stream)
//! - `/api/v1/state` — dashboard key OR device auth
//! - `/api/v1/bins`, `/api/v1/bins/{1,2,3}` — GET/PUT: dashboard key OR device auth
//! - `/api/v1/{checkin,event,heartbeat}` — device auth only
//! All failures are 401 with a generic message; never leak which factor failed.

use crate::auth::{self, NonceCache};
use crate::bins::Bins;
use crate::config::KeyStore;
use crate::http::{HandlerResult, Request, Response, SseSession};
use crate::json::{self, Value};
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
    ) || bin_id_of(&req.path).is_some();
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
