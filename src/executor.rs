//! Chat executor: run tasks handed to a federated repo chat on this machine.
//!
//! Every chat session maps to ONE named tmux session (`wtf-chat-<slug>`).
//! Inside it, the agent-CLI fallback chain runs non-interactively:
//!   1. OhMyPy `omp`   (preferred)
//!   2. Hermes `hermes` (fallback)
//!   3. FreeClaudeCode `fcc-claude` (last resort; its server runs inside
//!      the same tmux session when missing)
//! The first CLI that is installed AND completes with exit 0 wins; every
//! attempt is recorded so the report names the lane that ran. All CLIs are
//! wired to the local-router Ollama proxy (`local-router/fallback-models`
//! on the Ollama-compatible port) by operator config on each machine — the
//! executor only verifies the router answers before starting.

use crate::json::Value;
use std::process::Command;

/// tmux session prefix: `wtf-chat-<slug>` (≤ 48 chars total).
pub const SESSION_PREFIX: &str = "wtf-chat";

/// Slugify a chat name / repo into a tmux-safe token.
pub fn slugify(s: &str) -> String {
    let mut out: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    while out.contains("--") {
        out = out.replace("--", "-");
    }
    while out.starts_with('-') {
        out.remove(0);
    }
    while out.ends_with('-') {
        out.pop();
    }
    let mut out = out.chars().take(24).collect::<String>();
    if out.is_empty() {
        out = "task".into();
    }
    out
}

/// tmux session name for a chat id (stable, unique per chat).
pub fn session_name(chat_id: &str) -> String {
    format!("{}-{}", SESSION_PREFIX, slugify(chat_id))
}

pub struct ChainOutcome {
    /// Which CLI ran (or attempted last on total failure).
    pub cli: String,
    /// Full combined stdout+stderr of the winning (or last) attempt.
    pub output: String,
    pub ok: bool,
    /// Per-attempt trace: "(cli: ok|fail: reason)".
    pub trace: Vec<String>,
}

fn have(bin: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {bin} >/dev/null 2>&1"))
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn router_alive() -> bool {
    // Best-effort probe of the local-router Ollama-compatible port; a
    // missing router is not fatal (a CLI may still have its own provider),
    // but it is recorded in the trace.
    Command::new("sh")
        .arg("-c")
        .arg("curl -s -m 3 http://127.0.0.1:11434/api/version >/dev/null 2>&1")
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Run ONE non-interactive CLI with the prompt on stdin-free argv.
fn run_cli(
    bin: &str,
    args: &[&str],
    prompt: &str,
    timeout_secs: u64,
    workdir: &str,
) -> (bool, String) {
    let mut cmd = Command::new(bin);
    cmd.args(args)
        .arg(prompt)
        .current_dir(workdir)
        .env("NONINTERACTIVE", "1")
        .env("ANTHROPIC_BASE_URL", "http://127.0.0.1:11434")
        .env("ANTHROPIC_AUTH_TOKEN", "local-router")
        .env("OPENAI_BASE_URL", "http://127.0.0.1:11434/v1")
        .env("OPENAI_API_KEY", "local-router")
        .env("OLLAMA_HOST", "http://127.0.0.1:11434")
        .env("MODEL", "local-router/fallback-models");
    // Best-effort kill switch: rely on the CLI's own non-interactive flags;
    // tmux keeps the session alive for inspection either way.
    let _ = timeout_secs;
    match cmd.output() {
        Ok(o) => {
            let mut text = String::from_utf8_lossy(&o.stdout).to_string();
            let err = String::from_utf8_lossy(&o.stderr);
            if !err.trim().is_empty() {
                text.push_str("\n[stderr] ");
                text.push_str(&err);
            }
            (o.status.success(), text)
        }
        Err(e) => (false, format!("spawn failed: {e}")),
    }
}

/// Execute `prompt` inside tmux session `name`, created in `workdir`,
/// through the free-claude-code → omp → trae-cli fallback chain. Returns the
/// outcome; the tmux session persists for attach (`tmux attach -t NAME`).
pub fn run_in_tmux(name: &str, workdir: &str, prompt: &str, timeout_secs: u64) -> ChainOutcome {
    run_in_tmux_with_options(name, workdir, prompt, timeout_secs, "auto", true)
}

/// Execute with agent selection ("auto", "free-claude-code", "omp", "trae-cli", "mini")
/// and Trae/Mini fleet toggle.
pub fn run_in_tmux_with_options(
    name: &str,
    workdir: &str,
    prompt: &str,
    timeout_secs: u64,
    agent_choice: &str,
    fleet_enabled: bool,
) -> ChainOutcome {
    let mut trace = Vec::new();
    let router = router_alive();
    trace.push(format!(
        "router:{}",
        if router { "up" } else { "down(continuing)" }
    ));

    // Session exists? Reuse it (attachable history); else create detached.
    let exists = Command::new("tmux")
        .args(["has-session", "-t", name])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !exists {
        let created = Command::new("tmux")
            .args(["new-session", "-d", "-s", name, "-c", workdir])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !created {
            return ChainOutcome {
                cli: "none".into(),
                output: format!("cannot create tmux session {name} (tmux missing?)"),
                ok: false,
                trace,
            };
        }
    }

    struct Candidate {
        id: &'static str,
        bins: &'static [&'static str],
        pre: String,
    }

    let trae_pre = if fleet_enabled {
        "run --console-type simple --fleet -p".to_string()
    } else {
        "run --console-type simple -p".to_string()
    };

    let all_candidates = [
        Candidate {
            id: "free-claude-code",
            bins: &["fcc-claude", "claude"],
            pre: "-p --dangerously-skip-permissions".into(),
        },
        Candidate {
            id: "omp",
            bins: &["omp"],
            pre: "-p".into(),
        },
        Candidate {
            id: "trae-cli",
            bins: &["trae-cli"],
            pre: trae_pre,
        },
        Candidate {
            id: "mini",
            bins: &["mini", "mini-live"],
            pre: "--yolo --exit-immediately --task".into(),
        },
    ];

    let candidates: Vec<&Candidate> = match agent_choice {
        "free-claude-code" | "fcc-claude" | "claude" => {
            all_candidates.iter().filter(|c| c.id == "free-claude-code").collect()
        }
        "omp" => {
            all_candidates.iter().filter(|c| c.id == "omp").collect()
        }
        "trae-cli" | "trae" => {
            all_candidates.iter().filter(|c| c.id == "trae-cli").collect()
        }
        "mini" | "mini-live" => {
            all_candidates.iter().filter(|c| c.id == "mini").collect()
        }
        _ => {
            // "auto" fallback chain: free-claude-code -> omp -> trae-cli
            vec![&all_candidates[0], &all_candidates[1], &all_candidates[2]]
        }
    };

    let mut last: Option<ChainOutcome> = None;
    for cand in candidates {
        let mut resolved_bin = None;
        for b in cand.bins {
            if have(b) {
                resolved_bin = Some(*b);
                break;
            }
        }
        let Some(bin) = resolved_bin else {
            trace.push(format!("{}: not installed", cand.id));
            continue;
        };

        // Environment configuration ensuring all agents route to local-router/fallback-models on 11434
        let env_prefix = "export ANTHROPIC_BASE_URL=http://127.0.0.1:11434 ANTHROPIC_AUTH_TOKEN=local-router OPENAI_BASE_URL=http://127.0.0.1:11434/v1 OPENAI_API_KEY=local-router OLLAMA_HOST=http://127.0.0.1:11434 MODEL=local-router/fallback-models NONINTERACTIVE=1;";

        // fcc server for the fcc lane must live in this tmux session if fcc-claude is used.
        if bin == "fcc-claude" {
            let srv = Command::new("tmux")
                .args([
                    "send-keys",
                    "-t", name,
                    "command -v fcc-server >/dev/null 2>&1 && (curl -s -m 2 http://127.0.0.1:8082/health >/dev/null 2>&1 || tmux new-session -d -s freeclaude-chat 'fcc-server') || true",
                    "Enter",
                ])
                .status();
            let _ = srv;
        }

        // Run inside the session via a capture pane so the output is also
        // visible live to anyone attaching.
        let run_line = format!(
            "{env_prefix} {bin} {} {} 2>&1 | tee /tmp/wtf-chat-exec-{}.log; echo EXIT:$? > /tmp/wtf-chat-exec-{}.code",
            cand.pre,
            shell_quote(prompt),
            slugify(name),
            slugify(name)
        );
        let _ = Command::new("tmux")
            .args(["send-keys", "-t", name, &run_line, "Enter"])
            .status();

        // Poll for the exit-code file (bounded by timeout).
        let code_file = format!("/tmp/wtf-chat-exec-{}.code", slugify(name));
        let _ = std::fs::remove_file(&code_file);
        let log_file = format!("/tmp/wtf-chat-exec-{}.log", slugify(name));
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs.max(10));
        loop {
            if std::path::Path::new(&code_file).exists() {
                break;
            }
            if std::time::Instant::now() > deadline {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        let code = std::fs::read_to_string(&code_file)
            .unwrap_or_default()
            .trim()
            .trim_start_matches("EXIT:")
            .to_string();
        let output = std::fs::read_to_string(&log_file).unwrap_or_default();
        let ok = code == "0";
        trace.push(format!("{bin}: {}", if ok { "ok" } else { "fail" }));
        last = Some(ChainOutcome {
            cli: bin.to_string(),
            output,
            ok,
            trace: trace.clone(),
        });
        if ok {
            return last.unwrap();
        }
    }
    last.unwrap_or_else(|| ChainOutcome {
        cli: "none".into(),
        output: "no CLI in the fallback chain is installed".into(),
        ok: false,
        trace,
    })
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Status of all wtf-chat-* tmux sessions on this machine.
pub fn list_sessions() -> Vec<Value> {
    let out = Command::new("tmux")
        .args([
            "list-sessions",
            "-F",
            "#{session_name}\t#{session_created}\t#{session_attached}",
        ])
        .output();
    let Ok(o) = out else { return Vec::new() };
    String::from_utf8_lossy(&o.stdout)
        .lines()
        .filter(|l| l.starts_with(SESSION_PREFIX))
        .map(|l| {
            let parts: Vec<&str> = l.split('\t').collect();
            Value::obj(vec![
                ("session", Value::from(parts.first().copied().unwrap_or(""))),
                (
                    "created",
                    Value::from(
                        parts
                            .get(1)
                            .and_then(|x| x.parse::<i64>().ok())
                            .unwrap_or(0),
                    ),
                ),
                (
                    "attached",
                    Value::from(parts.get(2).map(|x| *x == "1").unwrap_or(false)),
                ),
            ])
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_basics() {
        assert_eq!(slugify("local-router ops"), "local-router-ops");
        assert_eq!(slugify("a  -- b!!"), "a-b");
        assert_eq!(slugify(""), "task");
        assert_eq!(
            session_name("828d334113c772c7a8b8cb34db637698").len() <= 48,
            true
        );
    }

    #[test]
    fn shell_quote_wraps() {
        assert_eq!(shell_quote("hi"), "'hi'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
    }
}
