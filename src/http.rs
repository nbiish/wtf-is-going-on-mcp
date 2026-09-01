//! Minimal HTTP/1.1 server on `std::net`. Thread-per-connection, keep-alive,
//! hard limits (head 32 KiB, body 1 MiB, 100 headers, 15 s timeouts), and
//! SSE via chunked transfer for the dashboard event stream.
//!
//! Requests using Transfer-Encoding (chunked bodies) are rejected with 501 —
//! the only clients are our own bridge and browsers, which send framed
//! bodies. Failing closed keeps the parser surface small and auditable.

use crate::json::Value;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::time::Duration;

pub const MAX_HEAD: usize = 32 * 1024;
pub const MAX_BODY: usize = 1024 * 1024;
const MAX_HEADERS: usize = 100;
const READ_TIMEOUT: Duration = Duration::from_secs(15);
const WRITE_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug)]
pub struct Request {
    pub method: String,
    pub target: String,                 // raw path+query as received
    pub path: String,                   // percent-decoded path (no query)
    pub query: String,                  // raw query (no '?')
    pub headers: Vec<(String, String)>, // names lowercased
    pub body: Vec<u8>,
    pub http10: bool,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn q(&self, name: &str) -> Option<String> {
        crate::util::parse_query(&self.query)
            .into_iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v)
    }
}

pub struct Response {
    pub status: u16,
    pub content_type: String,
    pub extra_headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub close: bool,
}

impl Response {
    pub fn json(status: u16, v: &Value) -> Response {
        Response {
            status,
            content_type: "application/json".into(),
            extra_headers: Vec::new(),
            body: v.to_json().into_bytes(),
            close: false,
        }
    }

    pub fn text(status: u16, body: impl Into<String>) -> Response {
        Response {
            status,
            content_type: "text/plain; charset=utf-8".into(),
            extra_headers: Vec::new(),
            body: body.into().into_bytes(),
            close: false,
        }
    }

    pub fn html(status: u16, body: impl Into<String>) -> Response {
        Response {
            status,
            content_type: "text/html; charset=utf-8".into(),
            extra_headers: vec![(
                "Content-Security-Policy".into(),
                "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'self'; img-src 'self' data:".into(),
            )],
            body: body.into().into_bytes(),
            close: false,
        }
    }

    pub fn error(status: u16, msg: &str) -> Response {
        Response::json(
            status,
            &Value::obj(vec![
                ("ok", Value::from(false)),
                ("error", Value::from(msg)),
            ]),
        )
    }

    pub fn with_close(mut self) -> Response {
        self.close = true;
        self
    }
}

pub struct SseSession<'a> {
    stream: &'a mut TcpStream,
}

impl SseSession<'_> {
    fn write_chunked(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        let mut frame = format!("{:x}\r\n", bytes.len()).into_bytes();
        frame.extend_from_slice(bytes);
        frame.extend_from_slice(b"\r\n");
        self.stream.write_all(&frame)?;
        self.stream.flush()
    }

    /// Send an SSE event; multi-line data is split into data: lines per spec.
    pub fn event(&mut self, name: &str, data: &str) -> std::io::Result<()> {
        let mut payload = format!("event: {name}\n");
        for line in data.split('\n') {
            payload.push_str(&format!("data: {line}\n"));
        }
        payload.push('\n');
        self.write_chunked(payload.as_bytes())
    }

    /// SSE comment (keepalive); ignored by EventSource.
    pub fn comment(&mut self, text: &str) -> std::io::Result<()> {
        let payload = format!(": {text}\n\n");
        self.write_chunked(payload.as_bytes())
    }
}

pub type SseHandler = Box<dyn FnOnce(&mut SseSession) + Send>;
pub type Handler = Arc<dyn Fn(&Request) -> HandlerResult + Send + Sync>;

pub enum HandlerResult {
    Respond(Response),
    Sse(SseHandler),
}

fn status_text(code: u16) -> &'static str {
    match code {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        413 => "Payload Too Large",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        505 => "HTTP Version Not Supported",
        _ => "Response",
    }
}

fn security_headers() -> Vec<(&'static str, &'static str)> {
    vec![
        ("X-Content-Type-Options", "nosniff"),
        ("Cache-Control", "no-store"),
        ("Referrer-Policy", "no-referrer"),
    ]
}

/// Serialize status line + headers (no body). Pure; unit-tested.
pub fn build_response_head(resp: &Response, close: bool) -> Vec<u8> {
    let mut out = format!("HTTP/1.1 {} {}\r\n", resp.status, status_text(resp.status));
    out.push_str(&format!("Content-Type: {}\r\n", resp.content_type));
    out.push_str(&format!("Content-Length: {}\r\n", resp.body.len()));
    for (k, v) in security_headers() {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    for (k, v) in &resp.extra_headers {
        out.push_str(&format!("{k}: {v}\r\n"));
    }
    out.push_str(if close {
        "Connection: close\r\n"
    } else {
        "Connection: keep-alive\r\n"
    });
    out.push_str("\r\n");
    out.into_bytes()
}

fn write_response(stream: &mut TcpStream, resp: &Response, close: bool) {
    let mut bytes = build_response_head(resp, close);
    bytes.extend_from_slice(&resp.body);
    let _ = stream.write_all(&bytes);
    let _ = stream.flush();
}

enum TryParse {
    NeedMore,
    Bad(Response),
    Ok(Request, usize /* consumed incl. body */),
}

fn header_token_char(c: u8) -> bool {
    c.is_ascii_alphanumeric()
        || matches!(
            c,
            b'!' | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'.'
                | b'^'
                | b'_'
                | b'`'
                | b'|'
                | b'~'
        )
}

/// Parse a complete request head from `acc`. Pure; unit-tested.
fn try_parse(acc: &[u8]) -> TryParse {
    let head_end = match acc.windows(4).position(|w| w == b"\r\n\r\n") {
        Some(i) => i,
        None => {
            if acc.len() > MAX_HEAD {
                return TryParse::Bad(Response::error(431, "request head too large").with_close());
            }
            return TryParse::NeedMore;
        }
    };
    if head_end > MAX_HEAD {
        return TryParse::Bad(Response::error(431, "request head too large").with_close());
    }
    let head = &acc[..head_end];
    let mut lines = head.split(|&b| b == b'\n').map(|l| {
        // trim trailing \r
        if l.last() == Some(&b'\r') {
            &l[..l.len() - 1]
        } else {
            l
        }
    });

    let first = match lines.next() {
        Some(l) => l,
        None => return TryParse::Bad(Response::error(400, "empty request").with_close()),
    };
    if first.iter().any(|&b| b == 0) {
        return TryParse::Bad(Response::error(400, "NUL in request line").with_close());
    }
    let first = String::from_utf8_lossy(first).into_owned();
    let mut parts = first.split_whitespace();
    let (method, target, version) = match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(m), Some(t), Some(v), None) => (m.to_string(), t.to_string(), v.to_string()),
        _ => return TryParse::Bad(Response::error(400, "malformed request line").with_close()),
    };
    if method.is_empty() || !method.bytes().all(|b| b.is_ascii_alphabetic()) {
        return TryParse::Bad(Response::error(400, "bad method").with_close());
    }
    if !target.starts_with('/') {
        return TryParse::Bad(Response::error(400, "bad target").with_close());
    }
    if target.bytes().any(|b| b < 0x21 || b == 0x7f) {
        return TryParse::Bad(Response::error(400, "bad bytes in target").with_close());
    }
    let http10 = match version.as_str() {
        "HTTP/1.1" => false,
        "HTTP/1.0" => true,
        _ => return TryParse::Bad(Response::error(501, "only HTTP/1.x supported").with_close()),
    };

    let mut headers: Vec<(String, String)> = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if line.iter().any(|&b| b == 0) {
            return TryParse::Bad(Response::error(400, "NUL in headers").with_close());
        }
        let line = String::from_utf8_lossy(line).into_owned();
        let Some((name, value)) = line.split_once(':') else {
            return TryParse::Bad(Response::error(400, "malformed header").with_close());
        };
        let name_l = name.trim().to_ascii_lowercase();
        if name_l.is_empty() || !name.bytes().all(|b| header_token_char(b)) {
            return TryParse::Bad(Response::error(400, "bad header name").with_close());
        }
        if headers.len() >= MAX_HEADERS {
            return TryParse::Bad(Response::error(431, "too many headers").with_close());
        }
        headers.push((name_l, value.trim().to_string()));
    }

    if headers.iter().any(|(k, _)| k == "transfer-encoding") {
        return TryParse::Bad(Response::error(501, "transfer-encoding not supported").with_close());
    }
    let content_len = match headers.iter().find(|(k, _)| k == "content-length") {
        None => 0usize,
        Some((_, v)) => match v.parse::<u64>() {
            Ok(n) if n <= MAX_BODY as u64 => n as usize,
            Ok(_) => return TryParse::Bad(Response::error(413, "body too large").with_close()),
            Err(_) => {
                return TryParse::Bad(Response::error(400, "bad content-length").with_close())
            }
        },
    };

    let total = head_end + 4 + content_len;
    if acc.len() < total {
        // body bytes still in flight; bound total growth
        if total > MAX_HEAD + MAX_BODY {
            return TryParse::Bad(Response::error(413, "body too large").with_close());
        }
        return TryParse::NeedMore;
    }
    let body = acc[head_end + 4..total].to_vec();
    let (raw_path, query) = match target.split_once('?') {
        Some((p, q)) => (p, q.to_string()),
        None => (target.as_str(), String::new()),
    };
    let path = crate::util::percent_decode(raw_path);
    TryParse::Ok(
        Request {
            method,
            target,
            path,
            query,
            headers,
            body,
            http10,
        },
        total,
    )
}

enum ReadOutcome {
    Request(Request, bool /* keep-alive requested */),
    Closed,
    Bad(Response),
}

fn read_request(stream: &mut TcpStream, acc: &mut Vec<u8>) -> ReadOutcome {
    loop {
        match try_parse(acc) {
            TryParse::Ok(req, consumed) => {
                let keep = if req.http10 {
                    req.header("connection")
                        .map(|v| v.eq_ignore_ascii_case("keep-alive"))
                        .unwrap_or(false)
                } else {
                    !req.header("connection")
                        .map(|v| v.eq_ignore_ascii_case("close"))
                        .unwrap_or(false)
                };
                acc.drain(..consumed);
                return ReadOutcome::Request(req, keep);
            }
            TryParse::Bad(resp) => {
                acc.clear();
                return ReadOutcome::Bad(resp);
            }
            TryParse::NeedMore => {}
        }
        let mut chunk = [0u8; 8192];
        match stream.read(&mut chunk) {
            Ok(0) => return ReadOutcome::Closed,
            Ok(n) => acc.extend_from_slice(&chunk[..n]),
            Err(e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                return ReadOutcome::Closed; // idle timeout / slowloris guard
            }
            Err(_) => return ReadOutcome::Closed,
        }
        if acc.len() > MAX_HEAD + MAX_BODY {
            acc.clear();
            return ReadOutcome::Bad(Response::error(413, "request too large").with_close());
        }
    }
}

fn run_sse(stream: &mut TcpStream, f: SseHandler) {
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nX-Accel-Buffering: no\r\nConnection: close\r\nTransfer-Encoding: chunked\r\n\r\n"
    );
    if stream.write_all(head.as_bytes()).is_err() {
        return;
    }
    let _ = stream.flush();
    let mut session = SseSession { stream };
    let _ = session.comment("hello");
    f(&mut session);
    // terminal chunk + close; errors irrelevant at this point
    let _ = stream.write_all(b"0\r\n\r\n");
    let _ = stream.flush();
}

fn handle_conn(mut stream: TcpStream, handler: Handler) {
    let _ = stream.set_read_timeout(Some(READ_TIMEOUT));
    let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
    let _ = stream.set_nodelay(true);
    let mut acc: Vec<u8> = Vec::with_capacity(8192);
    loop {
        match read_request(&mut stream, &mut acc) {
            ReadOutcome::Closed => return,
            ReadOutcome::Bad(resp) => {
                write_response(&mut stream, &resp, true);
                return;
            }
            ReadOutcome::Request(req, keep) => match handler(&req) {
                HandlerResult::Respond(resp) => {
                    let close = resp.close || !keep;
                    write_response(&mut stream, &resp, close);
                    if close {
                        return;
                    }
                }
                HandlerResult::Sse(f) => {
                    run_sse(&mut stream, f);
                    return;
                }
            },
        }
    }
}

/// Accept loop; one thread per connection. Never returns under normal use.
pub fn serve(listener: TcpListener, handler: Handler) {
    for stream in listener.incoming() {
        if let Ok(s) = stream {
            let h = handler.clone();
            std::thread::spawn(move || handle_conn(s, h));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_full(raw: &str) -> Result<Request, u16> {
        match try_parse(raw.as_bytes()) {
            TryParse::Ok(r, _) => Ok(r),
            TryParse::Bad(resp) => Err(resp.status),
            TryParse::NeedMore => Err(0),
        }
    }

    #[test]
    fn parses_get_with_query_and_headers() {
        let raw = "GET /api/v1/state?k=abc%20d HTTP/1.1\r\nHost: h\r\nX-Wtf-Device: box\r\n\r\n";
        let r = parse_full(raw).unwrap();
        assert_eq!(r.method, "GET");
        assert_eq!(r.path, "/api/v1/state");
        assert_eq!(r.query, "k=abc%20d");
        assert_eq!(r.q("k").as_deref(), Some("abc d"));
        assert_eq!(r.header("x-wtf-device"), Some("box"));
        assert_eq!(r.header("X-WTF-DEVICE"), Some("box"));
        assert!(!r.http10);
    }

    #[test]
    fn parses_post_with_body_and_keepalive() {
        let raw = "POST /api/v1/checkin HTTP/1.1\r\nContent-Length: 5\r\n\r\nhello";
        let r = parse_full(raw).unwrap();
        assert_eq!(r.body, b"hello");
        assert!(r.http10 == false);
        // pipelined extra bytes after the body are not consumed
        let raw2 = "POST /x HTTP/1.1\r\nContent-Length: 2\r\n\r\nhiGET /next HTTP/1.1\r\n\r\n";
        match try_parse(raw2.as_bytes()) {
            TryParse::Ok(r, consumed) => {
                assert_eq!(r.body, b"hi");
                assert_eq!(consumed, raw2.len() - "GET /next HTTP/1.1\r\n\r\n".len());
            }
            _ => panic!("expected ok"),
        }
    }

    #[test]
    fn rejects_malformed() {
        // non-alphabetic method bytes are rejected at parse; unknown but
        // token-valid methods surface later as 404/405 from the API layer
        assert_eq!(parse_full("GET2 / HTTP/1.1\r\n\r\n").unwrap_err(), 400);
        assert_eq!(
            parse_full("GET /extra parts HTTP/1.1\r\n\r\n").unwrap_err(),
            400
        );
        assert_eq!(parse_full("GET / HTTP/9.9\r\n\r\n").unwrap_err(), 501);
        assert_eq!(
            parse_full("GET / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n").unwrap_err(),
            501
        );
        assert_eq!(
            parse_full("GET / HTTP/1.1\r\nContent-Length: 999999999\r\n\r\n").unwrap_err(),
            413
        );
        assert_eq!(
            parse_full("GET / HTTP/1.1\r\nBad Header Here\r\n\r\n").unwrap_err(),
            400
        );
        assert_eq!(parse_full("GET /\u{0} HTTP/1.1\r\n\r\n").unwrap_err(), 400);
    }

    #[test]
    fn need_more_then_complete() {
        let part = "POST /x HTTP/1.1\r\nContent-Length: 4\r\n\r\nab";
        match try_parse(part.as_bytes()) {
            TryParse::NeedMore => {}
            _ => panic!("expected NeedMore"),
        }
        let full = format!("{part}cd");
        let r = parse_full(&full).unwrap();
        assert_eq!(r.body, b"abcd");
    }

    #[test]
    fn response_head_shape() {
        let resp = Response::json(200, &Value::obj(vec![("ok", Value::from(true))]));
        let head = String::from_utf8(build_response_head(&resp, false)).unwrap();
        assert!(head.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(head.contains("Content-Type: application/json\r\n"));
        assert!(head.contains("Content-Length: 11\r\n"));
        assert!(head.contains("Connection: keep-alive\r\n"));
        assert!(head.contains("X-Content-Type-Options: nosniff\r\n"));
        let close = String::from_utf8(build_response_head(&resp, true)).unwrap();
        assert!(close.contains("Connection: close\r\n"));
    }
}
