//! Remote agent runners: how a claimed task actually gets executed on the
//! machine that claimed it.
//!
//! A task record says *what* to do and *where*. This module decides *with
//! what*: it probes the target for an agent runtime and shells out to the
//! right one.
//!
//! Two runners, in preference order (operator directive 2026-11-07):
//!
//! 1. **Orca** (`orca`, `orca-ide`, `orca-dev`) — supervised orchestration.
//!    `orca orchestration worker-start --spec …` returns a dispatch id and
//!    Orca reports completion back through its own Run, which is what a
//!    dispatcher wants. A *remote* Orca needs the target paired as a server:
//!    `orca environment add --pairing-code orca://pair?...` on the caller
//!    once, then `--environment <name>` on every start.
//! 2. **Herdr** (`herdr`) — durable terminal multiplexer. Installed far more
//!    often than Orca, and it keeps agents alive across reconnects. Remote
//!    access is `herdr --machine <label>`, established once with
//!    `herdr machine add <ssh-target> --label <label>`.
//!
//! Selection is by *availability*, not by hope: `auto` probes in order and
//! reports which runner it picked and why. An explicit runner that is NOT
//! present fails closed with the reason and the install/pairing step — it
//! never silently downgrades to a different runtime, because "I ran your work
//! somewhere else" is worse than "I could not run it".
//!
//! Everything here is argv-array execution (`std::process::Command`), never a
//! shell string: task titles and specs are peer-supplied text and must not be
//! word-split by a shell. Zero external crates.

use crate::json::Value;
use std::process::Command;

/// Runners we know how to drive, in default preference order.
pub const RUNNERS: [&str; 2] = ["orca", "herdr"];

/// How long a probe may take before we treat the runtime as absent. A hung
/// `--version` must not stall a claim.
const PROBE_TIMEOUT_SECS: u64 = 6;

/// A resolved plan for one execution.
#[derive(Clone, Debug, PartialEq)]
pub struct RunnerPlan {
    /// Runner name: "orca" | "herdr".
    pub runner: String,
    /// The concrete executable that will be invoked.
    pub binary: String,
    /// Whether this runs on the local machine or a remote one.
    pub placement: String, // "local" | "remote"
    /// Remote machine label (herdr `--machine`) or Orca environment name.
    pub remote: String,
    /// Human-readable reason for this choice (surfaced to the caller).
    pub reason: String,
}

impl RunnerPlan {
    pub fn to_json(&self) -> Value {
        Value::obj(vec![
            ("runner", Value::from(self.runner.as_str())),
            ("binary", Value::from(self.binary.as_str())),
            ("placement", Value::from(self.placement.as_str())),
            ("remote", Value::from(self.remote.as_str())),
            ("reason", Value::from(self.reason.as_str())),
        ])
    }
}

/// Result of actually starting an agent.
#[derive(Clone, Debug)]
pub struct RunOutcome {
    pub ok: bool,
    pub runner: String,
    /// Runner-issued handle: Orca dispatch id, or herdr pane/agent id.
    pub handle: String,
    /// Whatever the runner printed, trimmed for display.
    pub detail: String,
}

impl RunOutcome {
    pub fn to_json(&self) -> Value {
        Value::obj(vec![
            ("ok", Value::from(self.ok)),
            ("runner", Value::from(self.runner.as_str())),
            ("handle", Value::from(self.handle.as_str())),
            ("detail", Value::from(self.detail.as_str())),
        ])
    }
}

/// Is `name` on PATH and executable?
pub fn which(name: &str) -> Option<String> {
    if name.is_empty() || name.contains('/') || name.contains('\\') {
        return None;
    }
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for candidate in path_candidates(&dir, name) {
            if candidate.is_file() {
                // Confirm it actually runs: a stale shim on PATH is not a runner.
                if probe_runs(&candidate.to_string_lossy()) {
                    return Some(candidate.to_string_lossy().to_string());
                }
            }
        }
    }
    None
}

/// Candidate file names for `name` inside `dir`.
///
/// Windows needs the PATHEXT suffixes: a bare `orca-ide` never exists there,
/// the launcher is `orca-ide.exe` (or `.cmd`/`.bat`).
#[cfg(windows)]
fn path_candidates(dir: &std::path::Path, name: &str) -> Vec<std::path::PathBuf> {
    let exts = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
    let mut out = vec![dir.join(name)];
    for ext in exts.split(';').filter(|e| !e.is_empty()) {
        out.push(dir.join(format!("{name}{}", ext.to_ascii_lowercase())));
    }
    out
}

#[cfg(not(windows))]
fn path_candidates(dir: &std::path::Path, name: &str) -> Vec<std::path::PathBuf> {
    vec![dir.join(name)]
}

/// Run `<binary> --version` with a hard cap; true only on a clean exit.
///
/// This is deliberately conservative: a binary that exists but cannot execute
/// (wrong arch, broken interpreter, a shim whose target is gone) is treated as
/// absent, because the alternative is dispatching work into a runtime that
/// cannot accept it.
fn probe_runs(binary: &str) -> bool {
    let mut cmd = Command::new(binary);
    cmd.arg("--version");
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    match run_with_timeout(cmd, PROBE_TIMEOUT_SECS) {
        Some(out) => out.status.success(),
        None => false,
    }
}

/// Run a command with a wall-clock cap. Returns None on timeout or spawn error.
///
/// std has no portable wait-with-timeout, so the child is polled. A hung
/// child is killed rather than leaked: a probe that never returns must not
/// hold a claim open.
fn run_with_timeout(mut cmd: Command, secs: u64) -> Option<std::process::Output> {
    use std::process::Stdio;
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().ok()?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return child.wait_with_output().ok(),
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

/// Canonical Orca CLI name for a binary path, for display.
fn orca_binary() -> Option<String> {
    // orca-ide on Linux (bare `orca` is the GNOME screen reader there),
    // orca-dev in an Orca dev checkout, orca on macOS/Windows.
    for name in ["orca-ide", "orca-dev", "orca"] {
        if let Some(p) = which(name) {
            return Some(p);
        }
    }
    None
}

/// Which runners are usable on this machine, with the reason for each.
pub fn probe() -> Vec<(String, bool, String)> {
    let mut out = Vec::new();
    match orca_binary() {
        Some(p) => out.push((
            "orca".to_string(),
            true,
            format!("available at {p}"),
        )),
        None => out.push((
            "orca".to_string(),
            false,
            "not installed (tried orca-ide, orca-dev, orca)".to_string(),
        )),
    }
    match which("herdr") {
        Some(p) => out.push((
            "herdr".to_string(),
            true,
            format!("available at {p}"),
        )),
        None => out.push((
            "herdr".to_string(),
            false,
            "not installed".to_string(),
        )),
    }
    out
}

/// Choose a runner.
///
/// `want` is the task's `runner` field: "" or "auto" probes in preference
/// order (Orca first); an explicit name is honored exactly and fails closed
/// when absent. `remote` is the target machine label as the runtime knows it
/// ("" = run locally).
pub fn choose(want: &str, remote: &str) -> Result<RunnerPlan, String> {
    let want = want.trim().to_ascii_lowercase();
    let remote = remote.trim();
    let placement = if remote.is_empty() { "local" } else { "remote" };

    let build = |runner: &str| -> Option<RunnerPlan> {
        let binary = match runner {
            "orca" => orca_binary()?,
            "herdr" => which("herdr")?,
            _ => return None,
        };
        Some(RunnerPlan {
            runner: runner.to_string(),
            binary,
            placement: placement.to_string(),
            remote: remote.to_string(),
            reason: String::new(),
        })
    };

    if want.is_empty() || want == "auto" {
        for runner in RUNNERS {
            if let Some(mut plan) = build(runner) {
                plan.reason = if runner == "orca" {
                    if placement == "remote" {
                        format!(
                            "orca is installed; starting a supervised worker on paired environment '{remote}'"
                        )
                    } else {
                        "orca is installed and preferred for supervised orchestration".to_string()
                    }
                } else {
                    format!(
                        "orca is not installed; falling back to herdr{}",
                        if placement == "remote" {
                            format!(" over ssh machine '{remote}'")
                        } else {
                            String::new()
                        }
                    )
                };
                return Ok(plan);
            }
        }
        return Err(
            "no agent runner on this machine: install Orca (or `orca serve` + pair it) or Herdr"
                .to_string(),
        );
    }

    if !RUNNERS.contains(&want.as_str()) {
        return Err(format!(
            "unknown runner '{want}'; use auto, orca, or herdr"
        ));
    }
    match build(&want) {
        Some(plan) => {
            let mut plan = plan;
            plan.reason = format!("runner '{want}' requested explicitly");
            Ok(plan)
        }
        None => Err(match want.as_str() {
            "orca" => {
                "runner 'orca' requested but not installed (tried orca-ide, orca-dev, orca)".to_string()
            }
            _ => format!("runner '{want}' requested but not installed"),
        }),
    }
}

/// Validate a task's `runner` field before it is stored.
pub fn valid_runner(runner: &str) -> Result<(), String> {
    let r = runner.trim().to_ascii_lowercase();
    if r.is_empty() || r == "auto" || RUNNERS.contains(&r.as_str()) {
        return Ok(());
    }
    Err(format!(
        "unknown runner '{runner}'; use auto, orca, or herdr"
    ))
}

/// Default Orca agent to launch when the caller does not name one.
/// `codex` is the id Orca's own guide uses for a supervised worker; Orca
/// refuses `worker-start` without an agent, so this must always be supplied.
pub const DEFAULT_ORCA_AGENT: &str = "codex";

/// Build the argv that starts a worker, without running it.
///
/// Split out from execution so the exact command line is unit-testable — the
/// shape of these invocations is the contract with Orca and Herdr, and a
/// silently wrong flag is the failure mode worth pinning in a test.
pub fn argv(plan: &RunnerPlan, spec: &str, workdir: &str) -> Vec<String> {
    argv_with_agent(plan, spec, workdir, DEFAULT_ORCA_AGENT)
}

/// `argv` with an explicit Orca agent id.
pub fn argv_with_agent(
    plan: &RunnerPlan,
    spec: &str,
    workdir: &str,
    agent: &str,
) -> Vec<String> {
    let mut argv: Vec<String> = Vec::new();
    match plan.runner.as_str() {
        "orca" => {
            argv.push(plan.binary.clone());
            argv.push("orchestration".to_string());
            argv.push("worker-start".to_string());
            argv.push("--spec".to_string());
            argv.push(spec.to_string());
            // worker-start refuses to create a terminal without an agent.
            argv.push("--agent".to_string());
            argv.push(if agent.trim().is_empty() {
                DEFAULT_ORCA_AGENT.to_string()
            } else {
                agent.trim().to_string()
            });
            // A remote Orca is addressed by its paired environment name; a
            // local one gets the worktree selector instead.
            if plan.placement == "remote" && !plan.remote.is_empty() {
                argv.push("--on".to_string());
                argv.push(plan.remote.clone());
                // Remote workers need an explicit repo + worktree mode.
                argv.push("--worktree".to_string());
                argv.push("new-top-level".to_string());
            } else if !workdir.is_empty() {
                argv.push("--worktree".to_string());
                argv.push(format!("path:{workdir}"));
            }
            argv.push("--json".to_string());
        }
        _ => {
            argv.push(plan.binary.clone());
            if plan.placement == "remote" && !plan.remote.is_empty() {
                argv.push("--machine".to_string());
                argv.push(plan.remote.clone());
            }
            argv.push("agent".to_string());
            argv.push("start".to_string());
            argv.push("--prompt".to_string());
            argv.push(spec.to_string());
            argv.push("--json".to_string());
        }
    }
    argv
}

/// Start an agent for `spec` using `plan`. Returns the runner's handle.
pub fn start(plan: &RunnerPlan, spec: &str, workdir: &str, timeout_secs: u64) -> RunOutcome {
    start_with_agent(plan, spec, workdir, timeout_secs, DEFAULT_ORCA_AGENT)
}

/// `start` with an explicit Orca agent id (ignored by Herdr).
pub fn start_with_agent(
    plan: &RunnerPlan,
    spec: &str,
    workdir: &str,
    timeout_secs: u64,
    agent: &str,
) -> RunOutcome {
    let args = argv_with_agent(plan, spec, workdir, agent);
    let Some((binary, rest)) = args.split_first() else {
        return RunOutcome {
            ok: false,
            runner: plan.runner.clone(),
            handle: String::new(),
            detail: "empty argv".to_string(),
        };
    };
    let mut cmd = Command::new(binary);
    cmd.args(rest);
    cmd.stdin(std::process::Stdio::null());
    // Run in the requested directory when it exists, so Orca/Herdr resolve the
    // project from the work tree instead of inheriting our cwd.
    if !workdir.is_empty() && std::path::Path::new(workdir).is_dir() {
        cmd.current_dir(workdir);
    }
    let Some(out) = run_with_timeout(cmd, timeout_secs.max(10)) else {
        return RunOutcome {
            ok: false,
            runner: plan.runner.clone(),
            handle: String::new(),
            detail: format!(
                "runner '{}' did not answer within {timeout_secs}s",
                plan.runner
            ),
        };
    };
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    if !out.status.success() {
        let reason = if !stderr.trim().is_empty() {
            stderr.trim().to_string()
        } else if !stdout.trim().is_empty() {
            stdout.trim().to_string()
        } else {
            format!("exited with {:?}", out.status.code())
        };
        return RunOutcome {
            ok: false,
            runner: plan.runner.clone(),
            handle: String::new(),
            detail: crate::util::clamp(&reason, 600),
        };
    }
    let handle = extract_handle(plan.runner.as_str(), &stdout);
    RunOutcome {
        ok: true,
        runner: plan.runner.clone(),
        handle,
        detail: crate::util::clamp(stdout.trim(), 600),
    }
}

/// Pull the runner's identifier out of its JSON reply.
///
/// Both CLIs print a JSON envelope; the id lives at different paths and a
/// shape change must degrade to "no handle" rather than fail the run, since
/// the work has already started by then.
fn extract_handle(runner: &str, stdout: &str) -> String {
    let Ok(v) = crate::json::parse(stdout.trim()) else {
        return String::new();
    };
    let paths: &[&[&str]] = match runner {
        "orca" => &[
            &["result", "dispatch", "id"],
            &["result", "dispatchId"],
            &["result", "id"],
            &["dispatch", "id"],
        ],
        _ => &[
            &["result", "pane_id"],
            &["result", "paneId"],
            &["result", "id"],
            &["pane_id"],
        ],
    };
    for path in paths {
        let mut cur = &v;
        let mut ok = true;
        for seg in *path {
            match cur.get(seg) {
                Some(next) => cur = next,
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            if let Some(s) = cur.as_str() {
                if !s.is_empty() {
                    return s.to_string();
                }
            }
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(runner: &str, placement: &str, remote: &str, binary: &str) -> RunnerPlan {
        RunnerPlan {
            runner: runner.to_string(),
            binary: binary.to_string(),
            placement: placement.to_string(),
            remote: remote.to_string(),
            reason: String::new(),
        }
    }

    #[test]
    fn runner_names_validate() {
        assert!(valid_runner("").is_ok());
        assert!(valid_runner("auto").is_ok());
        assert!(valid_runner("orca").is_ok());
        assert!(valid_runner("Herdr").is_ok());
        assert!(valid_runner("orcaide").is_err());
        assert!(valid_runner("codex").is_err());
    }

    #[test]
    fn explicit_missing_runner_fails_closed() {
        // An unknown runner name is rejected outright.
        let err = choose("nope", "").unwrap_err();
        assert!(err.contains("unknown runner"), "{err}");

        // A known-but-absent runtime must fail closed with the install step,
        // never downgrade to the other runner. Whichever of the two is
        // genuinely absent on this machine is the case to pin.
        for runner in RUNNERS {
            if which(runner).is_some() || (runner == "orca" && orca_binary().is_some()) {
                continue;
            }
            let err = choose(runner, "").unwrap_err();
            assert!(err.contains("not installed"), "{runner}: {err}");
            return;
        }
        // Both installed on this machine: the fail-closed branch is then
        // covered by `explicit_runner_is_honored_exactly` below.
    }

    #[test]
    fn explicit_runner_is_honored_exactly() {
        // When a runner IS present, asking for it by name must yield that
        // runner and not the preference-order winner.
        if which("herdr").is_some() {
            let p = choose("herdr", "").expect("herdr is installed");
            assert_eq!(p.runner, "herdr", "explicit choice must not be reordered");
        }
        if orca_binary().is_some() {
            let p = choose("orca", "").expect("orca is installed");
            assert_eq!(p.runner, "orca");
        }
    }

    #[test]
    fn auto_prefers_orca_then_herdr() {
        // Whatever is installed, auto must resolve to something or explain.
        match choose("auto", "") {
            Ok(p) => {
                assert!(RUNNERS.contains(&p.runner.as_str()));
                assert!(!p.binary.is_empty());
                // Orca is preference 1, so if both exist we must get orca.
                if orca_binary().is_some() {
                    assert_eq!(p.runner, "orca");
                }
            }
            Err(e) => assert!(e.contains("no agent runner"), "{e}"),
        }
    }

    #[test]
    fn remote_placement_is_recorded() {
        // Construct the plan directly so this test does not depend on what is
        // installed on the test machine.
        let p = plan("orca", "remote", "mac-paired", "/usr/bin/orca");
        assert_eq!(p.placement, "remote");
        assert_eq!(p.remote, "mac-paired");
    }

    #[test]
    fn orca_argv_always_supplies_an_agent() {
        // Orca refuses worker-start without --agent ("agent_unconfigured"),
        // so this flag is mandatory, not optional.
        let p = plan("orca", "local", "", "/usr/bin/orca");
        let a = argv(&p, "spec", "/repo");
        let idx = a.iter().position(|s| s == "--agent").expect("--agent present");
        assert_eq!(a[idx + 1], DEFAULT_ORCA_AGENT);

        // An explicit agent overrides the default.
        let b = argv_with_agent(&p, "spec", "/repo", "claude");
        let j = b.iter().position(|s| s == "--agent").unwrap();
        assert_eq!(b[j + 1], "claude");

        // An empty override falls back rather than sending an empty --agent.
        let c = argv_with_agent(&p, "spec", "/repo", "  ");
        let k = c.iter().position(|s| s == "--agent").unwrap();
        assert_eq!(c[k + 1], DEFAULT_ORCA_AGENT);
    }

    #[test]
    fn herdr_argv_does_not_take_an_orca_agent_flag() {
        let p = plan("herdr", "local", "", "/usr/bin/herdr");
        let a = argv_with_agent(&p, "spec", "", "claude");
        assert!(!a.contains(&"--agent".to_string()));
    }

    #[test]
    fn orca_argv_targets_a_paired_environment() {
        let p = plan("orca", "remote", "mac-paired", "/usr/bin/orca");
        let a = argv(&p, "do the thing", "/repo");
        assert_eq!(a[0], "/usr/bin/orca");
        assert_eq!(a[1], "orchestration");
        assert_eq!(a[2], "worker-start");
        assert!(a.contains(&"--spec".to_string()));
        assert!(a.contains(&"do the thing".to_string()));
        // A remote Orca is addressed by environment, never by local path.
        let on_idx = a.iter().position(|s| s == "--on").expect("--on present");
        assert_eq!(a[on_idx + 1], "mac-paired");
        assert!(!a.iter().any(|s| s.starts_with("path:")));
        assert!(a.contains(&"--json".to_string()));
    }

    #[test]
    fn orca_argv_local_uses_the_worktree_path() {
        let p = plan("orca", "local", "", "/usr/bin/orca");
        let a = argv(&p, "spec", "/repo/path");
        assert!(a.contains(&"path:/repo/path".to_string()));
        assert!(!a.contains(&"--on".to_string()));
    }

    #[test]
    fn herdr_argv_addresses_the_ssh_machine() {
        let p = plan("herdr", "remote", "mac", "/usr/bin/herdr");
        let a = argv(&p, "do the thing", "");
        assert_eq!(a[0], "/usr/bin/herdr");
        let m = a.iter().position(|s| s == "--machine").expect("--machine");
        assert_eq!(a[m + 1], "mac");
        assert!(a.contains(&"agent".to_string()));
        assert!(a.contains(&"start".to_string()));
        assert!(a.contains(&"--prompt".to_string()));
    }

    #[test]
    fn herdr_argv_local_has_no_machine_flag() {
        let p = plan("herdr", "local", "", "/usr/bin/herdr");
        let a = argv(&p, "spec", "");
        assert!(!a.contains(&"--machine".to_string()));
        assert!(a.contains(&"agent".to_string()));
    }

    #[test]
    fn specs_are_passed_as_single_argv_words() {
        // A peer-supplied spec with shell metacharacters must survive as one
        // argument: no shell ever sees it, so nothing can be word-split.
        let nasty = "rm -rf / ; echo $(whoami) `id` && | > out";
        let p = plan("orca", "local", "", "/usr/bin/orca");
        let a = argv(&p, nasty, "");
        let idx = a.iter().position(|s| s == "--spec").unwrap();
        assert_eq!(a[idx + 1], nasty);
        // Exactly one element carries it.
        assert_eq!(a.iter().filter(|s| *s == nasty).count(), 1);
    }

    #[test]
    fn handle_extraction_reads_both_shapes() {
        assert_eq!(
            extract_handle("orca", r#"{"result":{"dispatch":{"id":"d-123"}}}"#),
            "d-123"
        );
        assert_eq!(
            extract_handle("orca", r#"{"result":{"dispatchId":"d-9"}}"#),
            "d-9"
        );
        assert_eq!(
            extract_handle("herdr", r#"{"result":{"pane_id":"p-7"}}"#),
            "p-7"
        );
        assert_eq!(
            extract_handle("herdr", r#"{"result":{"id":"p-8"}}"#),
            "p-8"
        );
        // Unparseable or unexpected replies degrade to no handle, not a panic.
        assert_eq!(extract_handle("orca", "not json"), "");
        assert_eq!(extract_handle("herdr", r#"{"other":1}"#), "");
    }

    #[test]
    fn probe_reports_every_runner() {
        let p = probe();
        assert_eq!(p.len(), 2);
        let names: Vec<&str> = p.iter().map(|(n, _, _)| n.as_str()).collect();
        assert!(names.contains(&"orca"));
        assert!(names.contains(&"herdr"));
        for (_, ok, reason) in p {
            assert!(!reason.is_empty(), "every runner needs a reason");
            let _ = ok;
        }
    }

    #[test]
    fn which_rejects_path_like_names() {
        // Never resolve an attacker-supplied "path" as a binary name.
        assert!(which("").is_none());
        assert!(which("../evil").is_none());
        assert!(which("/bin/sh").is_none());
        assert!(which("definitely-not-a-real-binary-xyz").is_none());
    }
}
