//! File-backed pastebins: one JSON file per bin under `<home>/bins/`.
//!
//! No encryption, no accounts, no database. The hub is loopback-only; bins
//! are shared text (a spec, a log tail, a URL list) between humans and agents
//! on any machine that can reach the hub.

use crate::json::{self, Value};
use crate::util::{clamp, now_millis};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const MAX_BODY: usize = 256 * 1024;
pub const MAX_TITLE: usize = 120;
pub const MAX_BINS: usize = 400;

#[derive(Clone, Debug)]
pub struct Bin {
    pub id: String,
    pub title: String,
    pub body: String,
    pub machine: String,
    pub created_at: u64,
    pub updated_at: u64,
}

impl Bin {
    pub fn bytes(&self) -> usize {
        self.body.len()
    }

    pub fn summary_json(&self) -> Value {
        Value::obj(vec![
            ("id", Value::from(self.id.clone())),
            ("title", Value::from(self.title.clone())),
            ("machine", Value::from(self.machine.clone())),
            ("bytes", Value::num(self.body.len() as f64)),
            ("created_at", Value::num(self.created_at as f64)),
            ("updated_at", Value::num(self.updated_at as f64)),
            (
                "preview",
                Value::from(clamp(&self.body.replace('\n', " "), 160)),
            ),
        ])
    }

    pub fn to_json(&self) -> Value {
        Value::obj(vec![
            ("id", Value::from(self.id.clone())),
            ("title", Value::from(self.title.clone())),
            ("body", Value::from(self.body.clone())),
            ("machine", Value::from(self.machine.clone())),
            ("bytes", Value::num(self.body.len() as f64)),
            ("created_at", Value::num(self.created_at as f64)),
            ("updated_at", Value::num(self.updated_at as f64)),
        ])
    }

    fn from_json(v: &Value) -> Option<Bin> {
        Some(Bin {
            id: v.get("id")?.as_str()?.to_string(),
            title: v.get("title")?.as_str()?.to_string(),
            body: v.get("body")?.as_str()?.to_string(),
            machine: v
                .get("machine")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .into(),
            created_at: v.get("created_at").and_then(|x| x.as_i64()).unwrap_or(0) as u64,
            updated_at: v.get("updated_at").and_then(|x| x.as_i64()).unwrap_or(0) as u64,
        })
    }
}

pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn open(home: &Path) -> io::Result<Store> {
        let root = home.join("bins");
        fs::create_dir_all(&root)?;
        Ok(Store { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn list(&self) -> Vec<Bin> {
        let mut bins = Vec::new();
        let Ok(dir) = fs::read_dir(&self.root) else {
            return bins;
        };
        for entry in dir.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Some(bin) = read_bin(&path) {
                bins.push(bin);
            }
        }
        bins.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        bins
    }

    pub fn get(&self, id: &str) -> Option<Bin> {
        if !valid_id(id) {
            return None;
        }
        read_bin(&self.root.join(format!("{id}.json")))
    }

    pub fn put(&self, title: &str, body: &str, machine: &str) -> Result<Bin, String> {
        if body.trim().is_empty() {
            return Err("body is empty".into());
        }
        if body.len() > MAX_BODY {
            return Err(format!("body exceeds {} KiB", MAX_BODY / 1024));
        }
        if self.list().len() >= MAX_BINS {
            return Err(format!(
                "bin limit reached ({MAX_BINS}); delete an old bin first"
            ));
        }
        let title = if title.trim().is_empty() {
            first_line(body)
        } else {
            clamp(title.trim(), MAX_TITLE)
        };
        let now = now_millis();
        let id = format!("{now:x}-{:04x}", fnv1a(body.as_bytes()) & 0xffff);
        let bin = Bin {
            id: id.clone(),
            title,
            body: body.to_string(),
            machine: clamp(machine.trim(), 60),
            created_at: now,
            updated_at: now,
        };
        let path = self.root.join(format!("{id}.json"));
        let tmp = self.root.join(format!(".{id}.tmp"));
        let json = bin.to_json().to_json();
        fs::write(&tmp, json).map_err(|e| format!("write bin: {e}"))?;
        fs::rename(&tmp, &path).map_err(|e| format!("commit bin: {e}"))?;
        Ok(bin)
    }

    pub fn delete(&self, id: &str) -> bool {
        if !valid_id(id) {
            return false;
        }
        fs::remove_file(self.root.join(format!("{id}.json"))).is_ok()
    }
}

fn read_bin(path: &Path) -> Option<Bin> {
    let text = fs::read_to_string(path).ok()?;
    let value = json::parse(&text).ok()?;
    Bin::from_json(&value)
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 80
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn first_line(body: &str) -> String {
    clamp(body.lines().next().unwrap_or("untitled").trim(), MAX_TITLE)
}

fn fnv1a(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in data {
        h ^= *b as u64;
        h = h.wrapping_mul(0x1000_0000_01b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_delete() {
        let dir = std::env::temp_dir().join(format!("wtf-live-test-{}", now_millis()));
        let store = Store::open(&dir).unwrap();
        let bin = store.put("hello", "a body\nline two", "box").unwrap();
        assert_eq!(store.list().len(), 1);
        assert_eq!(store.get(&bin.id).unwrap().body, "a body\nline two");
        assert!(store.delete(&bin.id));
        assert!(store.list().is_empty());
        assert!(store.get("../../etc/passwd").is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_empty_and_oversize() {
        let dir = std::env::temp_dir().join(format!("wtf-live-test-{}", now_millis() + 1));
        let store = Store::open(&dir).unwrap();
        assert!(store.put("t", "   ", "m").is_err());
        assert!(store.put("t", &"x".repeat(MAX_BODY + 1), "m").is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
