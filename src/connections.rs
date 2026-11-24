//! Connection discovery: what can this machine actually reach, and what is
//! missing before it can.
//!
//! Operator directive (2026-11-07): *rely on the user configuring ssh*. The
//! tool must therefore do two things well — **report** the connections that
//! exist, including ones it cannot currently authenticate, and **name the exact
//! missing step** instead of failing vaguely.
//!
//! Sources, in trust order:
//!
//! 1. `~/.ssh/config` — the operator's declared intent. This is the primary
//!    surface: a `Host` block with `HostName`, `User`, `Port`, `IdentityFile`
//!    is a machine the operator wants reachable by name.
//! 2. Federated hub peers (`federation.json`) — hubs already linked.
//! 3. The local machine itself.
//!
//! What this module deliberately does NOT do is authenticate on its own or
//! rewrite ssh config. A probe performs a bounded, read-only reachability test
//! (`ssh -o BatchMode=yes … 'true'`) **only when the caller asks for one**,
//! because an outbound ssh attempt to every configured host is a side effect
//! the operator should trigger, not something a status call does silently.
//!
//! Zero external crates: the ssh_config grammar we support is spelled out in
//! `parse_ssh_config` and unit-tested against forms that appear in real configs.

use crate::json::Value;
use std::path::{Path, PathBuf};

/// A machine this box knows how to reach (or knows it wants to).
#[derive(Clone, Debug, PartialEq)]
pub struct Connection {
    /// The name you address it by: an ssh alias, a fed peer name, or "local".
    pub name: String,
    /// `ssh`, `fed`, or `local`.
    pub kind: String,
    /// Resolved hostname/IP ("" for local).
    pub host: String,
    /// SSH user ("" = whatever ssh defaults to).
    pub user: String,
    pub port: u16,
    /// Where this entry came from: `ssh-config`, `federation`, `local`.
    pub source: String,
    /// Config-declared identity file, if any.
    pub identity: String,
}

impl Connection {
    /// The argument you would hand `ssh` / `herdr machine add`.
    pub fn ssh_target(&self) -> String {
        if self.user.is_empty() {
            self.host.clone()
        } else {
            format!("{}@{}", self.user, self.host)
        }
    }

    pub fn to_json(&self) -> Value {
        Value::obj(vec![
            ("name", Value::from(self.name.as_str())),
            ("kind", Value::from(self.kind.as_str())),
            ("host", Value::from(self.host.as_str())),
            ("user", Value::from(self.user.as_str())),
            ("port", Value::from(self.port as i64)),
            ("source", Value::from(self.source.as_str())),
            ("identity", Value::from(self.identity.as_str())),
            ("target", Value::from(self.ssh_target())),
        ])
    }

    /// One display line: `name -> user@host (source)`.
    pub fn to_line(&self) -> String {
        match self.kind.as_str() {
            "local" => format!("{} (local machine)", self.name),
            "fed" => format!("{} -> {} (federation peer)", self.name, self.host),
            _ => {
                let mut line = self.name.clone();
                if self.host != self.name {
                    line.push_str(&format!(" -> {}", self.ssh_target()));
                }
                if self.port != 22 {
                    line.push_str(&format!(":{}", self.port));
                }
                line.push_str(&format!(" ({})", self.source));
                if !self.identity.is_empty() {
                    line.push_str(" [key]");
                }
                line
            }
        }
    }
}

/// One `Host` block from ssh_config, with the options we care about.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SshHostBlock {
    /// All patterns on the `Host` line (a block may declare several).
    pub patterns: Vec<String>,
    pub hostname: String,
    pub user: String,
    pub port: u16,
    pub identity: String,
}

/// Parse `~/.ssh/config` into host blocks.
///
/// Supported grammar (the subset real configs use): `Host`, `HostName`, `User`,
/// `Port`, `IdentityFile`, with `Key value` or `Key=value`, any case, leading
/// whitespace, and `#` comments. `Match` and `Include` end the tracked block —
/// we do not resolve them, and pretending otherwise would report a connection
/// whose options may not apply.
pub fn parse_ssh_config(content: &str) -> Vec<SshHostBlock> {
    let mut blocks: Vec<SshHostBlock> = Vec::new();
    let mut current: Option<SshHostBlock> = None;

    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Split "Key value" or "Key=value".
        let (key, value) = match line.split_once(|c: char| c.is_whitespace() || c == '=') {
            Some((k, v)) => (k.trim(), v.trim().trim_start_matches('=').trim()),
            None => (line, ""),
        };
        match key.to_ascii_lowercase().as_str() {
            "host" => {
                if let Some(b) = current.take() {
                    blocks.push(b);
                }
                let patterns: Vec<String> =
                    value.split_whitespace().map(|s| s.to_string()).collect();
                current = Some(SshHostBlock {
                    patterns,
                    port: 22,
                    ..Default::default()
                });
            }
            "match" | "include" => {
                // End the tracked block; we do not evaluate these.
                if let Some(b) = current.take() {
                    blocks.push(b);
                }
            }
            "hostname" => {
                if let Some(b) = current.as_mut() {
                    b.hostname = value.to_string();
                }
            }
            "user" => {
                if let Some(b) = current.as_mut() {
                    b.user = value.to_string();
                }
            }
            "port" => {
                if let Some(b) = current.as_mut() {
                    if let Ok(p) = value.parse::<u16>() {
                        b.port = p;
                    }
                }
            }
            "identityfile" => {
                if let Some(b) = current.as_mut() {
                    // Last wins: that is the most specific override in a block.
                    b.identity = value.trim_matches('"').to_string();
                }
            }
            _ => {}
        }
    }
    if let Some(b) = current.take() {
        blocks.push(b);
    }
    blocks
}

/// Expand a leading `~` in an IdentityFile.
fn expand_home(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return Path::new(&home).join(rest).to_string_lossy().to_string();
        }
    }
    p.to_string()
}

/// Is this pattern a concrete host we can address (not a glob, not a service)?
fn usable_pattern(p: &str) -> bool {
    if p.is_empty() || p.starts_with('!') {
        return false;
    }
    if p.contains('*') || p.contains('?') {
        return false;
    }
    let l = p.to_ascii_lowercase();
    ![
        "github.com",
        "gitlab.com",
        "bitbucket.org",
        "hf.co",
        "hf.space",
    ]
    .iter()
    .any(|x| l == *x)
}

/// Build the connection list from ssh config text.
pub fn from_ssh_config(content: &str) -> Vec<Connection> {
    let mut out = Vec::new();
    for b in parse_ssh_config(content) {
        let host = b.hostname.clone();
        for pattern in b.patterns.iter().filter(|p| usable_pattern(p)) {
            // No HostName: ssh resolves the alias itself, so the alias IS the host.
            let effective_host = if host.is_empty() {
                pattern.clone()
            } else {
                host.clone()
            };
            let port = if b.port == 0 { 22 } else { b.port };
            out.push(Connection {
                name: pattern.clone(),
                kind: "ssh".to_string(),
                host: effective_host,
                user: b.user.clone(),
                port,
                source: "ssh-config".to_string(),
                identity: expand_home(&b.identity),
            });
        }
    }
    out
}

/// Read `~/.ssh/config` (or `$WTF_SSH_CONFIG`), returning "" when absent.
pub fn read_ssh_config() -> String {
    if let Ok(p) = std::env::var("WTF_SSH_CONFIG") {
        return std::fs::read_to_string(p).unwrap_or_default();
    }
    let Some(home) = std::env::var_os("HOME") else {
        return String::new();
    };
    std::fs::read_to_string(PathBuf::from(home).join(".ssh").join("config")).unwrap_or_default()
}

/// Every connection this box knows about, de-duplicated by name.
///
/// Order is meaningful: ssh config first (the operator's declared intent), then
/// federation peers, then local. A name declared twice keeps the ssh entry,
/// because that is the one a human wrote.
pub fn discover() -> Vec<Connection> {
    let mut out = from_ssh_config(&read_ssh_config());

    let fed = crate::federation::FedConfig::load();
    for peer in &fed.peers {
        if out.iter().any(|c| c.name == peer.name) {
            continue;
        }
        let host = peer
            .url
            .trim_start_matches("http://")
            .trim_start_matches("https://")
            .split('/')
            .next()
            .unwrap_or("")
            .to_string();
        out.push(Connection {
            name: peer.name.clone(),
            kind: "fed".to_string(),
            host,
            user: String::new(),
            port: 7800,
            source: "federation".to_string(),
            identity: String::new(),
        });
    }

    if !out.iter().any(|c| c.name == "local") {
        out.push(Connection {
            name: "local".to_string(),
            kind: "local".to_string(),
            host: String::new(),
            user: String::new(),
            port: 22,
            source: "local".to_string(),
            identity: String::new(),
        });
    }
    out
}

/// Result of an actual reachability probe.
#[derive(Clone, Debug)]
pub struct ProbeResult {
    pub name: String,
    /// `ok` | `auth` | `unreachable` | `timeout` | `no-ssh`.
    pub status: String,
    /// One sentence a human can act on.
    pub detail: String,
    /// Remote OS name when login worked.
    pub os: String,
    /// Agent runtimes found on the target (empty when login failed).
    pub runners: Vec<String>,
}

impl ProbeResult {
    pub fn ok(&self) -> bool {
        self.status == "ok"
    }

    pub fn to_json(&self) -> Value {
        Value::obj(vec![
            ("name", Value::from(self.name.as_str())),
            ("status", Value::from(self.status.as_str())),
            ("detail", Value::from(self.detail.as_str())),
            ("os", Value::from(self.os.as_str())),
            (
                "runners",
                Value::arr(
                    self.runners
                        .iter()
                        .map(|r| Value::from(r.as_str()))
                        .collect(),
                ),
            ),
        ])
    }
}

/// Classify an ssh attempt into an actionable status.
///
/// ssh's own diagnostics are the only reliable signal, and the distinctions a
/// human must make are "no route", "host refused your key", and "worked".
pub fn classify_ssh_output(success: bool, stderr: &str) -> (&'static str, String) {
    if success {
        return ("ok", "ssh login succeeded".to_string());
    }
    let s = stderr.to_ascii_lowercase();
    if s.contains("permission denied") || s.contains("authentication failed") {
        (
            "auth",
            "host reachable, but it refused the key — install this machine's public key there"
                .to_string(),
        )
    } else if s.contains("timed out") {
        (
            "timeout",
            "no answer: host asleep, or a firewall is dropping the port".to_string(),
        )
    } else if s.contains("connection refused") {
        (
            "unreachable",
            "nothing is listening on that port (is sshd running?)".to_string(),
        )
    } else if s.contains("no route to host") || s.contains("network is unreachable") {
        ("unreachable", "no network route to that address".to_string())
    } else if s.contains("could not resolve") || s.contains("name or service not known") {
        ("unreachable", "that hostname does not resolve".to_string())
    } else if s.trim().is_empty() {
        ("unreachable", "ssh failed with no diagnostic".to_string())
    } else {
        ("unreachable", crate::util::clamp(stderr.trim(), 300))
    }
}

/// Probe one connection with a bounded, read-only ssh call.
///
/// Runs `ssh -o BatchMode=yes <target> 'uname -s; command -v …'` — BatchMode
/// forbids prompts, so a host that would ask for a password fails fast instead
/// of hanging. One round trip answers both "can I log in" and "what runtimes
/// are there".
pub fn probe(conn: &Connection, timeout_secs: u64) -> ProbeResult {
    if conn.kind != "ssh" {
        return ProbeResult {
            name: conn.name.clone(),
            status: "ok".to_string(),
            detail: match conn.kind.as_str() {
                "local" => "this machine".to_string(),
                _ => "federated peer (http, not ssh)".to_string(),
            },
            os: String::new(),
            runners: Vec::new(),
        };
    }
    if crate::runner::which("ssh").is_none() {
        return ProbeResult {
            name: conn.name.clone(),
            status: "no-ssh".to_string(),
            detail: "ssh is not installed on this machine".to_string(),
            os: String::new(),
            runners: Vec::new(),
        };
    }

    let connect = timeout_secs.max(3);
    let mut cmd = std::process::Command::new("ssh");
    cmd.arg("-o").arg("BatchMode=yes");
    cmd.arg("-o").arg(format!("ConnectTimeout={connect}"));
    cmd.arg("-o").arg("StrictHostKeyChecking=accept-new");
    if conn.port != 0 && conn.port != 22 {
        cmd.arg("-p").arg(conn.port.to_string());
    }
    if !conn.identity.is_empty() {
        cmd.arg("-i").arg(&conn.identity);
    }
    cmd.arg(conn.ssh_target());
    cmd.arg("uname -s 2>/dev/null; command -v herdr orca-ide orca 2>/dev/null");

    let Some(out) = run_capped(cmd, connect + 4) else {
        return ProbeResult {
            name: conn.name.clone(),
            status: "timeout".to_string(),
            detail: format!("no answer within {}s", connect + 4),
            os: String::new(),
            runners: Vec::new(),
        };
    };
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let (status, detail) = classify_ssh_output(out.status.success(), &stderr);
    if status != "ok" {
        return ProbeResult {
            name: conn.name.clone(),
            status: status.to_string(),
            detail,
            os: String::new(),
            runners: Vec::new(),
        };
    }
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let mut lines = stdout.lines();
    let os = lines.next().unwrap_or("").trim().to_string();
    let mut runners: Vec<String> = Vec::new();
    for l in lines {
        let base = l.trim().rsplit('/').next().unwrap_or("").to_string();
        if base.contains("herdr") && !runners.iter().any(|r| r == "herdr") {
            runners.push("herdr".to_string());
        } else if base.contains("orca") && !runners.iter().any(|r| r == "orca") {
            runners.push("orca".to_string());
        }
    }
    ProbeResult {
        name: conn.name.clone(),
        status: "ok".to_string(),
        detail,
        os,
        runners,
    }
}

/// Run a command with a wall-clock cap, killing it on expiry.
fn run_capped(mut cmd: std::process::Command, secs: u64) -> Option<std::process::Output> {
    use std::process::Stdio;
    cmd.stdin(Stdio::null());
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

/// Probe every ssh connection (bounded by `timeout_secs` per host).
pub fn probe_all(conns: &[Connection], timeout_secs: u64) -> Vec<ProbeResult> {
    conns.iter().map(|c| probe(c, timeout_secs)).collect()
}

/// Render the connection list as text, naming every gap and its fix.
pub fn format_connections(conns: &[Connection], probes: &[ProbeResult]) -> String {
    if conns.is_empty() {
        return "no connections configured — add a Host block to ~/.ssh/config, then retry".into();
    }
    let mut out = format!("connections ({}):\n", conns.len());
    for c in conns {
        out.push_str("  ");
        out.push_str(&c.to_line());
        if let Some(p) = probes.iter().find(|p| p.name == c.name) {
            out.push_str(&format!("  [{}]", p.status));
            if p.ok() {
                if !p.os.is_empty() {
                    out.push_str(&format!(" {}", p.os));
                }
                if !p.runners.is_empty() {
                    out.push_str(&format!(" runners:{}", p.runners.join(",")));
                }
            } else {
                out.push_str(&format!(" — {}", p.detail));
            }
        }
        out.push('\n');
    }
    let blocked: Vec<&ProbeResult> = probes.iter().filter(|p| p.status == "auth").collect();
    if !blocked.is_empty() {
        out.push_str("\nblocked on credentials: ");
        out.push_str(
            &blocked
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        );
        out.push('\n');
        for p in blocked {
            if let Some(c) = conns.iter().find(|c| c.name == p.name) {
                out.push_str(&format!(
                    "  fix: ssh-copy-id {}   (or append this machine's ~/.ssh/*.pub to that host's authorized_keys)\n",
                    c.ssh_target()
                ));
            }
        }
    }
    out
}

/// The operator recipe for adding a machine, shown when nothing is configured.
pub fn add_machine_recipe() -> String {
    "add a machine:\n  append to ~/.ssh/config\n    Host mac\n      HostName 192.168.1.68\n      User nbiish\n      IdentityFile ~/.ssh/id_ed25519_mac\n\n  then install the key there:\n    ssh-copy-id mac\n\n  then register it with herdr:\n    herdr machine add mac --label mac\n".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
# a comment
Host mac
    HostName 192.168.1.68
    User nbiish
    IdentityFile ~/.ssh/id_ed25519_mac

Host win win-box
    HostName 192.168.1.248
    User kenwa
    Port 2222
    IdentityFile=~/.ssh/win_key

Host *
    User default-user

Host github.com
    HostName github.com
"#;

    #[test]
    fn parses_host_blocks_with_every_option() {
        let blocks = parse_ssh_config(SAMPLE);
        let mac = blocks
            .iter()
            .find(|b| b.patterns.contains(&"mac".to_string()))
            .expect("mac block");
        assert_eq!(mac.hostname, "192.168.1.68");
        assert_eq!(mac.user, "nbiish");
        assert_eq!(mac.port, 22, "port defaults to 22");
        assert!(mac.identity.ends_with("id_ed25519_mac"));
    }

    #[test]
    fn parses_equals_form_and_multiple_aliases() {
        let blocks = parse_ssh_config(SAMPLE);
        let win = blocks
            .iter()
            .find(|b| b.patterns.contains(&"win".to_string()))
            .expect("win block");
        assert_eq!(win.port, 2222);
        assert!(win.patterns.contains(&"win-box".to_string()));
        assert!(win.identity.ends_with("win_key"));
    }

    #[test]
    fn globs_and_service_hosts_are_not_machines() {
        let conns = from_ssh_config(SAMPLE);
        let names: Vec<&str> = conns.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"mac"));
        assert!(names.contains(&"win"));
        assert!(names.contains(&"win-box"), "a block expands to every alias");
        assert!(!names.contains(&"*"), "a wildcard host is not addressable");
        assert!(!names.contains(&"github.com"), "a service host is not a machine");
    }

    #[test]
    fn match_ends_the_tracked_block() {
        // Options after Match must not leak into the preceding Host.
        let cfg = "Host good\n  HostName 10.0.0.1\nMatch host thing\n  User sneaky\n";
        let blocks = parse_ssh_config(cfg);
        let good = blocks
            .iter()
            .find(|b| b.patterns.contains(&"good".to_string()))
            .unwrap();
        assert_eq!(good.user, "", "Match options must not apply to the prior Host");
    }

    #[test]
    fn ssh_target_is_what_you_would_type() {
        let conns = from_ssh_config(SAMPLE);
        let mac = conns.iter().find(|c| c.name == "mac").unwrap();
        assert_eq!(mac.ssh_target(), "nbiish@192.168.1.68");
        let win = conns.iter().find(|c| c.name == "win").unwrap();
        assert_eq!(win.ssh_target(), "kenwa@192.168.1.248");
        assert_eq!(win.port, 2222);
    }

    #[test]
    fn ssh_output_is_classified_actionably() {
        let (s, d) = classify_ssh_output(false, "nbiish@host: Permission denied (publickey).");
        assert_eq!(s, "auth");
        assert!(d.contains("public key"), "{d}");

        let (s, _) = classify_ssh_output(false, "ssh: connect to host x port 22: Connection timed out");
        assert_eq!(s, "timeout");
        let (s, _) = classify_ssh_output(false, "ssh: connect to host x port 22: Connection refused");
        assert_eq!(s, "unreachable");
        let (s, _) = classify_ssh_output(false, "ssh: Could not resolve hostname nope");
        assert_eq!(s, "unreachable");
        let (s, _) = classify_ssh_output(true, "");
        assert_eq!(s, "ok");
    }

    #[test]
    fn empty_config_yields_local_only() {
        assert!(from_ssh_config("").is_empty());
        let all = discover();
        assert!(all.iter().any(|c| c.name == "local"));
    }

    #[test]
    fn format_names_the_missing_step() {
        let conns = from_ssh_config(SAMPLE);
        let probes = vec![ProbeResult {
            name: "mac".to_string(),
            status: "auth".to_string(),
            detail: "host reachable, but it refused the key".to_string(),
            os: String::new(),
            runners: Vec::new(),
        }];
        let text = format_connections(&conns, &probes);
        assert!(text.contains("blocked on credentials"));
        assert!(text.contains("ssh-copy-id nbiish@192.168.1.68"), "{text}");
    }

    #[test]
    fn probe_of_local_does_not_shell_out() {
        let local = Connection {
            name: "local".into(),
            kind: "local".into(),
            host: String::new(),
            user: String::new(),
            port: 22,
            source: "local".into(),
            identity: String::new(),
        };
        let p = probe(&local, 1);
        assert_eq!(p.status, "ok");
        assert_eq!(p.detail, "this machine");
    }

    #[test]
    fn recipe_covers_all_three_steps() {
        let r = add_machine_recipe();
        assert!(r.contains("ssh/config"));
        assert!(r.contains("ssh-copy-id"));
        assert!(r.contains("herdr machine add"));
    }
}
