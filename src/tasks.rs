//! Federated task dispatch: designate an agent on another paired machine,
//! have it run the work, and read the result back.
//!
//! This is the work-item lane that sits beside the encrypted chat lane. A
//! *task* is a durable, addressable unit of cross-machine work: one caller
//! names a target machine (or a compute/architecture class), the record is
//! queued on the hub, and an agent on the target machine claims it, runs it
//! with its OWN local tooling, and reports the result. One task optionally
//! binds one federated chat session, so the human-reviewable conversation
//! and the machine-readable result live on the same handle.
//!
//! Design boundaries (deliberate, see `src/llms.txt`):
//! - **Pull, never spawn.** The hub never launches an agent harness. Target
//!   machines claim work; a queued task is a legitimate resting state, not a
//!   failure. This preserves the harness-free contract from v0.16.0.
//! - **Leases, not locks.** A claim carries a TTL. If the claiming machine
//!   dies mid-task the record returns to the queue on the next read, so
//!   work is never stranded behind a crashed peer.
//! - **Hub-local durable state.** The record lives in a `0600` JSON file
//!   beside `sessions.json` and is replayed on boot. Confidential payloads
//!   still travel only inside encrypted sessions; a task record holds the
//!   work item and its result, which the operator may read on the dashboard.

use crate::json::Value;
use crate::util::{clamp, now_secs};
use std::path::PathBuf;
use std::sync::Mutex;

/// Lifecycle states. `queued` -> `claimed` -> `running` -> `done`|`failed`,
/// with `blocked` (needs a human/peer decision), `cancelled` (by the
/// dispatcher), and lease-expiry returning an abandoned claim to `queued`.
pub const STATES: [&str; 7] = [
    "queued", "claimed", "running", "done", "failed", "blocked", "cancelled",
];

/// Terminal states never transition again.
pub const TERMINAL: [&str; 3] = ["done", "failed", "cancelled"];

pub const MAX_TASKS: usize = 512;
pub const MAX_TITLE_CHARS: usize = 160;
pub const MAX_DETAIL_CHARS: usize = 8_000;
pub const MAX_RESULT_CHARS: usize = 16_000;
pub const MAX_ERROR_CHARS: usize = 2_000;
pub const MAX_TARGET_CHARS: usize = 160;
pub const MAX_REPO_CHARS: usize = 128;
/// A claim holds the task this long before it returns to the queue.
pub const CLAIM_TTL_SECS: u64 = 900;

/// One unit of cross-machine work.
#[derive(Clone, Debug, PartialEq)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub detail: String,
    pub repo: String,
    /// Machine selector: `any`, an exact machine name/alias, `tier:heavy`,
    /// `tier:standard`, `tier:edge`, or `arch:<os>-<arch>`.
    pub target: String,
    /// Agent runtime to start on the target: `auto` (Orca, else Herdr),
    /// `orca`, or `herdr`. Chosen at dispatch and reported back with the run.
    pub runner: String,
    /// Runner-issued handle for the started agent (Orca dispatch id or Herdr
    /// pane id); empty until the task is actually launched.
    pub run_handle: String,
    pub origin_device: String,
    pub origin_agent: String,
    /// Device that claimed it (`""` while queued).
    pub assignee: String,
    /// Federated chat session bound to this task (`""` = unbound).
    pub session: String,
    pub state: String,
    pub result: String,
    pub error: String,
    pub attempts: u32,
    pub created_at: u64,
    pub updated_at: u64,
    pub claimed_at: u64,
    pub finished_at: u64,
    /// Unix ts after which an unfinished claim is considered abandoned.
    pub lease_until: u64,
}

impl Task {
    pub fn is_terminal(&self) -> bool {
        TERMINAL.contains(&self.state.as_str())
    }

    /// Dashboard/wire projection. Ciphertext is never here; a task record is
    /// operator-readable work state.
    pub fn to_json(&self) -> Value {
        Value::obj(vec![
            ("id", Value::from(self.id.as_str())),
            ("title", Value::from(self.title.as_str())),
            ("detail", Value::from(self.detail.as_str())),
            ("repo", Value::from(self.repo.as_str())),
            ("target", Value::from(self.target.as_str())),
            ("runner", Value::from(self.runner.as_str())),
            ("run_handle", Value::from(self.run_handle.as_str())),
            ("origin_device", Value::from(self.origin_device.as_str())),
            ("origin_agent", Value::from(self.origin_agent.as_str())),
            ("assignee", Value::from(self.assignee.as_str())),
            ("session", Value::from(self.session.as_str())),
            ("state", Value::from(self.state.as_str())),
            ("result", Value::from(self.result.as_str())),
            ("error", Value::from(self.error.as_str())),
            ("attempts", Value::from(self.attempts as i64)),
            ("created_at", Value::from(self.created_at as i64)),
            ("updated_at", Value::from(self.updated_at as i64)),
            ("claimed_at", Value::from(self.claimed_at as i64)),
            ("finished_at", Value::from(self.finished_at as i64)),
            ("lease_until", Value::from(self.lease_until as i64)),
            ("terminal", Value::from(self.is_terminal())),
        ])
    }

    /// Compact one-line form used by the text board.
    pub fn to_line(&self) -> String {
        let who = if self.assignee.is_empty() {
            "-".to_string()
        } else {
            self.assignee.clone()
        };
        let mut line = format!(
            "[{}] {} -> {} ({}) {}",
            self.state, self.id, self.target, who, self.title
        );
        if !self.repo.is_empty() {
            line.push_str(&format!(" @{}", self.repo));
        }
        if !self.session.is_empty() {
            line.push_str(&format!(" ~chat:{}", short_id(&self.session)));
        }
        if !self.run_handle.is_empty() {
            line.push_str(&format!(" run:{}", self.run_handle));
        }
        line
    }

    pub fn from_json(v: &Value) -> Option<Task> {
        Some(Task {
            id: v.get("id")?.as_str()?.to_string(),
            title: v.get("title").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            detail: v.get("detail").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            repo: v.get("repo").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            target: v.get("target").and_then(|x| x.as_str()).unwrap_or("any").to_string(),
            runner: v
                .get("runner")
                .and_then(|x| x.as_str())
                .unwrap_or("auto")
                .to_string(),
            run_handle: v
                .get("run_handle")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            origin_device: v
                .get("origin_device")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            origin_agent: v
                .get("origin_agent")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            assignee: v.get("assignee").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            session: v.get("session").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            state: v.get("state").and_then(|x| x.as_str()).unwrap_or("queued").to_string(),
            result: v.get("result").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            error: v.get("error").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            attempts: v.get("attempts").and_then(|x| x.as_i64()).unwrap_or(0).max(0) as u32,
            created_at: v.get("created_at").and_then(|x| x.as_i64()).unwrap_or(0).max(0) as u64,
            updated_at: v.get("updated_at").and_then(|x| x.as_i64()).unwrap_or(0).max(0) as u64,
            claimed_at: v.get("claimed_at").and_then(|x| x.as_i64()).unwrap_or(0).max(0) as u64,
            finished_at: v
                .get("finished_at")
                .and_then(|x| x.as_i64())
                .unwrap_or(0)
                .max(0) as u64,
            lease_until: v
                .get("lease_until")
                .and_then(|x| x.as_i64())
                .unwrap_or(0)
                .max(0) as u64,
        })
    }
}

/// Abbreviate a session/task id for one-line display.
pub fn short_id(id: &str) -> String {
    if id.len() <= 8 {
        id.to_string()
    } else {
        id[..8].to_string()
    }
}

/// Does `target` select the machine described by (`machine`, `tier`, `arch`)?
///
/// `any`/`*`/empty match everything (cluster routing — the first claimer
/// wins). `tier:` and `arch:` select a capability class so a caller can say
/// "run this on the heaviest box" or "this needs darwin-arm64" without
/// naming a host. Anything else is an exact, case-insensitive machine name
/// or alias. Matching is deliberately total: an unresolvable selector simply
/// matches nothing, which leaves the task queued rather than mis-routing it.
pub fn target_matches(target: &str, machine: &str, tier: &str, arch: &str) -> bool {
    let t = target.trim().to_ascii_lowercase();
    if t.is_empty() || t == "any" || t == "*" {
        return true;
    }
    if let Some(want) = t.strip_prefix("tier:") {
        return !want.is_empty() && want == tier.trim().to_ascii_lowercase();
    }
    if let Some(want) = t.strip_prefix("arch:") {
        return !want.is_empty() && want == arch.trim().to_ascii_lowercase();
    }
    if machine.trim().is_empty() {
        return false;
    }
    t == machine.trim().to_ascii_lowercase()
}

/// Validate a target selector's *shape* (not its resolvability — the hub
/// cannot know every peer's hardware). Fails closed on unknown prefixes so a
/// typo cannot silently become "match everything".
pub fn valid_target(target: &str) -> Result<(), String> {
    let t = target.trim();
    if t.chars().count() > MAX_TARGET_CHARS {
        return Err(format!("target too long (max {MAX_TARGET_CHARS} chars)"));
    }
    if t.is_empty() || t == "any" || t == "*" {
        return Ok(());
    }
    if let Some(rest) = t.strip_prefix("tier:") {
        if !["heavy", "standard", "edge"].contains(&rest.to_ascii_lowercase().as_str()) {
            return Err(format!(
                "unknown tier '{rest}'; use tier:heavy, tier:standard, or tier:edge"
            ));
        }
        return Ok(());
    }
    if let Some(rest) = t.strip_prefix("arch:") {
        if rest.trim().is_empty() {
            return Err("arch: needs a value, e.g. arch:darwin-arm64".into());
        }
        return Ok(());
    }
    // A bare machine name must not smuggle a prefix-like typo through.
    if t.contains(':') {
        return Err(format!(
            "unknown target selector '{t}'; use a machine name, any, tier:<t>, or arch:<a>"
        ));
    }
    Ok(())
}

/// Legal transition check. Kept explicit so an out-of-order claim from a
/// stale peer fails closed instead of silently overwriting newer state.
pub fn can_transition(from: &str, to: &str) -> bool {
    match (from, to) {
        // A fresh dispatch starts queued.
        ("", "queued") => true,
        // Queue -> a worker picks it up.
        ("queued", "claimed") => true,
        // The claimer starts, or reports a blocker.
        ("claimed", "running") => true,
        ("claimed", "blocked") => true,
        ("claimed", "done") => true,   // short tasks skip `running`
        ("claimed", "failed") => true,
        ("running", "done") => true,
        ("running", "failed") => true,
        ("running", "blocked") => true,
        ("blocked", "running") => true,
        ("blocked", "done") => true,
        ("blocked", "failed") => true,
        // Any non-terminal state may be cancelled by the dispatcher, and a
        // lease expiry re-queues an abandoned claim.
        (_, "cancelled") => true,
        ("claimed", "queued") => true,
        ("running", "queued") => true,
        _ => false,
    }
}

pub struct TaskStore {
    inner: Mutex<Vec<Task>>,
    path: PathBuf,
}

impl TaskStore {
    pub fn load() -> TaskStore {
        Self::load_at(&crate::config::tasks_path())
    }

    /// Load from disk; missing or corrupt file yields empty state (same
    /// posture as the session registry — a bad file must not wedge the hub).
    pub fn load_at(path: &PathBuf) -> TaskStore {
        let mut tasks = Vec::new();
        if let Ok(Some(v)) = crate::config::load_json(path) {
            if let Some(arr) = v.get("tasks").and_then(|x| x.as_arr()) {
                for item in arr {
                    if let Some(t) = Task::from_json(item) {
                        tasks.push(t);
                    }
                }
            }
        }
        tasks.truncate(MAX_TASKS);
        TaskStore {
            inner: Mutex::new(tasks),
            path: path.clone(),
        }
    }

    fn persist(&self, tasks: &[Task]) -> Result<(), String> {
        let arr: Vec<Value> = tasks.iter().map(|t| t.to_json()).collect();
        crate::config::save_json(
            &self.path,
            &Value::obj(vec![("tasks", Value::Arr(arr))]),
            0o600,
        )
    }

    /// All tasks, newest first.
    pub fn list(&self) -> Vec<Task> {
        let mut v = self.inner.lock().unwrap().clone();
        v.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)));
        v
    }

    /// Tasks in open (non-terminal) states, oldest first — the work queue.
    pub fn open(&self) -> Vec<Task> {
        let mut v: Vec<Task> = self
            .inner
            .lock()
            .unwrap()
            .iter()
            .filter(|t| !t.is_terminal())
            .cloned()
            .collect();
        v.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
        v
    }

    pub fn get(&self, id: &str) -> Option<Task> {
        self.inner
            .lock()
            .unwrap()
            .iter()
            .find(|t| t.id == id)
            .cloned()
    }

    /// Queue a new task. Returns the stored record.
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        &self,
        title: &str,
        detail: &str,
        repo: &str,
        target: &str,
        origin_device: &str,
        origin_agent: &str,
        session: &str,
        runner: &str,
    ) -> Result<Task, String> {
        let title = title.trim();
        if title.is_empty() {
            return Err("title must not be empty".into());
        }
        if title.chars().count() > MAX_TITLE_CHARS {
            return Err(format!("title too long (max {MAX_TITLE_CHARS} chars)"));
        }
        let detail = detail.trim();
        if detail.chars().count() > MAX_DETAIL_CHARS {
            return Err(format!("detail too long (max {MAX_DETAIL_CHARS} chars)"));
        }
        let repo = repo.trim();
        if repo.chars().count() > MAX_REPO_CHARS {
            return Err(format!("repo too long (max {MAX_REPO_CHARS} chars)"));
        }
        let target = if target.trim().is_empty() { "any" } else { target.trim() };
        valid_target(target)?;
        let runner = if runner.trim().is_empty() {
            "auto"
        } else {
            runner.trim()
        };
        crate::runner::valid_runner(runner)?;

        let now = now_secs();
        let mut tasks = self.inner.lock().unwrap();
        if tasks.len() >= MAX_TASKS {
            return Err(format!("task registry full (max {MAX_TASKS})"));
        }
        let mut id = crate::rand::hex(8);
        while tasks.iter().any(|t| t.id == id) {
            id = crate::rand::hex(8);
        }
        let task = Task {
            id,
            title: clamp(title, MAX_TITLE_CHARS),
            detail: detail.to_string(),
            repo: repo.to_string(),
            target: target.to_string(),
            runner: runner.to_ascii_lowercase(),
            run_handle: String::new(),
            origin_device: origin_device.to_string(),
            origin_agent: origin_agent.to_string(),
            assignee: String::new(),
            session: session.trim().to_string(),
            state: "queued".to_string(),
            result: String::new(),
            error: String::new(),
            attempts: 0,
            created_at: now,
            updated_at: now,
            claimed_at: 0,
            finished_at: 0,
            lease_until: 0,
        };
        tasks.push(task.clone());
        self.persist(&tasks)?;
        Ok(task)
    }

    /// Apply a state change in place, persisting only on success.
    fn mutate<F>(&self, id: &str, f: F) -> Result<Task, String>
    where
        F: FnOnce(&mut Task) -> Result<(), String>,
    {
        let mut tasks = self.inner.lock().unwrap();
        let idx = tasks
            .iter()
            .position(|t| t.id == id)
            .ok_or_else(|| format!("unknown task '{id}'"))?;
        f(&mut tasks[idx])?;
        tasks[idx].updated_at = now_secs();
        let out = tasks[idx].clone();
        self.persist(&tasks)?;
        Ok(out)
    }

    /// Expire abandoned claims: any non-terminal task whose lease passed
    /// returns to the queue so a crashed peer cannot strand work.
    /// Returns the ids that were re-queued.
    pub fn expire_leases(&self) -> Vec<String> {
        let now = now_secs();
        let mut tasks = self.inner.lock().unwrap();
        let mut requeued = Vec::new();
        for t in tasks.iter_mut() {
            if t.is_terminal() || t.lease_until == 0 || t.lease_until > now {
                continue;
            }
            if t.state == "claimed" || t.state == "running" {
                t.state = "queued".to_string();
                t.assignee = String::new();
                t.claimed_at = 0;
                t.lease_until = 0;
                t.updated_at = now;
                requeued.push(t.id.clone());
            }
        }
        if !requeued.is_empty() {
            let _ = self.persist(&tasks);
        }
        requeued
    }

    /// Claim the oldest open task matching the caller's machine/tier/arch.
    /// `prefer` pins a specific task id when the caller already knows it.
    pub fn claim(
        &self,
        assignee: &str,
        machine: &str,
        tier: &str,
        arch: &str,
        prefer: Option<&str>,
    ) -> Result<Option<Task>, String> {
        let _ = self.expire_leases();
        let now = now_secs();
        let mut tasks = self.inner.lock().unwrap();
        let pick = tasks
            .iter()
            .filter(|t| t.state == "queued")
            .filter(|t| prefer.map(|p| t.id == p).unwrap_or(true))
            .filter(|t| target_matches(&t.target, machine, tier, arch))
            .min_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)))
            .map(|t| t.id.clone());
        let Some(id) = pick else {
            return Ok(None);
        };
        let idx = tasks.iter().position(|t| t.id == id).unwrap();
        let t = &mut tasks[idx];
        if !can_transition(&t.state, "claimed") {
            return Err(format!(
                "task {} is '{}' and cannot be claimed",
                t.id, t.state
            ));
        }
        t.state = "claimed".to_string();
        t.assignee = assignee.to_string();
        t.claimed_at = now;
        t.lease_until = now + CLAIM_TTL_SECS;
        t.attempts = t.attempts.saturating_add(1);
        t.updated_at = now;
        let out = t.clone();
        self.persist(&tasks)?;
        Ok(Some(out))
    }

    /// Move to `running` (heartbeat: also extends the lease).
    pub fn start(&self, id: &str, assignee: &str) -> Result<Task, String> {
        self.mutate(id, |t| {
            if t.assignee != assignee {
                return Err(format!(
                    "task {id} is claimed by '{}', not '{assignee}'",
                    t.assignee
                ));
            }
            if !can_transition(&t.state, "running") {
                return Err(format!("task {id} is '{}'; cannot start", t.state));
            }
            t.state = "running".to_string();
            t.lease_until = now_secs() + CLAIM_TTL_SECS;
            Ok(())
        })
    }

    /// Finish a task. `ok` selects `done` vs `failed`.
    pub fn finish(
        &self,
        id: &str,
        assignee: &str,
        ok: bool,
        result: &str,
        error: &str,
    ) -> Result<Task, String> {
        let to = if ok { "done" } else { "failed" };
        self.mutate(id, |t| {
            if !t.assignee.is_empty() && t.assignee != assignee {
                return Err(format!(
                    "task {id} is claimed by '{}', not '{assignee}'",
                    t.assignee
                ));
            }
            if !can_transition(&t.state, to) {
                return Err(format!("task {id} is '{}'; cannot finish", t.state));
            }
            t.state = to.to_string();
            t.result = clamp(result.trim(), MAX_RESULT_CHARS);
            t.error = clamp(error.trim(), MAX_ERROR_CHARS);
            t.finished_at = now_secs();
            t.lease_until = 0;
            if t.assignee.is_empty() {
                t.assignee = assignee.to_string();
            }
            Ok(())
        })
    }

    /// Record a blocker without releasing the claim (a human/peer decision
    /// is pending). The lease still runs, so a dead claimer re-queues.
    pub fn block(&self, id: &str, assignee: &str, reason: &str) -> Result<Task, String> {
        self.mutate(id, |t| {
            if !t.assignee.is_empty() && t.assignee != assignee {
                return Err(format!(
                    "task {id} is claimed by '{}', not '{assignee}'",
                    t.assignee
                ));
            }
            if !can_transition(&t.state, "blocked") {
                return Err(format!("task {id} is '{}'; cannot block", t.state));
            }
            t.state = "blocked".to_string();
            t.error = clamp(reason.trim(), MAX_ERROR_CHARS);
            Ok(())
        })
    }

    /// Cancel. Only the dispatcher (or the hub operator, who already holds
    /// the capability) may do this, so the caller passes the device.
    pub fn cancel(&self, id: &str, device: &str) -> Result<Task, String> {
        self.mutate(id, |t| {
            if t.is_terminal() {
                return Err(format!("task {id} is already '{}'", t.state));
            }
            if !t.origin_device.is_empty() && t.origin_device != device {
                return Err(format!(
                    "task {id} was dispatched by '{}'; only the dispatcher may cancel it",
                    t.origin_device
                ));
            }
            t.state = "cancelled".to_string();
            t.finished_at = now_secs();
            t.lease_until = 0;
            Ok(())
        })
    }

    /// Record the runner that actually started this task, and its handle.
    /// Called once the agent is launched; the task stays `claimed`/`running`.
    pub fn set_run(
        &self,
        id: &str,
        runner: &str,
        handle: &str,
    ) -> Result<Task, String> {
        self.mutate(id, |t| {
            if !runner.is_empty() {
                crate::runner::valid_runner(runner)?;
                t.runner = runner.to_ascii_lowercase();
            }
            if !handle.is_empty() {
                t.run_handle = clamp(handle, 200);
            }
            Ok(())
        })
    }

    /// Attach a federated chat session to an existing task.
    pub fn bind_session(&self, id: &str, session: &str) -> Result<Task, String> {
        self.mutate(id, |t| {
            if !t.session.is_empty() && t.session != session {
                return Err(format!(
                    "task {id} is already bound to chat {}",
                    short_id(&t.session)
                ));
            }
            t.session = session.to_string();
            Ok(())
        })
    }

    /// Board projection for the dashboard and `wtf tasks`.
    pub fn to_state_json(&self) -> Value {
        let tasks = self.list();
        let mut counts = vec![
            ("queued", 0i64),
            ("claimed", 0),
            ("running", 0),
            ("done", 0),
            ("failed", 0),
            ("blocked", 0),
            ("cancelled", 0),
        ];
        for t in &tasks {
            if let Some(c) = counts.iter_mut().find(|(k, _)| *k == t.state.as_str()) {
                c.1 += 1;
            }
        }
        Value::obj(vec![
            (
                "counts",
                Value::obj(
                    counts
                        .iter()
                        .map(|(k, v)| (*k, Value::from(*v)))
                        .collect(),
                ),
            ),
            ("total", Value::from(tasks.len() as i64)),
            (
                "tasks",
                Value::Arr(tasks.iter().take(200).map(|t| t.to_json()).collect()),
            ),
        ])
    }

    /// Text board for MCP/CLI.
    pub fn format_board(&self, limit: usize) -> String {
        let tasks = self.list();
        if tasks.is_empty() {
            return "no federated tasks yet — dispatch one with task_dispatch {title, target}".into();
        }
        let open = tasks.iter().filter(|t| !t.is_terminal()).count();
        let mut out = format!(
            "federated tasks: {} total · {} open\n",
            tasks.len(),
            open
        );
        for t in tasks.iter().take(limit) {
            out.push_str("  ");
            out.push_str(&t.to_line());
            out.push('\n');
        }
        if tasks.len() > limit {
            out.push_str(&format!("  … {} more\n", tasks.len() - limit));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(tag: &str) -> (TaskStore, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "wtf-task-{tag}-{}-{}",
            std::process::id(),
            crate::rand::hex(4)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tasks.json");
        (TaskStore::load_at(&path), path)
    }

    #[test]
    fn target_matching_covers_every_selector_shape() {
        // Cluster-wide routing.
        assert!(target_matches("any", "mac", "heavy", "darwin-arm64"));
        assert!(target_matches("", "mac", "heavy", "darwin-arm64"));
        assert!(target_matches("*", "mac", "heavy", "darwin-arm64"));
        // Exact machine, case-insensitive.
        assert!(target_matches("mac", "Mac", "heavy", "darwin-arm64"));
        assert!(!target_matches("mac", "windows", "heavy", "windows-x86_64"));
        // Capability classes.
        assert!(target_matches("tier:heavy", "anything", "HEAVY", "linux-x86_64"));
        assert!(!target_matches("tier:edge", "anything", "heavy", "linux-x86_64"));
        assert!(target_matches("arch:darwin-arm64", "x", "", "darwin-arm64"));
        assert!(!target_matches("arch:darwin-arm64", "x", "", "linux-arm64"));
        // An empty machine never matches a named selector.
        assert!(!target_matches("mac", "", "", ""));
    }

    #[test]
    fn target_validation_fails_closed_on_typos() {
        assert!(valid_target("any").is_ok());
        assert!(valid_target("mac").is_ok());
        assert!(valid_target("tier:heavy").is_ok());
        assert!(valid_target("tier:HEAVY").is_ok());
        assert!(valid_target("arch:darwin-arm64").is_ok());
        // A misspelled prefix must not degrade into "match everything".
        assert!(valid_target("teir:heavy").is_err());
        assert!(valid_target("tier:huge").is_err());
        assert!(valid_target("arch:").is_err());
        assert!(valid_target(&"x".repeat(MAX_TARGET_CHARS + 1)).is_err());
    }

    #[test]
    fn lifecycle_transitions_are_explicit() {
        assert!(can_transition("", "queued"));
        assert!(can_transition("queued", "claimed"));
        assert!(can_transition("claimed", "running"));
        assert!(can_transition("claimed", "done")); // short tasks
        assert!(can_transition("running", "failed"));
        assert!(can_transition("running", "queued")); // lease return
        assert!(can_transition("done", "cancelled"));
        // Illegal: a finished task may not be re-claimed or restarted.
        assert!(!can_transition("done", "claimed"));
        assert!(!can_transition("cancelled", "running"));
        assert!(!can_transition("queued", "running")); // must claim first
    }

    #[test]
    fn dispatch_claim_and_complete_roundtrip() {
        let (store, _path) = temp_store("roundtrip");
        let t = store
            .create("scan repo", "list branches", "proj", "mac", "box1", "agent-a", "", "auto")
            .unwrap();
        assert_eq!(t.state, "queued");
        assert_eq!(t.target, "mac");

        // Wrong machine cannot claim it.
        assert!(store.claim("box2", "windows", "heavy", "windows-x86_64", None)
            .unwrap()
            .is_none());

        let claimed = store
            .claim("box2", "mac", "heavy", "darwin-arm64", None)
            .unwrap()
            .expect("mac should claim its own work");
        assert_eq!(claimed.state, "claimed");
        assert_eq!(claimed.assignee, "box2");
        assert_eq!(claimed.attempts, 1);
        assert!(claimed.lease_until > claimed.claimed_at);

        let running = store.start(&claimed.id, "box2").unwrap();
        assert_eq!(running.state, "running");

        let done = store.finish(&claimed.id, "box2", true, "3 branches", "").unwrap();
        assert_eq!(done.state, "done");
        assert_eq!(done.result, "3 branches");
        assert!(done.is_terminal());
        // No further claims on a finished task.
        assert!(store.claim("box3", "mac", "heavy", "darwin-arm64", None)
            .unwrap()
            .is_none());
    }

    #[test]
    fn a_foreign_assignee_cannot_finish_someone_elses_task() {
        let (store, _p) = temp_store("foreign");
        let t = store.create("t", "", "", "any", "box1", "a", "", "auto").unwrap();
        store.claim("box2", "mac", "", "", None).unwrap().unwrap();
        let err = store.finish(&t.id, "box3", true, "x", "").unwrap_err();
        assert!(err.contains("claimed by 'box2'"));
    }

    #[test]
    fn lease_expiry_requeues_abandoned_work() {
        let (store, _p) = temp_store("lease");
        let t = store.create("t", "", "", "any", "box1", "a", "", "auto").unwrap();
        store.claim("box2", "mac", "", "", None).unwrap().unwrap();
        // Force the lease into the past, as a crashed peer would.
        {
            let mut inner = store.inner.lock().unwrap();
            let rec = inner.iter_mut().find(|x| x.id == t.id).unwrap();
            rec.lease_until = now_secs() - 1;
        }
        let requeued = store.expire_leases();
        assert_eq!(requeued, vec![t.id.clone()]);
        let after = store.get(&t.id).unwrap();
        assert_eq!(after.state, "queued");
        assert!(after.assignee.is_empty());
        // A different machine can now pick it up.
        assert!(store.claim("box3", "windows", "", "", None).unwrap().is_some());
    }

    #[test]
    fn terminal_tasks_cannot_be_cancelled_twice() {
        let (store, _p) = temp_store("cancel");
        let t = store.create("t", "", "", "any", "box1", "a", "", "auto").unwrap();
        let c = store.cancel(&t.id, "box1").unwrap();
        assert_eq!(c.state, "cancelled");
        assert!(store.cancel(&t.id, "box1").is_err());
    }

    #[test]
    fn only_the_dispatcher_may_cancel() {
        let (store, _p) = temp_store("cancel-owner");
        let t = store.create("t", "", "", "any", "box1", "a", "", "auto").unwrap();
        assert!(store.cancel(&t.id, "box9").is_err());
        assert!(store.cancel(&t.id, "box1").is_ok());
    }

    #[test]
    fn state_survives_a_reload() {
        let (store, path) = temp_store("persist");
        let t = store.create("durable", "d", "repo", "tier:edge", "box1", "a", "", "auto").unwrap();
        drop(store);
        let reloaded = TaskStore::load_at(&path);
        let got = reloaded.get(&t.id).expect("task replayed from disk");
        assert_eq!(got.title, "durable");
        assert_eq!(got.target, "tier:edge");
    }

    #[test]
    fn caps_reject_oversize_payloads() {
        let (store, _p) = temp_store("caps");
        assert!(store.create("", "", "", "any", "d", "a", "", "auto").is_err());
        assert!(store
            .create(&"x".repeat(MAX_TITLE_CHARS + 1), "", "", "any", "d", "a", "", "auto")
            .is_err());
        assert!(store
            .create("t", &"x".repeat(MAX_DETAIL_CHARS + 1), "", "any", "d", "a", "", "auto")
            .is_err());
    }

    #[test]
    fn board_renders_counts_and_lines() {
        let (store, _p) = temp_store("board");
        assert!(store.format_board(10).contains("no federated tasks"));
        let t = store.create("deploy", "", "proj", "mac", "box1", "a", "sess123456", "orca").unwrap();
        assert_eq!(t.runner, "orca", "runner choice is stored");
        assert_eq!(t.run_handle, "", "no handle until launched");
        store.claim("box2", "mac", "", "", None).unwrap().unwrap();
        let board = store.format_board(10);
        assert!(board.contains("1 total"));
        assert!(board.contains(&t.id));
        assert!(board.contains("~chat:sess1234"));
        let json = store.to_state_json();
        assert_eq!(json.get("total").and_then(|v| v.as_i64()), Some(1));
    }
}
