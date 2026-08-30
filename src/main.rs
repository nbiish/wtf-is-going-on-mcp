//! `wtf` command-line interface.
//!
//!   wtf serve            run the hub (dashboard + signed API)
//!   wtf key issue/list/revoke
//!   wtf setup            configure this machine's bridge credentials
//!   wtf agent            run the MCP stdio bridge (what MCP clients launch)
//!   wtf status           print current hub state as text

use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::process::Command;
use std::sync::{Arc, Mutex};
use wtf::client;
use wtf::config::{self, BridgeConfig, HubConfig, KeyStore};
use wtf::json;
use wtf::http;
use wtf::mcp;
use wtf::store::Store;
use wtf::util;
use wtf::VERSION;

/// The portable hub skill, embedded at build time so the single binary can
/// distribute it anywhere (`wtf skill install`). Kept byte-identical with
/// `.agents/skills/wtf-agent-hub/SKILL.md` and the ainish-coder mirror.
const AGENT_SKILL: &str = include_str!("../.agents/skills/wtf-agent-hub/SKILL.md");

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(|s| s.as_str()) {
        Some("serve") => cmd_serve(&args[1..]),
        Some("key") => cmd_key(&args[1..]),
        Some("setup") => cmd_setup(&args[1..]),
        Some("url") => cmd_url(&args[1..]),
        Some("join") => cmd_join(&args[1..]),
        Some("agent") => cmd_agent(),
        Some("status") => cmd_status(),
        Some("dashboard-url") => cmd_dashboard_url(),
        Some("skill") => cmd_skill(&args[1..]),
        Some("version") | Some("--version") | Some("-V") => {
            println!("wtf {VERSION}");
            0
        }
        Some("help") | Some("--help") | Some("-h") | None => {
            print_help();
            0
        }
        Some(other) => {
            eprintln!("unknown command: {other}\n");
            print_help();
            2
        }
    }
}

fn print_help() {
    println!("wtf {VERSION} — what the fuck is going on, across all my machines");
    println!();
    println!("USAGE:");
    println!("  wtf serve [--bind IP:PORT] [--no-open]     run the hub (dashboard + API)");
    println!("  wtf key issue [--json] <name>              provision a device key");
    println!("  wtf key list                               list device keys");
    println!("  wtf key revoke <name>                      revoke a device key");
    println!("  wtf url [URL | clear]                      show/set the URL handed to joiners");
    println!("  wtf setup --url URL --name N --key K       configure this machine's bridge");
    println!("  wtf join user@host [--name N] [--url U]    enroll this machine via ssh");
    println!("  wtf agent                                  run the MCP stdio bridge");
    println!("  wtf status                                 print hub state as text");
    println!("  wtf dashboard-url                          print the clickable dashboard URL (hub machine)");
    println!("  wtf skill install [--dir DIR] [--force]    install the hub skill into DIR/.agents/skills/");
    println!("  wtf skill print                            print the hub skill to stdout");
    println!("  wtf help                                   this text");
    println!();
    println!("state lives in $WTF_HOME (default: ~/.config/wtf-mcp)");
    println!("see README.md for the full story");
}

fn flag_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn has_flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

fn parse_ip(s: &str) -> Option<Ipv4Addr> {
    match s {
        "localhost" => Some(Ipv4Addr::LOCALHOST),
        other => other.parse().ok(),
    }
}

fn cmd_serve(args: &[String]) -> i32 {
    let cfg = match HubConfig::load_or_create() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let keys = match KeyStore::load() {
        Ok(k) => k,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let store = match Store::new(&config::events_path()) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            eprintln!("error: cannot open event log: {e}");
            return 1;
        }
    };
    let bins = Arc::new(wtf::bins::Bins::load());

    let mut bind_ip = cfg.bind_ip.clone();
    let mut port = cfg.port;
    if let Some(b) = flag_value(args, "--bind") {
        let parsed = b.rsplit_once(':').and_then(|(ip, p)| {
            let port: u16 = p.parse().ok()?;
            let ip = parse_ip(ip)?;
            Some((ip, port))
        });
        match parsed {
            Some((ip, p)) => {
                bind_ip = ip.to_string();
                port = p;
            }
            None => {
                eprintln!("error: --bind expects IP:PORT, got '{b}'");
                return 2;
            }
        }
    }
    let ip: Ipv4Addr = match bind_ip.parse() {
        Ok(i) => i,
        Err(_) => {
            eprintln!("error: bad bind ip '{bind_ip}' in config.json");
            return 2;
        }
    };

    let hub = Arc::new(wtf::api::Hub {
        store,
        bins,
        keys: Mutex::new(keys),
        nonces: Mutex::new(wtf::auth::NonceCache::new()),
        dashboard_key: cfg.dashboard_key.clone(),
        started_at: util::now_secs(),
    });
    let handler_hub = Arc::clone(&hub);
    let handler: http::Handler = Arc::new(move |req| wtf::api::handle(&handler_hub, req));

    let listener = match TcpListener::bind(SocketAddr::from((ip, port))) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("error: cannot bind {ip}:{port}: {e}");
            return 1;
        }
    };
    let local = listener
        .local_addr()
        .unwrap_or(SocketAddr::from((ip, port)));
    let display_ip = if ip == Ipv4Addr::UNSPECIFIED {
        util::lan_ip()
    } else {
        ip.to_string()
    };
    println!("wtf-hub v{VERSION} listening at http://{local}");
    println!(
        "dashboard: http://{display_ip}:{}/?k={}",
        local.port(),
        cfg.dashboard_key
    );
    println!("press Ctrl-C to stop");
    {
        use std::io::Write as _;
        let _ = std::io::stdout().flush();
    }

    if !has_flag(args, "--no-open") {
        let url = format!(
            "http://{}:{}/?k={}",
            display_ip,
            local.port(),
            cfg.dashboard_key
        );
        let _ = Command::new("xdg-open").arg(url).spawn();
    }

    http::serve(listener, handler);
    0
}

fn cmd_key(args: &[String]) -> i32 {
    let sub = args.first().map(|s| s.as_str()).unwrap_or("");
    let mut ks = match KeyStore::load() {
        Ok(k) => k,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    match sub {
        "issue" => {
            let Some(name) = args.iter().skip(1).find(|a| !a.starts_with('-')) else {
                eprintln!("usage: wtf key issue [--json] <name>");
                return 2;
            };
            let secret = match ks.issue(name) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("error: {e}");
                    return 1;
                }
            };
            let hub_url = HubConfig::load_or_create()
                .map(|c| c.lan_url())
                .unwrap_or_else(|_| "http://<hub-host>:7800".to_string());
            if has_flag(args, "--json") {
                let v = json::Value::obj(vec![
                    ("hub_url", json::Value::from(hub_url.as_str())),
                    ("device", json::Value::from(name.as_str())),
                    ("key", json::Value::from(secret.as_str())),
                ]);
                println!("{}", v.to_json());
                return 0;
            }
            println!("device '{name}' enrolled.");
            println!();
            println!("device key (shown once; store it now):");
            println!("  {secret}");
            println!();
            println!("on the device, either run:");
            println!("  wtf setup --url {hub_url} --name {name} --key {secret}");
            println!("or set these env vars for the MCP bridge:");
            println!("  WTF_HUB_URL={hub_url}");
            println!("  WTF_DEVICE_NAME={name}");
            println!("  WTF_DEVICE_KEY={secret}");
            0
        }
        "list" => {
            if ks.records.is_empty() {
                println!("no devices enrolled");
            }
            for r in &ks.records {
                let state = if r.revoked { "revoked" } else { "active" };
                println!("{:<24} {state:<8} created {}", r.name, r.created_at);
            }
            0
        }
        "revoke" => {
            let Some(name) = args.get(1) else {
                eprintln!("usage: wtf key revoke <name>");
                return 2;
            };
            match ks.revoke(name) {
                Ok(true) => {
                    println!("device '{name}' revoked; its key no longer authenticates.");
                    0
                }
                Ok(false) => {
                    eprintln!("error: no active device named '{name}'");
                    1
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    1
                }
            }
        }
        _ => {
            eprintln!("usage: wtf key <issue|list|revoke> ...");
            2
        }
    }
}

/// Shared enrollment path: verify hub reachability, persist bridge.json (0600),
/// then verify the credentials end-to-end with a signed state fetch.
fn run_setup(cfg: &BridgeConfig) -> Result<(), String> {
    match client::get_text(&format!("{}/healthz", cfg.hub_url)) {
        Ok((200, body)) if body.contains("wtf-hub") => {}
        Ok((status, _)) => {
            return Err(format!("hub responded HTTP {status} (is `wtf serve` running there?)"));
        }
        Err(e) => return Err(format!("cannot reach hub: {e}")),
    }
    cfg.save()?;
    mcp::fetch_state(cfg).map(|_| ())
}

fn cmd_setup(args: &[String]) -> i32 {
    let (url, name, key) = (
        flag_value(args, "--url"),
        flag_value(args, "--name"),
        flag_value(args, "--key"),
    );
    let (Some(url), Some(name), Some(key)) = (url, name, key) else {
        eprintln!("usage: wtf setup --url http://HUB:7800 --name DEVICE --key KEYHEX");
        return 2;
    };
    let cfg = BridgeConfig {
        hub_url: url.trim_end_matches('/').to_string(),
        device_name: name,
        device_key: key,
    };
    if let Err(e) = cfg.validate() {
        eprintln!("error: {e}");
        return 2;
    }
    if let Err(e) = run_setup(&cfg) {
        eprintln!("error: {e}");
        return 1;
    }
    println!("setup complete: bridge.json written, credentials verified against the hub.");
    println!();
    println!("add this to your MCP client configuration:");
    println!(r#"  {{ "command": "wtf", "args": ["agent"] }}"#);
    0
}

fn cmd_agent() -> i32 {
    let cfg = match BridgeConfig::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    mcp::run(cfg);
    0
}

fn cmd_status() -> i32 {
    let cfg = match BridgeConfig::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    match mcp::fetch_state(&cfg) {
        Ok(state) => {
            print!("{}", mcp::format_state(&state, &cfg.hub_url));
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// Distribute the portable hub skill to any project, repo, or agent
/// workspace: `wtf skill install [--dir DIR] [--force]`. Idempotent —
/// identical installs are a no-op; differing files need --force.
/// `wtf skill print` emits the raw SKILL.md for piping.
fn cmd_skill(args: &[String]) -> i32 {
    match args.first().map(|s| s.as_str()) {
        Some("print") => {
            print!("{AGENT_SKILL}");
            0
        }
        Some("install") => {
            let force = has_flag(args, "--force");
            let base = match flag_value(args, "--dir") {
                Some(d) => std::path::PathBuf::from(d),
                None => std::path::PathBuf::from("."),
            };
            let target = base
                .join(".agents")
                .join("skills")
                .join("wtf-agent-hub")
                .join("SKILL.md");
            if target.exists() {
                let existing = std::fs::read_to_string(&target).unwrap_or_default();
                if existing == AGENT_SKILL {
                    println!("already installed (identical): {}", target.display());
                    return 0;
                }
                if !force {
                    eprintln!(
                        "error: {} exists with different content; re-run with --force to overwrite",
                        target.display()
                    );
                    return 1;
                }
            }
            if let Some(parent) = target.parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    eprintln!("error: cannot create {}: {e}", parent.display());
                    return 1;
                }
            }
            match std::fs::write(&target, AGENT_SKILL) {
                Ok(_) => {
                    println!("skill installed: {}", target.display());
                    println!();
                    println!("point agents at .agents/skills/wtf-agent-hub/SKILL.md, then register the bridge:");
                    println!(r#"  {{ "command": "<absolute path to>/wtf", "args": ["agent"] }}"#);
                    0
                }
                Err(e) => {
                    eprintln!("error: cannot write {}: {e}", target.display());
                    1
                }
            }
        }
        _ => {
            eprintln!("usage: wtf skill <install [--dir DIR] [--force] | print>");
            2
        }
    }
}

/// Print the full dashboard URL (including the `?k=` key) for the operator
/// sitting on the hub machine. The key already lives in this machine's
/// config.json — this just saves retyping it. Agents can never fetch this
/// over MCP; they only get the hub address via the `hub_info` tool.
fn cmd_dashboard_url() -> i32 {
    let cfg = match HubConfig::load_or_create() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let bind: Ipv4Addr = match cfg.bind_ip.parse() {
        Ok(i) => i,
        Err(_) => {
            eprintln!("error: bad bind ip '{}' in config.json", cfg.bind_ip);
            return 2;
        }
    };
    println!(
        "dashboard: http://localhost:{}/?k={}",
        cfg.port, cfg.dashboard_key
    );
    if bind == Ipv4Addr::UNSPECIFIED {
        println!(
            "from other hosts: http://{}:{}/?k={}",
            util::lan_ip(),
            cfg.port,
            cfg.dashboard_key
        );
    }
    0
}

fn cmd_url(args: &[String]) -> i32 {
    match args.first().map(|s| s.as_str()) {
        None => match HubConfig::load_or_create() {
            Ok(c) => {
                println!("advertised url: {}", c.advertised_url.as_deref().unwrap_or("(not set; auto-detecting LAN address)"));
                0
            }
            Err(e) => {
                eprintln!("error: {e}");
                1
            }
        },
        Some("clear") => match HubConfig::set_advertised_url(None) {
            Ok(_) => {
                println!("advertised url cleared; joining devices get the auto-detected LAN URL.");
                0
            }
            Err(e) => {
                eprintln!("error: {e}");
                1
            }
        },
        Some(u) => match HubConfig::set_advertised_url(Some(u.to_string())) {
            Ok(c) => {
                println!("advertised url set: {}", c.advertised_url.as_deref().unwrap_or(""));
                println!("`wtf key issue` and `wtf join` now hand out this URL (overlay IP or public https host).");
                0
            }
            Err(e) => {
                eprintln!("error: {e}");
                1
            }
        },
    }
}

/// Enroll THIS machine by executing the enrollment on the hub over ssh.
/// The one-time device secret travels only inside the ssh channel; nothing is
/// written to intermediate files. Usage:
///   wtf join user@hub-host [--name DEVICE] [--url URL]
fn cmd_join(args: &[String]) -> i32 {
    let target = match args.first() {
        Some(t) if !t.trim().is_empty() && !t.starts_with('-') => t.trim(),
        _ => {
            eprintln!("usage: wtf join user@hub-host [--name DEVICE] [--url URL]");
            return 2;
        }
    };
    let name = flag_value(args, "--name").unwrap_or_else(|| format!("box-{}", wtf::rand::hex(3)));
    // The name is embedded in the remote shell command: restrict the charset
    // before it ever leaves this machine (also what the hub will validate).
    if !config::valid_name(&name) {
        eprintln!("error: --name must match [A-Za-z0-9._-]{{1,64}}");
        return 2;
    }
    let url_override = flag_value(args, "--url");

    eprintln!("enrolling '{name}' via ssh {target} ...");
    let remote_cmd = format!("wtf key issue --json {name}");
    let out = match Command::new("ssh")
        .args([
            "-o",
            "BatchMode=yes",
            "-o",
            "StrictHostKeyChecking=accept-new",
            target,
            "--",
            &remote_cmd,
        ])
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            eprintln!("error: cannot run ssh: {e}");
            return 1;
        }
    };
    if !out.status.success() {
        eprintln!(
            "error: remote enrollment failed (exit {:?}): {}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
        eprintln!("hint: sshd up on the hub? key authorized? `wtf` in the remote PATH?");
        return 1;
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let line = stdout
        .lines()
        .find(|l| l.trim_start().starts_with('{'))
        .unwrap_or("");
    let parsed = match json::parse(line.trim()) {
        Ok(v) => v,
        Err(_) => {
            eprintln!("error: remote did not return JSON; the hub's `wtf` is too old (needs `key issue --json`).");
            return 1;
        }
    };
    let get = |k: &str| {
        parsed
            .get(k)
            .and_then(|x| x.as_str())
            .map(|s| s.to_string())
    };
    let (remote_url, key) = (get("hub_url"), get("key"));
    let (Some(mut hub_url), Some(key)) = (remote_url, key) else {
        eprintln!("error: remote JSON missing hub_url/key fields.");
        return 1;
    };
    if let Some(u) = &url_override {
        hub_url = u.trim_end_matches('/').to_string();
    }
    let cfg = BridgeConfig {
        hub_url,
        device_name: name,
        device_key: key,
    };
    if let Err(e) = cfg.validate() {
        eprintln!("error: {e}");
        return 2;
    }
    if let Err(e) = run_setup(&cfg) {
        eprintln!("error: {e}");
        return 1;
    }
    println!(
        "joined: enrolled as '{}' reaching the hub at {}.",
        cfg.device_name, cfg.hub_url
    );
    println!();
    println!("add this to your MCP client configuration:");
    println!(r#"  {{ "command": "wtf", "args": ["agent"] }}"#);
    0
}
