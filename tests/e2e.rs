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
    assert_eq!(tools.len(), 16);

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

/// The v0.8.0 autonomous-enrollment lane: `wtf enroll-token` mints a
/// single-use token hub-side; redeeming it at /api/v1/enroll returns the
/// device key in the same shape as `key issue --json`. Wrong token, unknown
/// name, and reuse all get the same uniform 403; the redeemed key works
/// immediately (hot keystore reload).
#[test]
fn enroll_token_flow_end_to_end() {
    let home = temp_home("enroll");
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

    // Advertise the real (ephemeral) URL so the token json hands it out.
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

    // Mint a token hub-side (JSON shape for tooling).
    let mint = Command::new(env!("CARGO_BIN_EXE_wtf"))
        .args(["enroll-token", "--json", "boxtok"])
        .env("WTF_HOME", &home)
        .output()
        .expect("enroll-token");
    assert!(
        mint.status.success(),
        "enroll-token failed: {}",
        String::from_utf8_lossy(&mint.stderr)
    );
    let mline = String::from_utf8_lossy(&mint.stdout)
        .lines()
        .find(|l| l.trim_start().starts_with('{'))
        .expect("token json line")
        .to_string();
    let mv = wtf::json::parse(mline.trim()).expect("token json valid");
    let token = mv.get("token").unwrap().as_str().unwrap().to_string();
    let minted_hub_url = mv.get("hub_url").unwrap().as_str().unwrap();
    assert_eq!(minted_hub_url, url, "token json must advertise the real hub url");
    assert_eq!(token.len(), 64);

    let post = |name: &str, tok: &str| {
        wtf::client::request(
            &format!("{url}/api/v1/enroll"),
            "POST",
            &[],
            &format!(r#"{{"name":"{name}","token":"{tok}"}}"#).into_bytes(),
        )
        .expect("enroll request")
    };

    // Uniform refusals: wrong token and unknown name are indistinguishable.
    let wrong = post("boxtok", &"0".repeat(64));
    assert_eq!(
        wrong.status,
        403,
        "wrong token -> {} {}",
        wrong.status,
        wrong.text()
    );
    let ghost = post("ghost", &token);
    assert_eq!(
        ghost.status,
        403,
        "unknown name -> {} {}",
        ghost.status,
        ghost.text()
    );
    assert_eq!(
        post("boxtok", &token[..32]).status,
        403,
        "truncated token must fail"
    );

    // The right token redeems: key issued in `key issue --json` shape.
    let ok = post("boxtok", &token);
    assert_eq!(ok.status, 200, "body: {}", ok.text());
    let v = ok.json().expect("enroll json");
    let key = v.get("key").unwrap().as_str().unwrap().to_string();
    assert_eq!(v.get("device").unwrap().as_str().unwrap(), "boxtok");
    assert_eq!(v.get("hub_url").unwrap().as_str().unwrap(), url);
    assert_eq!(key.len(), 64);

    // Single-use: the burned token is dead.
    assert_eq!(post("boxtok", &token).status, 403);

    // The redeemed key authenticates immediately (hot keystore reload).
    let mut agent = Command::new(env!("CARGO_BIN_EXE_wtf"))
        .args(["agent"])
        .env("WTF_HUB_URL", &url)
        .env("WTF_DEVICE_NAME", "boxtok")
        .env("WTF_DEVICE_KEY", &key)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn agent");
    let mut reader = BufReader::new(agent.stdout.take().unwrap());
    rpc_write(
        &mut agent,
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"check_in","arguments":{"status":"working","task":"joined via token"}}}"#,
    );
    let ci = rpc_read(&mut reader);
    assert_eq!(
        ci.get("result")
            .unwrap()
            .get("isError")
            .and_then(|v| v.as_bool()),
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

/// Encrypted COMMS ledger channels end-to-end: the structured cross-repo/
/// cross-machine form of the AGENTS/{date}.COMMS.md protocol, carried inside
/// session channels. Verifies: the full join/seal handshake gates posting,
/// envelopes post + decrypt to ledger lines for members only, event
/// filtering and `after` pagination work, invalid events fail closed,
/// non-members cannot read, plain session messages still render, and the
/// hub stores no envelope plaintext (encrypted at rest + in transit).
#[test]
fn comms_channels_end_to_end() {
    let home = temp_home("comms");
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

    // Three devices: two members + one non-member.
    let mut secrets = std::collections::HashMap::new();
    for dev in ["box-a", "box-b", "box-c"] {
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

    let mut agents = Vec::new();
    for dev in ["box-a", "box-b", "box-c"] {
        let dev_home = temp_home(&format!("comms-{dev}"));
        let child = Command::new(env!("CARGO_BIN_EXE_wtf"))
            .args(["agent"])
            .env("WTF_HUB_URL", &url)
            .env("WTF_DEVICE_NAME", dev)
            .env("WTF_DEVICE_KEY", secrets[dev].clone())
            .env("WTF_HOME", &dev_home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn bridge");
        agents.push((dev.to_string(), dev_home, child));
    }
    let mut reader_a = BufReader::new(agents[0].2.stdout.take().unwrap());
    let mut reader_b = BufReader::new(agents[1].2.stdout.take().unwrap());
    let mut reader_c = BufReader::new(agents[2].2.stdout.take().unwrap());

    let mut id = 300u64;
    macro_rules! call {
        ($which:expr, $reader:expr, $tool:expr, $args:expr) => {{
            id += 1;
            let req = format!(
                r#"{{"jsonrpc":"2.0","id":{},"method":"tools/call","params":{{"name":"{}","arguments":{}}}}}"#,
                id, $tool, $args
            );
            rpc_write(&mut $which.2, &req);
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

    // Handshake: A creates, B joins, A seals, B re-joins + recovers the key.
    let (err, text) = call!(agents[0], reader_a, "session_create", r#"{"name":"team comms"}"#);
    assert!(!err, "session_create failed: {text}");
    let sid = text
        .split_whitespace()
        .find(|t| t.len() == 32 && t.chars().all(|c| c.is_ascii_hexdigit()))
        .unwrap_or_else(|| panic!("session id in create output: {text}"))
        .to_string();
    let (err, _) = call!(agents[1], reader_b, "session_join", &format!(r#"{{"session":"{sid}"}}"#));
    assert!(!err, "session_join failed");
    let (err, text) = call!(
        agents[0],
        reader_a,
        "session_seal",
        &format!(r#"{{"session":"{sid}","member":"box-b"}}"#)
    );
    assert!(!err, "session_seal failed: {text}");
    let (err, text) = call!(agents[1], reader_b, "session_join", &format!(r#"{{"session":"{sid}"}}"#));
    assert!(!err && text.contains("session key recovered"), "key recovery: {text}");

    // A posts a scoped checkin entry.
    let (err, text) = call!(
        agents[0],
        reader_a,
        "comms_post",
        &format!(
            r#"{{"session":"{sid}","event":"checkin","scope":"wtf-is-going-on-mcp/feat/comms-channels","note":"COMMSRA started ledger channel"}}"#
        )
    );
    assert!(!err, "A comms_post failed: {text}");
    assert!(text.contains("#1 [checkin]"), "post should report seq + event: {text}");

    // B posts an update, then a handoff.
    let (err, _) = call!(
        agents[1],
        reader_b,
        "comms_post",
        &format!(
            r#"{{"session":"{sid}","event":"update","note":"COMMSRB sealed key recovered; ack"}}"#
        )
    );
    assert!(!err, "B comms_post failed");
    let (err, _) = call!(
        agents[1],
        reader_b,
        "comms_post",
        &format!(
            r#"{{"session":"{sid}","event":"handoff","scope":"local-router/feat/windows-parity","note":"COMMSRB takes verification; secrets only in this channel"}}"#
        )
    );
    assert!(!err, "B handoff failed");

    // B reads the full ledger: sees A's entry with sender + scope.
    let (err, text) = call!(agents[1], reader_b, "comms_read", &format!(r#"{{"session":"{sid}"}}"#));
    assert!(!err, "B comms_read failed: {text}");
    assert!(
        text.contains("#1 [checkin] box-a (wtf-is-going-on-mcp/feat/comms-channels)"),
        "ledger line must carry event, sender, scope: {text}"
    );
    assert!(text.contains("COMMSRB takes verification"), "B must decrypt own handoff: {text}");

    // A filters by event type: only the update shows, not the checkin.
    let (err, text) = call!(
        agents[0],
        reader_a,
        "comms_read",
        &format!(r#"{{"session":"{sid}","event":"update"}}"#)
    );
    assert!(!err, "filtered read failed: {text}");
    assert!(text.contains("[update] box-b"), "filter must keep updates: {text}");
    assert!(!text.contains("[checkin]"), "filter must drop checkins: {text}");

    // Pagination: after seq 2 only the handoff remains.
    let (err, text) = call!(
        agents[0],
        reader_a,
        "comms_read",
        &format!(r#"{{"session":"{sid}","after":2}}"#)
    );
    assert!(!err && text.contains("[handoff]"), "after=2 must show handoff: {text}");
    assert!(!text.contains("COMMSRA started"), "after=2 must hide seq 1: {text}");

    // Fail closed: unknown event type rejected before encryption.
    let (err, text) = call!(
        agents[0],
        reader_a,
        "comms_post",
        &format!(r#"{{"session":"{sid}","event":"bogus","note":"x"}}"#)
    );
    assert!(err && text.contains("invalid event"), "bogus event must fail: {text}");

    // Non-member cannot read: C has no session key.
    let (err, text) = call!(agents[2], reader_c, "comms_read", &format!(r#"{{"session":"{sid}"}}"#));
    assert!(
        err && text.contains("no local session key"),
        "non-member must fail closed: {text}"
    );

    // Plain session messages still render (never crash the ledger view).
    let (err, _) = call!(
        agents[0],
        reader_a,
        "session_send",
        &format!(r#"{{"session":"{sid}","message":"plain ping from A"}}"#)
    );
    assert!(!err);
    let (err, text) = call!(agents[1], reader_b, "comms_read", &format!(r#"{{"session":"{sid}","after":3}}"#));
    assert!(
        !err && text.contains("<plain session message> plain ping from A"),
        "plain messages must render as raw: {text}"
    );

    // The hub stores no envelope plaintext: encrypted at rest + in transit.
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
    for secret_string in ["COMMSRA started", "COMMSRB takes verification", "secrets only in this channel"] {
        assert!(
            !body.contains(secret_string),
            "hub wire state must not carry envelope plaintext: {secret_string}"
        );
    }
    let sessions_file = std::fs::read_to_string(home.join("sessions.json")).unwrap();
    assert!(
        !sessions_file.contains("COMMSRA started") && !sessions_file.contains("COMMSRB"),
        "sessions.json must store only ciphertext"
    );

    // Cleanup.
    for (_, _, child) in agents.iter_mut() {
        let _ = child.kill();
        let _ = child.wait();
    }
    let _ = hub.kill();
    let _ = hub.wait();
    for (_, dev_home, _) in agents.iter() {
        let _ = std::fs::remove_dir_all(dev_home);
    }
    let _ = std::fs::remove_dir_all(&home);
}

/// The v0.9.0 signed-handshake lane: the operator copies ONE site
/// `enroll-secret`; the device proves possession via HMAC over the
/// transcript (the secret never crosses the wire) and receives its device
/// key ML-KEM-768-sealed to its encapsulation key (never plaintext).
/// Wrong secret, stale timestamp, tampered ek, and replayed nonce all get
/// the same uniform 403; `enroll-secret --rotate` kills outstanding copies.
#[test]
fn psk_handshake_end_to_end() {
    let hub_home = temp_home("pskhub");
    let dev_home = temp_home("pskdev");
    let bind = format!("{}:{}", std::net::Ipv4Addr::LOCALHOST, 0);
    let mut hub = Command::new(env!("CARGO_BIN_EXE_wtf"))
        .args(["serve", "--bind", &bind, "--no-open"])
        .env("WTF_HOME", &hub_home)
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

    // The site secret is auto-generated on first serve and persisted.
    let cfg_path = hub_home.join("config.json");
    let cfg_text = std::fs::read_to_string(&cfg_path).expect("hub config");
    let cfg_v = wtf::json::parse(cfg_text.trim()).expect("config json");
    let secret = cfg_v
        .get("enroll_secret")
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(secret.len(), 64, "site enroll secret is 256-bit hex");

    // Happy path via the real CLI: proof computed device-side, key arrives
    // sealed and is unwrapped into bridge.json — the operator's joiner flow.
    let enroll = Command::new(env!("CARGO_BIN_EXE_wtf"))
        .args(["enroll", "--url", &url, "--name", "boxpsk", "--psk", &secret])
        .env("WTF_HOME", &dev_home)
        .output()
        .expect("wtf enroll --psk");
    assert!(
        enroll.status.success(),
        "psk enroll failed: {}",
        String::from_utf8_lossy(&enroll.stderr)
    );
    let bridge_text = std::fs::read_to_string(dev_home.join("bridge.json")).expect("bridge.json");
    let bridge = wtf::json::parse(bridge_text.trim()).expect("bridge json");
    let dev_key = bridge
        .get("device_key")
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(bridge.get("device_name").unwrap().as_str().unwrap(), "boxpsk");
    assert_eq!(dev_key.len(), 64);

    // The sealed-then-unwrapped key authenticates immediately.
    let mut agent = Command::new(env!("CARGO_BIN_EXE_wtf"))
        .args(["agent"])
        .env("WTF_HUB_URL", &url)
        .env("WTF_DEVICE_NAME", "boxpsk")
        .env("WTF_DEVICE_KEY", &dev_key)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn agent");
    let mut reader = BufReader::new(agent.stdout.take().unwrap());
    rpc_write(
        &mut agent,
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"check_in","arguments":{"status":"working","task":"joined via psk handshake"}}}"#,
    );
    let ci = rpc_read(&mut reader);
    assert_eq!(
        ci.get("result")
            .unwrap()
            .get("isError")
            .and_then(|v| v.as_bool()),
        Some(false)
    );

    // Raw-handshake harness: proof over (name, proof_ek, ts, nonce) with
    // independent control of the body's ek for the tamper case.
    let dev2 = temp_home("pskdev2");
    let id2 = wtf::identity::load_or_create_at(&dev2.join("identity.json")).expect("identity");
    let ek2 = wtf::util::hex_encode(&id2.ek);
    let hs_post = |name: &str, sec: &str, proof_ek: &str, body_ek: &str, ts: u64, nonce: String| {
        let proof = wtf::hmac::hmac_sha256_hex(
            sec.as_bytes(),
            format!("wtf-enroll-v2\n{name}\n{proof_ek}\n{ts}\n{nonce}").as_bytes(),
        );
        let body = wtf::json::Value::obj(vec![
            ("name", wtf::json::Value::from(name)),
            ("ek", wtf::json::Value::from(body_ek)),
            ("ts", wtf::json::Value::from(ts as i64)),
            ("nonce", wtf::json::Value::from(nonce.as_str())),
            ("proof", wtf::json::Value::from(proof.as_str())),
        ]);
        wtf::client::request(
            &format!("{url}/api/v1/enroll"),
            "POST",
            &[],
            body.to_json().as_bytes(),
        )
        .expect("handshake post")
    };

    // A valid raw handshake succeeds: sealed package + fingerprint, and the
    // plaintext key field must never appear in psk-mode responses.
    let now = wtf::util::now_secs();
    let ok = hs_post("rawpsk", &secret, &ek2, &ek2, now, "b".repeat(32));
    assert_eq!(ok.status, 200, "raw handshake: {}", ok.text());
    let ov = ok.json().expect("handshake json");
    assert!(
        ov.get("sealed").and_then(|v| v.as_str()).is_some(),
        "psk response must carry the sealed package"
    );
    assert!(
        ov.get("ek_fp").and_then(|v| v.as_str()).is_some(),
        "psk response must carry the ek fingerprint"
    );
    assert!(ov.get("key").is_none(), "psk mode must never return plaintext key");
    assert_eq!(ov.get("device").unwrap().as_str().unwrap(), "rawpsk");

    // Every rejection below is the same uniform 403.
    let wrong = hs_post(
        "rawpsk",
        &"f".repeat(64),
        &ek2,
        &ek2,
        wtf::util::now_secs(),
        "c".repeat(32),
    );
    assert_eq!(wrong.status, 403, "wrong secret: {}", wrong.text());
    let stale = hs_post(
        "rawpsk",
        &secret,
        &ek2,
        &ek2,
        wtf::util::now_secs().saturating_sub(400),
        "d".repeat(32),
    );
    assert_eq!(stale.status, 403, "stale ts must fail");
    let mut ek3 = ek2.clone();
    ek3.pop();
    ek3.push(if ek2.ends_with('0') { '1' } else { '0' });
    let tampered = hs_post(
        "rawpsk",
        &secret,
        &ek2,
        &ek3,
        wtf::util::now_secs(),
        "e".repeat(32),
    );
    assert_eq!(tampered.status, 403, "ek tampering must fail");
    let replay = hs_post("rawpsk", &secret, &ek2, &ek2, now, "b".repeat(32));
    assert_eq!(replay.status, 403, "replayed nonce must fail");

    // Rotation: every outstanding copy dies instantly; the new one works.
    let rot = Command::new(env!("CARGO_BIN_EXE_wtf"))
        .args(["enroll-secret", "--rotate"])
        .env("WTF_HOME", &hub_home)
        .output()
        .expect("enroll-secret --rotate");
    assert!(
        rot.status.success(),
        "rotate failed: {}",
        String::from_utf8_lossy(&rot.stderr)
    );
    let stale_copy = hs_post(
        "rotdev",
        &secret,
        &ek2,
        &ek2,
        wtf::util::now_secs(),
        "f0".repeat(8),
    );
    assert_eq!(stale_copy.status, 403, "rotated-out secret must fail");
    let cfg2 = wtf::json::parse(
        std::fs::read_to_string(&cfg_path)
            .expect("config reread")
            .trim(),
    )
    .expect("config json");
    let secret2 = cfg2
        .get("enroll_secret")
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();
    assert_ne!(secret2, secret, "rotation must mint a fresh secret");
    let dev3 = temp_home("pskdev3");
    let enroll2 = Command::new(env!("CARGO_BIN_EXE_wtf"))
        .args(["enroll", "--url", &url, "--name", "boxrot", "--psk", &secret2])
        .env("WTF_HOME", &dev3)
        .output()
        .expect("enroll with rotated secret");
    assert!(
        enroll2.status.success(),
        "re-enroll with rotated secret failed: {}",
        String::from_utf8_lossy(&enroll2.stderr)
    );

    let _ = agent.kill();
    let _ = hub.kill();
    let _ = agent.wait();
    let _ = hub.wait();
    let _ = std::fs::remove_dir_all(&hub_home);
    let _ = std::fs::remove_dir_all(&dev_home);
    let _ = std::fs::remove_dir_all(&dev2);
    let _ = std::fs::remove_dir_all(&dev3);
}

/// The v0.10.0 operator-courier lane: an operator on a machine with NO wtf
/// state (empty home) pastes content into a hub bin using only --url and the
/// dashboard key, and the other side pulls it back byte-exact — pre-setup
/// bootstrap and general cross-machine copy/paste. A wrong key is refused
/// with the uniform 401. Finally, an enrolled agent (device auth, no
/// dashboard key) sees the same payload through the MCP read_bin tool.
#[test]
fn bin_operator_courier_end_to_end() {
    let home = temp_home("bin");
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

    // The operator holds only the dashboard key; read it from the hub's own
    // config the way the operator does after `wtf dashboard`.
    let cfg_text = std::fs::read_to_string(home.join("config.json")).unwrap();
    let key = wtf::json::parse(cfg_text.trim())
        .unwrap()
        .get("dashboard_key")
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(key.len(), 64);

    // Remote simulation: an empty operator home (no bridge.json/config.json)
    // and no env credentials — every run below must authenticate via --url
    // and --k alone.
    let op = temp_home("operator");
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_wtf"))
            .args(args)
            .env("WTF_HOME", &op)
            .env("WTF_HUB_URL", "")
            .env("WTF_DASHBOARD_KEY", "")
            .output()
            .expect("wtf bin")
    };

    // put by argv, get back byte-exact (raw stdout, no added newline).
    let payload = "paste me into the other agent";
    let put = run(&["bin", "put", "1", payload, "--url", &url, "--k", &key]);
    assert!(
        put.status.success(),
        "bin put failed: {}",
        String::from_utf8_lossy(&put.stderr)
    );
    let get = run(&["bin", "get", "1", "--url", &url, "--k", &key]);
    assert!(
        get.status.success(),
        "bin get failed: {}",
        String::from_utf8_lossy(&get.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&get.stdout), payload);

    // put by stdin (`-`), get back byte-exact including the embedded newline.
    let mut putin = Command::new(env!("CARGO_BIN_EXE_wtf"))
        .args(["bin", "put", "2", "-", "--url", &url, "--k", &key])
        .env("WTF_HOME", &op)
        .env("WTF_HUB_URL", "")
        .env("WTF_DASHBOARD_KEY", "")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn bin put -");
    putin
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"line one\nline two")
        .unwrap();
    drop(putin.stdin.take());
    let putin = putin.wait_with_output().expect("wait bin put -");
    assert!(
        putin.status.success(),
        "stdin put failed: {}",
        String::from_utf8_lossy(&putin.stderr)
    );
    let get2 = run(&["bin", "get", "2", "--url", &url, "--k", &key]);
    assert!(get2.status.success());
    assert_eq!(String::from_utf8_lossy(&get2.stdout), "line one\nline two");

    // ls lists both bins without leaking content.
    let ls = run(&["bin", "ls", "--url", &url, "--k", &key]);
    assert!(
        ls.status.success(),
        "bin ls failed: {}",
        String::from_utf8_lossy(&ls.stderr)
    );
    let lstext = String::from_utf8_lossy(&ls.stdout);
    assert!(lstext.contains("bin 1:"), "ls missing bin 1: {lstext}");
    assert!(lstext.contains("bin 2:"), "ls missing bin 2: {lstext}");
    assert!(!lstext.contains(payload), "ls must not leak bin content");

    // A wrong key is refused uniformly (401) with a nonzero exit.
    let bad = run(&["bin", "get", "1", "--url", &url, "--k", &"f".repeat(64)]);
    assert!(!bad.status.success(), "wrong key must fail");
    assert_eq!(bad.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&bad.stderr).contains("401"),
        "wrong key must report 401: {}",
        String::from_utf8_lossy(&bad.stderr)
    );

    // The other side of the courier: an enrolled agent reads the same bin
    // through MCP with plain device auth — no dashboard key involved.
    let issue = Command::new(env!("CARGO_BIN_EXE_wtf"))
        .args(["key", "issue", "--json", "courier-agent"])
        .env("WTF_HOME", &home)
        .output()
        .expect("key issue");
    assert!(
        issue.status.success(),
        "key issue failed: {}",
        String::from_utf8_lossy(&issue.stderr)
    );
    let jline = String::from_utf8_lossy(&issue.stdout)
        .lines()
        .find(|l| l.trim_start().starts_with('{'))
        .expect("key issue json line")
        .to_string();
    let jv = wtf::json::parse(jline.trim()).expect("key issue json valid");
    let agent_key = jv.get("key").unwrap().as_str().unwrap().to_string();
    let agent_name = jv.get("device").unwrap().as_str().unwrap().to_string();

    let mut agent = Command::new(env!("CARGO_BIN_EXE_wtf"))
        .args(["agent"])
        .env("WTF_HUB_URL", &url)
        .env("WTF_DEVICE_NAME", &agent_name)
        .env("WTF_DEVICE_KEY", &agent_key)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn agent");
    let mut reader = BufReader::new(agent.stdout.take().unwrap());
    rpc_write(
        &mut agent,
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"read_bin","arguments":{"bin":1}}}"#,
    );
    let rb = rpc_read(&mut reader);
    assert!(
        rb.to_json().contains(payload),
        "read_bin must see the courier payload: {}",
        rb.to_json()
    );

    let _ = agent.kill();
    let _ = hub.kill();
    let _ = agent.wait();
    let _ = hub.wait();
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&op);
}
