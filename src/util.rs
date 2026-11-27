//! Small shared helpers. No allocation-heavy magic, no panics on input.

/// Lowercase hex encode.
pub fn hex_encode(data: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(data.len() * 2);
    for &b in data {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Strict hex decode. Returns None on odd length or invalid digits.
pub fn hex_decode(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    if b.len() % 2 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(b.len() / 2);
    let mut i = 0;
    while i < b.len() {
        let hi = hex_val(b[i])?;
        let lo = hex_val(b[i + 1])?;
        out.push((hi << 4) | lo);
        i += 2;
    }
    Some(out)
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Truncate a string on char boundaries.
pub fn clamp(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect()
    }
}

/// Serialize tests that mutate the process-global `WTF_HOME` env var.
#[cfg(test)]
pub fn test_env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_roundtrip() {
        let data = [0u8, 1, 0x0f, 0xa5, 0xff];
        let hx = hex_encode(&data);
        assert_eq!(hx, "00010fa5ff");
        assert_eq!(hex_decode(&hx).unwrap(), data.to_vec());
        assert!(hex_decode("abc").is_none());
        assert!(hex_decode("zz").is_none());
        assert_eq!(hex_decode(""), Some(vec![]));
    }

    #[test]
    fn clamp_on_boundary() {
        assert_eq!(clamp("héllo", 3), "hél");
        assert_eq!(clamp("hi", 10), "hi");
    }
}
