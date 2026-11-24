//! Stateless local & SSH host inspection.
//!
//! Never starts daemons, never invokes a shell with peer/file data, and never
//! touches private keys. SSH probes build explicit argv arrays with
//! `BatchMode=yes` and bounded wall-clock timeouts.

use crate::config::valid_name;
use crate::fleet::{self, valid_host, SkillSight, SshAlias};
use crate::json::Value;
use crate::util::now_millis;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

const SSH_CONNECT_TIMEOUT_SECS: u64 = 5;
const SSH_WALL_TIMEOUT: Duration = Duration::from_secs(8);
const LOCAL_CMD_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Debug, PartialEq)]
pub struct RepoStatus {
    pub path: String,
    pub branch: String,
    pub dirty: usize,
}

impl RepoStatus {
    pub fn to_json(&self) -> Value {
        Value::obj(vec![
            ("path", Value::from(self.path.as_str())),
            ("branch", Value::from(self.branch.as_str())),
            ("dirty", Value::from(self.dirty as i64)),
        ])
    }
}

#[derive(Clone, Debug)]
pub struct LocalReport {
    pub hostname: String,
    pub os: &'static str,
    pub arch: &'static str,
    pub skills: Vec<SkillSight>,
    pub repo: Option<RepoStatus>,
    pub herdr_agents: Option<String>,
}

impl LocalReport {
    pub fn to_json(&self) -> Value {
        let skills_v: Vec<Value> = self.skills.iter().map(SkillSight::to_json).collect();
        let repo_v = match &self.repo {
            Some(r) => r.to_json(),
            None => Value::Null,
        };
        let herdr_v = match &self.herdr_agents {
            Some(s) => Value::from(s.as_str()),
            None => Value::Null,
        };
        Value::obj(vec![
            ("hostname", Value::from(self.hostname.as_str())),
            ("os", Value::from(self.os)),
            ("arch", Value::from(self.arch)),
            ("skills", Value::Arr(skills_v)),
            ("repo", repo_v),
            ("herdr_agents", herdr_v),
        ])
    }
}

#[derive(Clone, Debug)]
pub struct RemoteReport {
    pub alias: String,
    pub user: String,
    pub host: String,
    pub port: u16,
    pub reachable: bool,
    pub remote_hostname: Option<String>,
    pub latency_ms: u64,
    pub error: Option<String>,
}

impl RemoteReport {
    pub fn to_json(&self) -> Value {
        let hn = match &self.remote_hostname {
            Some(h) => Value::from(h.as_str()),
            None => Value::Null,
        };
        let err = match &self.error {
            Some(e) => Value::from(e.as_str()),
            None => Value::Null,
        };
        Value::obj(vec![
            ("alias", Value::from(self.alias.as_str())),
            ("user", Value::from(self.user.as_str())),
            ("host", Value::from(self.host.as_str())),
            ("port", Value::from(self.port as i64)),
            ("reachable", Value::from(self.reachable)),
            ("remote_hostname", hn),
            ("latency_ms", Value::from(self.latency_ms as i64)),
            ("error", err),
        ])
    }
}

/// Execute `cmd` with piped stdout/stderr under a wall-clock timeout. Kills and
/// reaps the child if it exceeds `timeout`.
pub fn run_with_timeout(cmd: &mut Command, timeout: Duration) -> Result<Output, String> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("spawn failed: {e}"))?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                return child
                    .wait_with_output()
                    .map_err(|e| format!("wait_with_output failed: {e}"));
            }
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("timed out after {}s", timeout.as_secs()));
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("try_wait failed: {e}"));
            }
        }
    }
}

/// Parse `git status --porcelain=v1 -b` output into branch name and dirty count.
pub fn parse_git_status_porcelain(path: &Path, text: &str) -> Option<RepoStatus> {
    let mut lines = text.lines();
    let header = lines.next()?;
    let branch_part = header.strip_prefix("## ")?;
    let branch = if let Some(rest) = branch_part.strip_prefix("No commits yet on ") {
        rest.trim().to_string()
    } else if let Some(rest) = branch_part.strip_prefix("Initial commit on ") {
        rest.trim().to_string()
    } else if let Some((head, _tracking)) = branch_part.split_once("...") {
        head.trim().to_string()
    } else {
        branch_part.trim().to_string()
    };
    let dirty = lines.filter(|l| !l.trim().is_empty()).count();
    Some(RepoStatus {
        path: path.display().to_string(),
        branch,
        dirty,
    })
}

fn local_hostname() -> String {
    let mut cmd = Command::new("hostname");
    if let Ok(out) = run_with_timeout(&mut cmd, LOCAL_CMD_TIMEOUT) {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return s;
            }
        }
    }
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

fn probe_git_repo(cwd: &Path) -> Option<RepoStatus> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(cwd).args(["status", "--porcelain=v1", "-b"]);
    let out = run_with_timeout(&mut cmd, LOCAL_CMD_TIMEOUT).ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    parse_git_status_porcelain(cwd, &text)
}

fn probe_herdr_agents() -> Option<String> {
    let mut cmd = Command::new("herdr");
    cmd.args(["agent", "list"]);
    let out = run_with_timeout(&mut cmd, LOCAL_CMD_TIMEOUT).ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

pub fn probe_local(cwd: &Path) -> LocalReport {
    let skills = fleet::probe_skills("local");
    let has_herdr = skills.iter().any(|s| s.name == "herdr" && s.present);
    let herdr_agents = if has_herdr { probe_herdr_agents() } else { None };
    LocalReport {
        hostname: local_hostname(),
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        skills,
        repo: probe_git_repo(cwd),
        herdr_agents,
    }
}

/// Build the exact `ssh` argv array for a batch-mode reachability + hostname probe.
pub fn build_ssh_argv(alias: &SshAlias, connect_timeout_secs: u64) -> Vec<String> {
    vec![
        "-o".to_string(),
        "BatchMode=yes".to_string(),
        "-o".to_string(),
        format!("ConnectTimeout={connect_timeout_secs}"),
        "-p".to_string(),
        alias.port.to_string(),
        format!("{}@{}", alias.user, alias.host),
        "--".to_string(),
        "hostname".to_string(),
    ]
}

pub fn probe_remote(alias: &SshAlias) -> RemoteReport {
    if !valid_name(&alias.user) || !valid_host(&alias.host) || alias.port == 0 {
        return RemoteReport {
            alias: alias.alias.clone(),
            user: alias.user.clone(),
            host: alias.host.clone(),
            port: alias.port,
            reachable: false,
            remote_hostname: None,
            latency_ms: 0,
            error: Some("invalid alias user, host, or port".to_string()),
        };
    }
    let started = now_millis();
    let argv = build_ssh_argv(alias, SSH_CONNECT_TIMEOUT_SECS);
    let mut cmd = Command::new("ssh");
    cmd.args(&argv);
    match run_with_timeout(&mut cmd, SSH_WALL_TIMEOUT) {
        Ok(out) if out.status.success() => {
            let latency_ms = now_millis().saturating_sub(started);
            let hn = String::from_utf8_lossy(&out.stdout).trim().to_string();
            RemoteReport {
                alias: alias.alias.clone(),
                user: alias.user.clone(),
                host: alias.host.clone(),
                port: alias.port,
                reachable: true,
                remote_hostname: if hn.is_empty() { None } else { Some(hn) },
                latency_ms,
                error: None,
            }
        }
        Ok(out) => {
            let latency_ms = now_millis().saturating_sub(started);
            let err_line = String::from_utf8_lossy(&out.stderr)
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .map(|l| l.trim().to_string())
                .unwrap_or_else(|| format!("exit {:?}", out.status.code()));
            RemoteReport {
                alias: alias.alias.clone(),
                user: alias.user.clone(),
                host: alias.host.clone(),
                port: alias.port,
                reachable: false,
                remote_hostname: None,
                latency_ms,
                error: Some(err_line),
            }
        }
        Err(e) => RemoteReport {
            alias: alias.alias.clone(),
            user: alias.user.clone(),
            host: alias.host.clone(),
            port: alias.port,
            reachable: false,
            remote_hostname: None,
            latency_ms: now_millis().saturating_sub(started),
            error: Some(e),
        },
    }
}

pub fn probe_all_remotes(aliases: &[SshAlias]) -> Vec<RemoteReport> {
    if aliases.is_empty() {
        return Vec::new();
    }
    std::thread::scope(|scope| {
        let handles: Vec<_> = aliases
            .iter()
            .map(|a| scope.spawn(move || probe_remote(a)))
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("probe thread panicked"))
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fleet::forbidden_keys;
    use std::path::PathBuf;

    #[test]
    fn ssh_argv_has_exact_flags() {
        let alias = SshAlias {
            alias: "wsl".into(),
            user: "nanoboozhoo".into(),
            host: "peopleofthedawn.local".into(),
            port: 2222,
        };
        assert_eq!(
            build_ssh_argv(&alias, 5),
            vec![
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=5",
                "-p",
                "2222",
                "nanoboozhoo@peopleofthedawn.local",
                "--",
                "hostname",
            ]
        );
    }

    #[test]
    fn parse_git_status_variants() {
        let p = PathBuf::from("/tmp/repo");
        let clean = parse_git_status_porcelain(&p, "## main...origin/main\n").unwrap();
        assert_eq!(clean.branch, "main");
        assert_eq!(clean.dirty, 0);

        let dirty = parse_git_status_porcelain(
            &p,
            "## feat/cli...origin/feat/cli [ahead 1]\n M src/main.rs\n?? src/probe.rs\n",
        )
        .unwrap();
        assert_eq!(dirty.branch, "feat/cli");
        assert_eq!(dirty.dirty, 2);

        let unborn = parse_git_status_porcelain(&p, "## No commits yet on main\n?? a\n").unwrap();
        assert_eq!(unborn.branch, "main");
        assert_eq!(unborn.dirty, 1);

        let detached = parse_git_status_porcelain(&p, "## HEAD (no branch)\n").unwrap();
        assert_eq!(detached.branch, "HEAD (no branch)");
        assert_eq!(detached.dirty, 0);
    }

    #[test]
    fn reports_are_keyless() {
        let local = LocalReport {
            hostname: "in-DIGI-nous".into(),
            os: "macos",
            arch: "aarch64",
            skills: vec![SkillSight {
                device: "local".into(),
                name: "herdr".into(),
                present: true,
            }],
            repo: Some(RepoStatus {
                path: "/tmp/repo".into(),
                branch: "main".into(),
                dirty: 0,
            }),
            herdr_agents: None,
        };
        let remote = RemoteReport {
            alias: "windows".into(),
            user: "kenwa".into(),
            host: "peopleofthedawn.local".into(),
            port: 22,
            reachable: true,
            remote_hostname: Some("PEOPLEOFTHEDAWN".into()),
            latency_ms: 42,
            error: None,
        };
        assert!(forbidden_keys(&local.to_json()).is_empty());
        assert!(forbidden_keys(&remote.to_json()).is_empty());
    }
}
