//! Small shared helpers: time, URL/query handling, terminal text hygiene.

use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn percent_decode(s: &str) -> String {
    fn hex(c: u8) -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            b'A'..=b'F' => Some(c - b'A' + 10),
            _ => None,
        }
    }
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn parse_query(q: &str) -> Vec<(String, String)> {
    q.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| match p.split_once('=') {
            Some((k, v)) => (percent_decode(k), percent_decode(v)),
            None => (percent_decode(p), String::new()),
        })
        .collect()
}

pub fn clamp(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Strip ANSI escapes and control bytes, collapse blank runs, trim, clamp.
pub fn clean_text(s: &str, max: usize) -> String {
    let mut stripped = String::with_capacity(s.len().min(max * 4));
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => {
                if chars.peek() == Some(&'[') {
                    chars.next();
                    for n in chars.by_ref() {
                        if ('@'..='~').contains(&n) {
                            break;
                        }
                    }
                } else if chars.peek() == Some(&']') {
                    chars.next();
                    for n in chars.by_ref() {
                        if n == '\u{7}' {
                            break;
                        }
                    }
                }
            }
            '\r' => {}
            '\n' | '\t' => stripped.push(c),
            c if (c as u32) < 0x20 || c == '\u{7f}' => {}
            c => stripped.push(c),
        }
    }
    let mut out = String::with_capacity(stripped.len());
    let mut blanks = 0usize;
    for line in stripped.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            blanks += 1;
            if blanks > 1 {
                continue;
            }
        } else {
            blanks = 0;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(line);
    }
    clamp(out.trim(), max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_and_parses() {
        assert_eq!(percent_decode("a%20b"), "a b");
        assert_eq!(percent_decode("%zz"), "%zz");
        let q = parse_query("fresh=1&name=two%20words");
        assert_eq!(q[0].0, "fresh");
        assert_eq!(q[1].1, "two words");
    }

    #[test]
    fn strips_ansi_and_collapses() {
        let raw = "\u{1b}[31mred\u{1b}[0m\r\n\n\n\nnext";
        assert_eq!(clean_text(raw, 80), "red\n\nnext");
    }
}
