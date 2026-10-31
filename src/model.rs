//! The wire model the page consumes: sessions, machines, sources, snapshot.

use crate::json::Value;

#[derive(Clone, Debug)]
pub struct Session {
    pub id: String,
    pub machine: String,
    pub source: &'static str, // "orca" | "herdr"
    pub agent: String,        // agent kind, or "shell"
    pub state: String,        // working | blocked | done | idle | unknown | shell
    pub title: String,
    pub cwd: String,
    pub detail: String, // recent output / last message, cleaned
    pub handle: String, // terminal handle or pane id, for copy
    pub updated_at: u64,
    pub live: bool,
}

impl Session {
    pub fn to_json(&self) -> Value {
        Value::obj(vec![
            ("id", Value::from(self.id.clone())),
            ("machine", Value::from(self.machine.clone())),
            ("source", Value::from(self.source)),
            ("agent", Value::from(self.agent.clone())),
            ("state", Value::from(self.state.clone())),
            ("title", Value::from(self.title.clone())),
            ("cwd", Value::from(self.cwd.clone())),
            ("detail", Value::from(self.detail.clone())),
            ("handle", Value::from(self.handle.clone())),
            ("updated_at", Value::num(self.updated_at as f64)),
            ("live", Value::Bool(self.live)),
        ])
    }

    pub fn busy(&self) -> bool {
        matches!(self.state.as_str(), "working" | "blocked")
    }
}

#[derive(Clone, Debug)]
pub struct Machine {
    pub name: String,
    pub kind: String, // local | ssh | paired
    pub detail: String,
    pub available: bool,
}

impl Machine {
    pub fn to_json(&self, session_count: usize) -> Value {
        Value::obj(vec![
            ("name", Value::from(self.name.clone())),
            ("kind", Value::from(self.kind.clone())),
            ("detail", Value::from(self.detail.clone())),
            ("available", Value::Bool(self.available)),
            ("sessions", Value::num(session_count as f64)),
        ])
    }
}

#[derive(Clone, Debug)]
pub struct SourceStatus {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
    pub ms: u128,
}

impl SourceStatus {
    pub fn to_json(&self) -> Value {
        Value::obj(vec![
            ("name", Value::from(self.name)),
            ("ok", Value::Bool(self.ok)),
            ("detail", Value::from(self.detail.clone())),
            ("ms", Value::num(self.ms as f64)),
        ])
    }
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub generated_at: u64,
    pub sessions: Vec<Session>,
    pub machines: Vec<Machine>,
    pub sources: Vec<SourceStatus>,
    pub bin_count: usize,
}

impl Snapshot {
    pub fn to_json(&self) -> Value {
        let working = self
            .sessions
            .iter()
            .filter(|s| matches!(s.state.as_str(), "working" | "blocked"))
            .count();
        let sessions: Vec<Value> = self.sessions.iter().map(|s| s.to_json()).collect();
        let machines: Vec<Value> = self
            .machines
            .iter()
            .map(|m| {
                let n = self.sessions.iter().filter(|s| s.machine == m.name).count();
                m.to_json(n)
            })
            .collect();
        let sources: Vec<Value> = self.sources.iter().map(|s| s.to_json()).collect();
        Value::obj(vec![
            ("generated_at", Value::num(self.generated_at as f64)),
            ("version", Value::from(crate::VERSION)),
            (
                "counts",
                Value::obj(vec![
                    ("sessions", Value::num(self.sessions.len() as f64)),
                    ("working", Value::num(working as f64)),
                    ("machines", Value::num(self.machines.len() as f64)),
                    ("bins", Value::num(self.bin_count as f64)),
                ]),
            ),
            ("machines", Value::arr(machines)),
            ("sessions", Value::arr(sessions)),
            ("sources", Value::arr(sources)),
        ])
    }
}
