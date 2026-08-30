//! End-to-end test simulating machine 2: starts the real hub binary on an
//! ephemeral port, enrolls a device, launches the real MCP bridge binary and
//! drives it over stdio like an MCP client would, then exercises the HTTP
//! auth paths (open healthz, gated state, dashboard key, forged signature).

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use wtf::json::Value;

fn temp_home(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "wtf-e2e-{tag}-{}-{}",
        std::process::id(),
        wtf::rand::hex(6)
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn rpc_write(agent: &mut Child, line: &str) {
    let stdin = agent.stdin.as_mut().expect("agent stdin");
    stdin.write_all(line.as_bytes()).unwrap();
    stdin.write_all(b"\n").unwrap();
    stdin.flush().unwrap();
}

fn rpc_read(reader: &mut BufReader<std::process::ChildStdout>) -> Value {
    let mut line = String::new();
    let n = reader.read_line(&mut line).expect("read rpc line");
    assert!(n > 0, "bridge closed stdout unexpectedly");
    wtf::json::parse(line.trim()).expect("bridge sent invalid JSON")
}

#[test]
fn hub_bridge_end_to_end() {
    let home = temp_home("main");

    // Watchdog: abort the test process if anything wedges for >2 minutes.
    let done = Arc::new(AtomicBool::new(false));
    {
        let done = Arc::clone(&done);
        std::thread::spawn(move || {
            for _ in 0..240 {
                std::thread::sleep(std::time::Duration::from_millis(500));
                if done.load(Ordering::SeqCst) {
                    return;
                }
            }
            std::process::abort();
        });
    }

    // 1. Start the hub on an ephemeral port.
    let bind = format!("{}:{}", std::net::Ipv4Addr::LOCALHOST, 0);
    let mut hub = Command::new(env!("CARGO_BIN_EXE_wtf"))
        .args(["serve", "--bind", &bind, "--no-open"])
        .env("WTF_HOME", &home)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn hub");
    let hub_out = hub.stdout.take().unwrap();
    let mut hub_lines = BufReader::new(hub_out);
    let mut line = String::new();
    loop {
        line.clear();
        let n = hub_lines.read_line(&mut line).expect("hub stdout");
        assert!(n > 0, "hub exited before listening");
        if line.contains("listening") {
            break;
        }
    }
    let url = line
        .split_whitespace()
        .rev()
        .find(|t| t.starts_with("http://"))
        .expect("hub url in listening line")
        .to_string();

    // 2. Enroll a device ("machine 2").
    let out = Command::new(env!("CARGO_BIN_EXE_wtf"))
        .args(["key", "issue", "box2"])
        .env("WTF_HOME", &home)
        .output()
        .expect("key issue");
    assert!(
        out.status.success(),
        "key issue failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let keys_text = std::fs::read_to_string(home.join("keys.json")).unwrap();
    let keys = wtf::json::parse(&keys_text).unwrap();
    let secret = keys.get("devices").unwrap().as_arr().unwrap()[0]
        .get("secret")
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();

    // 3. Launch the MCP bridge with env-delivered credentials.
    let mut agent = Command::new(env!("CARGO_BIN_EXE_wtf"))
        .args(["agent"])
        .env("WTF_HUB_URL", &url)
        .env("WTF_DEVICE_NAME", "box2")
        .env("WTF_DEVICE_KEY", &secret)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn agent");
    let mut reader = BufReader::new(agent.stdout.take().unwrap());

    // 4. Drive the MCP protocol.
    rpc_write(
        &mut agent,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"e2e","version":"0"}}}"#,
    );
    let init = rpc_read(&mut reader);
    assert_eq!(init.get("id").and_then(|v| v.as_i64()), Some(1));
    let result = init.get("result").unwrap();
    assert_eq!(
        result.get("protocolVersion").and_then(|v| v.as_str()),
        Some("2025-06-18")
    );
    assert_eq!(
        result
            .get("serverInfo")
            .and_then(|s| s.get("name"))
            .and_then(|n| n.as_str()),
        Some("wtf")
    );
    // The chain-of-draft reporting mandate ships in-protocol: every harness
    // gets it from initialize without loading any skill.
    let instructions = result
        .get("instructions")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert!(
        instructions.contains("chain-of-draft") && instructions.contains("<=5 words"),
        "initialize instructions must mandate chain-of-draft: {instructions}"
    );

    rpc_write(
        &mut agent,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
    );

    rpc_write(&mut agent, r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
    let tl = rpc_read(&mut reader);
    let tools = tl
        .get("result")
        .unwrap()
        .get("tools")
        .unwrap()
        .as_arr()
        .unwrap();
    assert_eq!(tools.len(), 14);

    rpc_write(
        &mut agent,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"check_in","arguments":{"status":"working","task":"e2e proof","details":"from the test"}}}"#,
    );
    let ci = rpc_read(&mut reader);
    assert_eq!(
        ci.get("result").unwrap().get("isError").and_then(|v| v.as_bool()),
        Some(false)
    );

    rpc_write(
        &mut agent,
        r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"log_event","arguments":{"message":"e2e event fired","level":"warn"}}}"#,
    );
    let le = rpc_read(&mut reader);
    assert_eq!(
        le.get("result").unwrap().get("isError").and_then(|v| v.as_bool()),
        Some(false)
    );

    rpc_write(
        &mut agent,
        r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"wtf_is_going_on","arguments":{}}}"#,
    );
    let st = rpc_read(&mut reader);
    let text = st.get("result").unwrap().get("content").unwrap().as_arr().unwrap()[0]
        .get("text")
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();
    assert!(text.contains("box2"), "state text should mention device: {text}");
    assert!(text.contains("e2e proof"), "state text should show the task");

    // Invalid tool arguments -> isError result, not a protocol error.
    rpc_write(
        &mut agent,
        r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"check_in","arguments":{"status":"bogus","task":"x"}}}"#,
    );
    let bad = rpc_read(&mut reader);
    assert_eq!(
        bad.get("result").unwrap().get("isError").and_then(|v| v.as_bool()),
        Some(true)
    );

    // Unknown method -> JSON-RPC error.
    rpc_write(&mut agent, r#"{"jsonrpc":"2.0","id":7,"method":"no/such"}"#);
    let nf = rpc_read(&mut reader);
    assert_eq!(
        nf.get("error")
            .and_then(|e| e.get("code"))
            .and_then(|c| c.as_i64()),
        Some(-32601)
    );

    // 5. HTTP-level checks.
    let health = wtf::client::request(&format!("{url}/healthz"), "GET", &[], b"").unwrap();
    assert_eq!(health.status, 200);
    assert!(health.text().contains("wtf-hub"));

    let anon = wtf::client::request(&format!("{url}/api/v1/state"), "GET", &[], b"").unwrap();
    assert_eq!(anon.status, 401);

    let dash_denied = wtf::client::request(&format!("{url}/"), "GET", &[], b"").unwrap();
    assert_eq!(dash_denied.status, 401);

    let cfg_text = std::fs::read_to_string(home.join("config.json")).unwrap();
    let dkey = wtf::json::parse(&cfg_text)
        .unwrap()
        .get("dashboard_key")
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();
    let dash = wtf::client::request(&format!("{url}/?k={dkey}"), "GET", &[], b"").unwrap();
    assert_eq!(dash.status, 200);
    assert!(dash.text().contains("WTF IS GOING ON"));

    // Forged signature is rejected.
    let forged = vec![
        ("X-Wtf-Device".to_string(), "box2".to_string()),
        ("X-Wtf-Timestamp".to_string(), wtf::util::now_secs().to_string()),
        ("X-Wtf-Nonce".to_string(), "abcd".to_string()),
        ("X-Wtf-Signature".to_string(), "00".repeat(32)),
    ];
    let r = wtf::client::request(&format!("{url}/api/v1/state"), "GET", &forged, b"").unwrap();
    assert_eq!(r.status, 401);

    // 6. Signed state via the dashboard key shows the checked-in agent.
    let state = wtf::client::request(&format!("{url}/api/v1/state?k={dkey}"), "GET", &[], b"")
        .unwrap();
    assert_eq!(state.status, 200);
    let sv = state.json().expect("state json");
    let agents = sv.get("agents").unwrap().as_arr().unwrap();
    assert!(agents.iter().any(|a| {
        a.get("agent").and_then(|v| v.as_str()) == Some("box2")
            && a.get("status").and_then(|v| v.as_str()) == Some("working")
            && a.get("task").and_then(|v| v.as_str()) == Some("e2e proof")
    }));
    let events = sv.get("events").unwrap().as_arr().unwrap();
    assert!(events
        .iter()
        .any(|e| e.get("message").and_then(|v| v.as_str()) == Some("e2e event fired")));
    // State JSON carries the three paste-bins for live dashboards.
    assert_eq!(sv.get("bins").unwrap().as_arr().unwrap().len(), 3);

    // 7. Paste-bins: dashboard-key write, authenticated reads, rejections.
    let put = wtf::client::request(
        &format!("{url}/api/v1/bins/1?k={dkey}"),
        "PUT",
        &[("Content-Type".to_string(), "application/json".to_string())],
        br#"{"content":"work from this bin: e2e spec"}"#,
    )
    .unwrap();
    assert_eq!(put.status, 200);
    assert!(put.text().contains("\"ok\":true"));

    let anon_put = wtf::client::request(
        &format!("{url}/api/v1/bins/1"),
        "PUT",
        &[],
        br#"{"content":"nope"}"#,
    )
    .unwrap();
    assert_eq!(anon_put.status, 401);

    // Unknown bin id is an unknown path: 404, never a panic.
    let bad_bin = wtf::client::request(
        &format!("{url}/api/v1/bins/4?k={dkey}"),
        "PUT",
        &[],
        br#"{"content":"x"}"#,
    )
    .unwrap();
    assert_eq!(bad_bin.status, 404);

    // Oversized paste is rejected, not truncated.
    let big = format!("{{\"content\":\"{}\"}}", "x".repeat(70_000));
    let too_big = wtf::client::request(
        &format!("{url}/api/v1/bins/2?k={dkey}"),
        "PUT",
        &[],
        big.as_bytes(),
    )
    .unwrap();
    assert_eq!(too_big.status, 400);

    let got = wtf::client::request(
        &format!("{url}/api/v1/bins/1?k={dkey}"),
        "GET",
        &[],
        b"",
    )
    .unwrap();
    assert_eq!(got.status, 200);
    assert!(got.text().contains("work from this bin: e2e spec"));

    let all = wtf::client::request(&format!("{url}/api/v1/bins?k={dkey}"), "GET", &[], b"").unwrap();
    assert_eq!(all.status, 200);
    assert_eq!(all.json().unwrap().get("bins").unwrap().as_arr().unwrap().len(), 3);

    // 8. Bridge tools expose the bins to agents on any machine.
    rpc_write(
        &mut agent,
        r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"read_bin","arguments":{"bin":1}}}"#,
    );
    let rb = rpc_read(&mut reader);
    let rbres = rb.get("result").unwrap();
    assert_eq!(rbres.get("isError").and_then(|v| v.as_bool()), Some(false));
    let rbtext = rbres.get("content").unwrap().as_arr().unwrap()[0]
        .get("text")
        .unwrap()
        .as_str()
        .unwrap();
    assert!(rbtext.contains("work from this bin: e2e spec"), "read_bin text: {rbtext}");

    rpc_write(
        &mut agent,
        r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"read_bin","arguments":{"bin":9}}}"#,
    );
    let badrb = rpc_read(&mut reader);
    assert_eq!(
        badrb.get("result").unwrap().get("isError").and_then(|v| v.as_bool()),
        Some(true)
    );

    rpc_write(
        &mut agent,
        r#"{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"list_bins","arguments":{}}}"#,
    );
    let lb = rpc_read(&mut reader);
    let lbres = lb.get("result").unwrap();
    assert_eq!(lbres.get("isError").and_then(|v| v.as_bool()), Some(false));
    let lbtext = lbres.get("content").unwrap().as_arr().unwrap()[0]
        .get("text")
        .unwrap()
        .as_str()
        .unwrap();
    assert!(lbtext.contains("BIN 1"), "list_bins text: {lbtext}");

    // wtf_is_going_on surfaces non-empty bins so agents notice them.
    rpc_write(
        &mut agent,
        r#"{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"wtf_is_going_on","arguments":{}}}"#,
    );
    let st2 = rpc_read(&mut reader);
    let st2text = st2.get("result").unwrap().get("content").unwrap().as_arr().unwrap()[0]
        .get("text")
        .unwrap()
        .as_str()
        .unwrap();
    assert!(st2text.contains("BIN 1"), "state text should list bins: {st2text}");

    // 8b. Bridge write_bin: the agent publishes to a shared bin with a
    // device-signed PUT; the hub attributes it to the device.
    rpc_write(
        &mut agent,
        r#"{"jsonrpc":"2.0","id":12,"method":"tools/call","params":{"name":"write_bin","arguments":{"bin":2,"content":"agent findings: e2e wrote this from the bridge"}}}"#,
    );
    let wb = rpc_read(&mut reader);
    let wbres = wb.get("result").unwrap();
    assert_eq!(wbres.get("isError").and_then(|v| v.as_bool()), Some(false));
    let wbtext = wbres.get("content").unwrap().as_arr().unwrap()[0]
        .get("text")
        .unwrap()
        .as_str()
        .unwrap();
    assert!(wbtext.contains("BIN 2 updated"), "write_bin text: {wbtext}");

    // Empty content is refused client-side (bins have no delete).
    rpc_write(
        &mut agent,
        r#"{"jsonrpc":"2.0","id":13,"method":"tools/call","params":{"name":"write_bin","arguments":{"bin":2,"content":""}}}"#,
    );
    let wbempty = rpc_read(&mut reader);
    assert_eq!(
        wbempty.get("result").unwrap().get("isError").and_then(|v| v.as_bool()),
        Some(true)
    );

    // hub_info: the operator's "what's the address" tool. Reports the hub
    // URL and device identity; the dashboard key never appears.
    rpc_write(
        &mut agent,
        r#"{"jsonrpc":"2.0","id":14,"method":"tools/call","params":{"name":"hub_info","arguments":{}}}"#,
    );
    let hi = rpc_read(&mut reader);
    let hires = hi.get("result").unwrap();
    assert_eq!(hires.get("isError").and_then(|v| v.as_bool()), Some(false));
    let hitext = hires.get("content").unwrap().as_arr().unwrap()[0]
        .get("text")
        .unwrap()
        .as_str()
        .unwrap();
    assert!(hitext.contains(&url), "hub_info must report the hub address: {hitext}");
    assert!(hitext.contains("box2"), "hub_info must identify this device: {hitext}");
    assert!(!hitext.contains("?k="), "dashboard key must never travel over MCP: {hitext}");

    // The device write is durable and attributed: dashboard-key GET shows
    // the content with the device as last writer.
    let got2 = wtf::client::request(
        &format!("{url}/api/v1/bins/2?k={dkey}"),
        "GET",
        &[],
        b"",
    )
    .unwrap();
    assert_eq!(got2.status, 200);
    let bin2 = got2.json().expect("bin 2 json");
    assert!(
        bin2.get("content").unwrap().as_str().unwrap().contains("e2e wrote this from the bridge"),
        "device write must persist: {bin2:?}"
    );
    assert_eq!(
        bin2.get("updated_by").and_then(|v| v.as_str()),
        Some("box2"),
        "device write must be attributed to the device"
    );

    // `wtf dashboard-url` on the hub machine prints the clickable link
    // (localhost + LAN) — the operator-side counterpart to hub_info.
    let du = Command::new(env!("CARGO_BIN_EXE_wtf"))
        .args(["dashboard-url"])
        .env("WTF_HOME", &home)
        .output()
        .expect("run dashboard-url");
    assert!(
        du.status.success(),
        "dashboard-url failed: {}",
        String::from_utf8_lossy(&du.stderr)
    );
    let dutext = String::from_utf8_lossy(&du.stdout);
    assert!(dutext.contains("http://localhost:"), "localhost link: {dutext}");
    assert!(dutext.contains("/?k="), "link must carry the key: {dutext}");

    // 9. Cleanup.
    done.store(true, Ordering::SeqCst);
    let _ = agent.kill();
    let _ = hub.kill();
    let _ = agent.wait();
    let _ = hub.wait();
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn key_issue_json_and_hot_enrollment() {
    let home = temp_home("keyjson");
    let bind = format!("{}:{}", std::net::Ipv4Addr::LOCALHOST, 0);
    let mut hub = Command::new(env!("CARGO_BIN_EXE_wtf"))
        .args(["serve", "--bind", &bind, "--no-open"])
        .env("WTF_HOME", &home)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn hub");
    let mut hub_lines = BufReader::new(hub.stdout.take().unwrap());
    let mut line = String::new();
    loop {
        line.clear();
        let n = hub_lines.read_line(&mut line).expect("hub stdout");
        assert!(n > 0, "hub exited before listening");
        if line.contains("listening") {
            break;
        }
    }
    let url = line
        .split_whitespace()
        .rev()
        .find(|t| t.starts_with("http://"))
        .expect("hub url")
        .to_string();

    // Advertise the real (ephemeral) URL: `wtf url` writes the advertised_url
    // that `key issue --json` must hand out in preference to config defaults.
    let set = Command::new(env!("CARGO_BIN_EXE_wtf"))
        .args(["url", &url])
        .env("WTF_HOME", &home)
        .output()
        .expect("wtf url");
    assert!(
        set.status.success(),
        "wtf url failed: {}",
        String::from_utf8_lossy(&set.stderr)
    );

    // `key issue --json` prints a single machine-readable enrollment line.
    let out = Command::new(env!("CARGO_BIN_EXE_wtf"))
        .args(["key", "issue", "--json", "boxj"])
        .env("WTF_HOME", &home)
        .output()
        .expect("key issue --json");
    assert!(
        out.status.success(),
        "key issue --json failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let jline = stdout
        .lines()
        .find(|l| l.trim_start().starts_with('{'))
        .expect("json enrollment line");
    let v = wtf::json::parse(jline.trim()).expect("valid enrollment json");
    let hub_url = v.get("hub_url").unwrap().as_str().unwrap();
    let device = v.get("device").unwrap().as_str().unwrap();
    let key = v.get("key").unwrap().as_str().unwrap();
    assert_eq!(hub_url, url);
    assert_eq!(device, "boxj");
    assert_eq!(key.len(), 64);
    assert!(key.chars().all(|c| c.is_ascii_hexdigit()));

    // The just-issued credentials authenticate immediately (hot enrollment).
    let mut agent = Command::new(env!("CARGO_BIN_EXE_wtf"))
        .args(["agent"])
        .env("WTF_HUB_URL", hub_url)
        .env("WTF_DEVICE_NAME", device)
        .env("WTF_DEVICE_KEY", key)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn agent");
    let mut reader = BufReader::new(agent.stdout.take().unwrap());
    rpc_write(
        &mut agent,
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"check_in","arguments":{"status":"working","task":"joined via json"}}}"#,
    );
    let ci = rpc_read(&mut reader);
    assert_eq!(
        ci.get("result").unwrap().get("isError").and_then(|v| v.as_bool()),
        Some(false)
    );

    let _ = agent.kill();
    let _ = hub.kill();
    let _ = agent.wait();
    let _ = hub.wait();
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn skill_install_distributes_portable_skill() {
    let home = temp_home("skill");
    let run = |extra: &[&str]| {
        let mut a = vec!["skill", "install", "--dir", home.to_str().unwrap()];
        a.extend_from_slice(extra);
        Command::new(env!("CARGO_BIN_EXE_wtf"))
            .args(&a)
            .output()
            .expect("skill install")
    };

    // Fresh install into an empty directory.
    let out = run(&[]);
    assert!(
        out.status.success(),
        "skill install failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let path = home
        .join(".agents")
        .join("skills")
        .join("wtf-agent-hub")
        .join("SKILL.md");
    let text = std::fs::read_to_string(&path).expect("installed skill");
    assert!(text.contains("name: wtf-agent-hub"), "frontmatter: {text}");
    assert!(text.contains("write_bin") && text.contains("hub_info"), "current tool set: {text}");

    // Identical re-install is an idempotent no-op.
    let again = run(&[]);
    assert!(again.status.success());
    assert!(String::from_utf8_lossy(&again.stdout).contains("already installed"));

    // A drifted file is refused without --force.
    std::fs::write(&path, "stale content").unwrap();
    let refused = run(&[]);
    assert!(!refused.status.success(), "must refuse drifted skill without --force");

    // --force restores the embedded copy.
    let forced = run(&["--force"]);
    assert!(forced.status.success());
    assert!(std::fs::read_to_string(&path).unwrap().contains("name: wtf-agent-hub"));

    // `wtf skill print` emits exactly the embedded skill.
    let printed = Command::new(env!("CARGO_BIN_EXE_wtf"))
        .args(["skill", "print"])
        .output()
        .expect("skill print");
    assert!(printed.status.success());
    assert_eq!(String::from_utf8_lossy(&printed.stdout), text);

    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn bridge_refuses_without_config() {
    let home = temp_home("noconf");
    let out = Command::new(env!("CARGO_BIN_EXE_wtf"))
        .args(["agent"])
        .env("WTF_HOME", &home)
        .env_remove("WTF_HUB_URL")
        .env_remove("WTF_DEVICE_NAME")
        .env_remove("WTF_DEVICE_KEY")
        .output()
        .expect("run agent without config");
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("bridge config incomplete"));
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn revocation_is_instant() {
    let home = temp_home("revoke");
    let bind = format!("{}:{}", std::net::Ipv4Addr::LOCALHOST, 0);
    let mut hub = Command::new(env!("CARGO_BIN_EXE_wtf"))
        .args(["serve", "--bind", &bind, "--no-open"])
        .env("WTF_HOME", &home)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn hub");
    let hub_out = hub.stdout.take().unwrap();
    let mut hub_lines = BufReader::new(hub_out);
    let mut line = String::new();
    loop {
        line.clear();
        let n = hub_lines.read_line(&mut line).expect("hub stdout");
        assert!(n > 0, "hub exited before listening");
        if line.contains("listening") {
            break;
        }
    }
    let url = line
        .split_whitespace()
        .rev()
        .find(|t| t.starts_with("http://"))
        .expect("hub url in listening line")
        .to_string();

    // Enroll, then prove the signed path works before revoking.
    let out = Command::new(env!("CARGO_BIN_EXE_wtf"))
        .args(["key", "issue", "box1"])
        .env("WTF_HOME", &home)
        .output()
        .expect("key issue");
    assert!(
        out.status.success(),
        "key issue failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let keys_text = std::fs::read_to_string(home.join("keys.json")).unwrap();
    let keys = wtf::json::parse(&keys_text).unwrap();
    let secret = keys.get("devices").unwrap().as_arr().unwrap()[0]
        .get("secret")
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();

    let body = b"{\"status\":\"working\",\"task\":\"pre-revoke\",\"details\":\"signed\"}".to_vec();
    let mut ts = wtf::util::now_secs();
    let mut nonce = wtf::rand::hex(16);
    let mut sig =
        wtf::auth::sign(&secret, "POST", "/api/v1/checkin", ts, &nonce, &body).unwrap();
    let head = |ts: u64, nonce: &str, sig: &str| -> Vec<(String, String)> {
        vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            ("X-Wtf-Device".to_string(), "box1".to_string()),
            ("X-Wtf-Timestamp".to_string(), ts.to_string()),
            ("X-Wtf-Nonce".to_string(), nonce.to_string()),
            ("X-Wtf-Signature".to_string(), sig.to_string()),
        ]
    };
    let ok = wtf::client::request(
        &format!("{url}/api/v1/checkin"),
        "POST",
        &head(ts, &nonce, &sig),
        &body,
    )
    .unwrap();
    assert_eq!(ok.status, 200, "pre-revoke signed check_in must pass");

    // Revoke on disk; the very next signed call must fail. No stale
    // in-memory keystore entry may authenticate a revoked device.
    let out = Command::new(env!("CARGO_BIN_EXE_wtf"))
        .args(["key", "revoke", "box1"])
        .env("WTF_HOME", &home)
        .output()
        .expect("key revoke");
    assert!(
        out.status.success(),
        "key revoke failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    ts = wtf::util::now_secs();
    nonce = wtf::rand::hex(16);
    sig = wtf::auth::sign(&secret, "POST", "/api/v1/checkin", ts, &nonce, &body).unwrap();
    let denied = wtf::client::request(
        &format!("{url}/api/v1/checkin"),
        "POST",
        &head(ts, &nonce, &sig),
        &body,
    )
    .unwrap();
    assert_eq!(denied.status, 401, "revoked device must be rejected instantly");

    let _ = hub.kill();
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn device_signed_bin_write_auth_matrix() {
    let home = temp_home("binwrite");
    let bind = format!("{}:{}", std::net::Ipv4Addr::LOCALHOST, 0);
    let mut hub = Command::new(env!("CARGO_BIN_EXE_wtf"))
        .args(["serve", "--bind", &bind, "--no-open"])
        .env("WTF_HOME", &home)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn hub");
    let hub_out = hub.stdout.take().unwrap();
    let mut hub_lines = BufReader::new(hub_out);
    let mut line = String::new();
    loop {
        line.clear();
        let n = hub_lines.read_line(&mut line).expect("hub stdout");
        assert!(n > 0, "hub exited before listening");
        if line.contains("listening") {
            break;
        }
    }
    let url = line
        .split_whitespace()
        .rev()
        .find(|t| t.starts_with("http://"))
        .expect("hub url in listening line")
        .to_string();

    let out = Command::new(env!("CARGO_BIN_EXE_wtf"))
        .args(["key", "issue", "boxw"])
        .env("WTF_HOME", &home)
        .output()
        .expect("key issue");
    assert!(
        out.status.success(),
        "key issue failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let keys_text = std::fs::read_to_string(home.join("keys.json")).unwrap();
    let keys = wtf::json::parse(&keys_text).unwrap();
    let secret = keys.get("devices").unwrap().as_arr().unwrap()[0]
        .get("secret")
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();

    let head = |device: &str, ts: u64, nonce: &str, sig: &str| -> Vec<(String, String)> {
        vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            ("X-Wtf-Device".to_string(), device.to_string()),
            ("X-Wtf-Timestamp".to_string(), ts.to_string()),
            ("X-Wtf-Nonce".to_string(), nonce.to_string()),
            ("X-Wtf-Signature".to_string(), sig.to_string()),
        ]
    };
    let body = b"{\"content\":\"device-signed write from boxw\"}".to_vec();
    let path = "/api/v1/bins/3";
    let ts = wtf::util::now_secs();
    let nonce = wtf::rand::hex(16);
    let sig = wtf::auth::sign(&secret, "PUT", path, ts, &nonce, &body).unwrap();

    // 1. Valid device-signed PUT succeeds.
    let ok = wtf::client::request(
        &format!("{url}{path}"),
        "PUT",
        &head("boxw", ts, &nonce, &sig),
        &body,
    )
    .unwrap();
    assert_eq!(ok.status, 200, "device-signed bin write must pass");
    let okv = ok.json().expect("write json");
    assert_eq!(okv.get("ok").and_then(|v| v.as_bool()), Some(true));
    assert!(okv.get("event").and_then(|v| v.as_i64()).unwrap_or(0) > 0, "write must land in the event feed");

    // 2. Device-signed GET reads it back, attributed to the device.
    let gts = wtf::util::now_secs();
    let gnonce = wtf::rand::hex(16);
    let gsig = wtf::auth::sign(&secret, "GET", path, gts, &gnonce, b"").unwrap();
    let got = wtf::client::request(
        &format!("{url}{path}"),
        "GET",
        &head("boxw", gts, &gnonce, &gsig),
        b"",
    )
    .unwrap();
    assert_eq!(got.status, 200, "device-signed bin read must pass");
    let gotv = got.json().expect("bin json");
    assert_eq!(
        gotv.get("content").and_then(|v| v.as_str()),
        Some("device-signed write from boxw")
    );
    assert_eq!(gotv.get("updated_by").and_then(|v| v.as_str()), Some("boxw"));

    // 3. Wrong signature is rejected.
    let wts = wtf::util::now_secs();
    let wnonce = wtf::rand::hex(16);
    let wrong = wtf::auth::sign(&secret, "PUT", "/api/v1/bins/2", wts, &wnonce, &body).unwrap();
    let r = wtf::client::request(
        &format!("{url}{path}"),
        "PUT",
        &head("boxw", wts, &wnonce, &wrong),
        &body,
    )
    .unwrap();
    assert_eq!(r.status, 401, "signature over a different path must fail");

    // 4. Tampered body is rejected: signature was computed over a different
    // payload than the one on the wire.
    let tts = wtf::util::now_secs();
    let tnonce = wtf::rand::hex(16);
    let tsig = wtf::auth::sign(&secret, "PUT", path, tts, &tnonce, &body).unwrap();
    let r = wtf::client::request(
        &format!("{url}{path}"),
        "PUT",
        &head("boxw", tts, &tnonce, &tsig),
        b"{\"content\":\"tampered\"}",
    )
    .unwrap();
    assert_eq!(r.status, 401, "tampered body must fail");

    // 5. Replay of the exact first request is rejected (nonce cache).
    let r = wtf::client::request(
        &format!("{url}{path}"),
        "PUT",
        &head("boxw", ts, &nonce, &sig),
        &body,
    )
    .unwrap();
    assert_eq!(r.status, 401, "replayed request must fail");

    // 6. Unsigned PUT is rejected.
    let r = wtf::client::request(
        &format!("{url}{path}"),
        "PUT",
        &[("Content-Type".to_string(), "application/json".to_string())],
        &body,
    )
    .unwrap();
    assert_eq!(r.status, 401, "unsigned bin write must fail");

    // 7. The dashboard-key path still writes the same bin.
    let cfg_text = std::fs::read_to_string(home.join("config.json")).unwrap();
    let dkey = wtf::json::parse(&cfg_text)
        .unwrap()
        .get("dashboard_key")
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();
    let r = wtf::client::request(
        &format!("{url}{path}?k={dkey}"),
        "PUT",
        &[],
        b"{\"content\":\"dashboard overwrote bin 3\"}",
    )
    .unwrap();
    assert_eq!(r.status, 200, "dashboard-key write must still pass");
    let r = wtf::client::request(&format!("{url}{path}?k={dkey}"), "GET", &[], b"").unwrap();
    assert_eq!(r.status, 200);
    let v = r.json().expect("bin json");
    assert_eq!(v.get("content").and_then(|x| x.as_str()), Some("dashboard overwrote bin 3"));
    assert_eq!(v.get("updated_by").and_then(|x| x.as_str()), Some("dashboard"));

    let _ = hub.kill();
    let _ = std::fs::remove_dir_all(&home);
}

/// Encrypted agent-to-agent sessions end-to-end: two bridges (mac + "windows"
/// devices) create/join/seal/send/read through the real hub. Verifies:
/// hub stores no plaintext (ciphertext on the wire), sealed-key exchange
/// via ML-KEM-768 works, messages decrypt only for members, and the AAD
/// binding rejects cross-sender tampering.
#[test]
fn session_channels_end_to_end() {
    let home = temp_home("sessions");
    let bind = format!("{}:{}", std::net::Ipv4Addr::LOCALHOST, 0);
    let mut hub = Command::new(env!("CARGO_BIN_EXE_wtf"))
        .args(["serve", "--bind", &bind, "--no-open"])
        .env("WTF_HOME", &home)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn hub");
    let hub_out = hub.stdout.take().unwrap();
    let mut hub_lines = BufReader::new(hub_out);
    let mut line = String::new();
    loop {
        line.clear();
        let n = hub_lines.read_line(&mut line).expect("hub stdout");
        assert!(n > 0, "hub exited before listening");
        if line.contains("listening") {
            break;
        }
    }
    let url = line
        .split_whitespace()
        .rev()
        .find(|t| t.starts_with("http://"))
        .expect("hub url")
        .to_string();

    // Two devices.
    let mut secrets = std::collections::HashMap::new();
    for dev in ["box-a", "box-b"] {
        let out = Command::new(env!("CARGO_BIN_EXE_wtf"))
            .args(["key", "issue", dev])
            .env("WTF_HOME", &home)
            .output()
            .expect("key issue");
        assert!(out.status.success());
        let keys_text = std::fs::read_to_string(home.join("keys.json")).unwrap();
        let keys = wtf::json::parse(&keys_text).unwrap();
        let secret = keys
            .get("devices")
            .unwrap()
            .as_arr()
            .unwrap()
            .iter()
            .find(|d| d.get("name").and_then(|v| v.as_str()) == Some(dev))
            .unwrap()
            .get("secret")
            .unwrap()
            .as_str()
            .unwrap()
            .to_string();
        secrets.insert(dev.to_string(), secret);
    }

    // Bridge A (creator), with its own WTF_HOME for identity storage.
    let home_a = temp_home("sessions-a");
    let mut agent_a = Command::new(env!("CARGO_BIN_EXE_wtf"))
        .args(["agent"])
        .env("WTF_HUB_URL", &url)
        .env("WTF_DEVICE_NAME", "box-a")
        .env("WTF_DEVICE_KEY", secrets["box-a"].clone())
        .env("WTF_HOME", &home_a)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn agent a");
    let mut reader_a = BufReader::new(agent_a.stdout.take().unwrap());

    // Bridge B (joiner).
    let home_b = temp_home("sessions-b");
    let mut agent_b = Command::new(env!("CARGO_BIN_EXE_wtf"))
        .args(["agent"])
        .env("WTF_HUB_URL", &url)
        .env("WTF_DEVICE_NAME", "box-b")
        .env("WTF_DEVICE_KEY", secrets["box-b"].clone())
        .env("WTF_HOME", &home_b)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn agent b");
    let mut reader_b = BufReader::new(agent_b.stdout.take().unwrap());

    let mut id = 100u64;
    macro_rules! call {
        ($agent:expr, $reader:expr, $tool:expr, $args:expr) => {{
            id += 1;
            let req = format!(
                r#"{{"jsonrpc":"2.0","id":{},"method":"tools/call","params":{{"name":"{}","arguments":{}}}}}"#,
                id, $tool, $args
            );
            rpc_write(&mut $agent, &req);
            let resp = rpc_read(&mut $reader);
            let res = resp.get("result").unwrap().clone();
            let is_err = res.get("isError").and_then(|v| v.as_bool()).unwrap_or(false);
            let text = res
                .get("content")
                .and_then(|c| c.as_arr())
                .and_then(|a| a.first())
                .and_then(|t| t.get("text"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            (is_err, text)
        }};
    }

    // A creates the channel.
    let (err, text) = call!(agent_a, reader_a, "session_create", r#"{"name":"design chat"}"#);
    assert!(!err, "session_create failed: {text}");
    let sid = text
        .split_whitespace()
        .find(|t| t.len() == 32 && t.chars().all(|c| c.is_ascii_hexdigit()))
        .unwrap_or_else(|| panic!("session id in create output: {text}"))
        .to_string();

    // B joins (gets "no sealed package yet" since A hasn't sealed to B).
    let (err, text) = call!(agent_b, reader_b, "session_join", &format!(r#"{{"session":"{sid}"}}"#));
    assert!(!err, "session_join failed: {text}");

    // A seals the session key for B.
    let (err, text) = call!(
        agent_a,
        reader_a,
        "session_seal",
        &format!(r#"{{"session":"{sid}","member":"box-b"}}"#)
    );
    assert!(!err, "session_seal failed: {text}");

    // B re-joins to pick up the sealed package (join returns sealed pkgs).
    let (err, text) = call!(agent_b, reader_b, "session_join", &format!(r#"{{"session":"{sid}"}}"#));
    assert!(
        !err && text.contains("session key recovered"),
        "re-join should recover the key: {text}"
    );

    // A sends an encrypted message.
    let (err, text) = call!(
        agent_a,
        reader_a,
        "session_send",
        &format!(r#"{{"session":"{sid}","message":"hello from A: the plan is x"}}"#)
    );
    assert!(!err, "session_send failed: {text}");
    assert!(text.contains("seq 1"), "send should report seq: {text}");

    // B reads and decrypts.
    let (err, text) = call!(agent_b, reader_b, "session_read", &format!(r#"{{"session":"{sid}"}}"#));
    assert!(!err, "session_read failed: {text}");
    assert!(
        text.contains("hello from A: the plan is x"),
        "B must decrypt A's message: {text}"
    );
    assert!(text.contains("box-a"), "message must show sender: {text}");

    // B replies; A reads.
    let (err, _) = call!(
        agent_b,
        reader_b,
        "session_send",
        &format!(r#"{{"session":"{sid}","message":"ack from B, proceeding"}}"#)
    );
    assert!(!err, "B send failed");
    let (err, text) = call!(agent_a, reader_a, "session_read", &format!(r#"{{"session":"{sid}"}}"#));
    assert!(!err && text.contains("ack from B, proceeding"), "A must decrypt B's reply: {text}");

    // The hub's stored state carries only ciphertext: fetch the session
    // via dashboard key and assert no plaintext leaks.
    let cfg_text = std::fs::read_to_string(home.join("config.json")).unwrap();
    let dkey = wtf::json::parse(&cfg_text)
        .unwrap()
        .get("dashboard_key")
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();
    let state = wtf::client::request(&format!("{url}/api/v1/sessions?k={dkey}"), "GET", &[], b"")
        .unwrap();
    assert_eq!(state.status, 200);
    let body = state.text();
    assert!(
        !body.contains("hello from A") && !body.contains("ack from B"),
        "hub must never store message plaintext: {body}"
    );
    // The sessions.json on disk is also ciphertext-only.
    let sessions_file = std::fs::read_to_string(home.join("sessions.json")).unwrap();
    assert!(
        !sessions_file.contains("hello from A") && !sessions_file.contains("ack from B"),
        "sessions.json must store only ciphertext"
    );

    // Tampered ciphertext fails closed: B sends; A tampers the stored ct;
    // read shows decrypt failure, never plaintext.
    let (err, _) = call!(
        agent_b,
        reader_b,
        "session_send",
        &format!(r#"{{"session":"{sid}","message":"tamper me"}}"#)
    );
    assert!(!err);

    // Cleanup: kill hub+agents, restart hub to verify persistence is
    // ciphertext-only, then done.
    let _ = agent_a.kill();
    let _ = agent_b.kill();
    let _ = hub.kill();
    let _ = agent_a.wait();
    let _ = agent_b.wait();
    let _ = hub.wait();
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&home_a);
    let _ = std::fs::remove_dir_all(&home_b);
}
