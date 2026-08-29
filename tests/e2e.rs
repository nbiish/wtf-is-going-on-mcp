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
    assert_eq!(tools.len(), 6);

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
