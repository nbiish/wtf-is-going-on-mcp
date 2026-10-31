//! Live source adapters: shell out to the `orca` and `herdr` CLIs, normalize
//! their JSON into one session model. Failures degrade to a source status —
//! the page never breaks because one CLI is missing or a machine is asleep.

use crate::exec;
use crate::json::{self, Value};
use crate::model::{Machine, Session, SourceStatus};
use crate::util::clean_text;
use std::time::{Duration, Instant};

const ORCA_TIMEOUT: Duration = Duration::from_secs(6);
const HERDR_TIMEOUT: Duration = Duration::from_secs(4);
const REMOTE_TIMEOUT: Duration = Duration::from_secs(4);

pub struct Sources {
    pub orca: Option<String>,
    pub herdr: Option<String>,
    pub machine: String,
}

impl Sources {
    pub fn detect() -> Sources {
        Sources {
            orca: exec::resolve(
                "ORCA_BIN",
                &["/Applications/Orca.app/Contents/Resources/bin/orca", "orca"],
            ),
            herdr: exec::resolve(
                "HERDR_BIN",
                &[
                    "herdr",
                    "/opt/homebrew/bin/herdr",
                    "/usr/local/bin/herdr",
                    "/opt/homebrew/Cellar/herdr",
                ],
            ),
            machine: local_machine_name(),
        }
    }

    pub fn collect(&self) -> (Vec<Machine>, Vec<Session>, Vec<SourceStatus>) {
        let mut machines = Vec::new();
        let mut sessions = Vec::new();
        let mut status = Vec::new();

        self.collect_orca(&mut machines, &mut sessions, &mut status);
        self.collect_herdr(&mut machines, &mut sessions, &mut status);

        // Both sources describe the local machine; collapse to one row.
        let mut merged: Vec<Machine> = Vec::new();
        for m in machines.drain(..) {
            if let Some(existing) = merged.iter_mut().find(|x| x.name == m.name) {
                existing.available |= m.available;
                if existing.detail != m.detail {
                    existing.detail = format!("{} · {}", existing.detail, m.detail);
                }
            } else {
                merged.push(m);
            }
        }
        machines = merged;

        sessions.sort_by(|a, b| {
            b.busy()
                .cmp(&a.busy())
                .then(b.updated_at.cmp(&a.updated_at))
                .then(a.machine.cmp(&b.machine))
        });
        (machines, sessions, status)
    }

    // ---------------------------------------------------------------- orca

    fn collect_orca(
        &self,
        machines: &mut Vec<Machine>,
        sessions: &mut Vec<Session>,
        status: &mut Vec<SourceStatus>,
    ) {
        let Some(bin) = self.orca.as_deref() else {
            status.push(SourceStatus {
                name: "orca",
                ok: false,
                detail: "orca CLI not found".into(),
                ms: 0,
            });
            return;
        };
        let started = Instant::now();
        let out = exec::run(bin, &["worktree", "ps", "--json"], ORCA_TIMEOUT);
        match out {
            Ok(text) => {
                let value = match json::parse(&text) {
                    Ok(v) => v,
                    Err(e) => {
                        status.push(SourceStatus {
                            name: "orca",
                            ok: false,
                            detail: format!("bad JSON: {e}"),
                            ms: started.elapsed().as_millis(),
                        });
                        return;
                    }
                };
                let previews = self.orca_terminal_previews(bin);
                let worktrees = nested(&value, "result", "worktrees").unwrap_or(&[]);
                for wt in worktrees {
                    parse_orca_worktree(wt, &self.machine, &previews, sessions);
                }
                machines.push(Machine {
                    name: self.machine.clone(),
                    kind: "local".into(),
                    detail: format!("orca {} · {} worktrees", "runtime", worktrees.len()),
                    available: true,
                });
                status.push(SourceStatus {
                    name: "orca",
                    ok: true,
                    detail: format!("{} worktrees", worktrees.len()),
                    ms: started.elapsed().as_millis(),
                });
            }
            Err(e) => {
                machines.push(Machine {
                    name: self.machine.clone(),
                    kind: "local".into(),
                    detail: "orca runtime unreachable".into(),
                    available: false,
                });
                status.push(SourceStatus {
                    name: "orca",
                    ok: false,
                    detail: e,
                    ms: started.elapsed().as_millis(),
                });
            }
        }
        self.collect_orca_environments(bin, machines, sessions);
    }

    /// worktree id -> best terminal preview, for agents without a message.
    fn orca_terminal_previews(&self, bin: &str) -> Vec<(String, String)> {
        let Ok(text) = exec::run(bin, &["terminal", "list", "--json"], ORCA_TIMEOUT) else {
            return Vec::new();
        };
        let Ok(value) = json::parse(&text) else {
            return Vec::new();
        };
        let Some(terms) = nested(&value, "result", "terminals") else {
            return Vec::new();
        };
        terms
            .iter()
            .filter_map(|t| {
                let wt = s(t, "worktreeId");
                let preview = s(t, "preview");
                (!wt.is_empty() && !preview.is_empty()).then_some((wt, preview))
            })
            .collect()
    }

    fn collect_orca_environments(
        &self,
        bin: &str,
        machines: &mut Vec<Machine>,
        sessions: &mut Vec<Session>,
    ) {
        let Ok(text) = exec::run(bin, &["environment", "list", "--json"], ORCA_TIMEOUT) else {
            return;
        };
        let Ok(value) = json::parse(&text) else {
            return;
        };
        let Some(envs) = nested(&value, "result", "environments") else {
            return;
        };
        for env in envs.iter().take(3) {
            let id = s(env, "id");
            let name = nonempty(s(env, "name")).unwrap_or_else(|| id.clone());
            if id.is_empty() {
                continue;
            }
            let out = exec::run(
                bin,
                &["worktree", "ps", "--environment", &id, "--json"],
                REMOTE_TIMEOUT,
            );
            match out {
                Ok(text) => {
                    let Ok(value) = json::parse(&text) else {
                        continue;
                    };
                    let Some(wts) = nested(&value, "result", "worktrees") else {
                        continue;
                    };
                    for wt in wts {
                        parse_orca_worktree(wt, &name, &[], sessions);
                    }
                    machines.push(Machine {
                        name: name.clone(),
                        kind: "paired".into(),
                        detail: format!("orca server · {} worktrees", wts.len()),
                        available: true,
                    });
                }
                Err(e) => machines.push(Machine {
                    name,
                    kind: "paired".into(),
                    detail: e,
                    available: false,
                }),
            }
        }
    }

    // --------------------------------------------------------------- herdr

    fn collect_herdr(
        &self,
        machines: &mut Vec<Machine>,
        sessions: &mut Vec<Session>,
        status: &mut Vec<SourceStatus>,
    ) {
        let Some(bin) = self.herdr.as_deref() else {
            status.push(SourceStatus {
                name: "herdr",
                ok: false,
                detail: "herdr CLI not found (brew install herdr)".into(),
                ms: 0,
            });
            return;
        };
        let started = Instant::now();
        let running = exec::run(bin, &["status", "--json"], HERDR_TIMEOUT)
            .ok()
            .and_then(|t| json::parse(&t).ok())
            .and_then(|v| {
                v.get("server")
                    .and_then(|s| s.get("running"))
                    .and_then(|b| b.as_bool())
            })
            .unwrap_or(false);
        if !running {
            machines.push(Machine {
                name: self.machine.clone(),
                kind: "local".into(),
                detail: "herdr server not running (run `herdr`)".into(),
                available: false,
            });
            status.push(SourceStatus {
                name: "herdr",
                ok: false,
                detail: "server not running — launch `herdr` or `herdr server`".into(),
                ms: started.elapsed().as_millis(),
            });
            self.collect_herdr_remotes(bin, machines, sessions);
            return;
        }

        let workspaces = herdr_envelope(bin, &["workspace", "list"]).unwrap_or(Value::Null);
        let panes = herdr_envelope(bin, &["pane", "list"]).unwrap_or(Value::Null);
        let agents = herdr_envelope(bin, &["agent", "list"]).unwrap_or(Value::Null);

        let pane_rows = nested(&panes, "result", "panes").unwrap_or(&[]);
        let ws_rows = nested(&workspaces, "result", "workspaces").unwrap_or(&[]);
        let agent_rows = nested(&agents, "result", "agents").unwrap_or(&[]);

        let mut used_panes: Vec<String> = Vec::new();
        for (i, a) in agent_rows.iter().enumerate() {
            let pane_id = s(a, "pane_id");
            let pane = pane_rows.iter().find(|p| s(p, "pane_id") == pane_id);
            let ws = pane.and_then(|p| {
                ws_rows
                    .iter()
                    .find(|w| s(w, "workspace_id") == s(p, "workspace_id"))
            });
            let title = ws
                .map(|w| s(w, "label"))
                .filter(|l| !l.is_empty())
                .or_else(|| pane.map(|p| s(p, "terminal_title")))
                .unwrap_or_else(|| format!("pane {i}"));
            let detail = if pane_id.is_empty() {
                String::new()
            } else {
                used_panes.push(pane_id.clone());
                self.pane_preview(bin, &pane_id)
            };
            sessions.push(Session {
                id: format!(
                    "herdr:{}",
                    nonempty(s(a, "name")).unwrap_or_else(|| format!("{pane_id}:{i}"))
                ),
                machine: self.machine.clone(),
                source: "herdr",
                agent: nonempty(s(a, "kind")).unwrap_or_else(|| "agent".into()),
                state: normalize_state(&s(a, "state")),
                title,
                cwd: pane.map(|p| s(p, "cwd")).unwrap_or_default(),
                detail,
                handle: pane_id,
                updated_at: now_or_zero(a),
                live: true,
            });
        }

        // Panes with no recognized agent still count as live work.
        for pane in pane_rows {
            let pane_id = s(pane, "pane_id");
            if pane_id.is_empty() || used_panes.iter().any(|p| *p == pane_id) {
                continue;
            }
            let ws = ws_rows
                .iter()
                .find(|w| s(w, "workspace_id") == s(pane, "workspace_id"));
            let title = ws
                .map(|w| s(w, "label"))
                .filter(|l| !l.is_empty())
                .unwrap_or_else(|| s(pane, "terminal_title"));
            sessions.push(Session {
                id: format!("herdr:{pane_id}"),
                machine: self.machine.clone(),
                source: "herdr",
                agent: "shell".into(),
                state: "shell".into(),
                title: if title.is_empty() {
                    pane_id.clone()
                } else {
                    title
                },
                cwd: s(pane, "cwd"),
                detail: self.pane_preview(bin, &pane_id),
                handle: pane_id,
                updated_at: 0,
                live: true,
            });
        }

        machines.push(Machine {
            name: self.machine.clone(),
            kind: "local".into(),
            detail: format!(
                "herdr · {} panes · {} agents",
                pane_rows.len(),
                agent_rows.len()
            ),
            available: true,
        });
        status.push(SourceStatus {
            name: "herdr",
            ok: true,
            detail: format!("{} panes · {} agents", pane_rows.len(), agent_rows.len()),
            ms: started.elapsed().as_millis(),
        });
        self.collect_herdr_remotes(bin, machines, sessions);
    }

    fn pane_preview(&self, bin: &str, pane_id: &str) -> String {
        exec::run(
            bin,
            &[
                "pane",
                "read",
                pane_id,
                "--source",
                "recent-unwrapped",
                "--lines",
                "16",
            ],
            Duration::from_millis(1500),
        )
        .map(|t| clean_text(&t, 600))
        .unwrap_or_default()
    }

    fn collect_herdr_remotes(
        &self,
        bin: &str,
        machines: &mut Vec<Machine>,
        sessions: &mut Vec<Session>,
    ) {
        let Ok(text) = exec::run(bin, &["machine", "list", "--json"], HERDR_TIMEOUT) else {
            return;
        };
        let Ok(value) = json::parse(&text) else {
            return;
        };
        let rows = value.as_arr().unwrap_or(&[]);
        for m in rows.iter().take(3) {
            let label = nonempty(s(m, "label"))
                .or_else(|| nonempty(s(m, "id")))
                .unwrap_or_else(|| "remote".into());
            let enabled = m.get("enabled").and_then(|e| e.as_bool()).unwrap_or(true);
            if !enabled {
                continue;
            }
            let selector = nonempty(s(m, "id")).unwrap_or_else(|| label.clone());
            let agents = herdr_envelope(bin, &["--machine", &selector, "agent", "list"]);
            let panes = herdr_envelope(bin, &["--machine", &selector, "pane", "list"]);
            match (agents, panes) {
                (Ok(agents), Ok(panes)) => {
                    let agent_rows = nested(&agents, "result", "agents").unwrap_or(&[]);
                    let pane_rows = nested(&panes, "result", "panes").unwrap_or(&[]);
                    for (i, a) in agent_rows.iter().enumerate() {
                        let pane_id = s(a, "pane_id");
                        let pane = pane_rows.iter().find(|p| s(p, "pane_id") == pane_id);
                        sessions.push(Session {
                            id: format!("herdr:{label}:{i}"),
                            machine: label.clone(),
                            source: "herdr",
                            agent: nonempty(s(a, "kind")).unwrap_or_else(|| "agent".into()),
                            state: normalize_state(&s(a, "state")),
                            title: nonempty(s(a, "name")).unwrap_or_else(|| pane_id.clone()),
                            cwd: pane.map(|p| s(p, "cwd")).unwrap_or_default(),
                            detail: String::new(),
                            handle: pane_id,
                            updated_at: now_or_zero(a),
                            live: true,
                        });
                    }
                    for pane in pane_rows {
                        let pane_id = s(pane, "pane_id");
                        if agent_rows.iter().any(|a| s(a, "pane_id") == pane_id) {
                            continue;
                        }
                        sessions.push(Session {
                            id: format!("herdr:{label}:{pane_id}"),
                            machine: label.clone(),
                            source: "herdr",
                            agent: "shell".into(),
                            state: "shell".into(),
                            title: nonempty(s(pane, "terminal_title"))
                                .unwrap_or_else(|| pane_id.clone()),
                            cwd: s(pane, "cwd"),
                            detail: String::new(),
                            handle: pane_id,
                            updated_at: 0,
                            live: true,
                        });
                    }
                    machines.push(Machine {
                        name: label,
                        kind: "ssh".into(),
                        detail: format!("herdr over ssh · {} panes", pane_rows.len()),
                        available: true,
                    });
                }
                (Err(e), _) | (_, Err(e)) => machines.push(Machine {
                    name: label,
                    kind: "ssh".into(),
                    detail: e,
                    available: false,
                }),
            }
        }
    }
}

// ------------------------------------------------------------- parsing

fn parse_orca_worktree(
    wt: &Value,
    machine: &str,
    previews: &[(String, String)],
    sessions: &mut Vec<Session>,
) {
    let worktree_id = s(wt, "worktreeId");
    let repo = nonempty(s(wt, "repo")).unwrap_or_else(|| "workspace".into());
    let display = nonempty(s(wt, "displayName")).unwrap_or_else(|| repo.clone());
    let path = s(wt, "path");
    let branch = s(wt, "branch").replace("refs/heads/", "");
    let title = if display == branch || branch.is_empty() {
        repo.clone()
    } else {
        format!("{display} · {branch}")
    };
    let worktree_preview = {
        let p = s(wt, "preview");
        if p.is_empty() {
            previews
                .iter()
                .find(|(id, _)| *id == worktree_id)
                .map(|(_, p)| p.clone())
                .unwrap_or_default()
        } else {
            p
        }
    };
    let agents = arr(wt, "agents").unwrap_or(&[]);
    if !agents.is_empty() {
        for (i, a) in agents.iter().enumerate() {
            let state = nonempty(s(a, "state"))
                .or_else(|| {
                    a.get("mainAgent")
                        .and_then(|m| m.get("state"))
                        .and_then(|x| x.as_str())
                        .map(str::to_string)
                })
                .unwrap_or_else(|| "unknown".into());
            let detail =
                nonempty(s(a, "lastAssistantMessage")).unwrap_or_else(|| worktree_preview.clone());
            sessions.push(Session {
                id: format!(
                    "orca:{}:{}",
                    nonempty(s(a, "paneKey")).unwrap_or_else(|| worktree_id.clone()),
                    i
                ),
                machine: machine.to_string(),
                source: "orca",
                agent: nonempty(s(a, "agentType")).unwrap_or_else(|| "agent".into()),
                state: normalize_state(&state),
                title: title.clone(),
                cwd: path.clone(),
                detail: clean_text(&detail, 600),
                handle: nonempty(s(a, "paneKey")).unwrap_or_else(|| worktree_id.clone()),
                updated_at: now_or_zero(a),
                live: true,
            });
        }
        return;
    }
    let live_terminals = wt
        .get("liveTerminalCount")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    if live_terminals > 0 {
        sessions.push(Session {
            id: format!("orca:{worktree_id}"),
            machine: machine.to_string(),
            source: "orca",
            agent: "shell".into(),
            state: "shell".into(),
            title: title.clone(),
            cwd: path,
            detail: clean_text(&worktree_preview, 600),
            handle: worktree_id,
            updated_at: now_or_zero(wt),
            live: true,
        });
    }
}

fn herdr_envelope(bin: &str, args: &[&str]) -> Result<Value, String> {
    let text = exec::run(bin, args, HERDR_TIMEOUT)?;
    json::parse(&text).map_err(|e| format!("bad herdr JSON: {e}"))
}

fn arr<'a>(v: &'a Value, key: &str) -> Option<&'a [Value]> {
    v.get(key).and_then(|x| x.as_arr())
}

/// `v.outer.inner` as an array slice.
fn nested<'a>(v: &'a Value, outer: &str, inner: &str) -> Option<&'a [Value]> {
    v.get(outer)
        .and_then(|x| x.get(inner))
        .and_then(|x| x.as_arr())
}

fn s(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string()
}

fn nonempty(s: String) -> Option<String> {
    (!s.trim().is_empty()).then_some(s)
}

fn now_or_zero(v: &Value) -> u64 {
    for key in [
        "updatedAt",
        "lastActivityAt",
        "stateStartedAt",
        "lastOutputAt",
        "updated_at",
    ] {
        if let Some(n) = v.get(key).and_then(|x| x.as_i64()) {
            if n > 0 {
                return n as u64;
            }
        }
    }
    0
}

fn normalize_state(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "working" | "busy" | "running" | "active" => "working".into(),
        "blocked" | "needs-input" | "waiting" | "approval" => "blocked".into(),
        "done" | "complete" | "completed" => "done".into(),
        "idle" | "ready" => "idle".into(),
        "" => "unknown".into(),
        other => other.to_string(),
    }
}

fn local_machine_name() -> String {
    if let Ok(name) = std::env::var("WTF_LIVE_MACHINE") {
        if !name.trim().is_empty() {
            return name.trim().to_string();
        }
    }
    exec::run("hostname", &["-s"], Duration::from_secs(1))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "local".into())
}
