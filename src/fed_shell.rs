//! Federated multi-machine shell: virtual cluster navigation, LKGL tracking,
//! distributed OMP execution, and cross-architecture compute routing.
//!
//! In the federated shell, the virtual root (`~/`) consists of folders representing
//! each machine in the cluster (e.g. `~/mac`, `~/windows`, `~/creeper-pi`).
//!
//! Each architecture/machine tracks its Last Known Good Location (LKGL) across
//! sessions (persisted to `$WTF_HOME/lkgl.json`), allowing commands and ACP
//! agent/fleet masters (`omp`, `trae-cli`, `mini`, `free-claude-code`) to execute
//! in their native project workspaces with synchronized configuration (`fed_omp_config.json`).
//!
//! Commands targeting the local machine run locally; commands targeting remote
//! machines run via SSH or federated peer dispatch. Zero external crates.

use crate::json::Value;
use std::collections::BTreeMap;
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
    pub kind: String,         // "local", "peer-hub", "ssh-host"
    pub arch: String,         // e.g. "darwin-arm64", "windows-x86_64", "linux-arm64"
    pub compute_tier: String, // "heavy", "standard", "edge"
    pub lkgl: String,         // Last Known Good Location on this architecture
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
            ("arch", Value::from(self.arch.as_str())),
            ("compute_tier", Value::from(self.compute_tier.as_str())),
            ("lkgl", Value::from(self.lkgl.as_str())),
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

/// Load the Last Known Good Location (LKGL) map from a specified path.
pub fn load_lkgl_map_at(path: &Path) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    if let Ok(c) = std::fs::read_to_string(path) {
        if let Ok(val) = crate::json::parse(&c) {
            if let Some(obj) = val.as_obj() {
                for (k, v) in obj {
                    if let Some(s) = v.as_str() {
                        map.insert(k.to_ascii_lowercase(), s.to_string());
                    }
                }
            }
        }
    }
    map
}

/// Persist an updated LKGL for a given machine to a specified path (0600).
pub fn save_lkgl_at(path: &Path, machine: &str, p: &str) {
    let m = machine.trim().to_ascii_lowercase();
    let p_clean = p.trim();
    if m.is_empty() || p_clean.is_empty() || p_clean == "/" || p_clean == "~" {
        return;
    }
    let mut map = load_lkgl_map_at(path);
    map.insert(m, p_clean.to_string());
    let mut entries = Vec::new();
    for (k, v) in &map {
        entries.push((k.as_str(), Value::from(v.as_str())));
    }
    let val = Value::obj(entries);
    let _ = crate::config::save_json(path, &val, 0o600);
}

/// Retrieve the Last Known Good Location for a specific machine from a specified path.
pub fn get_machine_lkgl_at(path: &Path, machine: &str) -> Option<String> {
    let map = load_lkgl_map_at(path);
    map.get(&machine.trim().to_ascii_lowercase()).cloned()
}

/// Load the Last Known Good Location (LKGL) map from disk ($WTF_HOME/lkgl.json).
pub fn load_lkgl_map() -> BTreeMap<String, String> {
    load_lkgl_map_at(&crate::config::home().join("lkgl.json"))
}

/// Persist an updated LKGL for a given machine to disk ($WTF_HOME/lkgl.json, 0600).
pub fn save_lkgl(machine: &str, path: &str) {
    save_lkgl_at(&crate::config::home().join("lkgl.json"), machine, path);
}

/// Retrieve the Last Known Good Location for a specific machine.
pub fn get_machine_lkgl(machine: &str) -> Option<String> {
    get_machine_lkgl_at(&crate::config::home().join("lkgl.json"), machine)
}

/// Load the federated OMP / Coding Fleet configuration ($WTF_HOME/fed_omp_config.json).
pub fn load_fed_omp_config() -> Value {
    let path = crate::config::home().join("fed_omp_config.json");
    if let Ok(c) = std::fs::read_to_string(&path) {
        if let Ok(v) = crate::json::parse(&c) {
            return v;
        }
    }
    // Default federated configuration
    Value::obj(vec![
        ("model", Value::from("local-router/fallback-models")),
        ("proxy_url", Value::from("http://127.0.0.1:11434/v1")),
        (
            "fallback_chain",
            Value::arr(vec![
                Value::from("free-claude-code"),
                Value::from("omp"),
                Value::from("trae-cli"),
                Value::from("mini"),
            ]),
        ),
        ("fleet_mode", Value::from(true)),
    ])
}

/// Save the federated OMP / Coding Fleet configuration.
pub fn save_fed_omp_config(cfg: &Value) -> Result<(), String> {
    let path = crate::config::home().join("fed_omp_config.json");
    crate::config::save_json(&path, cfg, 0o600)
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
    let arch_raw = std::env::consts::ARCH;
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

    let local_arch = format!("{}-{}", os, arch_raw);
    let local_lkgl = get_machine_lkgl(&local_name).unwrap_or_else(|| {
        std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| ".".to_string())
    });
    let local_tier = if local_name.contains("pi") || (arch_raw.contains("arm") && os != "macos") {
        "edge".to_string()
    } else {
        "heavy".to_string()
    };

    machines.push(ClusterMachine {
        name: local_name,
        aliases: local_aliases,
        is_local: true,
        host: "127.0.0.1".to_string(),
        port: 7800,
        kind: "local".to_string(),
        arch: local_arch,
        compute_tier: local_tier,
        lkgl: local_lkgl,
    });

    // 2. Peer hubs from federation.json
    for (pname, purl) in peers {
        let (phost, pport) = parse_host_port(purl);
        let mut aliases = Vec::new();
        let short = pname
            .strip_prefix("hub-")
            .or_else(|| pname.strip_prefix("fed-"))
            .unwrap_or(pname.as_str());
        if short != pname {
            aliases.push(short.to_string());
        }

        let peer_arch = if pname.contains("win") || phost.contains("win") {
            "windows-x86_64".to_string()
        } else if pname.contains("mac") || phost.contains("mac") {
            "darwin-arm64".to_string()
        } else if pname.contains("pi") {
            "linux-arm64".to_string()
        } else {
            "linux-x86_64".to_string()
        };

        let peer_tier = if pname.contains("pi") || pname.contains("edge") {
            "edge".to_string()
        } else if pname.contains("win") || pname.contains("mac") || pname.contains("gpu") {
            "heavy".to_string()
        } else {
            "standard".to_string()
        };

        let peer_lkgl = get_machine_lkgl(pname).unwrap_or_else(|| {
            if pname.contains("win") {
                "/mnt/d/Code".to_string()
            } else if pname.contains("pi") {
                "/home/pi".to_string()
            } else {
                "~".to_string()
            }
        });

        machines.push(ClusterMachine {
            name: pname.clone(),
            aliases,
            is_local: false,
            host: phost,
            port: pport,
            kind: "peer-hub".to_string(),
            arch: peer_arch,
            compute_tier: peer_tier,
            lkgl: peer_lkgl,
        });
    }

    // 3. Registered devices from keystore
    for dev in devices {
        if !machines.iter().any(|m| m.name == *dev || m.aliases.contains(dev)) {
            let d_arch = if dev.contains("win") {
                "windows-x86_64".to_string()
            } else if dev.contains("mac") {
                "darwin-arm64".to_string()
            } else if dev.contains("pi") {
                "linux-arm64".to_string()
            } else {
                "linux-x86_64".to_string()
            };

            let d_tier = if dev.contains("pi") || dev.contains("edge") {
                "edge".to_string()
            } else if dev.contains("win") || dev.contains("mac") {
                "heavy".to_string()
            } else {
                "standard".to_string()
            };

            let d_lkgl = get_machine_lkgl(dev).unwrap_or_else(|| {
                if dev.contains("win") {
                    "/mnt/d/Code".to_string()
                } else if dev.contains("pi") {
                    "/home/pi".to_string()
                } else {
                    "~".to_string()
                }
            });

            machines.push(ClusterMachine {
                name: dev.clone(),
                aliases: Vec::new(),
                is_local: false,
                host: dev.clone(),
                port: 7800,
                kind: "device".to_string(),
                arch: d_arch,
                compute_tier: d_tier,
                lkgl: d_lkgl,
            });
        }
    }

    // 4. SSH hosts from ~/.ssh/config
    if let Ok(home_dir) = std::env::var("HOME") {
        let ssh_cfg = Path::new(&home_dir).join(".ssh").join("config");
        if ssh_cfg.exists() {
            if let Ok(c) = std::fs::read_to_string(&ssh_cfg) {
                for host in parse_ssh_config_hosts(&c) {
                    if !machines.iter().any(|m| m.name == host || m.aliases.contains(&host)) {
                        let h_arch = if host.contains("win") {
                            "windows-x86_64".to_string()
                        } else if host.contains("mac") {
                            "darwin-arm64".to_string()
                        } else if host.contains("pi") {
                            "linux-arm64".to_string()
                        } else {
                            "linux-x86_64".to_string()
                        };

                        let h_tier = if host.contains("pi") || host.contains("edge") {
                            "edge".to_string()
                        } else if host.contains("win") || host.contains("mac") {
                            "heavy".to_string()
                        } else {
                            "standard".to_string()
                        };

                        let h_lkgl = get_machine_lkgl(&host).unwrap_or_else(|| {
                            if host.contains("win") {
                                "/mnt/d/Code".to_string()
                            } else if host.contains("pi") {
                                "/home/pi".to_string()
                            } else {
                                "~".to_string()
                            }
                        });

                        machines.push(ClusterMachine {
                            name: host.clone(),
                            aliases: Vec::new(),
                            is_local: false,
                            host: host.clone(),
                            port: 22,
                            kind: "ssh-host".to_string(),
                            arch: h_arch,
                            compute_tier: h_tier,
                            lkgl: h_lkgl,
                        });
                    }
                }
            }
        }
    }

    machines
}

/// Parse host and port from a URL string like "http://192.168.1.248:7800"
fn parse_host_port(url: &str) -> (String, u16) {
    let stripped = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .unwrap_or(url)
        .trim_end_matches('/');

    if let Some((h, p)) = stripped.split_once(':') {
        let port: u16 = p.parse().unwrap_or(7800);
        (h.to_string(), port)
    } else {
        (stripped.to_string(), 7800)
    }
}

/// Simple parser for Host directives in ~/.ssh/config.
fn parse_ssh_config_hosts(content: &str) -> Vec<String> {
    let mut hosts = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("Host ") {
            let host = rest.trim();
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
                    format!("~/{}", machine)
                } else {
                    format!("~/{}/{}", machine, subpath.trim_start_matches('/'))
                }
            }
        }
    }
}

/// Execute a compound federated shell command across virtual paths and cluster machines.
pub fn exec_federated(
    command_line: &str,
    current_vpath: &str,
    machines: &[ClusterMachine],
    timeout_secs: u64,
) -> ShellOutcome {
    let trimmed = command_line.trim();
    if trimmed.is_empty() {
        return ShellOutcome {
            ok: true,
            exit_code: 0,
            output: String::new(),
            new_cwd: current_vpath.to_string(),
            machine: "cluster".to_string(),
        };
    }

    let mut vpath = VirtualPath::parse(current_vpath);
    let mut combined_output = String::new();
    let mut last_code = 0;
    let mut last_machine = match &vpath {
        VirtualPath::ClusterRoot => "cluster".to_string(),
        VirtualPath::MachinePath { machine, .. } => machine.clone(),
    };

    // Split compound commands into sequence: handles "&&", ";"
    let segments = parse_command_segments(trimmed);

    for seg in &segments {
        if seg.trim().is_empty() {
            continue;
        }
        let s = seg.trim();

        // Handle cd transitions
        if s.starts_with("cd ") || s == "cd" {
            let target_dir = s.strip_prefix("cd").unwrap_or("").trim();
            vpath = navigate_vpath(&vpath, target_dir, machines);
            last_machine = match &vpath {
                VirtualPath::ClusterRoot => "cluster".to_string(),
                VirtualPath::MachinePath { machine, .. } => machine.clone(),
            };
            if segments.len() == 1 {
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
                if s.starts_with("ls") || s == "dir" {
                    let mut list = String::from("Federated Cluster Root (~/):\n\n");
                    for m in machines {
                        let status_chip = if m.is_local {
                            "[LOCAL]"
                        } else {
                            "[REMOTE]"
                        };
                        let tier_chip = format!("[{}]", m.compute_tier.to_ascii_uppercase());
                        let lkgl_info = if !m.lkgl.is_empty() {
                            format!("LKGL: {}", m.lkgl)
                        } else {
                            String::new()
                        };
                        list.push_str(&format!(
                            "  drwxr-xr-x  {:<14} {:<8} {:<9} {:<16} {}\n",
                            format!("{}/", m.name),
                            status_chip,
                            tier_chip,
                            m.arch,
                            lkgl_info
                        ));
                    }
                    list.push_str("\nTip: Use 'cd <machine>' (e.g. 'cd mac' or 'cd windows') to execute commands.\n");
                    (list, 0)
                } else if s.starts_with("pwd") {
                    ("~/\n".to_string(), 0)
                } else {
                    (
                        format!(
                            "error: currently in federated cluster root (~/). cd into a machine folder first (e.g. 'cd mac' or 'cd windows') to run '{}'.\n",
                            s
                        ),
                        1,
                    )
                }
            }
            VirtualPath::MachinePath { machine, subpath } => {
                let resolved = resolve_machine(machine, machines);
                match resolved {
                    Some(m) if m.is_local => {
                        // Run locally using machine's LKGL and subpath
                        let outcome = run_local_cmd(s, subpath, &m.lkgl, timeout_secs);
                        if outcome.1 == 0 && (s.starts_with("cd ") || subpath != "/") {
                            save_lkgl(machine, subpath);
                        }
                        outcome
                    }
                    Some(m) => {
                        // Run remotely via SSH or peer dispatch
                        let outcome = run_remote_cmd(m, s, subpath, timeout_secs);
                        if outcome.1 == 0 && (s.starts_with("cd ") || subpath != "/") {
                            save_lkgl(machine, subpath);
                        }
                        outcome
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
            combined_output.push_str(&format!(">>> {}\n", s));
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
        let parsed = VirtualPath::parse(t);
        if let VirtualPath::MachinePath { ref machine, ref subpath } = parsed {
            if subpath != "/" && !subpath.is_empty() {
                save_lkgl(machine, subpath);
            }
        }
        return parsed;
    }

    match current {
        VirtualPath::ClusterRoot => {
            if let Some((mach, rest)) = t.split_once('/') {
                let sub = format!("/{}", rest.trim_start_matches('/'));
                save_lkgl(mach, &sub);
                VirtualPath::MachinePath {
                    machine: mach.to_string(),
                    subpath: sub,
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
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_else(|| "/".to_string());
                    let new_sub = if parent.is_empty() { "/".to_string() } else { parent };
                    save_lkgl(machine, &new_sub);
                    VirtualPath::MachinePath {
                        machine: machine.clone(),
                        subpath: new_sub,
                    }
                }
            } else if t.starts_with("../") {
                let rest = t.strip_prefix("../").unwrap_or("");
                let next_vpath = navigate_vpath(
                    &VirtualPath::MachinePath {
                        machine: machine.clone(),
                        subpath: "/".to_string(),
                    },
                    "..",
                    machines,
                );
                navigate_vpath(&next_vpath, rest, machines)
            } else {
                let new_sub = format!("{}/{}", subpath.trim_end_matches('/'), t);
                save_lkgl(machine, &new_sub);
                VirtualPath::MachinePath {
                    machine: machine.clone(),
                    subpath: new_sub,
                }
            }
        }
    }
}

/// Run a command on the local machine with LKGL anchor and loopback router proxy.
fn run_local_cmd(cmd: &str, subpath: &str, lkgl: &str, _timeout_secs: u64) -> (String, i32) {
    let mut c = Command::new("sh");
    c.arg("-c");

    // Anchor OMP and agent fleet tools to singular loopback proxy :11434
    c.env("OLLAMA_HOST", "127.0.0.1:11434");
    c.env("LOCAL_ROUTER_URL", "http://127.0.0.1:11434/v1");

    // Resolve working directory: subpath if valid dir, else machine LKGL
    let run_dir = if subpath != "/" && !subpath.is_empty() && Path::new(subpath).is_dir() {
        subpath.to_string()
    } else if !lkgl.is_empty() && Path::new(lkgl).is_dir() {
        lkgl.to_string()
    } else {
        String::new()
    };

    let run_cmd = if !run_dir.is_empty() {
        format!("cd '{}' && {}", run_dir, cmd)
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

/// Run a command on a remote machine (via SSH or peer dispatch) with remote LKGL anchor.
fn run_remote_cmd(
    machine: &ClusterMachine,
    cmd: &str,
    subpath: &str,
    timeout_secs: u64,
) -> (String, i32) {
    let remote_dir = if subpath != "/" && !subpath.is_empty() {
        subpath.to_string()
    } else if !machine.lkgl.is_empty() {
        machine.lkgl.clone()
    } else {
        String::new()
    };

    let remote_cmd = if !remote_dir.is_empty() {
        format!(
            "export OLLAMA_HOST=127.0.0.1:11434 LOCAL_ROUTER_URL=http://127.0.0.1:11434/v1; cd '{}' 2>/dev/null || true; {}",
            remote_dir, cmd
        )
    } else {
        format!(
            "export OLLAMA_HOST=127.0.0.1:11434 LOCAL_ROUTER_URL=http://127.0.0.1:11434/v1; {}",
            cmd
        )
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

/// Simple parser for command lines with "&&" or ";".
fn parse_command_segments(input: &str) -> Vec<&str> {
    let mut segments = Vec::new();
    let mut start = 0;
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if i + 1 < len && bytes[i] == b'&' && bytes[i + 1] == b'&' {
            let seg = input[start..i].trim();
            if !seg.is_empty() {
                segments.push(seg);
            }
            i += 2;
            start = i;
        } else if bytes[i] == b';' {
            let seg = input[start..i].trim();
            if !seg.is_empty() {
                segments.push(seg);
            }
            i += 1;
            start = i;
        } else {
            i += 1;
        }
    }
    let rest = input[start..].trim();
    if !rest.is_empty() {
        segments.push(rest);
    }
    segments
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vpath_parsing_and_display() {
        assert_eq!(VirtualPath::parse("").to_display(), "~/");
        assert_eq!(VirtualPath::parse("~/").to_display(), "~/");
        assert_eq!(VirtualPath::parse("~/mac").to_display(), "~/mac");
        assert_eq!(VirtualPath::parse("~/mac/").to_display(), "~/mac");
        assert_eq!(VirtualPath::parse("~/mac/src").to_display(), "~/mac/src");
        assert_eq!(VirtualPath::parse("~/windows/backend").to_display(), "~/windows/backend");
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
                arch: "darwin-arm64".into(),
                compute_tier: "heavy".into(),
                lkgl: "/tmp".into(),
            },
            ClusterMachine {
                name: "windows".into(),
                aliases: vec!["win".into()],
                is_local: false,
                host: "192.168.1.248".into(),
                port: 7800,
                kind: "peer-hub".into(),
                arch: "windows-x86_64".into(),
                compute_tier: "heavy".into(),
                lkgl: "/mnt/d/Code".into(),
            },
        ];

        let root = VirtualPath::ClusterRoot;
        let to_mac = navigate_vpath(&root, "mac", &machines);
        assert!(to_mac.to_display().starts_with("~/mac"));

        let to_sub = navigate_vpath(&to_mac, "code", &machines);
        assert_eq!(to_sub.to_display(), "~/mac/code");

        let back = navigate_vpath(&to_sub, "..", &machines);
        assert!(back.to_display().starts_with("~/mac"));

        let to_root = navigate_vpath(&back, "..", &machines);
        assert_eq!(to_root.to_display(), "~/");

        let switch = navigate_vpath(&to_mac, "../windows", &machines);
        assert!(switch.to_display().starts_with("~/windows"));
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
                arch: "darwin-arm64".into(),
                compute_tier: "heavy".into(),
                lkgl: "/tmp".into(),
            },
            ClusterMachine {
                name: "windows".into(),
                aliases: vec!["win".into()],
                is_local: false,
                host: "192.168.1.248".into(),
                port: 7800,
                kind: "peer-hub".into(),
                arch: "windows-x86_64".into(),
                compute_tier: "heavy".into(),
                lkgl: "/mnt/d/Code".into(),
            },
        ];

        let outcome = exec_federated("ls", "~/", &machines, 5);
        assert!(outcome.ok);
        assert!(outcome.output.contains("mac/"));
        assert!(outcome.output.contains("windows/"));
        assert!(outcome.output.contains("[HEAVY]"));
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
            arch: "darwin-arm64".into(),
            compute_tier: "heavy".into(),
            lkgl: ".".into(),
        }];

        let outcome = exec_federated("echo 'WTF_SHELL_TEST_OK'", "~/mac", &machines, 5);
        assert!(outcome.ok);
        assert!(outcome.output.contains("WTF_SHELL_TEST_OK"));
    }

    #[test]
    fn lkgl_and_fed_omp_config_roundtrip() {
        let tmp = format!("/tmp/wtf-lkgl-test-{}", crate::rand::nonce_hex());
        let path = std::path::PathBuf::from(&tmp).join("lkgl.json");
        save_lkgl_at(&path, "test-node", "/var/log/test");
        assert_eq!(get_machine_lkgl_at(&path, "test-node"), Some("/var/log/test".to_string()));
        let _ = std::fs::remove_dir_all(&tmp);

        let cfg = load_fed_omp_config();
        assert!(cfg.get("model").is_some());
        assert!(cfg.get("proxy_url").is_some());
        assert!(cfg.get("fallback_chain").is_some());
    }
}
