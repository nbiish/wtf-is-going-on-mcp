//! `wtf` command-line interface — stateless SSH/local fleet status, goals, and pickup.

use std::path::PathBuf;
use wtf::fleet::{forbidden_keys, FleetStore, Goal, SshAlias};
use wtf::json::Value;
use wtf::probe::{self, LocalReport, RemoteReport};
use wtf::VERSION;

const SKILL_MD: &str = include_str!("../skill/wtf-is-going-on-mcp.md");

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(|s| s.as_str()) {
        None => cmd_status(&[]),
        Some("status") => cmd_status(&args[1..]),
        Some("pickup") => cmd_pickup(&args[1..]),
        Some("goal") => cmd_goal(&args[1..]),
        Some("alias") => cmd_alias(&args[1..]),
        Some("skill") => cmd_skill(&args[1..]),
        Some("version") | Some("--version") | Some("-V") => {
            println!("wtf {VERSION}");
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
    println!("wtf {VERSION} — stateless SSH/local fleet status, goals, and pickup CLI");
    println!();
    println!("USAGE:");
    println!("  wtf [status] [--local] [--json]               probe local machine + SSH aliases + open goals");
    println!("  wtf pickup [--json]                           offline pickup snapshot (local, goals, SSH targets)");
    println!("  wtf goal [list] [--all] [--json]              list goals (open by default, or --all)");
    println!("  wtf goal add <title> [--detail D] [--repo R] [--target H]");
    println!("  wtf goal done <id> | wtf goal rm <id>         complete or remove a goal");
    println!("  wtf alias [list] [--json]                     list keyless SSH aliases");
    println!("  wtf alias set <alias> <user> <host> [--port N]");
    println!("  wtf alias rm <alias>                          remove an SSH alias");
    println!("  wtf skill print | install [--dir DIR] [--force]");
    println!("  wtf version | wtf help");
    println!();
    println!("State lives in $WTF_HOME/fleet.json (default: ~/.config/wtf-mcp/fleet.json).");
    println!("Use plain ssh/scp for remote execution and file transfer.");
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

fn positional_args<'a>(args: &'a [String], value_flags: &[&str]) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if value_flags.contains(&a) {
            i += 2;
            continue;
        }
        if a.starts_with('-') {
            i += 1;
            continue;
        }
        out.push(a);
        i += 1;
    }
    out
}

fn current_cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn cmd_status(args: &[String]) -> i32 {
    let local_only = has_flag(args, "--local");
    let as_json = has_flag(args, "--json");
    let cwd = current_cwd();
    let store = FleetStore::load();
    let aliases = store.aliases();
    let open_goals: Vec<Goal> = store
        .goals()
        .into_iter()
        .filter(|g| g.status == "open")
        .collect();
    let local = probe::probe_local(&cwd);
    let remotes = if local_only {
        Vec::new()
    } else {
        probe::probe_all_remotes(&aliases)
    };

    if as_json {
        let doc = Value::obj(vec![
            ("version", Value::from(VERSION)),
            ("local", local.to_json()),
            (
                "remotes",
                Value::Arr(remotes.iter().map(RemoteReport::to_json).collect()),
            ),
            (
                "goals",
                Value::Arr(open_goals.iter().map(Goal::to_json).collect()),
            ),
        ]);
        debug_assert!(forbidden_keys(&doc).is_empty());
        println!("{}", doc.to_json());
        return 0;
    }

    print_local_text(&local);
    println!();
    if local_only {
        println!("REMOTES — skipped (--local)");
    } else if remotes.is_empty() {
        println!("REMOTES — (none — wtf alias set <alias> <user> <host>)");
    } else {
        println!("REMOTES");
        for r in &remotes {
            if r.reachable {
                let hn = r.remote_hostname.as_deref().unwrap_or("?");
                println!(
                    "  [up]   {} ({}@{}:{}) -> {} ({}ms)",
                    r.alias, r.user, r.host, r.port, hn, r.latency_ms
                );
            } else {
                let err = r.error.as_deref().unwrap_or("unreachable");
                println!(
                    "  [down] {} ({}@{}:{}) — {}",
                    r.alias, r.user, r.host, r.port, err
                );
            }
        }
    }
    println!();
    print_goals_text(&open_goals, false);
    0
}

fn cmd_pickup(args: &[String]) -> i32 {
    let as_json = has_flag(args, "--json");
    let cwd = current_cwd();
    let store = FleetStore::load();
    let aliases = store.aliases();
    let open_goals: Vec<Goal> = store
        .goals()
        .into_iter()
        .filter(|g| g.status == "open")
        .collect();
    let local = probe::probe_local(&cwd);

    if as_json {
        let alias_rows: Vec<Value> = aliases.iter().map(alias_pickup_json).collect();
        let doc = Value::obj(vec![
            ("version", Value::from(VERSION)),
            ("local", local.to_json()),
            ("goals", Value::Arr(open_goals.iter().map(Goal::to_json).collect())),
            ("ssh_aliases", Value::Arr(alias_rows)),
        ]);
        debug_assert!(forbidden_keys(&doc).is_empty());
        println!("{}", doc.to_json());
        return 0;
    }

    print_local_text(&local);
    println!();
    print_goals_text(&open_goals, false);
    println!();
    print_aliases_text(&aliases);
    0
}

fn alias_pickup_json(a: &SshAlias) -> Value {
    let ssh_cmd = if a.port == 22 {
        format!("ssh {}@{}", a.user, a.host)
    } else {
        format!("ssh -p {} {}@{}", a.port, a.user, a.host)
    };
    let scp_prefix = format!("{}@{}:", a.user, a.host);
    Value::obj(vec![
        ("alias", Value::from(a.alias.as_str())),
        ("user", Value::from(a.user.as_str())),
        ("host", Value::from(a.host.as_str())),
        ("port", Value::from(a.port as i64)),
        ("ssh", Value::from(ssh_cmd.as_str())),
        ("scp_prefix", Value::from(scp_prefix.as_str())),
    ])
}

fn print_local_text(local: &LocalReport) {
    let tools: Vec<&str> = local
        .skills
        .iter()
        .filter(|s| s.present)
        .map(|s| s.name.as_str())
        .collect();
    let tools_str = if tools.is_empty() {
        "none".to_string()
    } else {
        tools.join(", ")
    };
    println!(
        "LOCAL: {} ({}/{}) · tools: [{}]",
        local.hostname, local.os, local.arch, tools_str
    );
    if let Some(repo) = &local.repo {
        println!(
            "REPO:  {} [{}] ({} dirty)",
            repo.path, repo.branch, repo.dirty
        );
    }
    if let Some(herdr) = &local.herdr_agents {
        println!("HERDR:\n  {}", herdr.replace('\n', "\n  "));
    }
}

fn print_goals_text(goals: &[Goal], all: bool) {
    if goals.is_empty() {
        if all {
            println!("GOALS — (none)");
        } else {
            println!("OPEN GOALS — (none)");
        }
        return;
    }
    println!("{}", if all { "GOALS" } else { "OPEN GOALS" });
    for g in goals {
        let mut meta = Vec::new();
        if !g.repo.is_empty() {
            meta.push(format!("repo={}", g.repo));
        }
        if !g.target.is_empty() {
            meta.push(format!("target={}", g.target));
        }
        let suffix = if meta.is_empty() {
            String::new()
        } else {
            format!(" ({})", meta.join(", "))
        };
        println!("  [{}] {} {}{}", g.status, g.id, g.title, suffix);
        if !g.detail.is_empty() {
            println!("      {}", g.detail);
        }
    }
}

fn print_aliases_text(aliases: &[SshAlias]) {
    if aliases.is_empty() {
        println!("SSH ALIASES — (none — wtf alias set <alias> <user> <host>)");
        return;
    }
    println!("SSH ALIASES");
    for a in aliases {
        println!("  {} -> {}@{}:{}", a.alias, a.user, a.host, a.port);
    }
}

fn cmd_goal(args: &[String]) -> i32 {
    let store = FleetStore::load();
    match args.first().map(|s| s.as_str()) {
        None | Some("list") => {
            let sub = args.get(1..).unwrap_or(&[]);
            let show_all = has_flag(sub, "--all") || has_flag(args, "--all");
            let as_json = has_flag(sub, "--json") || has_flag(args, "--json");
            let goals: Vec<Goal> = store
                .goals()
                .into_iter()
                .filter(|g| show_all || g.status == "open")
                .collect();
            if as_json {
                let doc = Value::obj(vec![(
                    "goals",
                    Value::Arr(goals.iter().map(Goal::to_json).collect()),
                )]);
                println!("{}", doc.to_json());
                return 0;
            }
            print_goals_text(&goals, show_all);
            0
        }
        Some("add") => {
            let rest = &args[1..];
            let title = flag_value(rest, "--title").or_else(|| {
                positional_args(rest, &["--detail", "--repo", "--target", "--title"])
                    .into_iter()
                    .next()
                    .map(|s| s.to_string())
            });
            let Some(title) = title else {
                eprintln!("usage: wtf goal add <title> [--detail D] [--repo R] [--target H]");
                return 2;
            };
            let detail = flag_value(rest, "--detail").unwrap_or_default();
            let repo = flag_value(rest, "--repo").unwrap_or_default();
            let target = flag_value(rest, "--target").unwrap_or_default();
            match store.add_goal(&title, &detail, &repo, &target) {
                Ok(g) => {
                    println!("goal {} open: {}", g.id, g.title);
                    0
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    2
                }
            }
        }
        Some("done") => {
            let Some(id) = args.get(1).filter(|s| !s.starts_with('-')) else {
                eprintln!("usage: wtf goal done <id>");
                return 2;
            };
            match store.done_goal(id) {
                Ok(g) => {
                    println!("goal {} done", g.id);
                    0
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    1
                }
            }
        }
        Some("rm") => {
            let Some(id) = args.get(1).filter(|s| !s.starts_with('-')) else {
                eprintln!("usage: wtf goal rm <id>");
                return 2;
            };
            match store.remove_goal(id) {
                Ok(true) => {
                    println!("goal {id} removed");
                    0
                }
                Ok(false) => {
                    eprintln!("error: no goal {id}");
                    1
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    2
                }
            }
        }
        Some(other) => {
            eprintln!("error: unknown goal command '{other}'");
            2
        }
    }
}

fn cmd_alias(args: &[String]) -> i32 {
    let store = FleetStore::load();
    match args.first().map(|s| s.as_str()) {
        None | Some("list") => {
            let as_json = has_flag(args, "--json");
            let aliases = store.aliases();
            if as_json {
                let doc = Value::obj(vec![(
                    "ssh_aliases",
                    Value::Arr(aliases.iter().map(SshAlias::to_json).collect()),
                )]);
                println!("{}", doc.to_json());
                return 0;
            }
            print_aliases_text(&aliases);
            0
        }
        Some("set") => {
            let rest = &args[1..];
            let pos = positional_args(rest, &["--port"]);
            if pos.len() < 3 {
                eprintln!("usage: wtf alias set <alias> <user> <host> [--port N]");
                return 2;
            }
            let port: u16 = match flag_value(rest, "--port") {
                Some(p) => match p.parse::<u16>() {
                    Ok(n) if n > 0 => n,
                    _ => {
                        eprintln!("error: --port must be 1-65535");
                        return 2;
                    }
                },
                None => 22,
            };
            match store.upsert_alias(pos[0], pos[1], pos[2], port) {
                Ok(a) => {
                    println!("alias {} -> {}@{}:{}", a.alias, a.user, a.host, a.port);
                    0
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    2
                }
            }
        }
        Some("rm") => {
            let Some(alias) = args.get(1).filter(|s| !s.starts_with('-')) else {
                eprintln!("usage: wtf alias rm <alias>");
                return 2;
            };
            match store.remove_alias(alias) {
                Ok(true) => {
                    println!("alias {alias} removed");
                    0
                }
                Ok(false) => {
                    eprintln!("error: no alias {alias}");
                    1
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    2
                }
            }
        }
        Some(other) => {
            eprintln!("error: unknown alias command '{other}'");
            2
        }
    }
}

fn cmd_skill(args: &[String]) -> i32 {
    match args.first().map(|s| s.as_str()) {
        Some("print") => {
            print!("{SKILL_MD}");
            0
        }
        Some("install") => {
            let force = has_flag(args, "--force");
            let base = flag_value(args, "--dir")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."));
            let target = base
                .join(".agents")
                .join("skills")
                .join("wtf-is-going-on-mcp")
                .join("SKILL.md");
            if target.exists() {
                let existing = std::fs::read_to_string(&target).unwrap_or_default();
                if existing == SKILL_MD {
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
            match std::fs::write(&target, SKILL_MD) {
                Ok(_) => {
                    println!("skill installed: {}", target.display());
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
