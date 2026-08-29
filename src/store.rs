//! Hub state: live agent statuses and the event log.
//!
//! Events are the source of truth: the append-only JSONL log is replayed on
//! startup to rebuild agent state, so a hub restart loses nothing. The log
//! rotates at 10 MB (renamed to events.jsonl.old) so it cannot grow forever.

use crate::json::Value;
use crate::util::{clamp, now_secs};
use std::collections::{HashMap, VecDeque};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

pub const MAX_EVENTS_IN_MEMORY: usize = 1000;
pub const MAX_LOG_BYTES: u64 = 10 * 1024 * 1024;
pub const STALE_SECS: u64 = 600;

pub const STATUSES: [&str; 4] = ["working", "blocked", "done", "idle"];
pub const LEVELS: [&str; 3] = ["info", "warn", "error"];

#[derive(Clone, Debug)]
pub struct AgentEntry {
    pub device: String,
    pub agent: String,
    pub status: String,
    pub task: String,
    pub details: String,
    pub first_seen: u64,
    pub last_seen: u64,
}

#[derive(Clone, Debug)]
pub struct Event {
    pub id: u64,
    pub ts: u64,
    pub device: String,
    pub agent: String,
    pub level: String,
    pub message: String,
    pub status: String,
    pub task: String,
    pub details: String,
    pub kind: String, // "checkin" | "event"
}

impl Event {
    fn to_line(&self) -> String {
        Value::obj(vec![
            ("kind", Value::from(self.kind.as_str())),
            ("id", Value::from(self.id as i64)),
            ("ts", Value::from(self.ts as i64)),
            ("device", Value::from(self.device.as_str())),
            ("agent", Value::from(self.agent.as_str())),
            ("level", Value::from(self.level.as_str())),
            ("message", Value::from(self.message.as_str())),
            ("status", Value::from(self.status.as_str())),
            ("task", Value::from(self.task.as_str())),
            ("details", Value::from(self.details.as_str())),
        ])
        .to_json()
    }

    fn from_line(v: &Value) -> Option<Event> {
        Some(Event {
            kind: v.get("kind")?.as_str()?.to_string(),
            id: v.get("id")?.as_i64()? as u64,
            ts: v.get("ts")?.as_i64()? as u64,
            device: v.get("device")?.as_str()?.to_string(),
            agent: v.get("agent")?.as_str()?.to_string(),
            level: v.get("level")?.as_str()?.to_string(),
            message: v.get("message")?.as_str()?.to_string(),
            status: v.get("status").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            task: v.get("task").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            details: v.get("details").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        })
    }
}

struct Inner {
    agents: HashMap<(String, String), AgentEntry>,
    events: VecDeque<Event>,
    next_id: u64,
    file: Option<std::fs::File>,
}

pub struct Store {
    inner: Mutex<Inner>,
    log_path: PathBuf,
    generation: AtomicU64,
}

/// "3m ago" style relative age.
pub fn rel_age(now: u64, ts: u64) -> String {
    let d = now.saturating_sub(ts);
    if d < 60 {
        format!("{d}s")
    } else if d < 3600 {
        format!("{}m", d / 60)
    } else if d < 86400 {
        format!("{}h", d / 3600)
    } else {
        format!("{}d", d / 86400)
    }
}

fn is_stale(now: u64, last_seen: u64) -> bool {
    now.saturating_sub(last_seen) > STALE_SECS
}

impl Store {
    pub fn new(data_file: &Path) -> std::io::Result<Store> {
        if let Some(dir) = data_file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut agents = HashMap::new();
        let mut events = VecDeque::new();
        let mut next_id = 1u64;
        match std::fs::read_to_string(data_file) {
            Ok(text) => {
                for line in text.lines() {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    let Ok(v) = crate::json::parse(line) else { continue };
                    let Some(ev) = Event::from_line(&v) else { continue };
                    next_id = next_id.max(ev.id + 1);
                    if ev.kind == "checkin" {
                        let key = (ev.device.clone(), ev.agent.clone());
                        agents.insert(
                            key,
                            AgentEntry {
                                device: ev.device.clone(),
                                agent: ev.agent.clone(),
                                status: ev.status.clone(),
                                task: ev.task.clone(),
                                details: ev.details.clone(),
                                first_seen: ev.ts,
                                last_seen: ev.ts,
                            },
                        );
                    }
                    events.push_back(ev);
                    while events.len() > MAX_EVENTS_IN_MEMORY {
                        events.pop_front();
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        let file = Some(
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(data_file)?,
        );
        Ok(Store {
            inner: Mutex::new(Inner { agents, events, next_id, file }),
            log_path: data_file.to_path_buf(),
            generation: AtomicU64::new(1),
        })
    }

    fn append_locked(inner: &mut Inner, log_path: &Path, ev: &Event) {
        // Rotate before appending if oversized.
        if let Ok(meta) = std::fs::metadata(log_path) {
            if meta.len() > MAX_LOG_BYTES {
                let old = log_path.with_extension("jsonl.old");
                let _ = std::fs::remove_file(&old);
                if std::fs::rename(log_path, &old).is_ok() {
                    inner.file = OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(log_path)
                        .ok();
                }
            }
        }
        if let Some(f) = inner.file.as_mut() {
            let _ = writeln!(f, "{}", ev.to_line());
            let _ = f.flush();
        }
    }

    fn record(&self, kind: &str, device: &str, agent: &str, level: &str, message: &str, status: &str, task: &str, details: &str, bump_agent: bool) -> Event {
        let ts = now_secs();
        let mut ev = Event {
            id: 0,
            ts,
            device: clamp(device, 64),
            agent: clamp(agent, 64),
            level: clamp(level, 16),
            message: clamp(message, 2000),
            status: clamp(status, 16),
            task: clamp(task, 500),
            details: clamp(details, 2000),
            kind: kind.to_string(),
        };
        let mut inner = self.inner.lock().unwrap();
        ev.id = inner.next_id;
        inner.next_id += 1;
        if bump_agent {
            let key = (ev.device.clone(), ev.agent.clone());
            match inner.agents.get_mut(&key) {
                Some(a) => {
                    if kind == "checkin" {
                        a.status = ev.status.clone();
                        a.task = ev.task.clone();
                        a.details = ev.details.clone();
                    }
                    a.last_seen = ev.ts;
                }
                None => {
                    inner.agents.insert(
                        key,
                        AgentEntry {
                            device: ev.device.clone(),
                            agent: ev.agent.clone(),
                            status: if kind == "checkin" { ev.status.clone() } else { "idle".into() },
                            task: if kind == "checkin" { ev.task.clone() } else { String::new() },
                            details: if kind == "checkin" { ev.details.clone() } else { String::new() },
                            first_seen: ev.ts,
                            last_seen: ev.ts,
                        },
                    );
                }
            }
        }
        inner.events.push_back(ev.clone());
        while inner.events.len() > MAX_EVENTS_IN_MEMORY {
            inner.events.pop_front();
        }
        Self::append_locked(&mut inner, &self.log_path, &ev);
        self.generation.fetch_add(1, Ordering::SeqCst);
        ev
    }

    pub fn check_in(&self, device: &str, agent: &str, status: &str, task: &str, details: &str) -> Event {
        self.record("checkin", device, agent, "info", &format!("status: {status} — {task}"), status, task, details, true)
    }

    pub fn log_event(&self, device: &str, agent: &str, level: &str, message: &str) -> Event {
        self.record("event", device, agent, level, message, "", "", "", true)
    }

    pub fn heartbeat(&self, device: &str, agent: &str) {
        self.record("event", device, agent, "info", "heartbeat", "", "", "", true);
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// (agents sorted by device/agent, events oldest→newest)
    pub fn snapshot(&self) -> (Vec<AgentEntry>, Vec<Event>) {
        let inner = self.inner.lock().unwrap();
        let mut agents: Vec<AgentEntry> = inner.agents.values().cloned().collect();
        agents.sort_by(|a, b| (&a.device, &a.agent).cmp(&(&b.device, &b.agent)));
        (agents, inner.events.iter().cloned().collect())
    }

    pub fn to_state_json(&self, started_at: u64) -> Value {
        let now = now_secs();
        let (agents, events) = self.snapshot();
        let agents_v: Vec<Value> = agents
            .iter()
            .map(|a| {
                Value::obj(vec![
                    ("device", Value::from(a.device.as_str())),
                    ("agent", Value::from(a.agent.as_str())),
                    ("status", Value::from(a.status.as_str())),
                    ("task", Value::from(a.task.as_str())),
                    ("details", Value::from(a.details.as_str())),
                    ("first_seen", Value::from(a.first_seen as i64)),
                    ("last_seen", Value::from(a.last_seen as i64)),
                    ("stale", Value::from(is_stale(now, a.last_seen))),
                ])
            })
            .collect();
        let events_v: Vec<Value> = events
            .iter()
            .map(|e| {
                Value::obj(vec![
                    ("id", Value::from(e.id as i64)),
                    ("ts", Value::from(e.ts as i64)),
                    ("device", Value::from(e.device.as_str())),
                    ("agent", Value::from(e.agent.as_str())),
                    ("level", Value::from(e.level.as_str())),
                    ("message", Value::from(e.message.as_str())),
                ])
            })
            .collect();
        Value::obj(vec![
            (
                "server",
                Value::obj(vec![
                    ("now", Value::from(now as i64)),
                    ("started_at", Value::from(started_at as i64)),
                    ("version", Value::from(crate::VERSION)),
                ]),
            ),
            ("agents", Value::Arr(agents_v)),
            ("events", Value::Arr(events_v)),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(tag: &str) -> (Store, PathBuf) {
        let d = std::env::temp_dir().join(format!(
            "wtf-store-{tag}-{}-{}",
            std::process::id(),
            crate::rand::hex(6)
        ));
        let p = d.join("events.jsonl");
        (Store::new(&p).unwrap(), d)
    }

    #[test]
    fn checkin_event_flow() {
        let (s, d) = temp_store("flow");
        s.check_in("box1", "oz", "working", "build hub", "compiling");
        s.log_event("box1", "oz", "warn", "clippy unhappy");
        s.heartbeat("box1", "oz");
        let (agents, events) = s.snapshot();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].status, "working"); // heartbeat touches last_seen only
        assert_eq!(agents[0].task, "build hub");
        assert_eq!(events.len(), 3);
        assert_eq!(events[2].message, "heartbeat");
        assert!(s.generation() >= 4);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn persistence_replay() {
        let (s, d) = temp_store("replay");
        s.check_in("box1", "oz", "working", "task A", "");
        s.check_in("box2", "claude", "blocked", "task B", "waiting on tests");
        s.log_event("box1", "oz", "error", "boom");
        let path = d.join("events.jsonl");
        let s2 = Store::new(&path).unwrap();
        let (agents, events) = s2.snapshot();
        assert_eq!(agents.len(), 2);
        let oz = agents.iter().find(|a| a.agent == "oz").unwrap();
        assert_eq!(oz.status, "working");
        assert_eq!(events.len(), 3);
        assert_eq!(events[2].level, "error");
        assert_eq!(events[2].id, 3);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn clamping_and_ids() {
        let (s, d) = temp_store("clamp");
        let big = "x".repeat(5000);
        let ev = s.log_event("d", "a", "info", &big);
        assert_eq!(ev.message.len(), 2000);
        assert_eq!(ev.id, 1);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn rel_age_format() {
        let now = 1_000_000;
        assert_eq!(rel_age(now, now), "0s");
        assert_eq!(rel_age(now, now - 59), "59s");
        assert_eq!(rel_age(now, now - 120), "2m");
        assert_eq!(rel_age(now, now - 7200), "2h");
        assert_eq!(rel_age(now, now - 172800), "2d");
    }
}
