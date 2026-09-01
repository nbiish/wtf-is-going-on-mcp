//! Federation replication: outbound push + anti-entropy pull.
//!
//! One thread per peer (spawned at hub start for every configured peer):
//! - Watches the store generation; on change, pushes locally-originated
//!   events the peer has not acknowledged (cursor-tracked per peer, persisted
//!   in memory — a restart re-syncs via pull, so cursor loss is harmless).
//! - On a 30 s cadence (and immediately after startup), sweeps the peer:
//!   asks for `origin == THIS hub's name` events the PEER has from us
//!   (catch-up if our push failed while the peer was down), then pulls the
//!   peer's own origin events into our store (ingest dedupes).
//!
//! All requests ride the existing HMAC-SHA256 device lane using the device
//! credential the peer issued to this hub (`federation.json`, 0600).
//! Failures log a warn event at most every 5 minutes and never crash the
//! hub — the mesh heals on the next cadence.

use crate::client;
use crate::config::KeyStore;
use crate::federation::{FedConfig, Peer};
use crate::json::Value;
use crate::rand;
use crate::store::{Event, Store};
use crate::util::now_secs;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const CADENCE_SECS: u64 = 10;
const MAX_PUSH_BATCH: usize = 200;
const WARN_THROTTLE_SECS: u64 = 300;

pub struct Replicator {
    pub store: Arc<Store>,
    pub hub_url: String,
    pub hub_name: String,
    pub fed: Arc<Mutex<FedConfig>>,
    pub nonces: Mutex<HashMap<String, Vec<String>>>, // per-peer replay caches
    pub last_warn: Mutex<HashMap<String, u64>>,
    pub push_gen: Arc<AtomicU64>,
    pub wake: Arc<AtomicBool>,
}

/// Sign + send a request to a peer as this hub's federated device.
fn fed_request(
    rep: &Replicator,
    peer: &Peer,
    method: &str,
    path_and_query: &str,
    body: &[u8],
) -> Result<client::ClientResponse, String> {
    let ts = now_secs();
    let nonce = rand::hex(16);
    let sig = crate::auth::sign(&peer.device_key, method, path_and_query, ts, &nonce, body)
        .ok_or_else(|| "peer device key is not valid hex".to_string())?;
    let headers = vec![
        ("X-Wtf-Device".to_string(), peer.device.clone()),
        ("X-Wtf-Timestamp".to_string(), ts.to_string()),
        ("X-Wtf-Nonce".to_string(), nonce),
        ("X-Wtf-Signature".to_string(), sig),
    ];
    client::request(
        &format!("{}{}", peer.url, path_and_query),
        method,
        &headers,
        body,
    )
}

fn event_to_value(e: &Event) -> Value {
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

fn event_from_value(v: &Value) -> Option<Event> {
    let kind = v.get("kind").and_then(|x| x.as_str())?;
    if kind != "checkin" && kind != "event" {
        return None;
    }
    let origin_id = v.get("origin_id").and_then(|x| x.as_i64())?;
    if origin_id <= 0 {
        return None;
    }
    Some(Event {
        id: 0,
        ts: v.get("ts").and_then(|x| x.as_i64())? as u64,
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
        level: v
            .get("level")
            .and_then(|x| x.as_str())
            .unwrap_or("info")
            .to_string(),
        message: v
            .get("message")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        status: v
            .get("status")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        task: v
            .get("task")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        details: v
            .get("details")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        kind: kind.to_string(),
        origin: v
            .get("origin")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        origin_id: origin_id as u64,
        repo: v
            .get("repo")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
    })
}

/// Push events from `origin` with origin_id > after to `peer`. Returns the
/// peer's acked cursor (== last pushed origin_id) or an error.
pub fn push_since(rep: &Replicator, peer: &Peer, after: &AtomicU64) -> Result<u64, String> {
    let from = after.load(Ordering::SeqCst);
    let events = rep.store.events_since(&rep.hub_name, from);
    if events.is_empty() {
        return Ok(from);
    }
    for chunk in events.chunks(MAX_PUSH_BATCH) {
        let body = Value::obj(vec![
            ("origin", Value::from(rep.hub_name.as_str())),
            (
                "events",
                Value::Arr(chunk.iter().map(event_to_value).collect()),
            ),
        ])
        .to_json();
        let resp = fed_request(rep, peer, "POST", "/api/v1/fed/push", body.as_bytes())?;
        if resp.status != 200 {
            return Err(format!("push refused (HTTP {})", resp.status));
        }
    }
    let last = events.last().map(|e| e.origin_id).unwrap_or(from);
    after.store(last, Ordering::SeqCst);
    Ok(last)
}

/// One anti-entropy sweep against `peer`:
/// 1. pull the peer's own events we may have missed (origin == peer.name),
/// 2. pull our own events the peer may hold beyond our acked cursor (we
///    re-ingest them locally; dedupe makes this a no-op, but it repairs the
///    peer's missing ranges on ITS next pull from us).
/// Returns Ok(()) or an error string (throttled to a warn event by caller).
pub fn anti_entropy(
    rep: &Replicator,
    peer: &Peer,
    pushed_cursor: &AtomicU64,
) -> Result<(), String> {
    // 1. peer's own origin → our store
    let mut after = 0u64;
    loop {
        let resp = fed_request(
            rep,
            peer,
            "GET",
            &format!(
                "/api/v1/fed/pull?origin={}&after={after}",
                urlencode(&peer.name)
            ),
            b"",
        )?;
        if resp.status != 200 {
            return Err(format!("pull refused (HTTP {})", resp.status));
        }
        let v = resp.json().ok_or("pull returned non-JSON")?;
        let events = v
            .get("events")
            .and_then(|x| x.as_arr())
            .ok_or("pull response missing events")?;
        let mut ingested = 0usize;
        for ev_v in events {
            if let Some(mut ev) = event_from_value(ev_v) {
                ev.origin = peer.name.clone();
                if rep.store.ingest(&ev) {
                    ingested += 1;
                }
            }
        }
        let cursor = v
            .get("cursor")
            .and_then(|x| x.as_i64())
            .unwrap_or(after as i64) as u64;
        if ingested > 0 {
            // Flood guard (defect found by windows-1, 2026-09-01): these
            // ingest logs themselves replicate, so peers re-ingest + re-log
            // them — a feedback loop that drowned the event ring (~20
            // events/min). Never log ingests that are themselves
            // federation-internal: receipts from fed_push carry device
            // "fed-hub-<peer>", so any device named "federation" or with a
            // "fed-" prefix is hub machinery, not agent signal.
            let only_fed_internal = events.iter().all(|e| {
                e.get("device")
                    .and_then(|x| x.as_str())
                    .map(|d| d == "federation" || d.starts_with("fed-"))
                    .unwrap_or(false)
            });
            if !only_fed_internal {
                let mut lw = rep.last_warn.lock().unwrap();
                let last = *lw.get(&format!("pull-{}", peer.name)).unwrap_or(&0);
                let now2 = now_secs();
                if now2.saturating_sub(last) > WARN_THROTTLE_SECS {
                    lw.insert(format!("pull-{}", peer.name), now2);
                    drop(lw);
                    let _ = rep.store.log_event(
                        "federation",
                        &format!("fed-{}", peer.name),
                        "info",
                        &format!("federation: +{ingested} event(s) from {}", peer.name),
                        "",
                    );
                }
            }
        }
        if events.is_empty() || cursor == after {
            break;
        }
        after = cursor;
        if after > 10_000_000 {
            break; // paranoia bound; real cursors are small
        }
    }
    // 2. ask the peer for OUR events beyond what it acked — proves the link
    //    both ways; anything we get back is already in our store (dedupe).
    let ours = fed_request(
        rep,
        peer,
        "GET",
        &format!(
            "/api/v1/fed/pull?origin={}&after={}",
            urlencode(&rep.hub_name),
            pushed_cursor.load(Ordering::SeqCst)
        ),
        b"",
    )?;
    if ours.status != 200 {
        return Err(format!("self-pull refused (HTTP {})", ours.status));
    }
    Ok(())
}

fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Warn (throttled) into the store when a peer misbehaves.
fn warn_throttled(rep: &Replicator, peer_name: &str, msg: &str) {
    let now = now_secs();
    let mut lw = rep.last_warn.lock().unwrap();
    let last = *lw.get(peer_name).unwrap_or(&0);
    if now.saturating_sub(last) > WARN_THROTTLE_SECS {
        lw.insert(peer_name.to_string(), now);
        drop(lw);
        let _ = rep
            .store
            .log_event("federation", &format!("fed-{peer_name}"), "warn", msg, "");
    }
}

/// Spawn the per-peer replication loop. Never returns.
pub fn spawn(store: Arc<Store>, hub_name: String, fed: Arc<Mutex<FedConfig>>) {
    let gen = Arc::new(AtomicU64::new(0));
    let wake = Arc::new(AtomicBool::new(false));
    let rep = Arc::new(Replicator {
        store,
        hub_url: String::new(), // unused outbound; kept for symmetry
        hub_name,
        fed,
        nonces: Mutex::new(HashMap::new()),
        last_warn: Mutex::new(HashMap::new()),
        push_gen: gen.clone(),
        wake: wake.clone(),
    });
    std::thread::Builder::new()
        .name("wtf-replicator".into())
        .spawn(move || loop {
            let peers: Vec<Peer> = rep.fed.lock().unwrap().peers.clone();
            for peer in &peers {
                // push local events; then anti-entropy sweep
                // (cursors live per-peer for the process lifetime; restarts
                // re-sync through pull + dedupe, so loss is benign)
                match push_since(&rep, peer, &rep.push_gen) {
                    Ok(_) => {}
                    Err(e) => warn_throttled(
                        &rep,
                        &peer.name,
                        &format!("push to {} failed: {e}", peer.name),
                    ),
                }
                match anti_entropy(&rep, peer, &rep.push_gen) {
                    Ok(_) => {}
                    Err(e) => warn_throttled(
                        &rep,
                        &peer.name,
                        &format!("sync with {} failed: {e}", peer.name),
                    ),
                }
            }
            // sleep until: generation bump (new local events), explicit wake,
            // or the cadence elapses — poll every 2 s, cheap.
            let deadline = now_secs() + CADENCE_SECS;
            let start_gen = rep.store.generation();
            loop {
                std::thread::sleep(Duration::from_secs(2));
                let now = now_secs();
                if rep.wake.swap(false, Ordering::SeqCst) {
                    break;
                }
                if rep.store.generation() != start_gen {
                    break;
                }
                if now >= deadline {
                    break;
                }
            }
        })
        .expect("fatal: cannot spawn replicator thread");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_value_roundtrip() {
        let ev = Event {
            id: 7,
            ts: 12345,
            device: "d".into(),
            agent: "a".into(),
            level: "info".into(),
            message: "m".into(),
            status: "working".into(),
            task: "t".into(),
            details: "x".into(),
            kind: "checkin".into(),
            origin: "hub-a".into(),
            origin_id: 9,
            repo: "r".into(),
        };
        let v = event_to_value(&ev);
        let back = event_from_value(&v).unwrap();
        assert_eq!(back.origin_id, 9);
        assert_eq!(back.ts, 12345);
        assert_eq!(back.repo, "r");
        assert_eq!(back.kind, "checkin");
        // origin is overwritten by the ingest path; wire value ignored
        assert_eq!(back.origin, "hub-a");
    }

    #[test]
    fn event_value_rejects_garbage() {
        let v = crate::json::parse(r#"{"kind":"checkin","origin_id":0,"ts":1}"#).unwrap();
        assert!(event_from_value(&v).is_none()); // origin_id must be > 0
        let v2 = crate::json::parse(r#"{"kind":"bogus","origin_id":1,"ts":1}"#).unwrap();
        assert!(event_from_value(&v2).is_none());
        let v3 = crate::json::parse(r#"{"kind":"event"}"#).unwrap();
        assert!(event_from_value(&v3).is_none());
    }

    #[test]
    fn urlencode_shapes() {
        assert_eq!(urlencode("hub-a"), "hub-a");
        assert_eq!(urlencode("hub a"), "hub%20a");
    }
}
