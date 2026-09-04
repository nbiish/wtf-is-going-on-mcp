//! Chat executor: run tasks handed to a federated repo chat on this machine.
//!
//! Every chat session maps to ONE named tmux session (`wtf-chat-<slug>`).
//! Inside it, the agent-CLI fallback chain runs non-interactively:
//!   1. Claude Code / FreeClaudeCode `claude` / `fcc-claude`
//!   2. OhMyPy `omp`
//!   3. Hermes Agent `hermes`
//!   4. Trae-CLI `trae-cli` (AST Refactoring Master)
//!   5. Mini-SWE `mini` / `mini-live` (TDD Reproduction Engineer)
//!   6. Codex `codex`
//!   7. OpenCode `opencode`
//!   8. Aider `aider`
//!   9. Cline `cline`
//!   10. Pi `pi`
//! The first CLI that is installed AND completes with exit 0 wins; every
//! attempt is recorded so the report names the lane that ran. All CLIs are
//! pre-configured to route through the local-router proxy (`local-router/fallback-models`
//! on the Ollama/OpenAI/Anthropic compatible ports on 11434). Cross-platform execution
//! supports native tmux, WSL Ubuntu tmux, and direct process fallback.

use crate::json::Value;
use std::process::Command;

/// tmux session prefix: `wtf-chat-<slug>` (<= 48 chars total).
pub const SESSION_PREFIX: &str = "wtf-chat";

/// Backend used for running terminal / agent processes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalBackend {
    NativeTmux,
    WslTmux,
    Direct,
}

/// Detect the best terminal execution backend on this machine.
pub fn detect_backend() -> TerminalBackend {
    if let Ok(s) = Command::new("tmux").arg("-V").status() {
        if s.success() {
            return TerminalBackend::NativeTmux;
        }
    }
    if cfg!(windows) {
        if let Ok(s) = Command::new("wsl")
            .args(["-d", "Ubuntu", "-e", "tmux", "-V"])
            .status()
        {
            if s.success() {
                return TerminalBackend::WslTmux;
            }
        }
    }
    TerminalBackend::Direct
}

pub fn detect_backend_str() -> &'static str {
    match detect_backend() {
        TerminalBackend::NativeTmux => "native-tmux",
        TerminalBackend::WslTmux => "wsl-tmux",
        TerminalBackend::Direct => "direct",
    }
}

/// Translate Windows path to unix/WSL path (e.g. `D:\Code\...` -> `/mnt/d/Code/...`).
pub fn to_unix_path(p: &str) -> String {
    let s = p.replace('\\', "/");
    if s.len() >= 2 && s.as_bytes()[1] == b':' {
        let drive = (s.as_bytes()[0] as char).to_ascii_lowercase();
        let rest = &s[2..];
        format!("/mnt/{drive}{rest}")
    } else {
        s
    }
}

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

/// Information about a supported CLI agent.
#[derive(Debug, Clone)]
pub struct AgentSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub bins: &'static [&'static str],
    pub pre: &'static str,
    pub protocol: &'static str, // "anthropic", "openai", "ollama"
}

pub const ALL_AGENTS: &[AgentSpec] = &[
    AgentSpec {
        id: "free-claude-code",
        name: "Claude Code / FCC",
        bins: &["claude", "fcc-claude"],
        pre: "-p --dangerously-skip-permissions",
        protocol: "anthropic",
    },
    AgentSpec {
        id: "omp",
        name: "OhMyPy (omp)",
        bins: &["omp"],
        pre: "-p",
        protocol: "openai",
    },
    AgentSpec {
        id: "hermes",
        name: "Hermes Agent",
        bins: &["hermes", "hermes-agent"],
        pre: "chat -q",
        protocol: "openai",
    },
    AgentSpec {
        id: "trae-cli",
        name: "Trae-CLI (AST Refactoring Master)",
        bins: &["trae-cli"],
        pre: "run -p openai -m local-router/fallback-models --model-base-url http://127.0.0.1:11434/v1 -k local-router --console-type simple --max-steps 30",
        protocol: "openai",
    },
    AgentSpec {
        id: "mini",
        name: "Mini-SWE (TDD Reproduction Engineer)",
        bins: &["mini", "mini-live"],
        pre: "--yolo --exit-immediately --task",
        protocol: "openai",
    },
    AgentSpec {
        id: "codex",
        name: "Codex CLI",
        bins: &["codex"],
        pre: "exec",
        protocol: "openai",
    },
    AgentSpec {
        id: "opencode",
        name: "OpenCode CLI",
        bins: &["opencode"],
        pre: "run",
        protocol: "openai",
    },
    AgentSpec {
        id: "aider",
        name: "Aider CLI",
        bins: &["aider"],
        pre: "--openai-api-base http://127.0.0.1:11434/v1 --model openai/local-router/fallback-models --no-git --yes --message",
        protocol: "openai",
    },
    AgentSpec {
        id: "cline",
        name: "Cline CLI",
        bins: &["cline"],
        pre: "-p",
        protocol: "openai",
    },
    AgentSpec {
        id: "pi",
        name: "Pi Coding Agent",
        bins: &["pi"],
        pre: "-p",
        protocol: "anthropic",
    },
];

/// Probes whether a binary exists on this machine (native or via WSL).
pub fn have(bin: &str) -> bool {
    if cfg!(windows) {
        if let Ok(o) = Command::new("where.exe").arg(bin).output() {
            if o.status.success() && !o.stdout.is_empty() {
                return true;
            }
        }
        if let Ok(o) = Command::new("wsl")
            .args([
                "-d",
                "Ubuntu",
                "-e",
                "/bin/bash",
                "-lc",
                &format!("command -v {bin} >/dev/null 2>&1"),
            ])
            .status()
        {
            if o.success() {
                return true;
            }
        }
        false
    } else {
        Command::new("sh")
            .arg("-c")
            .arg(format!("export PATH=\"$HOME/.local/bin:$PATH\"; command -v {bin} >/dev/null 2>&1"))
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

/// Returns dynamic inventory of all supported agents and their live availability.
pub fn available_agents() -> Vec<Value> {
    let backend = detect_backend_str();
    let mut out = Vec::new();

    // Meta-Agents
    out.push(Value::obj(vec![
        ("id", Value::from("auto")),
        ("name", Value::from("⚡ Auto Fallback Cascade")),
        ("installed", Value::from(true)),
        ("target_model", Value::from("local-router/fallback-models")),
        ("endpoint", Value::from("http://127.0.0.1:11434/v1")),
        ("backend", Value::from(backend)),
    ]));
    out.push(Value::obj(vec![
        ("id", Value::from("fleet")),
        ("name", Value::from("🤖 SWE-bench Fleet (Trae + Mini)")),
        ("installed", Value::from(have("trae-cli") || have("mini"))),
        ("target_model", Value::from("local-router/fallback-models")),
        ("endpoint", Value::from("http://127.0.0.1:11434/v1")),
        ("backend", Value::from(backend)),
    ]));

    for spec in ALL_AGENTS {
        let mut installed = false;
        let mut detected_bin = "";
        for b in spec.bins {
            if have(b) {
                installed = true;
                detected_bin = *b;
                break;
            }
        }
        let endpoint = match spec.protocol {
            "anthropic" => "http://127.0.0.1:11434",
            _ => "http://127.0.0.1:11434/v1",
        };
        out.push(Value::obj(vec![
            ("id", Value::from(spec.id)),
            ("name", Value::from(spec.name)),
            ("installed", Value::from(installed)),
            ("detected_bin", Value::from(detected_bin)),
            ("target_model", Value::from("local-router/fallback-models")),
            ("endpoint", Value::from(endpoint)),
            ("backend", Value::from(backend)),
        ]));
    }
    out
}

pub fn router_alive() -> bool {
    let check = "curl -s -m 3 http://127.0.0.1:11434/api/version >/dev/null 2>&1";
    if cfg!(windows) && detect_backend() == TerminalBackend::WslTmux {
        Command::new("wsl")
            .args(["-d", "Ubuntu", "-e", "sh", "-c", check])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    } else {
        Command::new("sh")
            .arg("-c")
            .arg(check)
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

pub fn tmux_has_session(name: &str) -> bool {
    match detect_backend() {
        TerminalBackend::NativeTmux => Command::new("tmux")
            .args(["has-session", "-t", name])
            .status()
            .map(|s| s.success())
            .unwrap_or(false),
        TerminalBackend::WslTmux => Command::new("wsl")
            .args(["-d", "Ubuntu", "-e", "tmux", "has-session", "-t", name])
            .status()
            .map(|s| s.success())
            .unwrap_or(false),
        TerminalBackend::Direct => true,
    }
}

pub fn tmux_new_session(name: &str, workdir: &str) -> bool {
    match detect_backend() {
        TerminalBackend::NativeTmux => Command::new("tmux")
            .args(["new-session", "-d", "-s", name, "-c", workdir])
            .status()
            .map(|s| s.success())
            .unwrap_or(false),
        TerminalBackend::WslTmux => {
            let udir = to_unix_path(workdir);
            Command::new("wsl")
                .args([
                    "-d", "Ubuntu", "-e", "tmux", "new-session", "-d", "-s", name, "-c", &udir,
                ])
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        }
        TerminalBackend::Direct => true,
    }
}

pub fn tmux_capture_pane(name: &str, lines: usize) -> Result<String, String> {
    let lines_arg = format!("-{lines}");
    let out = match detect_backend() {
        TerminalBackend::NativeTmux => Command::new("tmux")
            .args(["capture-pane", "-t", name, "-p", "-S", &lines_arg])
            .output(),
        TerminalBackend::WslTmux => Command::new("wsl")
            .args([
                "-d",
                "Ubuntu",
                "-e",
                "tmux",
                "capture-pane",
                "-t",
                name,
                "-p",
                "-S",
                &lines_arg,
            ])
            .output(),
        TerminalBackend::Direct => {
            let log_file = format!("/tmp/wtf-chat-exec-{}.log", slugify(name));
            if let Ok(c) = std::fs::read_to_string(&log_file) {
                return Ok(c);
            }
            return Err("session not found".into());
        }
    };
    match out {
        Ok(o) if o.status.success() => Ok(String::from_utf8_lossy(&o.stdout).into_owned()),
        Ok(_) => Err("session not found".into()),
        Err(e) => Err(format!("tmux capture-pane failed: {e}")),
    }
}

pub fn tmux_send_keys(name: &str, keys: &str) -> Result<(), String> {
    let out = match detect_backend() {
        TerminalBackend::NativeTmux => Command::new("tmux")
            .args(["send-keys", "-t", name, "--", keys, "Enter"])
            .output(),
        TerminalBackend::WslTmux => Command::new("wsl")
            .args([
                "-d", "Ubuntu", "-e", "tmux", "send-keys", "-t", name, "--", keys, "Enter",
            ])
            .output(),
        TerminalBackend::Direct => {
            return Err("interactive send-keys not supported in direct mode".into());
        }
    };
    match out {
        Ok(o) if o.status.success() => Ok(()),
        Ok(_) => Err("tmux send-keys failed".into()),
        Err(e) => Err(format!("tmux send-keys error: {e}")),
    }
}

/// Execute `prompt` inside tmux session `name`, created in `workdir`,
/// through the automated fallback chain. Returns the outcome; the tmux
/// session persists for attach (`tmux attach -t NAME`).
pub fn run_in_tmux(name: &str, workdir: &str, prompt: &str, timeout_secs: u64) -> ChainOutcome {
    run_in_tmux_with_options(name, workdir, prompt, timeout_secs, "auto", true)
}

/// Execute with agent selection ("auto", "free-claude-code", "omp", "hermes", "trae-cli",
/// "mini", "codex", "opencode", "aider", "cline", "pi", "fleet") and Trae/Mini fleet toggle.
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
        if router {
            "up"
        } else {
            "down(continuing)"
        }
    ));

    let backend = detect_backend();
    let is_wsl = backend == TerminalBackend::WslTmux;

    // Session exists? Reuse it; else create detached.
    if backend != TerminalBackend::Direct {
        let exists = tmux_has_session(name);
        if !exists {
            let created = tmux_new_session(name, workdir);
            if !created {
                return ChainOutcome {
                    cli: "none".into(),
                    output: format!("cannot create tmux session {name} (backend: {:?})", backend),
                    ok: false,
                    trace,
                };
            }
        }
    }

    let candidates: Vec<&AgentSpec> = match agent_choice {
        "free-claude-code" | "fcc-claude" | "claude" => ALL_AGENTS
            .iter()
            .filter(|c| c.id == "free-claude-code")
            .collect(),
        "omp" => ALL_AGENTS.iter().filter(|c| c.id == "omp").collect(),
        "hermes" | "hermes-agent" => ALL_AGENTS.iter().filter(|c| c.id == "hermes").collect(),
        "trae-cli" | "trae" => ALL_AGENTS.iter().filter(|c| c.id == "trae-cli").collect(),
        "mini" | "mini-live" => ALL_AGENTS.iter().filter(|c| c.id == "mini").collect(),
        "codex" => ALL_AGENTS.iter().filter(|c| c.id == "codex").collect(),
        "opencode" => ALL_AGENTS.iter().filter(|c| c.id == "opencode").collect(),
        "aider" => ALL_AGENTS.iter().filter(|c| c.id == "aider").collect(),
        "cline" => ALL_AGENTS.iter().filter(|c| c.id == "cline").collect(),
        "pi" => ALL_AGENTS.iter().filter(|c| c.id == "pi").collect(),
        "fleet" | "swe-bench" => ALL_AGENTS
            .iter()
            .filter(|c| c.id == "trae-cli" || c.id == "mini")
            .collect(),
        _ => {
            if fleet_enabled {
                ALL_AGENTS.iter().collect()
            } else {
                ALL_AGENTS
                    .iter()
                    .filter(|c| c.id != "trae-cli" && c.id != "mini")
                    .collect()
            }
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

        // Standardized environment variables routing all agents to local-router/fallback-models on 11434
        let env_prefix = "export PATH=\"$HOME/.local/bin:$PATH\" ANTHROPIC_BASE_URL=http://127.0.0.1:11434 ANTHROPIC_AUTH_TOKEN=local-router CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY=1 CLAUDE_CODE_AUTO_COMPACT_WINDOW=190000 DISABLE_AUTOUPDATER=1 OPENAI_BASE_URL=http://127.0.0.1:11434/v1 OPENAI_API_KEY=local-router OPENAI_API_BASE=http://127.0.0.1:11434/v1 OLLAMA_HOST=http://127.0.0.1:11434 MODEL=local-router/fallback-models HERMES_OPENAI_BASE_URL=http://127.0.0.1:11434/v1 HERMES_MODEL=local-router/fallback-models NONINTERACTIVE=1;";

        let slug = slugify(name);
        let run_line = format!(
            "{env_prefix} {bin} {} {} 2>&1 | tee /tmp/wtf-chat-exec-{slug}.log; echo EXIT:$? > /tmp/wtf-chat-exec-{slug}.code",
            cand.pre,
            shell_quote(prompt),
        );

        let (code_file, log_file) = if is_wsl && cfg!(windows) {
            (
                format!(r"\\wsl$\Ubuntu\tmp\wtf-chat-exec-{slug}.code"),
                format!(r"\\wsl$\Ubuntu\tmp\wtf-chat-exec-{slug}.log"),
            )
        } else {
            (
                format!("/tmp/wtf-chat-exec-{slug}.code"),
                format!("/tmp/wtf-chat-exec-{slug}.log"),
            )
        };

        let _ = std::fs::remove_file(&code_file);

        match backend {
            TerminalBackend::NativeTmux => {
                let _ = Command::new("tmux")
                    .args(["send-keys", "-t", name, &run_line, "Enter"])
                    .status();
            }
            TerminalBackend::WslTmux => {
                let _ = Command::new("wsl")
                    .args([
                        "-d", "Ubuntu", "-e", "tmux", "send-keys", "-t", name, &run_line, "Enter",
                    ])
                    .status();
            }
            TerminalBackend::Direct => {
                let mut cmd = Command::new(bin);
                cmd.args(cand.pre.split_whitespace())
                    .arg(prompt)
                    .current_dir(workdir)
                    .env("NONINTERACTIVE", "1")
                    .env("ANTHROPIC_BASE_URL", "http://127.0.0.1:11434")
                    .env("ANTHROPIC_AUTH_TOKEN", "local-router")
                    .env("OPENAI_BASE_URL", "http://127.0.0.1:11434/v1")
                    .env("OPENAI_API_KEY", "local-router")
                    .env("OPENAI_API_BASE", "http://127.0.0.1:11434/v1")
                    .env("OLLAMA_HOST", "http://127.0.0.1:11434")
                    .env("MODEL", "local-router/fallback-models")
                    .env("HERMES_OPENAI_BASE_URL", "http://127.0.0.1:11434/v1")
                    .env("HERMES_MODEL", "local-router/fallback-models");
                let outcome = cmd.output();
                let (code_str, output) = match outcome {
                    Ok(o) => {
                        let mut text = String::from_utf8_lossy(&o.stdout).to_string();
                        let err = String::from_utf8_lossy(&o.stderr);
                        if !err.trim().is_empty() {
                            text.push_str("\n[stderr] ");
                            text.push_str(&err);
                        }
                        (o.status.code().unwrap_or(1).to_string(), text)
                    }
                    Err(e) => ("1".into(), format!("spawn failed: {e}")),
                };
                let _ = std::fs::write(&log_file, &output);
                let _ = std::fs::write(&code_file, format!("EXIT:{code_str}"));
            }
        }

        // Poll for the exit-code file (bounded by timeout).
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
    let out = match detect_backend() {
        TerminalBackend::NativeTmux => Command::new("tmux")
            .args([
                "list-sessions",
                "-F",
                "#{session_name}\t#{session_created}\t#{session_attached}",
            ])
            .output(),
        TerminalBackend::WslTmux => Command::new("wsl")
            .args([
                "-d",
                "Ubuntu",
                "-e",
                "tmux",
                "list-sessions",
                "-F",
                "#{session_name}\t#{session_created}\t#{session_attached}",
            ])
            .output(),
        TerminalBackend::Direct => {
            return Vec::new();
        }
    };
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

    #[test]
    fn to_unix_path_translation() {
        assert_eq!(to_unix_path(r"D:\Code\test"), "/mnt/d/Code/test");
        assert_eq!(to_unix_path("/home/user/test"), "/home/user/test");
    }

    #[test]
    fn all_agents_inventory() {
        assert!(ALL_AGENTS.len() >= 10);
        assert!(ALL_AGENTS.iter().any(|a| a.id == "hermes"));
        assert!(ALL_AGENTS.iter().any(|a| a.id == "omp"));
        assert!(ALL_AGENTS.iter().any(|a| a.id == "free-claude-code"));
        assert!(ALL_AGENTS.iter().any(|a| a.id == "trae-cli"));
        assert!(ALL_AGENTS.iter().any(|a| a.id == "mini"));
    }
}
