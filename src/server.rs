//! HTTP surface: one page, one snapshot endpoint, a small pastebin API.

use crate::bins::Store;
use crate::http::{self, Handler, HandlerResult, Request, Response};
use crate::json::{self, Value};
use crate::model::Snapshot;
use crate::sources::Sources;
use crate::util::{now_millis, parse_query};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

const INDEX_HTML: &str = include_str!("../assets/index.html");
const CACHE_TTL_MS: u64 = 2_500;

pub struct State {
    pub sources: Sources,
    pub bins: Store,
    cache: Mutex<Option<(u64, Arc<Snapshot>)>>,
}

impl State {
    pub fn new(sources: Sources, bins: Store) -> Arc<State> {
        Arc::new(State {
            sources,
            bins,
            cache: Mutex::new(None),
        })
    }

    /// Cached snapshot; `fresh` bypasses the TTL.
    pub fn snapshot(&self, fresh: bool) -> Arc<Snapshot> {
        let now = now_millis();
        if !fresh {
            if let Ok(guard) = self.cache.lock() {
                if let Some((at, snap)) = guard.as_ref() {
                    if now.saturating_sub(*at) < CACHE_TTL_MS {
                        return Arc::clone(snap);
                    }
                }
            }
        }
        let (machines, sessions, sources) = self.sources.collect();
        let snap = Arc::new(Snapshot {
            generated_at: now_millis(),
            sessions,
            machines,
            sources,
            bin_count: self.bins.list().len(),
        });
        if let Ok(mut guard) = self.cache.lock() {
            *guard = Some((now_millis(), Arc::clone(&snap)));
        }
        snap
    }
}

pub fn start(bind: &str, port: u16, state: Arc<State>) -> std::io::Result<TcpListener> {
    let listener = TcpListener::bind((bind, port))?;
    let handler: Handler = {
        let state = Arc::clone(&state);
        Arc::new(move |req: &Request| HandlerResult::Respond(route(&state, req)))
    };
    let listener_clone = listener.try_clone()?;
    std::thread::spawn(move || http::serve(listener_clone, handler));
    Ok(listener)
}

fn route(state: &State, req: &Request) -> Response {
    let method = req.method.as_str();
    let path = req.path.as_str();
    match (method, path) {
        ("GET", "/") | ("GET", "/index.html") => Response::html(200, INDEX_HTML),
        ("GET", "/healthz") => json_response(200, &obj(vec![("ok", Value::Bool(true))])),
        ("GET", "/api/snapshot") => {
            let fresh = req
                .q("fresh")
                .map(|v| v == "1" || v == "true")
                .unwrap_or(false);
            let snap = state.snapshot(fresh);
            let mut body = snap.to_json();
            if let Value::Obj(pairs) = &mut body {
                pairs.push(("ok".into(), Value::Bool(true)));
            }
            json_response(200, &body)
        }
        ("GET", "/api/bins") => {
            let bins: Vec<Value> = state.bins.list().iter().map(|b| b.summary_json()).collect();
            json_response(
                200,
                &obj(vec![("ok", Value::Bool(true)), ("bins", Value::arr(bins))]),
            )
        }
        ("POST", "/api/bins") => create_bin(state, req),
        ("GET", path) if path.starts_with("/api/bins/") => {
            let id = &path["/api/bins/".len()..];
            match state.bins.get(id) {
                Some(bin) => json_response(200, &bin.to_json()),
                None => Response::error(404, "no such bin"),
            }
        }
        ("DELETE", path) if path.starts_with("/api/bins/") => {
            let id = &path["/api/bins/".len()..];
            if state.bins.delete(id) {
                json_response(200, &obj(vec![("ok", Value::Bool(true))]))
            } else {
                Response::error(404, "no such bin")
            }
        }
        ("DELETE", "/api/bins") => Response::error(405, "use DELETE /api/bins/<id>"),
        _ => Response::error(404, "not found"),
    }
}

fn create_bin(state: &State, req: &Request) -> Response {
    let Ok(text) = std::str::from_utf8(&req.body) else {
        return Response::error(400, "body must be UTF-8");
    };
    if text.len() > crate::bins::MAX_BODY + 4096 {
        return Response::error(413, "paste too large");
    }
    let Ok(value) = json::parse(text) else {
        return Response::error(400, "body must be JSON");
    };
    let title = value.get("title").and_then(|v| v.as_str()).unwrap_or("");
    let body = value.get("body").and_then(|v| v.as_str()).unwrap_or("");
    let machine = value
        .get("machine")
        .and_then(|v| v.as_str())
        .unwrap_or(&state.sources.machine);
    match state.bins.put(title, body, machine) {
        Ok(bin) => json_response(201, &bin.to_json()),
        Err(e) => Response::error(400, &e),
    }
}

fn obj(pairs: Vec<(&str, Value)>) -> Value {
    Value::obj(pairs)
}

fn json_response(status: u16, body: &Value) -> Response {
    let mut resp = Response::json(status, body);
    resp.extra_headers
        .push(("Cache-Control".into(), "no-store, max-age=0".into()));
    resp
}

#[allow(dead_code)]
fn query_flag(req: &Request, key: &str) -> bool {
    parse_query(&req.query)
        .into_iter()
        .any(|(k, v)| k == key && (v == "1" || v == "true"))
}
