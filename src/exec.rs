//! Bounded external-command execution.
//!
//! The hub shells out to the `orca` and `herdr` CLIs for live state. Every
//! call is argument-vector based (never a shell), has a hard deadline, and
//! kills the child on timeout so a wedged CLI cannot stall the page.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// Resolve a program: explicit env override, then known absolute paths, then
/// a PATH lookup. Returns `None` when nothing executable exists.
pub fn resolve(env_key: &str, candidates: &[&str]) -> Option<String> {
    if let Ok(v) = std::env::var(env_key) {
        if !v.trim().is_empty() && Path::new(&v).is_file() {
            return Some(v);
        }
    }
    for c in candidates {
        if c.contains('/') {
            if Path::new(c).is_file() {
                return Some((*c).to_string());
            }
        } else if let Some(found) = path_lookup(c) {
            return Some(found);
        }
    }
    None
}

fn path_lookup(name: &str) -> Option<String> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate.to_string_lossy().into_owned());
        }
    }
    None
}

/// Run `program args...` with stdin closed and a hard timeout.
/// Returns stdout on success; stderr (trimmed) on failure.
pub fn run(program: &str, args: &[&str], timeout: Duration) -> Result<String, String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn {program}: {e}"))?;

    let mut out = child.stdout.take().expect("stdout piped");
    let mut err = child.stderr.take().expect("stderr piped");
    let out_reader = thread::spawn(move || {
        let mut s = String::new();
        let _ = out.read_to_string(&mut s);
        s
    });
    let err_reader = thread::spawn(move || {
        let mut s = String::new();
        let _ = err.read_to_string(&mut s);
        s
    });

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err(format!(
                        "{} timed out after {}ms",
                        program,
                        timeout.as_millis()
                    ));
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(format!("wait {program}: {e}"));
            }
        }
    };

    let stdout = out_reader.join().unwrap_or_default();
    let stderr = err_reader.join().unwrap_or_default();
    match status {
        Ok(s) if s.success() => Ok(stdout),
        Ok(s) => Err(format!(
            "{} exited {}: {}",
            program,
            s.code().unwrap_or(-1),
            first_line(&stderr)
        )),
        Err(e) => Err(e),
    }
}

fn first_line(s: &str) -> String {
    let line = s.lines().next().unwrap_or("").trim();
    if line.len() > 240 {
        format!("{}…", &line[..240])
    } else {
        line.to_string()
    }
}
