//! Route smoke test: the HTTP surface, the pastebin roundtrip, and the
//! graceful "no CLIs" path all in one process.

use std::io::{Read, Write};
use std::net::TcpStream;
use wtf_live::bins::Store;
use wtf_live::server::{self, State};
use wtf_live::sources::Sources;

fn request(port: u16, raw: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream.write_all(raw.as_bytes()).expect("write");
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    let mut buf = Vec::new();
    let _ = stream.read_to_end(&mut buf);
    String::from_utf8_lossy(&buf).into_owned()
}

fn get(port: u16, path: &str) -> String {
    request(
        port,
        &format!("GET {path} HTTP/1.1\r\nHost: t\r\nConnection: close\r\n\r\n"),
    )
}

#[test]
fn serves_page_and_pastebin_api() {
    let dir = std::env::temp_dir().join(format!("wtf-live-server-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).expect("store");
    let state = State::new(
        Sources {
            orca: None,
            herdr: None,
            machine: "test-box".into(),
        },
        store,
    );
    let listener = server::start("127.0.0.1", 0, state).expect("start");
    let port = listener.local_addr().unwrap().port();

    let health = get(port, "/healthz");
    assert!(health.starts_with("HTTP/1.1 200"), "{health}");
    assert!(health.contains("\"ok\":true"));

    let page = get(port, "/");
    assert!(page.starts_with("HTTP/1.1 200"));
    assert!(page.contains("wtf·live"));

    let snap = get(port, "/api/snapshot");
    assert!(snap.contains("\"machines\""));
    assert!(snap.contains("\"sources\""));

    let body = r#"{"title":"smoke","body":"hello\nworld","machine":"box"}"#;
    let created = request(
        port,
        &format!(
            "POST /api/bins HTTP/1.1\r\nHost: t\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        ),
    );
    assert!(created.starts_with("HTTP/1.1 201"), "{created}");
    let id = created
        .split('"')
        .skip_while(|part| *part != "id")
        .nth(2)
        .expect("id in response")
        .to_string();

    let one = get(port, &format!("/api/bins/{id}"));
    assert!(one.contains("hello\\nworld"));

    let missing = get(port, "/api/bins/does-not-exist");
    assert!(missing.starts_with("HTTP/1.1 404"), "{missing}");

    let deleted = request(
        port,
        &format!("DELETE /api/bins/{id} HTTP/1.1\r\nHost: t\r\nConnection: close\r\n\r\n"),
    );
    assert!(deleted.starts_with("HTTP/1.1 200"), "{deleted}");

    let _ = std::fs::remove_dir_all(&dir);
}
