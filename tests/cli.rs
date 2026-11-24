use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use wtf::fleet::forbidden_keys;
use wtf::json;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_wtf")
}

fn temp_home(tag: &str) -> PathBuf {
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let p = std::env::temp_dir().join(format!("wtf-cli-test-{tag}-{n}"));
    std::fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn cli_version_and_unknown_command() {
    let home = temp_home("ver");
    let out = Command::new(bin())
        .arg("version")
        .env("WTF_HOME", &home)
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains(env!("CARGO_PKG_VERSION")), "{stdout}");

    let bad = Command::new(bin())
        .arg("no-such-cmd")
        .env("WTF_HOME", &home)
        .output()
        .unwrap();
    assert!(!bad.status.success());
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn cli_alias_and_goal_lifecycle_and_pickup_json() {
    let home = temp_home("crud");

    // Reject path-like host (no key material allowed).
    let bad_alias = Command::new(bin())
        .args(["alias", "set", "bad", "kenwa", "/home/kenwa/.ssh/id_ed25519"])
        .env("WTF_HOME", &home)
        .output()
        .unwrap();
    assert!(!bad_alias.status.success());

    // Set valid SSH alias.
    let ok_alias = Command::new(bin())
        .args([
            "alias",
            "set",
            "windows",
            "kenwa",
            "peopleofthedawn.local",
            "--port",
            "22",
        ])
        .env("WTF_HOME", &home)
        .output()
        .unwrap();
    assert!(ok_alias.status.success(), "{:?}", ok_alias);

    let list_alias = Command::new(bin())
        .args(["alias", "list", "--json"])
        .env("WTF_HOME", &home)
        .output()
        .unwrap();
    assert!(list_alias.status.success());
    let alias_doc = json::parse(String::from_utf8_lossy(&list_alias.stdout).trim()).unwrap();
    let aliases = alias_doc.get("ssh_aliases").unwrap().as_arr().unwrap();
    assert_eq!(aliases.len(), 1);
    assert_eq!(aliases[0].get("alias").unwrap().as_str(), Some("windows"));

    // Add goal -> list -> done -> pickup -> rm.
    let add_goal = Command::new(bin())
        .args([
            "goal",
            "add",
            "ship stateless cli",
            "--detail",
            "verify ssh probes",
            "--repo",
            "wtf-is-going-on-mcp",
            "--target",
            "mac",
        ])
        .env("WTF_HOME", &home)
        .output()
        .unwrap();
    assert!(add_goal.status.success(), "{:?}", add_goal);

    let list_goals = Command::new(bin())
        .args(["goal", "list", "--json"])
        .env("WTF_HOME", &home)
        .output()
        .unwrap();
    assert!(list_goals.status.success());
    let goals_doc = json::parse(String::from_utf8_lossy(&list_goals.stdout).trim()).unwrap();
    let goal_id = goals_doc.get("goals").unwrap().as_arr().unwrap()[0]
        .get("id")
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();

    let pickup = Command::new(bin())
        .args(["pickup", "--json"])
        .env("WTF_HOME", &home)
        .output()
        .unwrap();
    assert!(pickup.status.success());
    let pickup_doc = json::parse(String::from_utf8_lossy(&pickup.stdout).trim()).unwrap();
    assert!(forbidden_keys(&pickup_doc).is_empty());
    assert_eq!(pickup_doc.get("goals").unwrap().as_arr().unwrap().len(), 1);
    assert_eq!(
        pickup_doc.get("ssh_aliases").unwrap().as_arr().unwrap().len(),
        1
    );

    let status_local = Command::new(bin())
        .args(["status", "--local", "--json"])
        .env("WTF_HOME", &home)
        .output()
        .unwrap();
    assert!(status_local.status.success());
    let status_doc = json::parse(String::from_utf8_lossy(&status_local.stdout).trim()).unwrap();
    assert!(forbidden_keys(&status_doc).is_empty());
    assert!(status_doc.get("local").is_some());

    let done_goal = Command::new(bin())
        .args(["goal", "done", &goal_id])
        .env("WTF_HOME", &home)
        .output()
        .unwrap();
    assert!(done_goal.status.success());

    let rm_goal = Command::new(bin())
        .args(["goal", "rm", &goal_id])
        .env("WTF_HOME", &home)
        .output()
        .unwrap();
    assert!(rm_goal.status.success());

    let rm_alias = Command::new(bin())
        .args(["alias", "rm", "windows"])
        .env("WTF_HOME", &home)
        .output()
        .unwrap();
    assert!(rm_alias.status.success());

    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn cli_skill_print_and_install() {
    let home = temp_home("skill");
    let print_out = Command::new(bin())
        .args(["skill", "print"])
        .env("WTF_HOME", &home)
        .output()
        .unwrap();
    assert!(print_out.status.success());
    let md = String::from_utf8_lossy(&print_out.stdout);
    assert!(md.contains("wtf status"), "{md}");

    let install_out = Command::new(bin())
        .args(["skill", "install", "--dir", home.to_str().unwrap()])
        .env("WTF_HOME", &home)
        .output()
        .unwrap();
    assert!(install_out.status.success());
    assert!(home
        .join(".agents/skills/wtf-is-going-on-mcp/SKILL.md")
        .exists());
    let _ = std::fs::remove_dir_all(&home);
}
