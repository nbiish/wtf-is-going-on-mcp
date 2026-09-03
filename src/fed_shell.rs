//! Federated multi-machine shell: virtual cluster navigation and execution.
//!
//! In the federated shell, the virtual root (`~/`) consists of folders representing
//! each machine in the cluster (e.g. `~/mac`, `~/windows`, `~/creeper-pi`).
//!
//! An operator or agent can `cd` to any machine and execute commands, or chain
//! multi-machine commands in a single prompt:
//!
//!   `cd ~/mac/frontend && npm run build && cd ~/windows/backend && cargo build`
//!
//! Commands targeting the local machine run locally; commands targeting remote
//! machines run via SSH or federated peer dispatch. Zero external crates.

use crate::json::Value;
use std::path::Path;
use std::process::Command;

/// A machine known to the federated cluster.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusterMachine {
    pub name: String,
    pub aliases: Vec<String>,
    pub is_local: bool,
    pub host: String,
    pub port: u16,
    pub kind: String, // "local", "peer-hub", "ssh-host"
}

impl ClusterMachine {
    pub fn to_json(&self) -> Value {
        Value::obj(vec![
            ("name", Value::from(self.name.as_str())),
            (
                "aliases",
                Value::arr(
                    self.aliases
                        .iter()
                        .map(|a| Value::from(a.as_str()))
                        .collect(),
                ),
            ),
            ("is_local", Value::from(self.is_local)),
            ("host", Value::from(self.host.as_str())),
            ("port", Value::from(self.port as i64)),
            ("kind", Value::from(self.kind.as_str())),
        ])
    }
}

/// Output of a federated shell execution.
#[derive(Debug, Clone)]
pub struct ShellOutcome {
    pub ok: bool,
    pub exit_code: i32,
    pub output: String,
    pub new_cwd: String,
    pub machine: String,
}

impl ShellOutcome {
    pub fn to_json(&self) -> Value {
        Value::obj(vec![
            ("ok", Value::from(self.ok)),
            ("exit_code", Value::from(self.exit_code as i64)),
            ("output", Value::from(self.output.as_str())),
            ("new_cwd", Value::from(self.new_cwd.as_str())),
            ("machine", Value::from(self.machine.as_str())),
        ])
    }
}

/// Discover cluster machines from local system, federation config, and SSH config.
pub fn discover_machines(
    local_fed_name: &str,
    peers: &[(String, String)], // (name, url)
    devices: &[String],
) -> Vec<ClusterMachine> {
    let mut machines = Vec::new();

    // 1. Local machine
    let os = std::env::consts::OS;
    let local_name = if !local_fed_name.is_empty() {
        local_fed_name.to_string()
    } else {
        match os {
            "macos" => "mac".to_string(),
            "windows" => "windows".to_string(),
            "linux" => "linux".to_string(),
            _ => "local".to_string(),
        }
    };

    let mut local_aliases = vec!["local".to_string(), "localhost".to_string()];
    if os == "macos" && local_name != "mac" {
        local_aliases.push("mac".to_string());
    } else if os == "windows" && local_name != "windows" {
        local_aliases.push("windows".to_string());
    } else if os == "linux" && local_name != "linux" {
        local_aliases.push("linux".to_string());
    }

    machines.push(ClusterMachine {
        name: local_name,
        aliases: local_aliases,
        is_local: true,
        host: "127.0.0.1".to_string(),
        port: 7800,
        kind: "local".to_string(),
    });

    // 2. Peer hubs from federation
    for (peer_name, peer_url) in peers {
        let (host, port) = parse_host_port(peer_url);
        let mut aliases = Vec::new();
        if peer_name.contains("windows") || host == "192.168.1.248" {
            aliases.push("windows".to_string());
            aliases.push("windows-1".to_string());
            aliases.push("win".to_string());
        } else if peer_name.contains("mac") {
            aliases.push("mac".to_string());
        } else if peer_name.contains("linux") {
            aliases.push("linux".to_string());
        }

        // Avoid duplicating if already present
        if !machines.iter().any(|m| m.name == *peer_name) {
            machines.push(ClusterMachine {
                name: peer_name.clone(),
                aliases,
                is_local: false,
                host,
                port,
                kind: "peer-hub".to_string(),
            });
        }
    }

    // 3. Known devices from keystore
    for dev in devices {
        if dev.starts_with("fed-") {
            continue;
        }
        let is_local_dev = dev == "mac-agent" && os == "macos"
            || dev == "windows-agent" && os == "windows";
        if !is_local_dev && !machines.iter().any(|m| m.name == *dev || m.aliases.contains(dev)) {
            let mut aliases = Vec::new();
            if dev.contains("windows") {
                aliases.push("windows".to_string());
            } else if dev.contains("mac") {
                aliases.push("mac".to_string());
            }
            machines.push(ClusterMachine {
                name: dev.clone(),
                aliases,
                is_local: false,
                host: dev.clone(),
                port: 22,
                kind: "device".to_string(),
            });
        }
    }

    // 4. Discover SSH config hosts (~/.ssh/config)
    for host in parse_ssh_hosts() {
        if !machines.iter().any(|m| m.name == host || m.aliases.contains(&host)) {
            let is_win = host.contains("win");
            let is_mac = host.contains("mac");
            let mut aliases = Vec::new();
            if is_win {
                aliases.push("windows".to_string());
            } else if is_mac {
                aliases.push("mac".to_string());
            }
            machines.push(ClusterMachine {
                name: host.clone(),
                aliases,
                is_local: false,
                host: host.clone(),
                port: 22,
                kind: "ssh-host".to_string(),
            });
        }
    }

    machines
}

fn parse_host_port(url: &str) -> (String, u16) {
    let clean = url.trim_start_matches("http://").trim_start_matches("https://");
    let host_part = clean.split('/').next().unwrap_or(clean);
    if let Some((h, p)) = host_part.split_once(':') {
        (h.to_string(), p.parse::<u16>().unwrap_or(7800))
    } else {
        (host_part.to_string(), 7800)
    }
}

/// Simple parser for ~/.ssh/config Host entries.
fn parse_ssh_hosts() -> Vec<String> {
    let home = std::env::var("HOME").unwrap_or_default();
    if home.is_empty() {
        return Vec::new();
    }
    let config_path = Path::new(&home).join(".ssh").join("config");
    let Ok(content) = std::fs::read_to_string(config_path) else {
        return Vec::new();
    };

    let mut hosts = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("Host ") {
            let host = rest.trim();
            // Skip wildcards or non-machine names like github.com, hf.space
            if !host.contains('*')
                && !host.contains('?')
                && !host.contains("github")
                && !host.contains("hf.space")
                && !host.contains("hf.co")
            {
                hosts.push(host.to_string());
            }
        }
    }
    hosts
}

/// Match a target machine name or alias from a list of machines.
pub fn resolve_machine<'a>(target: &str, machines: &'a [ClusterMachine]) -> Option<&'a ClusterMachine> {
    let t = target.trim().to_ascii_lowercase();
    for m in machines {
        if m.name.to_ascii_lowercase() == t {
            return Some(m);
        }
        for a in &m.aliases {
            if a.to_ascii_lowercase() == t {
                return Some(m);
            }
        }
    }
    None
}

/// Represents the virtual path inside the federated shell.
/// Can be:
/// - `ClusterRoot`: `~/` (displays all machine directories)
/// - `MachinePath`: `~/<machine>/<path>`
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VirtualPath {
    ClusterRoot,
    MachinePath {
        machine: String,
        subpath: String,
    },
}

impl VirtualPath {
    pub fn parse(cwd: &str) -> Self {
        let trimmed = cwd.trim();
        if trimmed.is_empty() || trimmed == "~" || trimmed == "~/" || trimmed == "/" {
            return VirtualPath::ClusterRoot;
        }
        let norm = trimmed.trim_start_matches("~/").trim_start_matches('/');
        if norm.is_empty() {
            return VirtualPath::ClusterRoot;
        }
        if let Some((mach, rest)) = norm.split_once('/') {
            VirtualPath::MachinePath {
                machine: mach.to_string(),
                subpath: format!("/{}", rest.trim_start_matches('/')),
            }
        } else {
            VirtualPath::MachinePath {
                machine: norm.to_string(),
                subpath: "/".to_string(),
            }
        }
    }

    pub fn to_display(&self) -> String {
        match self {
            VirtualPath::ClusterRoot => "~/".to_string(),
            VirtualPath::MachinePath { machine, subpath } => {
                if subpath == "/" || subpath.is_empty() {
                    format!("~/{machine}")
                } else {
                    format!("~/{machine}{}", subpath)
                }
            }
        }
    }
}

/// Execute a federated shell command across one or multiple machines.
pub fn exec_federated(
    raw_cmd: &str,
    current_cwd: &str,
    machines: &[ClusterMachine],
    timeout_secs: u64,
) -> ShellOutcome {
    let mut vpath = VirtualPath::parse(current_cwd);
    let trimmed = raw_cmd.trim();

    // Check for empty
    if trimmed.is_empty() {
        return ShellOutcome {
            ok: true,
            exit_code: 0,
            output: String::new(),
            new_cwd: vpath.to_display(),
            machine: match &vpath {
                VirtualPath::ClusterRoot => "cluster".into(),
                VirtualPath::MachinePath { machine, .. } => machine.clone(),
            },
        };
    }

    // Split compound commands chained with &&
    let segments: Vec<&str> = if trimmed.contains("&&") {
        trimmed.split("&&").map(|s| s.trim()).collect()
    } else if trimmed.contains(';') {
        trimmed.split(';').map(|s| s.trim()).collect()
    } else {
        vec![trimmed]
    };

    let mut combined_output = String::new();
    let mut last_code = 0;
    let mut last_machine = match &vpath {
        VirtualPath::ClusterRoot => "cluster".to_string(),
        VirtualPath::MachinePath { machine, .. } => machine.clone(),
    };

    for (_idx, seg) in segments.iter().enumerate() {
        if seg.is_empty() {
            continue;
        }
        // Handle cd transitions
        if seg.starts_with("cd ") || *seg == "cd" {
            let target_dir = seg.strip_prefix("cd").unwrap_or("").trim();
            vpath = navigate_vpath(&vpath, target_dir, machines);
            last_machine = match &vpath {
                VirtualPath::ClusterRoot => "cluster".to_string(),
                VirtualPath::MachinePath { machine, .. } => machine.clone(),
            };
            if segments.len() == 1 {
                // Just cd command
                return ShellOutcome {
                    ok: true,
                    exit_code: 0,
                    output: String::new(),
                    new_cwd: vpath.to_display(),
                    machine: last_machine,
                };
            }
            continue;
        }

        // Execute segment on active machine
        let (out, code) = match &vpath {
            VirtualPath::ClusterRoot => {
                // Command in cluster root
                if seg.starts_with("ls") || *seg == "dir" {
                    let mut list = String::from("Federated Cluster Root (~/):\n\n");
                    for m in machines {
                        let status_chip = if m.is_local {
                            "[LOCAL]"
                        } else {
                            "[REMOTE]"
                        };
                        let kind_info = format!("{} (host: {}:{})", m.kind, m.host, m.port);
                        list.push_str(&format!(
                            "  drwxr-xr-x  {:<16} {:<10} {}\n",
                            format!("{}/", m.name),
                            status_chip,
                            kind_info
                        ));
                    }
                    list.push_str("\nTip: Use 'cd <machine>' (e.g. 'cd mac' or 'cd windows') to execute commands.\n");
                    (list, 0)
                } else if seg.starts_with("pwd") {
                    ("~/\n".to_string(), 0)
                } else {
                    (
                        format!(
                            "error: currently in federated cluster root (~/). cd into a machine folder first (e.g. 'cd mac' or 'cd windows') to run '{}'.\n",
                            seg
                        ),
                        1,
                    )
                }
            }
            VirtualPath::MachinePath { machine, subpath } => {
                let resolved = resolve_machine(machine, machines);
                match resolved {
                    Some(m) if m.is_local => {
                        // Run locally
                        run_local_cmd(seg, subpath, timeout_secs)
                    }
                    Some(m) => {
                        // Run remotely via SSH or peer dispatch
                        run_remote_cmd(m, seg, subpath, timeout_secs)
                    }
                    None => (
                        format!("error: unknown machine '{}' in cluster.\n", machine),
                        1,
                    ),
                }
            }
        };

        let badge = format!("[{}] ", last_machine);
        if segments.len() > 1 {
            combined_output.push_str(&format!(">>> {}\n", seg));
        }
        for line in out.lines() {
            combined_output.push_str(&badge);
            combined_output.push_str(line);
            combined_output.push('\n');
        }
        if out.is_empty() && code == 0 && segments.len() > 1 {
            combined_output.push_str(&badge);
            combined_output.push_str("(ok)\n");
        }

        last_code = code;
        if code != 0 && trimmed.contains("&&") {
            // Stop chain on failure if chained with &&
            break;
        }
    }

    ShellOutcome {
        ok: last_code == 0,
        exit_code: last_code,
        output: combined_output,
        new_cwd: vpath.to_display(),
        machine: last_machine,
    }
}

/// Navigate virtual path when user inputs `cd <target>`.
fn navigate_vpath(
    current: &VirtualPath,
    target: &str,
    machines: &[ClusterMachine],
) -> VirtualPath {
    let t = target.trim();
    if t.is_empty() || t == "~" || t == "~/" || t == "/" {
        return VirtualPath::ClusterRoot;
    }

    // Absolute virtual path ~/machine/path
    if t.starts_with("~/") || t.starts_with('/') {
        return VirtualPath::parse(t);
    }

    match current {
        VirtualPath::ClusterRoot => {
            // cd <machine>
            if let Some((mach, rest)) = t.split_once('/') {
                VirtualPath::MachinePath {
                    machine: mach.to_string(),
                    subpath: format!("/{}", rest.trim_start_matches('/')),
                }
            } else {
                VirtualPath::MachinePath {
                    machine: t.to_string(),
                    subpath: "/".to_string(),
                }
            }
        }
        VirtualPath::MachinePath { machine, subpath } => {
            if t == ".." || t == "../" {
                if subpath == "/" || subpath.is_empty() {
                    VirtualPath::ClusterRoot
                } else {
                    let parent = Path::new(subpath)
                        .parent()
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "/".to_string());
                    let final_sub = if parent.is_empty() {
                        "/".to_string()
                    } else {
                        parent
                    };
                    VirtualPath::MachinePath {
                        machine: machine.clone(),
                        subpath: final_sub,
                    }
                }
            } else if t.starts_with("../") {
                let rest = t.trim_start_matches("../");
                if subpath == "/" || subpath.is_empty() {
                    // Navigate to another machine: e.g. cd ../windows
                    VirtualPath::parse(&format!("~/{}", rest))
                } else {
                    let parent = Path::new(subpath)
                        .parent()
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "/".to_string());
                    VirtualPath::MachinePath {
                        machine: machine.clone(),
                        subpath: format!("{}/{}", parent.trim_end_matches('/'), rest),
                    }
                }
            } else {
                // cd into subdirectory
                let new_sub = format!("{}/{}", subpath.trim_end_matches('/'), t);
                VirtualPath::MachinePath {
                    machine: machine.clone(),
                    subpath: new_sub,
                }
            }
        }
    }
}

/// Run a command on the local machine.
fn run_local_cmd(cmd: &str, subpath: &str, _timeout_secs: u64) -> (String, i32) {
    let mut c = Command::new("sh");
    c.arg("-c");

    // Resolve directory
    let run_cmd = if subpath != "/" && !subpath.is_empty() && Path::new(subpath).is_dir() {
        format!("cd '{}' && {}", subpath, cmd)
    } else {
        cmd.to_string()
    };
    c.arg(&run_cmd);

    match c.output() {
        Ok(o) => {
            let mut out = String::from_utf8_lossy(&o.stdout).to_string();
            let err = String::from_utf8_lossy(&o.stderr);
            if !err.is_empty() {
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                out.push_str(&err);
            }
            (out, o.status.code().unwrap_or(1))
        }
        Err(e) => (format!("execution failed: {e}\n"), 1),
    }
}

/// Run a command on a remote machine (via SSH or peer dispatch).
fn run_remote_cmd(
    machine: &ClusterMachine,
    cmd: &str,
    subpath: &str,
    timeout_secs: u64,
) -> (String, i32) {
    // If subpath is specified, prefix cd
    let remote_cmd = if subpath != "/" && !subpath.is_empty() {
        format!("cd '{}' 2>/dev/null || true; {}", subpath, cmd)
    } else {
        cmd.to_string()
    };

    let target_host = if !machine.host.is_empty() {
        &machine.host
    } else {
        &machine.name
    };

    let timeout_str = timeout_secs.max(5).to_string();

    let mut ssh = Command::new("ssh");
    ssh.args([
        "-o",
        "BatchMode=yes",
        "-o",
        &format!("ConnectTimeout={}", timeout_str),
        "-o",
        "StrictHostKeyChecking=accept-new",
        target_host,
        &remote_cmd,
    ]);

    match ssh.output() {
        Ok(o) => {
            let mut out = String::from_utf8_lossy(&o.stdout).to_string();
            let err = String::from_utf8_lossy(&o.stderr);
            if !err.is_empty() {
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                out.push_str(&err);
            }
            (out, o.status.code().unwrap_or(1))
        }
        Err(e) => (
            format!(
                "remote execution on {} failed: {}\n(ensure SSH key access is configured for {})\n",
                machine.name, e, target_host
            ),
            1,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vpath_parsing_and_display() {
        assert_eq!(VirtualPath::parse(""), VirtualPath::ClusterRoot);
        assert_eq!(VirtualPath::parse("~/"), VirtualPath::ClusterRoot);
        assert_eq!(VirtualPath::parse("/"), VirtualPath::ClusterRoot);

        let p1 = VirtualPath::parse("~/mac");
        assert_eq!(
            p1,
            VirtualPath::MachinePath {
                machine: "mac".into(),
                subpath: "/".into()
            }
        );
        assert_eq!(p1.to_display(), "~/mac");

        let p2 = VirtualPath::parse("~/windows/backend/src");
        assert_eq!(
            p2,
            VirtualPath::MachinePath {
                machine: "windows".into(),
                subpath: "/backend/src".into()
            }
        );
        assert_eq!(p2.to_display(), "~/windows/backend/src");
    }

    #[test]
    fn machine_navigation_transitions() {
        let machines = vec![
            ClusterMachine {
                name: "mac".into(),
                aliases: vec!["local".into()],
                is_local: true,
                host: "127.0.0.1".into(),
                port: 7800,
                kind: "local".into(),
            },
            ClusterMachine {
                name: "windows".into(),
                aliases: vec!["win".into()],
                is_local: false,
                host: "192.168.1.248".into(),
                port: 7800,
                kind: "peer-hub".into(),
            },
        ];

        let root = VirtualPath::ClusterRoot;
        let to_mac = navigate_vpath(&root, "mac", &machines);
        assert_eq!(to_mac.to_display(), "~/mac");

        let to_sub = navigate_vpath(&to_mac, "code", &machines);
        assert_eq!(to_sub.to_display(), "~/mac/code");

        let back = navigate_vpath(&to_sub, "..", &machines);
        assert_eq!(back.to_display(), "~/mac");

        let to_root = navigate_vpath(&back, "..", &machines);
        assert_eq!(to_root.to_display(), "~/");

        let switch = navigate_vpath(&back, "../windows", &machines);
        assert_eq!(switch.to_display(), "~/windows");
    }

    #[test]
    fn cluster_root_ls_returns_machines() {
        let machines = vec![
            ClusterMachine {
                name: "mac".into(),
                aliases: vec!["local".into()],
                is_local: true,
                host: "127.0.0.1".into(),
                port: 7800,
                kind: "local".into(),
            },
            ClusterMachine {
                name: "windows".into(),
                aliases: vec!["win".into()],
                is_local: false,
                host: "192.168.1.248".into(),
                port: 7800,
                kind: "peer-hub".into(),
            },
        ];

        let outcome = exec_federated("ls", "~/", &machines, 5);
        assert!(outcome.ok);
        assert!(outcome.output.contains("mac/"));
        assert!(outcome.output.contains("windows/"));
    }

    #[test]
    fn local_execution_runs_cleanly() {
        let machines = vec![ClusterMachine {
            name: "mac".into(),
            aliases: vec!["local".into()],
            is_local: true,
            host: "127.0.0.1".into(),
            port: 7800,
            kind: "local".into(),
        }];

        let outcome = exec_federated("echo 'WTF_SHELL_TEST_OK'", "~/mac", &machines, 5);
        assert!(outcome.ok);
        assert!(outcome.output.contains("WTF_SHELL_TEST_OK"));
    }
}
