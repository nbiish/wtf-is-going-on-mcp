//! `wtf-live` — one localhost view of every cross-machine agent session.
//!
//!   wtf-live                       serve the page (same as `serve`)
//!   wtf-live serve [--port N]      run the hub
//!   wtf-live bin put|ls|get|rm     the shared pastebin
//!   wtf-live snapshot              one-shot text view
//!   wtf-live doctor                what the hub can see right now

use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use wtf_live::bins::Store;
use wtf_live::json::Value;
use wtf_live::model::Snapshot;
use wtf_live::server::{self, State};
use wtf_live::sources::Sources;
use wtf_live::{DEFAULT_PORT, VERSION};

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None | Some("serve") => cmd_serve(&args[1..]),
        Some("bin") => cmd_bin(&args[1..]),
        Some("snapshot") | Some("status") => cmd_snapshot(&args[1..]),
        Some("doctor") => cmd_doctor(),
        Some("version") | Some("--version") | Some("-V") => {
            println!("wtf-live {VERSION}");
            0
        }
        Some("help") | Some("--help") | Some("-h") => {
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
    println!("wtf-live {VERSION} — what the fuck is going on, across all my machines");
    println!();
    println!("USAGE:");
    println!(
        "  wtf-live serve [--port N] [--bind IP] [--data-dir DIR] [--no-open] [--allow-remote]"
    );
    println!("  wtf-live bin put [--title T] [--file -|PATH]   paste from stdin or a file");
    println!("  wtf-live bin ls [--json]                       list bins");
    println!("  wtf-live bin get <id>                          print a bin body");
    println!("  wtf-live bin rm <id>                           delete a bin");
    println!("  wtf-live snapshot [--json] [--fresh]           one-shot session view");
    println!("  wtf-live doctor                                sources, paths, counts");
    println!();
    println!("ENV:");
    println!("  WTF_LIVE_HOME     state dir (default ~/.wtf-live)");
    println!("  WTF_LIVE_MACHINE  this machine's display name (default: hostname -s)");
    println!("  ORCA_BIN          path to the orca CLI");
    println!("  HERDR_BIN         path to the herdr CLI");
}

// ------------------------------------------------------------------ serve

fn cmd_serve(args: &[String]) -> i32 {
    let mut port = DEFAULT_PORT;
    let mut bind = "127.0.0.1".to_string();
    let mut data_dir: Option<String> = None;
    let mut open = true;
    let mut allow_remote = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--port" | "-p" => {
                i += 1;
                match args.get(i).and_then(|v| v.parse::<u16>().ok()) {
                    Some(p) => port = p,
                    None => return usage_error("--port needs a number"),
                }
            }
            "--bind" => {
                i += 1;
                match args.get(i) {
                    Some(v) => bind = v.clone(),
                    None => return usage_error("--bind needs an address"),
                }
            }
            "--data-dir" => {
                i += 1;
                match args.get(i) {
                    Some(v) => data_dir = Some(v.clone()),
                    None => return usage_error("--data-dir needs a path"),
                }
            }
            "--no-open" => open = false,
            "--open" => open = true,
            "--allow-remote" => allow_remote = true,
            other => return usage_error(&format!("unknown flag: {other}")),
        }
        i += 1;
    }

    let loopback = bind == "127.0.0.1" || bind == "::1" || bind == "localhost";
    if !loopback && !allow_remote {
        eprintln!("refusing to bind {bind}: the hub has no auth. Pass --allow-remote to override.");
        return 2;
    }

    let home = data_home(data_dir);
    let store = match Store::open(&home) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cannot open {}: {e}", home.display());
            return 1;
        }
    };
    let sources = Sources::detect();
    let orca = sources.orca.clone();
    let herdr = sources.herdr.clone();
    let state = State::new(sources, store);
    let listener = match server::start(&bind, port, Arc::clone(&state)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("cannot bind {bind}:{port}: {e}");
            return 1;
        }
    };
    let actual = listener.local_addr().map(|a| a.port()).unwrap_or(port);
    let url = format!("http://{bind}:{actual}/");

    println!("wtf-live {VERSION}");
    println!("  url      {url}");
    println!("  data     {}", home.display());
    println!(
        "  orca     {}",
        orca.as_deref().unwrap_or("not found (install Orca)")
    );
    println!(
        "  herdr    {}",
        herdr.as_deref().unwrap_or("not found (brew install herdr)")
    );
    println!("  ctrl-c   stop");

    if open {
        let _ = std::process::Command::new("open").arg(&url).spawn();
    }
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}

// -------------------------------------------------------------------- bin

fn cmd_bin(args: &[String]) -> i32 {
    let Some(sub) = args.first().map(String::as_str) else {
        return usage_error("bin needs put|ls|get|rm");
    };
    let home = data_home(None);
    let store = match Store::open(&home) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cannot open {}: {e}", home.display());
            return 1;
        }
    };
    match sub {
        "put" => {
            let mut title = String::new();
            let mut file: Option<String> = None;
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--title" | "-t" => {
                        i += 1;
                        title = args.get(i).cloned().unwrap_or_default();
                    }
                    "--file" | "-f" => {
                        i += 1;
                        file = args.get(i).cloned();
                    }
                    other => return usage_error(&format!("unknown flag: {other}")),
                }
                i += 1;
            }
            let body = match file.as_deref() {
                Some("-") | None => {
                    let mut buf = String::new();
                    if let Err(e) = std::io::stdin().read_to_string(&mut buf) {
                        eprintln!("read stdin: {e}");
                        return 1;
                    }
                    buf
                }
                Some(path) => match std::fs::read_to_string(path) {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("read {path}: {e}");
                        return 1;
                    }
                },
            };
            match store.put(&title, &body, &Sources::detect().machine) {
                Ok(bin) => {
                    println!("{}", bin.id);
                    0
                }
                Err(e) => {
                    eprintln!("{e}");
                    1
                }
            }
        }
        "ls" | "list" => {
            let json_out = args.iter().any(|a| a == "--json");
            let bins = store.list();
            if json_out {
                let rows: Vec<Value> = bins.iter().map(|b| b.summary_json()).collect();
                println!("{}", Value::obj(vec![("bins", Value::arr(rows))]).to_json());
            } else if bins.is_empty() {
                println!("(no bins)");
            } else {
                for b in bins {
                    println!(
                        "{:<20} {:>8}  {:<12} {}",
                        b.id,
                        format!("{}B", b.bytes()),
                        b.machine,
                        b.title
                    );
                }
            }
            0
        }
        "get" => {
            let Some(id) = args.get(1) else {
                return usage_error("bin get needs an id");
            };
            match store.get(id) {
                Some(bin) => {
                    print!("{}", bin.body);
                    0
                }
                None => {
                    eprintln!("no such bin: {id}");
                    1
                }
            }
        }
        "rm" | "delete" => {
            let Some(id) = args.get(1) else {
                return usage_error("bin rm needs an id");
            };
            if store.delete(id) {
                println!("deleted {id}");
                0
            } else {
                eprintln!("no such bin: {id}");
                1
            }
        }
        other => usage_error(&format!("unknown bin command: {other}")),
    }
}

// --------------------------------------------------------------- snapshot

fn cmd_snapshot(args: &[String]) -> i32 {
    let json_out = args.iter().any(|a| a == "--json");
    let fresh = args.iter().any(|a| a == "--fresh");
    let sources = Sources::detect();
    let (machines, sessions, status) = if fresh {
        sources.collect()
    } else {
        sources.collect()
    };
    let snap = Snapshot {
        generated_at: wtf_live::util::now_millis(),
        sessions,
        machines,
        sources: status,
        bin_count: Store::open(&data_home(None))
            .map(|s| s.list().len())
            .unwrap_or(0),
    };
    if json_out {
        println!("{}", snap.to_json().to_json());
        return 0;
    }
    let counts = snap.to_json();
    let working = counts
        .get("counts")
        .and_then(|c| c.get("working"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    println!(
        "{} sessions ({} working) across {} machines; {} bins",
        snap.sessions.len(),
        working,
        snap.machines.len(),
        snap.bin_count
    );
    for m in &snap.machines {
        println!(
            "  [{}] {:<24} {:<8} {}",
            if m.available { "up" } else { "down" },
            m.name,
            m.kind,
            m.detail
        );
    }
    for s in &snap.sessions {
        let detail = s.detail.lines().next().unwrap_or("");
        println!(
            "  {:<8} {:<8} {:<12} {:<28} {}",
            s.state,
            s.source,
            s.agent,
            truncate(&s.title, 28),
            truncate(detail, 60)
        );
        let _ = &s.handle;
        let _ = s.live;
    }
    0
}

fn cmd_doctor() -> i32 {
    let sources = Sources::detect();
    println!("wtf-live {VERSION}");
    println!("  machine   {}", sources.machine);
    println!("  data      {}", data_home(None).display());
    println!(
        "  orca      {}",
        sources.orca.as_deref().unwrap_or("NOT FOUND")
    );
    println!(
        "  herdr     {}",
        sources.herdr.as_deref().unwrap_or("NOT FOUND")
    );
    let (machines, sessions, status) = sources.collect();
    println!();
    for st in &status {
        println!(
            "  {:<8} {} ({}ms){}",
            st.name,
            if st.ok { "ok" } else { "unavailable" },
            st.ms,
            if st.detail.is_empty() {
                String::new()
            } else {
                format!(" — {}", st.detail)
            }
        );
    }
    println!();
    println!("  {} machines, {} sessions", machines.len(), sessions.len());
    for m in machines {
        println!(
            "    [{}] {} ({}) {}",
            if m.available { "up" } else { "down" },
            m.name,
            m.kind,
            m.detail
        );
    }
    0
}

// ------------------------------------------------------------------ util

fn data_home(explicit: Option<String>) -> PathBuf {
    if let Some(dir) = explicit {
        return PathBuf::from(dir);
    }
    if let Ok(dir) = std::env::var("WTF_LIVE_HOME") {
        if !dir.trim().is_empty() {
            return PathBuf::from(dir);
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".wtf-live")
}

fn usage_error(msg: &str) -> i32 {
    eprintln!("{msg}\n");
    print_help();
    2
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max.saturating_sub(1)).collect::<String>() + "…"
    }
}
