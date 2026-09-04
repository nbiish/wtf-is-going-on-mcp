//! Hub state: live agent statuses and the event log.
//!
//! Events are the source of truth: the append-only JSONL log is replayed on
//! startup to rebuild agent state, so a hub restart loses nothing. The log
//! rotates at 10 MB (renamed to events.jsonl.old) so it cannot grow forever.
//!
//! Federation: every event carries `origin` (the originating hub's stable
//! name) and `origin_id` (monotonic AT the origin). Locally-recorded events
//! stamp this hub's own name as origin; replicated events keep the remote
//! origin and are deduped on (origin, origin_id). Pre-federation events
//! replay with origin "" and remain first-class.

use crate::bins::Bins;
use crate::json::Value;
use crate::util::{clamp, now_secs};
use std::collections::{HashMap, HashSet, VecDeque};
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

/// Dashboard-facing session summary (metadata only — no keys, no ciphertext).
#[derive(Clone, Debug)]
pub struct SessionSummary {
    pub id: String,
    pub name: String,
    pub repo: String,
    pub members: usize,
    pub msg_count: usize,
}

#[derive(Clone, Debug)]
pub struct AgentEntry {
    pub device: String,
    pub agent: String,
    pub status: String,
    pub task: String,
    pub details: String,
    pub repo: String,
    pub origin: String,
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
    /// Originating hub's stable name ("" = pre-federation local event).
    pub origin: String,
    /// Monotonic sequence AT the origin hub. Dedupe key = (origin, origin_id).
    pub origin_id: u64,
    /// Optional repository label (checked-out project dir name).
    pub repo: String,
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
            ("origin", Value::from(self.origin.as_str())),
            ("origin_id", Value::from(self.origin_id as i64)),
            ("repo", Value::from(self.repo.as_str())),
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
            origin: v
                .get("origin")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            origin_id: v.get("origin_id").and_then(|x| x.as_i64()).unwrap_or(0) as u64,
            repo: v
                .get("repo")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
        })
    }
}

struct Inner {
    agents: HashMap<(String, String), AgentEntry>,
    events: VecDeque<Event>,
    next_id: u64,
    file: Option<std::fs::File>,
    /// (origin, origin_id) of every event ever seen — dedupe for replication.
    seen: HashSet<(String, u64)>,
}

pub struct Store {
    inner: Mutex<Inner>,
    log_path: PathBuf,
    generation: AtomicU64,
    /// Origin name stamped on locally-recorded events (this hub's fed name).
    origin_name: Mutex<String>,
    /// Live session-summary provider, wired by the hub at serve time so the
    /// dashboard state payload carries SESSIONS metadata without coupling
    /// Store to the Sessions registry.
    sessions_provider: Mutex<Option<Box<dyn Fn() -> Vec<SessionSummary> + Send>>>,
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
        let mut seen = HashSet::new();
        let mut next_id = 1u64;
        match std::fs::read_to_string(data_file) {
            Ok(text) => {
                for line in text.lines() {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    let Ok(v) = crate::json::parse(line) else {
                        continue;
                    };
                    let Some(ev) = Event::from_line(&v) else {
                        continue;
                    };
                    next_id = next_id.max(ev.id + 1);
                    seen.insert((ev.origin.clone(), ev.origin_id));
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
                                repo: ev.repo.clone(),
                                origin: ev.origin.clone(),
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
            inner: Mutex::new(Inner {
                agents,
                events,
                next_id,
                file,
                seen,
            }),
            log_path: data_file.to_path_buf(),
            generation: AtomicU64::new(1),
            origin_name: Mutex::new(String::new()),
            sessions_provider: Mutex::new(None),
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

    #[allow(clippy::too_many_arguments)]
    fn record(
        &self,
        kind: &str,
        device: &str,
        agent: &str,
        level: &str,
        message: &str,
        status: &str,
        task: &str,
        details: &str,
        repo: &str,
        bump_agent: bool,
        quiet: bool,
    ) -> Event {
        let ts = now_secs();
        let mut inner = self.inner.lock().unwrap();
        let origin = self.origin_name();
        let ev = Event {
            id: inner.next_id,
            ts,
            device: clamp(device, 64),
            agent: clamp(agent, 64),
            level: clamp(level, 16),
            message: clamp(message, 2000),
            status: clamp(status, 16),
            task: clamp(task, 500),
            details: clamp(details, 2000),
            kind: kind.to_string(),
            origin: origin.clone(),
            origin_id: inner.next_id,
            repo: clamp(repo, 128),
        };
        inner.next_id += 1;
        inner.seen.insert((ev.origin.clone(), ev.origin_id));
        if bump_agent {
            let key = (ev.device.clone(), ev.agent.clone());
            match inner.agents.get_mut(&key) {
                Some(a) => {
                    if kind == "checkin" {
                        a.status = ev.status.clone();
                        a.task = ev.task.clone();
                        a.details = ev.details.clone();
                        a.repo = ev.repo.clone();
                    }
                    a.last_seen = ev.ts;
                }
                None => {
                    inner.agents.insert(
                        key,
                        AgentEntry {
                            device: ev.device.clone(),
                            agent: ev.agent.clone(),
                            status: if kind == "checkin" {
                                ev.status.clone()
                            } else {
                                "idle".into()
                            },
                            task: if kind == "checkin" {
                                ev.task.clone()
                            } else {
                                String::new()
                            },
                            details: if kind == "checkin" {
                                ev.details.clone()
                            } else {
                                String::new()
                            },
                            repo: ev.repo.clone(),
                            origin: ev.origin.clone(),
                            first_seen: ev.ts,
                            last_seen: ev.ts,
                        },
                    );
                }
            }
        }
        // Quiet events (heartbeats, replication churn) update agent
        // presence but never enter the ring, the dashboard feed, or the
        // log file — operator directive: the dashboard shows work, not
        // connection noise; disk keeps only troubleshooting-worthy logs.
        if quiet {
            drop(inner);
            return ev;
        }
        inner.events.push_back(ev.clone());
        while inner.events.len() > MAX_EVENTS_IN_MEMORY {
            inner.events.pop_front();
        }
        Self::append_locked(&mut inner, &self.log_path, &ev);
        drop(inner);
        self.generation.fetch_add(1, Ordering::SeqCst);
        ev
    }

    pub fn check_in(
        &self,
        device: &str,
        agent: &str,
        status: &str,
        task: &str,
        details: &str,
        repo: &str,
    ) -> Event {
        self.record(
            "checkin",
            device,
            agent,
            "info",
            &format!("status: {status} — {task}"),
            status,
            task,
            details,
            repo,
            true,
            false,
        )
    }

    pub fn log_event(
        &self,
        device: &str,
        agent: &str,
        level: &str,
        message: &str,
        repo: &str,
    ) -> Event {
        self.record(
            "event", device, agent, level, message, "", "", "", repo, true, false,
        )
    }

    pub fn heartbeat(&self, device: &str, agent: &str) {
        self.record(
            "event",
            device,
            agent,
            "info",
            "heartbeat",
            "",
            "",
            "",
            "",
            true,
            true,
        );
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// The origin name stamped on locally-recorded events. Wired by the hub
    /// at startup; empty = pre-federation (events keep origin "").
    pub fn origin_name(&self) -> String {
        self.origin_name.lock().unwrap().clone()
    }

    pub fn set_origin_name(&self, name: &str) {
        *self.origin_name.lock().unwrap() = name.to_string();
    }

    /// (agents sorted by device/agent, events oldest→newest)
    pub fn snapshot(&self) -> (Vec<AgentEntry>, Vec<Event>) {
        let inner = self.inner.lock().unwrap();
        let mut agents: Vec<AgentEntry> = inner.agents.values().cloned().collect();
        agents.sort_by(|a, b| (&a.device, &a.agent).cmp(&(&b.device, &b.agent)));
        (agents, inner.events.iter().cloned().collect())
    }

    /// Highest origin_id recorded for a given origin (0 if none) — the pull
    /// cursor a peer hands us. For origin "" (pre-federation events) the
    /// cursor is meaningless; peers always also exchange counts via
    /// `stats_since` for anti-entropy.
    pub fn max_origin_id(&self, origin: &str) -> u64 {
        let inner = self.inner.lock().unwrap();
        inner
            .events
            .iter()
            .filter(|e| e.origin == origin)
            .map(|e| e.origin_id)
            .max()
            .unwrap_or(0)
    }

    /// All events with origin == `origin` and origin_id > `after`, oldest first.
    pub fn events_since(&self, origin: &str, after: u64) -> Vec<Event> {
        let inner = self.inner.lock().unwrap();
        inner
            .events
            .iter()
            .filter(|e| e.origin == origin && e.origin_id > after)
            .cloned()
            .collect()
    }

    /// Ingest a replicated event. Dedupes on (origin, origin_id); assigns a
    /// LOCAL id for ordering (the wire order defines it, monotonic local
    /// sequence); persists; bumps generation. Agent-card effects apply only
    /// when the ingest is newer than the card's current state (last-writer
    /// wins by origin ts, matching the append-only posture).
    /// Returns true if the event was new.
    pub fn ingest(&self, ev: &Event) -> bool {
        let key = (ev.origin.clone(), ev.origin_id);
        {
            let inner = self.inner.lock().unwrap();
            if inner.seen.contains(&key) {
                return false;
            }
        }
        let mut inner = self.inner.lock().unwrap();
        // Re-check under the same lock acquisition to close the race window.
        if inner.seen.contains(&key) {
            return false;
        }
        let mut ev = ev.clone();
        ev.id = inner.next_id;
        inner.next_id += 1;
        inner.seen.insert(key);
        if ev.kind == "checkin" {
            let akey = (ev.device.clone(), ev.agent.clone());
            match inner.agents.get_mut(&akey) {
                Some(a) if a.last_seen <= ev.ts => {
                    a.status = ev.status.clone();
                    a.task = ev.task.clone();
                    a.details = ev.details.clone();
                    a.repo = ev.repo.clone();
                    a.origin = ev.origin.clone();
                    a.last_seen = ev.ts;
                }
                Some(_) => {}
                None => {
                    inner.agents.insert(
                        akey,
                        AgentEntry {
                            device: ev.device.clone(),
                            agent: ev.agent.clone(),
                            status: ev.status.clone(),
                            task: ev.task.clone(),
                            details: ev.details.clone(),
                            repo: ev.repo.clone(),
                            origin: ev.origin.clone(),
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
        drop(inner);
        self.generation.fetch_add(1, Ordering::SeqCst);
        true
    }

    pub fn to_state_json(&self, started_at: u64, bins: &Bins) -> Value {
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
                    ("repo", Value::from(a.repo.as_str())),
                    ("origin", Value::from(a.origin.as_str())),
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
                    ("origin", Value::from(e.origin.as_str())),
                    ("repo", Value::from(e.repo.as_str())),
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
            ("bins", bins.to_state_json()),
            (
                "bin_scopes",
                Value::Arr(
                    bins.scopes()
                        .iter()
                        .map(|s| Value::from(s.as_str()))
                        .collect(),
                ),
            ),
            ("sessions", self.sessions_v()),
        ])
    }

    /// Dashboard session summaries: metadata only (no ciphertext, no
    /// member keys) — id, name, repo, member count, message count.
    /// Sourced from the hub's live Sessions registry, wired in at serve
    /// time (default = empty vec, so pre-wiring callers stay correct).
    fn sessions_v(&self) -> Value {
        let snap = self.sessions_snapshot();
        let arr: Vec<Value> = snap
            .iter()
            .map(|s| {
                Value::obj(vec![
                    ("id", Value::from(s.id.as_str())),
                    ("name", Value::from(s.name.as_str())),
                    ("repo", Value::from(s.repo.as_str())),
                    ("members", Value::from(s.members as i64)),
                    ("msg_count", Value::from(s.msg_count as i64)),
                ])
            })
            .collect();
        Value::Arr(arr)
    }
    pub fn set_sessions_provider(&self, provider: Box<dyn Fn() -> Vec<SessionSummary> + Send>) {
        *self.sessions_provider.lock().unwrap() = Some(provider);
    }

    fn sessions_snapshot(&self) -> Vec<SessionSummary> {
        let p = self.sessions_provider.lock().unwrap();
        match p.as_ref() {
            Some(f) => f(),
            None => Vec::new(),
        }
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
        s.check_in("box1", "oz", "working", "build hub", "compiling", "wtf");
        s.log_event("box1", "oz", "warn", "clippy unhappy", "wtf");
        s.heartbeat("box1", "oz");
        let (agents, events) = s.snapshot();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].status, "working"); // heartbeat touches last_seen only
        assert_eq!(agents[0].task, "build hub");
        assert_eq!(agents[0].repo, "wtf");
        // v0.15.0: heartbeats are quiet — they update presence but never
        // enter the ring, the feed, or the log.
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].message, "clippy unhappy");
        assert!(s.generation() >= 3);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn persistence_replay() {
        let (s, d) = temp_store("replay");
        s.check_in("box1", "oz", "working", "task A", "", "repo-a");
        s.check_in(
            "box2",
            "claude",
            "blocked",
            "task B",
            "waiting on tests",
            "",
        );
        s.log_event("box1", "oz", "error", "boom", "");
        let path = d.join("events.jsonl");
        let s2 = Store::new(&path).unwrap();
        let (agents, events) = s2.snapshot();
        assert_eq!(agents.len(), 2);
        let oz = agents.iter().find(|a| a.agent == "oz").unwrap();
        assert_eq!(oz.status, "working");
        assert_eq!(oz.repo, "repo-a");
        assert_eq!(events.len(), 3);
        assert_eq!(events[2].level, "error");
        assert_eq!(events[2].id, 3);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn clamping_and_ids() {
        let (s, d) = temp_store("clamp");
        let big = "x".repeat(5000);
        let ev = s.log_event("d", "a", "info", &big, "");
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

    #[test]
    fn ingest_dedupes_and_cursors() {
        let (s, d) = temp_store("ingest");
        let mk = |oid: u64, ts: u64, msg: &str| Event {
            id: 0,
            ts,
            device: "box2".into(),
            agent: "remote-agent".into(),
            level: "info".into(),
            message: msg.into(),
            status: "working".into(),
            task: "remote task".into(),
            details: String::new(),
            kind: "checkin".into(),
            origin: "hub-peer".into(),
            origin_id: oid,
            repo: "other-repo".into(),
        };
        assert!(s.ingest(&mk(1, 100, "first")));
        assert!(!s.ingest(&mk(1, 100, "first"))); // duplicate dropped
        assert!(s.ingest(&mk(2, 200, "second")));
        let (_, events) = s.snapshot();
        assert_eq!(events.len(), 2);
        assert_eq!(s.max_origin_id("hub-peer"), 2);
        assert_eq!(s.max_origin_id("nobody"), 0);
        assert_eq!(s.events_since("hub-peer", 1).len(), 1);
        assert_eq!(s.events_since("hub-peer", 2).len(), 0);
        // agent card reflects the newer ingest
        let (agents, _) = s.snapshot();
        let ra = agents.iter().find(|a| a.agent == "remote-agent").unwrap();
        assert_eq!(ra.task, "remote task");
        assert_eq!(ra.origin, "hub-peer");
        assert_eq!(ra.repo, "other-repo");
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn stale_agent_card_not_regressed_by_old_ingest() {
        let (s, d) = temp_store("noregress");
        s.check_in("box3", "ag", "done", "new work", "", "");
        let (_, events) = s.snapshot();
        let local_ts = events.last().unwrap().ts;
        let old = Event {
            id: 0,
            ts: local_ts.saturating_sub(500),
            device: "box3".into(),
            agent: "ag".into(),
            level: "info".into(),
            message: "old".into(),
            status: "working".into(),
            task: "old work".into(),
            details: String::new(),
            kind: "checkin".into(),
            origin: "hub-peer".into(),
            origin_id: 9,
            repo: String::new(),
        };
        assert!(s.ingest(&old));
        let (agents, _) = s.snapshot();
        let ag = agents.iter().find(|a| a.agent == "ag").unwrap();
        assert_eq!(ag.status, "done"); // newer local card kept
        std::fs::remove_dir_all(&d).ok();
    }
}
