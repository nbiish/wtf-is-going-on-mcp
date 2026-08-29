//! Minimal HTTP/1.1 client (http:// only) used by the bridge and `wtf status`.
//! Every request is sent with `Connection: close` and read to EOF — simple
//! and stateless; latency on a LAN makes this a non-issue.

use crate::json::{self, Value};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

pub struct ClientResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl ClientResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub fn json(&self) -> Option<Value> {
        json::parse(&String::from_utf8_lossy(&self.body)).ok()
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

fn chunked_decode(data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut i = 0usize;
    loop {
        let line_end = data[i..].windows(2).position(|w| w == b"\r\n")? + i;
        let size_str = std::str::from_utf8(&data[i..line_end]).ok()?;
        let size = usize::from_str_radix(size_str.split(';').next()?.trim(), 16).ok()?;
        i = line_end + 2;
        if size == 0 {
            return Some(out);
        }
        if i + size > data.len() {
            return None;
        }
        out.extend_from_slice(&data[i..i + size]);
        i += size + 2; // skip chunk + CRLF
    }
}

/// Perform an http:// request. Returns Err with a human-readable message.
pub fn request(
    url: &str,
    method: &str,
    headers: &[(String, String)],
    body: &[u8],
) -> Result<ClientResponse, String> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| "only http:// URLs are supported".to_string())?;
    let (hostport, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    if hostport.is_empty() {
        return Err("empty host".into());
    }
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) => (
            h,
            p.parse::<u16>().map_err(|_| format!("bad port in '{hostport}'"))?,
        ),
        None => (hostport, 80),
    };

    let addrs: Vec<_> = (host, port)
        .to_socket_addrs()
        .map_err(|e| format!("resolve '{host}': {e}"))?
        .collect();
    let mut conn: Option<TcpStream> = None;
    for a in addrs {
        if let Ok(s) = TcpStream::connect_timeout(&a, Duration::from_secs(3)) {
            conn = Some(s);
            break;
        }
    }
    let mut stream = conn.ok_or_else(|| format!("connect to {hostport} failed"))?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));

    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: {hostport}\r\n");
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str(&format!("Content-Length: {}\r\n", body.len()));
    req.push_str("Connection: close\r\n\r\n");
    stream
        .write_all(req.as_bytes())
        .and_then(|_| stream.write_all(body))
        .and_then(|_| stream.flush())
        .map_err(|e| format!("write to {hostport}: {e}"))?;

    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .map_err(|e| format!("read from {hostport}: {e}"))?;

    let head_end = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| "truncated response (no header terminator)".to_string())?;
    let head = String::from_utf8_lossy(&raw[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let status_line = lines.next().ok_or("empty response")?;
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| format!("bad status line: {status_line}"))?;
    let mut resp_headers = Vec::new();
    for l in lines {
        if let Some((k, v)) = l.split_once(':') {
            resp_headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }
    let mut body_bytes = raw[head_end + 4..].to_vec();
    let chunked = resp_headers
        .iter()
        .any(|(k, v)| k == "transfer-encoding" && v.to_ascii_lowercase().contains("chunked"));
    if chunked {
        body_bytes = chunked_decode(&body_bytes).ok_or("bad chunked body")?;
    } else if let Some((_, cl)) = resp_headers.iter().find(|(k, _)| k == "content-length") {
        let n: usize = cl.parse().map_err(|_| "bad content-length")?;
        body_bytes.truncate(n.min(body_bytes.len()));
    }

    Ok(ClientResponse { status, headers: resp_headers, body: body_bytes })
}

/// GET returning (status, text) — convenience for probes.
pub fn get_text(url: &str) -> Result<(u16, String), String> {
    let r = request(url, "GET", &[], b"")?;
    Ok((r.status, r.text()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunked_decode_basics() {
        let data = b"4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n";
        assert_eq!(chunked_decode(data).unwrap(), b"Wikipedia".to_vec());
        assert!(chunked_decode(b"4\r\nWik").is_none());
        assert_eq!(chunked_decode(b"0\r\n\r\n").unwrap(), Vec::<u8>::new());
        let with_ext = b"a;ext=1\r\n0123456789\r\n0\r\n\r\n";
        assert_eq!(
            chunked_decode(with_ext).unwrap(),
            b"0123456789".to_vec()
        );
    }

    #[test]
    fn rejects_non_http() {
        assert!(request("https://x/", "GET", &[], b"").is_err());
        assert!(request("ftp://x/", "GET", &[], b"").is_err());
    }
}
