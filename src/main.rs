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
use wtf::hmac;
use wtf::http;
use wtf::identity;
use wtf::json;
use wtf::mcp;
use wtf::session_crypto;
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
        Some("enroll-token") => cmd_enroll_token(&args[1..]),
        Some("enroll") => cmd_enroll(&args[1..]),
        Some("enroll-secret") => cmd_enroll_secret(&args[1..]),
        Some("bin") => cmd_bin(&args[1..]),
        Some("sessions") => cmd_sessions(&args[1..]),
        Some("federate") => cmd_federate(&args[1..]),
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
    println!("  wtf enroll-token <name> [--ttl SECS]       mint a one-time enrollment token (hub side)");
    println!("  wtf enroll --url URL --name N --token T    redeem a token to enroll this machine");
    println!("  wtf enroll --url URL --name N --psk S      signed-handshake enroll (key arrives sealed)");
    println!("  wtf enroll-secret [--rotate] [--json]      print/rotate the site enrollment secret (hub)");
    println!("  wtf bin ls [--url U] [--k K] [--json]      operator bin courier: list bins");
    println!("  wtf bin get N [-o F] [--url U] [--k K]     read a bin raw to stdout (pre-setup OK)");
    println!("  wtf bin put N TEXT|--file F|- [--url U] [--k K]   write a bin (dashboard-key gated)");
    println!("  wtf federate add <name> --url URL --psk SECRET [--as DEV]  link a peer hub (one secret copy per edge)");
    println!("  wtf federate list                         show the federation peer table");
    println!("  wtf sessions [--url U] [--k K]            list session chats (id, name, repo, members); adds local pairing keys on the hub machine");
    println!("  wtf federate remove <name>                unlink a peer hub");
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
    let capability = match wtf::federation::load_or_create_capability() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };

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

    // Federation: load the peer table, mint the hub name on first serve,
    // and stamp it as the event origin. Spawn replication only when peers
    // exist (a lone hub has nothing to sync).
    let mut fed = wtf::federation::FedConfig::load();
    let _ = fed.ensure_name();
    store.set_origin_name(&fed.name);
    let fed_arc = Arc::new(Mutex::new(fed.clone()));
    let fed_name_for_rep = fed.name.clone();
    let rep_store = Arc::clone(&store);
    let rep_fed = Arc::clone(&fed_arc);
    let rep_peers = fed.peers.len();
    if rep_peers > 0 {
        wtf::replicate::spawn(rep_store, fed_name_for_rep, rep_fed);
    }

    let hub = Arc::new(wtf::api::Hub {
        store,
        bins,
        keys: Mutex::new(keys),
        nonces: Mutex::new(wtf::auth::NonceCache::new()),
        dashboard_key: cfg.dashboard_key.clone(),
        started_at: util::now_secs(),
        identities: Mutex::new(Vec::new()),
        sessions: wtf::sessions::Sessions::load(),
        enroll_hits: Mutex::new(Vec::new()),
        enroll_nonces: Mutex::new(Vec::new()),
        fed_name: fed.name.clone(),
        fed: fed_arc,
        env_reports: Mutex::new(Vec::new()),
        capability: capability.clone(),
        loopback_only: ip.is_loopback(),
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
    if ip.is_loopback() {
        println!("dashboard: http://localhost:{}/w/{}", local.port(), capability);
        println!("(loopback-only: the capability path is the gate; LAN cannot reach this hub)");
    } else {
        println!(
            "dashboard: http://{display_ip}:{}/?k={}",
            local.port(),
            cfg.dashboard_key
        );
        println!("(LAN-visible: legacy dashboard-key gate; `wtf dashboard-url` prints the local capability link)");
    }
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

/// Mint a one-time enrollment token (hub machine). The token is printed once;
/// only its SHA-256 hash is stored. It expires on its own, burns on
/// redemption, and can be dropped early with `enroll-token revoke <name>` —
/// so it can travel to the joining device over any channel.
fn cmd_enroll_token(args: &[String]) -> i32 {
    if args.first().map(|s| s.as_str()) == Some("revoke") {
        let Some(name) = args.get(1) else {
            eprintln!("usage: wtf enroll-token revoke <name>");
            return 2;
        };
        let mut tokens = match config::EnrollTokenStore::load() {
            Ok(t) => t,
            Err(e) => {
                eprintln!("error: {e}");
                return 1;
            }
        };
        if tokens.revoke(name) {
            println!("pending enrollment tokens for '{name}' dropped.");
            return 0;
        }
        eprintln!("error: no pending enrollment token for '{name}'");
        return 1;
    }
    let Some(name) = args.iter().find(|a| !a.starts_with('-')) else {
        eprintln!("usage: wtf enroll-token <name> [--ttl SECS] [--json] | wtf enroll-token revoke <name>");
        return 2;
    };
    let ttl: u64 = flag_value(args, "--ttl")
        .and_then(|v| v.parse().ok())
        .unwrap_or(600);
    let mut tokens = match config::EnrollTokenStore::load() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let token = match tokens.issue(name, ttl) {
        Ok(t) => t,
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
            ("token", json::Value::from(token.as_str())),
            ("expires_in", json::Value::from(ttl as i64)),
        ]);
        println!("{}", v.to_json());
        return 0;
    }
    println!("enrollment token for '{name}' (valid {ttl}s, shown once):");
    println!("  {token}");
    println!();
    println!("on the joining device, run:");
    println!("  wtf enroll --url {hub_url} --name {name} --token {token}");
    0
}

/// Enroll this machine over HTTP. Two modes:
/// - `--token T` (v0.8.0): the single-use token is the credential; the fresh
///   device key crosses in the one-time response.
/// - `--psk S` (v0.9.0): signed handshake — we prove possession of the site
///   secret via HMAC (the secret itself never travels) and receive the device
///   key ML-KEM-768-sealed to this machine's encapsulation key.
/// `--url` is the address to store — use it to override the hub's
/// auto-detected/advertised address (overlay IPs, NAT, TLS proxies).
fn cmd_enroll(args: &[String]) -> i32 {
    let url = match flag_value(args, "--url") {
        Some(u) => u.trim_end_matches('/').to_string(),
        None => {
            eprintln!("usage: wtf enroll --url http://HUB:7800 --name DEVICE (--token TOKEN | --psk SECRET)");
            return 2;
        }
    };
    let name = match flag_value(args, "--name") {
        Some(n) => n,
        None => {
            eprintln!("error: --name is required");
            return 2;
        }
    };
    if !config::valid_name(&name) {
        eprintln!("error: --name must match [A-Za-z0-9._-]{{1,64}}");
        return 2;
    }
    let token = flag_value(args, "--token");
    let psk = flag_value(args, "--psk");
    let key = match (token, psk) {
        (Some(_), Some(_)) => {
            eprintln!("error: --token and --psk are mutually exclusive");
            return 2;
        }
        (Some(t), None) => match redeem_token(&url, &name, &t) {
            Ok(k) => k,
            Err(e) => {
                eprintln!("error: {e}");
                return 1;
            }
        },
        (None, Some(s)) => match redeem_psk(&url, &name, &s) {
            Ok(k) => k,
            Err(e) => {
                eprintln!("error: {e}");
                return 1;
            }
        },
        (None, None) => {
            eprintln!("usage: wtf enroll --url http://HUB:7800 --name DEVICE (--token TOKEN | --psk SECRET)");
            return 2;
        }
    };
    let cfg = BridgeConfig {
        hub_url: url,
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
        "enrolled: '{}' reaching the hub at {}.",
        cfg.device_name, cfg.hub_url
    );
    println!();
    println!("add this to your MCP client configuration:");
    println!(r#"  {{ "command": "wtf", "args": ["agent"] }}"#);
    0
}

/// Token mode: POST { name, token }, expect the one-time `key` response.
fn redeem_token(url: &str, name: &str, token: &str) -> Result<String, String> {
    let body = json::Value::obj(vec![
        ("name", json::Value::from(name)),
        ("token", json::Value::from(token)),
    ]);
    eprintln!("enrolling '{name}' at {url} (token) ...");
    let resp = client::request(
        &format!("{url}/api/v1/enroll"),
        "POST",
        &[],
        body.to_json().as_bytes(),
    )
    .map_err(|e| format!("cannot reach hub: {e}"))?;
    if resp.status != 200 {
        eprintln!("enrollment refused (HTTP {}): {}", resp.status, resp.text().trim());
        return Err("tokens are single-use and expire; ask the hub operator for a fresh `wtf enroll-token`.".into());
    }
    let parsed = resp.json().ok_or("hub returned a non-JSON response")?;
    parsed
        .get("key")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .ok_or("hub response missing key field".into())
}

/// PSK mode: prove possession of the site secret with an HMAC over the
/// handshake transcript, then open the ML-KEM-768-sealed key package that
/// comes back. The secret never crosses the wire; the device key arrives
/// sealed and is unwrapped only in memory.
fn redeem_psk(url: &str, name: &str, psk: &str) -> Result<String, String> {
    let psk = psk.trim().to_lowercase();
    if psk.len() != 64 || !psk.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("--psk must be the 64-hex site enrollment secret (print it on the hub with `wtf enroll-secret`)".into());
    }
    let id = identity::load_or_create()?;
    let ek = util::hex_encode(&id.ek);
    let ts = util::now_secs();
    let nonce = wtf::rand::hex(16);
    let proof = hmac::hmac_sha256_hex(
        psk.as_bytes(),
        format!("wtf-enroll-v2\n{name}\n{ek}\n{ts}\n{nonce}").as_bytes(),
    );
    let body = json::Value::obj(vec![
        ("name", json::Value::from(name)),
        ("ek", json::Value::from(ek.as_str())),
        ("ts", json::Value::from(ts as i64)),
        ("nonce", json::Value::from(nonce.as_str())),
        ("proof", json::Value::from(proof.as_str())),
    ]);
    eprintln!("enrolling '{name}' at {url} (signed handshake) ...");
    let resp = client::request(
        &format!("{url}/api/v1/enroll"),
        "POST",
        &[],
        body.to_json().as_bytes(),
    )
    .map_err(|e| format!("cannot reach hub: {e}"))?;
    if resp.status != 200 {
        eprintln!("enrollment refused (HTTP {}): {}", resp.status, resp.text().trim());
        return Err("handshake rejected: wrong/expired secret, stale clock, replayed handshake, or the secret was rotated (`wtf enroll-secret --rotate` invalidates every copy)".into());
    }
    let parsed = resp.json().ok_or("hub returned a non-JSON response")?;
    let Some(sealed) = parsed.get("sealed").and_then(|x| x.as_str()) else {
        return Err("hub response missing sealed key package".into());
    };
    let key32 = session_crypto::open_sealed_package(
        sealed,
        &id.dk,
        &format!("wtf-enroll-v2:{name}"),
    )?;
    Ok(util::hex_encode(&key32))
}

/// Hub-side: print (or rotate) the site enrollment secret that the operator
/// copies once to each joining machine (`wtf enroll --psk`).
fn cmd_enroll_secret(args: &[String]) -> i32 {
    let rotate = args.iter().any(|a| a == "--rotate");
    let as_json = args.iter().any(|a| a == "--json");
    let secret = if rotate {
        match HubConfig::rotate_enroll_secret() {
            Ok(s) => {
                eprintln!("rotated: every previously copied secret is now invalid.");
                s
            }
            Err(e) => {
                eprintln!("error: {e}");
                return 1;
            }
        }
    } else {
        match HubConfig::load_or_create() {
            Ok(c) => c.enroll_secret,
            Err(e) => {
                eprintln!("error: {e}");
                return 1;
            }
        }
    };
    if as_json {
        let url = HubConfig::load_or_create().map(|c| c.lan_url()).unwrap_or_default();
        println!(
            "{}",
            json::Value::obj(vec![
                ("hub_url", json::Value::from(url.as_str())),
                ("enroll_secret", json::Value::from(secret.as_str())),
            ])
            .to_json()
        );
        return 0;
    }
    let url = HubConfig::load_or_create().map(|c| c.lan_url()).unwrap_or_default();
    println!("site enrollment secret (copy once per joining machine):");
    println!("{secret}");
    println!();
    println!("then on the joining machine:");
    println!("  wtf enroll --url {url} --name DEVICE --psk {secret}");
    println!();
    println!("rotate with `wtf enroll-secret --rotate` to invalidate every outstanding copy.");
    0
}

/// Operator session-chat view: lists every chat on the hub (id, name,
/// paired repo, members, messages) so the operator can see which chat maps
/// to which repo and hand out pairing keys. On the machine that CREATED a
/// chat (the creator's session_keys.json holds its pairing key), the local
/// pairing keys are printed too — that is the key the operator copies to
/// the other machine/agent. Hub URL + key resolve like `wtf bin`.
fn cmd_sessions(args: &[String]) -> i32 {
    let rest = &args[..];
    let url_flag = flag_value(rest, "--url");
    let k_flag = flag_value(rest, "--k");
    let home = config::home();
    let hub_url = match url_flag {
        Some(u) => Some(u.trim_end_matches('/').to_string()),
        None => match std::env::var("WTF_HUB_URL") {
            Ok(u) if !u.trim().is_empty() => Some(u.trim().trim_end_matches('/').to_string()),
            _ => {
                let bridge = home.join("bridge.json");
                let cfg = home.join("config.json");
                if bridge.exists() {
                    read_json_field(&bridge, "hub_url")
                } else if cfg.exists() {
                    HubConfig::load_or_create_at(&cfg).map(|c| c.lan_url()).ok()
                } else {
                    None
                }
            }
        },
    };
    let Some(hub_url) = hub_url else {
        eprintln!("error: no hub URL: pass --url, set WTF_HUB_URL, or run where bridge.json/config.json exists");
        return 2;
    };
    let key = match k_flag {
        Some(k) => Some(k),
        None => match std::env::var("WTF_DASHBOARD_KEY") {
            Ok(k) if !k.trim().is_empty() => Some(k.trim().to_string()),
            _ => {
                let cfg = home.join("config.json");
                if cfg.exists() {
                    read_json_field(&cfg, "dashboard_key")
                } else {
                    None
                }
            }
        },
    };
    let Some(key) = key else {
        eprintln!("error: no dashboard key: pass --k or set WTF_DASHBOARD_KEY");
        return 2;
    };
    let resp = match client::request(
        &format!("{hub_url}/api/v1/sessions?k={key}"),
        "GET",
        &[],
        b"",
    ) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: cannot reach hub: {e}");
            return 1;
        }
    };
    if resp.status != 200 {
        eprintln!("error: hub refused (HTTP {}): {}", resp.status, resp.text().trim());
        return 1;
    }
    let Some(v) = resp.json() else {
        eprintln!("error: hub returned a non-JSON response");
        return 1;
    };
    // Local pairing keys (creator machine only; file is 0600).
    let local_pairings: Vec<(String, String)> = crate::config::load_json(&home.join("session_keys.json"))
        .ok()
        .flatten()
        .and_then(|v| {
            v.get("pairings").and_then(|x| x.as_obj()).map(|o| {
                o.iter()
                    .filter_map(|(k, val)| val.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
        })
        .unwrap_or_default();
    println!("AGENT CHATS (id · name · repo · members · msgs)");
    let sessions = v.get("sessions").and_then(|x| x.as_arr()).unwrap_or(&[]);
    if sessions.is_empty() {
        println!("  (none — an agent creates one with session_create)");
    }
    for s in sessions {
        let id = s.get("id").and_then(|x| x.as_str()).unwrap_or("?");
        let name = s.get("name").and_then(|x| x.as_str()).unwrap_or("?");
        let repo = s.get("repo").and_then(|x| x.as_str()).unwrap_or("");
        let members = s.get("members").and_then(|x| x.as_arr()).map(|a| a.len()).unwrap_or(0);
        let msgs = s.get("msg_count").and_then(|x| x.as_i64()).unwrap_or(0);
        let mut member_names: Vec<String> = s
            .get("members")
            .and_then(|x| x.as_arr())
            .map(|a| {
                a.iter()
                    .filter_map(|m| m.get("device").and_then(|v| v.as_str()).map(|d| d.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        member_names.sort();
        println!(
            "  {id} · '{name}' · repo: {} · {members} member(s) [{}] · {msgs} msg(s)",
            if repo.is_empty() { "-" } else { repo },
            member_names.join(", ")
        );
        if let Some((_, pk)) = local_pairings.iter().find(|(sid, _)| sid == id) {
            println!("    pairing key (copy to joiners on other machines): {pk}");
        }
    }
    0
}

/// Federation: link/unlink peer hubs. `add` runs the PSK handshake against
/// the peer (the peer's SITE secret is the one secret you copy; this hub's
/// device credential on the peer arrives ML-KEM-768-sealed and the peer
/// simultaneously learns nothing else), records the peer in federation.json
/// (0600), and verifies the link with a signed round-trip. Both hubs
/// replicate after this single command on ONE side.
fn cmd_federate(args: &[String]) -> i32 {
    let usage = "usage:\n  wtf federate add <name> --url URL --psk SECRET [--as DEV]\n  wtf federate list\n  wtf federate remove <name>";
    let Some(op) = args.first().map(|s| s.as_str()) else {
        eprintln!("{usage}");
        return 2;
    };
    let rest = &args[1..];
    match op {
        "list" => {
            let fed = wtf::federation::FedConfig::load();
            if fed.name.is_empty() && fed.peers.is_empty() {
                println!("no federation configured (this hub has no name yet)");
                return 0;
            }
            println!("this hub: {}", if fed.name.is_empty() { "(unnamed)" } else { &fed.name });
            if fed.peers.is_empty() {
                println!("no peers linked");
            }
            for p in &fed.peers {
                println!("  {} — {} (device {})", p.name, p.url, p.device);
            }
            0
        }
        "remove" => {
            let Some(name) = rest.first() else {
                eprintln!("{usage}");
                return 2;
            };
            let mut fed = wtf::federation::FedConfig::load();
            let before = fed.peers.len();
            fed.peers.retain(|p| &p.name != name);
            if fed.peers.len() == before {
                eprintln!("error: no peer named '{name}'");
                return 1;
            }
            if let Err(e) = fed.save() {
                eprintln!("error: {e}");
                return 1;
            }
            println!("peer '{name}' unlinked; restart the hub to stop replication to it.");
            0
        }
        "add" => {
            let Some(name) = rest.iter().find(|a| !a.starts_with('-')).cloned() else {
                eprintln!("{usage}");
                return 2;
            };
            if !config::valid_name(&name) || name.len() > 32 {
                eprintln!("error: peer name must match [A-Za-z0-9._-]{{1,32}}");
                return 2;
            }
            let Some(url) = flag_value(rest, "--url") else {
                eprintln!("error: --url is required (the peer hub's address)");
                return 2;
            };
            let Some(psk) = flag_value(rest, "--psk") else {
                eprintln!("error: --psk is required (the peer's site secret from `wtf enroll-secret` there)");
                return 2;
            };
            let url = url.trim_end_matches('/').to_string();
            let mut fed = wtf::federation::FedConfig::load();
            let hub_name = fed.ensure_name().unwrap_or_default();
            let device = match flag_value(rest, "--as") {
                Some(d) => d,
                None => format!("{}{hub_name}", wtf::federation::FED_NAME_PREFIX),
            };
            if !config::valid_name(&device) {
                eprintln!("error: --as must match [A-Za-z0-9._-]{{1,64}}");
                return 2;
            }
            if fed.find_peer(&name).is_some() {
                eprintln!("error: peer '{name}' already linked (remove it first)");
                return 1;
            }
            // PSK handshake against the peer, as a device named <device>.
            let key = match redeem_psk(&url, &device, &psk) {
                Ok(k) => k,
                Err(e) => {
                    eprintln!("error: handshake with peer failed: {e}");
                    return 1;
                }
            };
            fed.peers.push(wtf::federation::Peer {
                name: name.clone(),
                url: url.clone(),
                device: device.clone(),
                device_key: key,
                added_at: util::now_secs(),
            });
            if let Err(e) = fed.save() {
                eprintln!("error: cannot persist federation.json: {e}");
                return 1;
            }
            // Adopt the peer's REAL federation identity: ask it (signed
            // with the credential it just issued) for its fed name. The
            // peer's origin-stamped events carry that name, so pull cursors
            // must address it, not the operator's label.
            let peer_key = fed.peers.last().unwrap().device_key.clone();
            let real_name = (|| -> Option<String> {
                let ts = util::now_secs();
                let nonce = wtf::rand::hex(16);
                let sig = hmac::hmac_sha256_hex(
                    &util::hex_decode(&peer_key)?,
                    format!("wtf-hmac-v1\nGET\n/api/v1/fed/peers\n{ts}\n{nonce}\n{}",
                        wtf::sha256::hexdigest(b"")).as_bytes(),
                );
                let headers = vec![
                    ("X-Wtf-Device".to_string(), device.clone()),
                    ("X-Wtf-Timestamp".to_string(), ts.to_string()),
                    ("X-Wtf-Nonce".to_string(), nonce),
                    ("X-Wtf-Signature".to_string(), sig),
                ];
                let resp = client::request(
                    &format!("{url}/api/v1/fed/peers"), "GET", &headers, b"",
                ).ok()?;
                let v = resp.json()?;
                v.get("name").and_then(|x| x.as_str()).map(|s| s.to_string())
            })();
            let peer_name = match real_name {
                Some(n) if !n.is_empty() && config::valid_name(&n) && n != fed.name => n,
                _ => name.clone(),
            };
            if peer_name != name {
                println!("peer identity: {} (label {name})", peer_name);
            }
            // Rewrite the stored peer with the real name.
            fed.peers.pop();
            fed.peers.push(wtf::federation::Peer {
                name: peer_name.clone(),
                url: url.clone(),
                device: device.clone(),
                device_key: peer_key.clone(),
                added_at: util::now_secs(),
            });
            let _ = fed.save();

            // Verify with a signed round-trip on the new credential.
            let peer = wtf::federation::Peer {
                name: peer_name.clone(),
                url: url.clone(),
                device: device.clone(),
                device_key: peer_key.clone(),
                added_at: 0,
            };
            let rep = wtf::replicate::Replicator {
                store: Arc::new(Store::new(&std::env::temp_dir().join(format!("wtf-fedverify-{}", wtf::rand::hex(4)))).expect("tmp store")),
                hub_url: String::new(),
                hub_name,
                fed: Arc::new(Mutex::new(wtf::federation::FedConfig::default())),
                nonces: Mutex::new(std::collections::HashMap::new()),
                last_warn: Mutex::new(std::collections::HashMap::new()),
                push_gen: Arc::new(std::sync::atomic::AtomicU64::new(0)),
                wake: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            };
            match wtf::replicate::anti_entropy(&rep, &peer, &rep.push_gen) {
                Ok(_) => {
                    println!("peer '{peer_name}' linked at {url} (device '{device}').");
                    println!("restart the hub to begin replication; both hubs now carry the full ledger.");
                    0
                }
                Err(e) => {
                    eprintln!("warning: link saved but first sync failed: {e}");
                    eprintln!("replication will retry automatically once the hub restarts.");
                    0
                }
            }
        }
        _ => {
            eprintln!("{usage}");
            2
        }
    }
}

/// Operator bin courier: read/write the hub's three paste-bins with the
/// dashboard key — no enrolled agent required, so this works pre-setup on
/// any machine or harness (the hub records "dashboard" as the last writer).
/// `get` prints raw content to stdout (pipe/copy friendly); `put` takes a
/// positional TEXT, `--file F`, or `-` for stdin. Hub URL resolves from
/// --url, $WTF_HUB_URL, bridge.json, or the local hub config; the key from
/// --k, $WTF_DASHBOARD_KEY, or the local hub config. Prefer the env var:
/// a key passed as --k can leak through shell history.
fn cmd_bin(args: &[String]) -> i32 {
    let usage = "usage:\n  wtf bin ls [--url U] [--k K] [--json]\n  wtf bin get N [-o FILE] [--url U] [--k K]\n  wtf bin put N (TEXT | --file F | -) [--url U] [--k K] [--json]";
    let Some(op) = args.first().map(|s| s.as_str()) else {
        eprintln!("{usage}");
        return 2;
    };
    if !matches!(op, "ls" | "get" | "put") {
        eprintln!("{usage}");
        return 2;
    }
    let rest = &args[1..];
    let url_flag = flag_value(rest, "--url");
    let k_flag = flag_value(rest, "--k");
    let out_flag = flag_value(rest, "-o");
    let file_flag = flag_value(rest, "--file");
    let as_json = rest.iter().any(|a| a == "--json");

    let home = config::home();
    let hub_url = match url_flag {
        Some(u) => Some(u.trim_end_matches('/').to_string()),
        None => match std::env::var("WTF_HUB_URL") {
            Ok(u) if !u.trim().is_empty() => Some(u.trim().trim_end_matches('/').to_string()),
            _ => {
                let bridge = home.join("bridge.json");
                let cfg = home.join("config.json");
                if bridge.exists() {
                    read_json_field(&bridge, "hub_url")
                } else if cfg.exists() {
                    HubConfig::load_or_create_at(&cfg).map(|c| c.lan_url()).ok()
                } else {
                    None
                }
            }
        },
    };
    let Some(hub_url) = hub_url else {
        eprintln!("error: no hub URL: pass --url, set WTF_HUB_URL, or run where bridge.json/config.json exists");
        return 2;
    };

    let key = match k_flag {
        Some(k) => Some(k),
        None => match std::env::var("WTF_DASHBOARD_KEY") {
            Ok(k) if !k.trim().is_empty() => Some(k.trim().to_string()),
            _ => {
                let cfg = home.join("config.json");
                if cfg.exists() {
                    read_json_field(&cfg, "dashboard_key")
                } else {
                    None
                }
            }
        },
    };
    let Some(key) = key else {
        eprintln!("error: no dashboard key: pass --k or set WTF_DASHBOARD_KEY (argv can leak via shell history; prefer the env var)");
        return 2;
    };

    let bin_id = |rest: &[String]| -> Option<u8> {
        rest.first()
            .and_then(|s| s.parse::<u8>().ok())
            .filter(|n| (1..=3).contains(n))
    };

    match op {
        "ls" => {
            let resp = match client::request(
                &format!("{hub_url}/api/v1/bins?k={key}"),
                "GET",
                &[],
                b"",
            ) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("error: cannot reach hub: {e}");
                    return 1;
                }
            };
            if resp.status != 200 {
                eprintln!("error: hub refused (HTTP {}): {}", resp.status, resp.text().trim());
                return 1;
            }
            let Some(v) = resp.json() else {
                eprintln!("error: hub returned a non-JSON response");
                return 1;
            };
            if as_json {
                println!("{}", v.to_json());
                return 0;
            }
            let bins = v.get("bins").and_then(|x| x.as_arr()).unwrap_or(&[]);
            for b in bins {
                let id = b.get("id").and_then(|x| x.as_i64()).unwrap_or(0);
                let size = b.get("size").and_then(|x| x.as_i64()).unwrap_or(0);
                let by = b.get("updated_by").and_then(|x| x.as_str()).unwrap_or("?");
                println!("bin {id}: {size} chars, by {by}");
            }
            0
        }
        "get" => {
            let Some(id) = bin_id(rest) else {
                eprintln!("{usage}");
                return 2;
            };
            let resp = match client::request(
                &format!("{hub_url}/api/v1/bins/{id}?k={key}"),
                "GET",
                &[],
                b"",
            ) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("error: cannot reach hub: {e}");
                    return 1;
                }
            };
            if resp.status != 200 {
                eprintln!("error: hub refused (HTTP {}): {}", resp.status, resp.text().trim());
                return 1;
            }
            let Some(content) = resp
                .json()
                .and_then(|v| v.get("content").and_then(|x| x.as_str()).map(|s| s.to_string()))
            else {
                eprintln!("error: hub response missing content");
                return 1;
            };
            match out_flag {
                Some(f) => {
                    if let Err(e) = std::fs::write(&f, &content) {
                        eprintln!("error: cannot write {f}: {e}");
                        return 1;
                    }
                    println!("wrote {} chars to {f}", content.chars().count());
                }
                None => print!("{content}"),
            }
            0
        }
        "put" => {
            let Some(id) = bin_id(rest) else {
                eprintln!("{usage}");
                return 2;
            };
            let content = if let Some(f) = file_flag {
                match std::fs::read_to_string(&f) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("error: cannot read {f}: {e}");
                        return 1;
                    }
                }
            } else if rest.get(1).map(|s| s.as_str()) == Some("-") {
                let mut s = String::new();
                use std::io::Read;
                if let Err(e) = std::io::stdin().read_to_string(&mut s) {
                    eprintln!("error: cannot read stdin: {e}");
                    return 1;
                }
                s
            } else if let Some(t) = rest.get(1) {
                t.clone()
            } else {
                eprintln!("{usage}");
                return 2;
            };
            if content.chars().count() > 65_536 {
                eprintln!(
                    "error: content is {} chars; bins hold at most 65,536",
                    content.chars().count()
                );
                return 1;
            }
            let body = json::Value::obj(vec![("content", json::Value::from(content.as_str()))]);
            let resp = match client::request(
                &format!("{hub_url}/api/v1/bins/{id}?k={key}"),
                "PUT",
                &[],
                body.to_json().as_bytes(),
            ) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("error: cannot reach hub: {e}");
                    return 1;
                }
            };
            if resp.status != 200 {
                eprintln!("error: hub refused (HTTP {}): {}", resp.status, resp.text().trim());
                return 1;
            }
            if as_json {
                println!("{}", resp.text().trim());
            } else {
                println!("bin {id} updated ({} chars, by dashboard)", content.chars().count());
            }
            0
        }
        _ => {
            eprintln!("{usage}");
            2
        }
    }
}

/// Read one string field from a JSON file without creating or upgrading it.
fn read_json_field(path: &std::path::Path, field: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let v = json::parse(text.trim()).ok()?;
    v.get(field).and_then(|x| x.as_str()).map(|s| s.to_string())
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
    let cap = match wtf::federation::load_or_create_capability() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let loopback = bind.is_loopback();
    if loopback {
        println!("dashboard: http://localhost:{}/w/{cap}", cfg.port);
        println!("(loopback-only hub — this link opens ONLY on this machine)");
    } else {
        println!("dashboard: http://localhost:{}/w/{cap}", cfg.port);
        println!(
            "from other hosts: http://{}:{}/?k={} (LAN path uses the dashboard key)",
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
