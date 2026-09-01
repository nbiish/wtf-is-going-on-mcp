//! Agent-to-agent encrypted session channels (hub side).
//!
//! The hub is an UNTRUSTED rendezvous: it stores session metadata, member
//! lists, ML-KEM-768 sealed key packages, and AES-256-GCM ciphertext — and
//! can read none of it. All message confidentiality/integrity lives in the
//! sealed envelope: the creator generates a random 256-bit session key,
//! encapsulates it (ML-KEM-768) to each member's registered public key,
//! and only members can decapsulate. Messages carry per-sender monotonic
//! sequence numbers (replay/ordering protection) and AAD binding the
//! session id + sender + sequence so ciphertexts cannot be replayed across
//! sessions or members.
//!
//! Storage: `$WTF_HOME/sessions.json` (0600), atomic writes, same
//! persistence discipline as bins.json.

use crate::config;
use crate::json::Value;
use crate::util::{clamp, now_secs};
use std::path::PathBuf;
use std::sync::Mutex;

/// Hard caps (fail closed, never truncate).
pub const MAX_SESSIONS: usize = 64;
pub const MAX_MEMBERS: usize = 16;
pub const MAX_SEALED_PKGS: usize = 16;
pub const MAX_SESSION_NAME_CHARS: usize = 128;
pub const MAX_REPO_CHARS: usize = 128;
pub const MAX_CIPHERTEXT_CHARS: usize = 16_384;
pub const MAX_MSGS_IN_MEMORY: usize = 200; // per session ring buffer
pub const MAX_MSG_TOTAL: usize = 20_000; // across all sessions

#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    pub device: String,
    /// ML-KEM-768 encapsulation key (hex), captured at join/invite time.
    pub ek: String,
    pub joined_at: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SealedPkg {
    /// ML-KEM-768 ciphertext encapsulating the session key for one member.
    pub ct: String,
    pub ek_fp: String, // sha3-256 hex (16) of the member ek it was sealed to
}

#[derive(Clone, Debug, PartialEq)]
pub struct SessionMsg {
    pub seq: u64,
    pub sender: String,
    pub nonce: String, // hex, 24 bytes (96-bit GCM nonce)
    pub ct: String,    // hex, AES-256-GCM ciphertext||tag
    pub ts: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    pub id: String, // 128-bit hex, assigned by hub
    pub name: String,
    pub created_by: String,
    pub created_at: u64,
    /// Current message seq counter (monotonic per session, hub-assigned).
    pub next_seq: u64,
    pub members: Vec<Member>,
    pub sealed: Vec<SealedPkg>,
    pub msgs: Vec<SessionMsg>,
    /// SHA-256 of the pairing key (hex). The key itself NEVER lives on the
    /// hub — joiners present it, the hub constant-time-compares its hash.
    /// Empty = legacy session (join via sealed packages only).
    pub pairing_hash: String,
    /// Repository/project label this chat is paired with (operator-set).
    pub repo: String,
}

impl Session {
    fn to_file_json(&self) -> Value {
        let members: Vec<Value> = self
            .members
            .iter()
            .map(|m| {
                Value::obj(vec![
                    ("device", Value::from(m.device.as_str())),
                    ("ek", Value::from(m.ek.as_str())),
                    ("joined_at", Value::from(m.joined_at as i64)),
                ])
            })
            .collect();
        let sealed: Vec<Value> = self
            .sealed
            .iter()
            .map(|s| {
                Value::obj(vec![
                    ("ct", Value::from(s.ct.as_str())),
                    ("ek_fp", Value::from(s.ek_fp.as_str())),
                ])
            })
            .collect();
        let msgs: Vec<Value> = self
            .msgs
            .iter()
            .map(|m| {
                Value::obj(vec![
                    ("seq", Value::from(m.seq as i64)),
                    ("sender", Value::from(m.sender.as_str())),
                    ("nonce", Value::from(m.nonce.as_str())),
                    ("ct", Value::from(m.ct.as_str())),
                    ("ts", Value::from(m.ts as i64)),
                ])
            })
            .collect();
        Value::obj(vec![
            ("id", Value::from(self.id.as_str())),
            ("name", Value::from(self.name.as_str())),
            ("created_by", Value::from(self.created_by.as_str())),
            ("created_at", Value::from(self.created_at as i64)),
            ("next_seq", Value::from(self.next_seq as i64)),
            ("pairing_hash", Value::from(self.pairing_hash.as_str())),
            ("repo", Value::from(self.repo.as_str())),
            ("members", Value::Arr(members)),
            ("sealed", Value::Arr(sealed)),
            ("msgs", Value::Arr(msgs)),
        ])
    }

    fn from_json(v: &Value) -> Option<Session> {
        let mut members = Vec::new();
        if let Some(arr) = v.get("members").and_then(|x| x.as_arr()) {
            for mv in arr {
                members.push(Member {
                    device: mv.get("device")?.as_str()?.to_string(),
                    ek: mv.get("ek")?.as_str()?.to_string(),
                    joined_at: mv.get("joined_at").and_then(|x| x.as_i64()).unwrap_or(0) as u64,
                });
            }
        }
        let mut sealed = Vec::new();
        if let Some(arr) = v.get("sealed").and_then(|x| x.as_arr()) {
            for sv in arr {
                sealed.push(SealedPkg {
                    ct: sv.get("ct")?.as_str()?.to_string(),
                    ek_fp: sv.get("ek_fp")?.as_str()?.to_string(),
                });
            }
        }
        let mut msgs = Vec::new();
        if let Some(arr) = v.get("msgs").and_then(|x| x.as_arr()) {
            for mv in arr {
                msgs.push(SessionMsg {
                    seq: mv.get("seq").and_then(|x| x.as_i64()).unwrap_or(0) as u64,
                    sender: mv.get("sender")?.as_str()?.to_string(),
                    nonce: mv.get("nonce")?.as_str()?.to_string(),
                    ct: mv.get("ct")?.as_str()?.to_string(),
                    ts: mv.get("ts").and_then(|x| x.as_i64()).unwrap_or(0) as u64,
                });
            }
        }
        Some(Session {
            id: v.get("id")?.as_str()?.to_string(),
            name: v.get("name")?.as_str()?.to_string(),
            created_by: v.get("created_by")?.as_str()?.to_string(),
            created_at: v.get("created_at").and_then(|x| x.as_i64()).unwrap_or(0) as u64,
            next_seq: v.get("next_seq").and_then(|x| x.as_i64()).unwrap_or(1) as u64,
            pairing_hash: v
                .get("pairing_hash")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            repo: v
                .get("repo")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            members,
            sealed,
            msgs,
        })
    }

    /// Wire shape for GET (messages included; sealed packages omitted —
    /// only the owning member needs them and they arrive at join).
    pub fn to_wire_json(&self, include_msgs: bool) -> Value {
        let members: Vec<Value> = self
            .members
            .iter()
            .map(|m| {
                // ek is a PUBLIC encapsulation key — safe to expose to
                // members (they need it to verify seal routing).
                Value::obj(vec![
                    ("device", Value::from(m.device.as_str())),
                    ("ek", Value::from(m.ek.as_str())),
                    ("joined_at", Value::from(m.joined_at as i64)),
                ])
            })
            .collect();
        let mut pairs = vec![
            ("id", Value::from(self.id.as_str())),
            ("name", Value::from(self.name.as_str())),
            ("created_by", Value::from(self.created_by.as_str())),
            ("created_at", Value::from(self.created_at as i64)),
            ("next_seq", Value::from(self.next_seq as i64)),
            ("repo", Value::from(self.repo.as_str())),
            ("pairing", Value::from(!self.pairing_hash.is_empty())),
            ("members", Value::Arr(members)),
            ("msg_count", Value::from(self.msgs.len() as i64)),
        ];
        if include_msgs {
            let msgs: Vec<Value> = self
                .msgs
                .iter()
                .map(|m| {
                    Value::obj(vec![
                        ("seq", Value::from(m.seq as i64)),
                        ("sender", Value::from(m.sender.as_str())),
                        ("nonce", Value::from(m.nonce.as_str())),
                        ("ct", Value::from(m.ct.as_str())),
                        ("ts", Value::from(m.ts as i64)),
                    ])
                })
                .collect();
            pairs.push(("msgs", Value::Arr(msgs)));
        }
        Value::obj(pairs)
    }
}

pub struct Sessions {
    inner: Mutex<Vec<Session>>,
    path: PathBuf,
}

impl Sessions {
    pub fn load() -> Sessions {
        Self::load_at(&config::sessions_path())
    }

    /// Load from disk; missing or corrupt file yields empty state.
    pub fn load_at(path: &PathBuf) -> Sessions {
        let mut sessions = Vec::new();
        if let Ok(Some(v)) = config::load_json(path) {
            if let Some(arr) = v.get("sessions").and_then(|x| x.as_arr()) {
                for item in arr {
                    if let Some(s) = Session::from_json(item) {
                        sessions.push(s);
                    }
                }
            }
        }
        Sessions {
            inner: Mutex::new(sessions),
            path: path.clone(),
        }
    }

    fn persist(&self, sessions: &[Session]) -> Result<(), String> {
        let arr: Vec<Value> = sessions.iter().map(|s| s.to_file_json()).collect();
        config::save_json(
            &self.path,
            &Value::obj(vec![("sessions", Value::Arr(arr))]),
            0o600,
        )
    }

    pub fn list(&self) -> Vec<Session> {
        self.inner.lock().unwrap().clone()
    }

    pub fn get(&self, id: &str) -> Option<Session> {
        self.inner
            .lock()
            .unwrap()
            .iter()
            .find(|s| s.id == id)
            .cloned()
    }

    /// Create a session with a repo label. Generates a 256-bit pairing
    /// key, stores ONLY its SHA-256 on the hub, and returns the key once
    /// (the operator/creator copies it to joiners out-of-band — same trust
    /// model as the site enroll secret). The creator still self-seals the
    /// session key; joiners redeem the pairing key and get the session key
    /// sealed to their own ek automatically.
    pub fn create(
        &self,
        name: &str,
        created_by: &str,
        ek: &str,
        repo: &str,
    ) -> Result<(Session, String), String> {
        if name.trim().is_empty() || name.chars().count() > MAX_SESSION_NAME_CHARS {
            return Err(format!(
                "session name must be 1..{MAX_SESSION_NAME_CHARS} chars"
            ));
        }
        if repo.chars().count() > MAX_REPO_CHARS {
            return Err(format!("repo label must be 1..{MAX_REPO_CHARS} chars"));
        }
        let mut sessions = self.inner.lock().unwrap();
        if sessions.len() >= MAX_SESSIONS {
            return Err(format!("session registry full (max {MAX_SESSIONS})"));
        }
        let pairing_key = crate::rand::key_hex();
        let pairing_hash = crate::sha256::hexdigest(pairing_key.as_bytes());
        let session = Session {
            id: crate::rand::hex(16),
            name: clamp(name.trim(), MAX_SESSION_NAME_CHARS),
            created_by: clamp(created_by, 64),
            created_at: now_secs(),
            next_seq: 1,
            members: vec![Member {
                device: clamp(created_by, 64),
                ek: ek.to_string(),
                joined_at: now_secs(),
            }],
            sealed: Vec::new(),
            msgs: Vec::new(),
            pairing_hash,
            repo: clamp(repo.trim(), MAX_REPO_CHARS),
        };
        sessions.push(session.clone());
        self.persist(&sessions)?;
        Ok((session, pairing_key))
    }

    /// Constant-time pairing-key check against the stored hash. Empty
    /// stored hash = session predates pairing keys; fail closed.
    pub fn check_pairing(&self, id: &str, key: &str) -> bool {
        let sessions = self.inner.lock().unwrap();
        let Some(s) = sessions.iter().find(|s| s.id == id) else {
            return false;
        };
        if s.pairing_hash.is_empty() || key.is_empty() || key.len() > 256 {
            return false;
        }
        crate::util::ct_eq_str(&s.pairing_hash, &crate::sha256::hexdigest(key.as_bytes()))
    }

    /// Set the repo label on an existing session (creator or dashboard
    /// key holder may re-tag).
    pub fn set_repo(&self, id: &str, repo: &str) -> Result<(), String> {
        if repo.chars().count() > MAX_REPO_CHARS {
            return Err(format!("repo label must be 1..{MAX_REPO_CHARS} chars"));
        }
        let mut sessions = self.inner.lock().unwrap();
        let Some(s) = sessions.iter_mut().find(|s| s.id == id) else {
            return Err("session not found".into());
        };
        s.repo = clamp(repo.trim(), MAX_REPO_CHARS);
        self.persist(&sessions)
    }

    /// Join a session as a new member with a fresh encapsulation key.
    /// Returns (session, sealed packages for this member).
    pub fn join(
        &self,
        id: &str,
        device: &str,
        ek: &str,
    ) -> Result<(Session, Vec<SealedPkg>), String> {
        let mut sessions = self.inner.lock().unwrap();
        {
            let Some(session) = sessions.iter_mut().find(|s| s.id == id) else {
                return Err("session not found".into());
            };
            if session.members.len() >= MAX_MEMBERS {
                return Err(format!("session full (max {MAX_MEMBERS} members)"));
            }
            if session.members.iter().any(|m| m.device == device) {
                return Err("already a member".into());
            }
            session.members.push(Member {
                device: clamp(device, 64),
                ek: ek.to_string(),
                joined_at: now_secs(),
            });
        }
        let session = sessions
            .iter()
            .find(|s| s.id == id)
            .expect("just inserted")
            .clone();
        let sealed: Vec<SealedPkg> = session.sealed.clone();
        self.persist(&sessions)?;
        Ok((session, sealed))
    }

    /// Pairing-validated join: admit the member, refreshing an existing
    /// membership's ek in place (identity rotation) instead of failing.
    /// Returns (session, sealed packages for this member, refreshed?).
    pub fn join_or_refresh(
        &self,
        id: &str,
        device: &str,
        ek: &str,
    ) -> Result<(Session, Vec<SealedPkg>, bool), String> {
        let mut sessions = self.inner.lock().unwrap();
        {
            let Some(session) = sessions.iter_mut().find(|s| s.id == id) else {
                return Err("session not found".into());
            };
            if session.members.len() >= MAX_MEMBERS
                && !session.members.iter().any(|m| m.device == device)
            {
                return Err(format!("session full (max {MAX_MEMBERS} members)"));
            }
            match session.members.iter_mut().find(|m| m.device == device) {
                Some(m) => {
                    m.ek = ek.to_string();
                    m.joined_at = now_secs();
                }
                None => session.members.push(Member {
                    device: clamp(device, 64),
                    ek: ek.to_string(),
                    joined_at: now_secs(),
                }),
            }
        }
        let session = sessions
            .iter()
            .find(|s| s.id == id)
            .expect("just updated")
            .clone();
        let sealed: Vec<SealedPkg> = session.sealed.clone();
        self.persist(&sessions)?;
        Ok((session, sealed, true))
    }

    /// A member posts ML-KEM ciphertexts sealing the session key for
    /// members that joined after creation.
    pub fn post_sealed(
        &self,
        id: &str,
        device: &str,
        pkgs: &[(String, String)],
    ) -> Result<(), String> {
        let mut sessions = self.inner.lock().unwrap();
        let Some(session) = sessions.iter_mut().find(|s| s.id == id) else {
            return Err("session not found".into());
        };
        if !session.members.iter().any(|m| m.device == device) {
            return Err("not a member".into());
        }
        for (ct, fp) in pkgs {
            if ct.len() > 4096 || fp.len() > 64 {
                return Err("sealed package oversized".into());
            }
            session.sealed.push(SealedPkg {
                ct: ct.clone(),
                ek_fp: clamp(fp, 64),
            });
        }
        if session.sealed.len() > MAX_SEALED_PKGS {
            // Keep the most recent packages; old ones are re-sealable.
            let drop = session.sealed.len() - MAX_SEALED_PKGS;
            session.sealed.drain(0..drop);
        }
        self.persist(&sessions)?;
        Ok(())
    }

    /// Fetch sealed packages addressed to a member (by ek fingerprint).
    pub fn take_sealed(&self, id: &str, ek_fp: &str) -> Result<Vec<SealedPkg>, String> {
        let sessions = self.inner.lock().unwrap();
        let Some(session) = sessions.iter().find(|s| s.id == id) else {
            return Err("session not found".into());
        };
        Ok(session
            .sealed
            .iter()
            .filter(|p| p.ek_fp == ek_fp)
            .cloned()
            .collect())
    }

    /// Append a message. The hub assigns the monotonic seq (ordering
    /// authority) and rejects members, oversize, and overflow.
    pub fn post_message(
        &self,
        id: &str,
        sender: &str,
        nonce: &str,
        ct: &str,
    ) -> Result<SessionMsg, String> {
        if nonce.len() != 24 || !nonce.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("nonce must be 24 hex chars (96-bit GCM nonce)".into());
        }
        if ct.is_empty() || ct.len() > MAX_CIPHERTEXT_CHARS * 2 {
            return Err(format!(
                "ciphertext must be 1..{MAX_CIPHERTEXT_CHARS} chars (hex)"
            ));
        }
        let mut sessions = self.inner.lock().unwrap();
        let total: usize = sessions.iter().map(|s| s.msgs.len()).sum();
        if total >= MAX_MSG_TOTAL {
            return Err("message store full".into());
        }
        let Some(session) = sessions.iter_mut().find(|s| s.id == id) else {
            return Err("session not found".into());
        };
        if !session.members.iter().any(|m| m.device == sender) {
            return Err("not a member".into());
        }
        let msg = SessionMsg {
            seq: session.next_seq,
            sender: clamp(sender, 64),
            nonce: nonce.to_string(),
            ct: ct.to_string(),
            ts: now_secs(),
        };
        session.next_seq += 1;
        session.msgs.push(msg.clone());
        if session.msgs.len() > MAX_MSGS_IN_MEMORY {
            let drop = session.msgs.len() - MAX_MSGS_IN_MEMORY;
            session.msgs.drain(0..drop);
        }
        self.persist(&sessions)?;
        Ok(msg)
    }

    /// Messages after a sequence number (for polling).
    pub fn read_messages(&self, id: &str, after_seq: u64) -> Result<Vec<SessionMsg>, String> {
        let sessions = self.inner.lock().unwrap();
        let Some(session) = sessions.iter().find(|s| s.id == id) else {
            return Err("session not found".into());
        };
        Ok(session
            .msgs
            .iter()
            .filter(|m| m.seq > after_seq)
            .cloned()
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_sessions(tag: &str) -> (Sessions, PathBuf) {
        let d = std::env::temp_dir().join(format!(
            "wtf-sessions-{tag}-{}-{}",
            std::process::id(),
            crate::rand::hex(6)
        ));
        let p = d.join("sessions.json");
        (Sessions::load_at(&p), d)
    }

    #[test]
    fn create_join_persist() {
        let (ss, d) = temp_sessions("persist");
        let (s, pairing) = ss
            .create("design chat", "mac-agent", "aakey", "wtf-mcp")
            .unwrap();
        assert_eq!(s.members.len(), 1);
        assert_eq!(s.next_seq, 1);
        assert_eq!(s.repo, "wtf-mcp");
        assert_eq!(pairing.len(), 64);
        assert!(ss.check_pairing(&s.id, &pairing));
        assert!(!ss.check_pairing(&s.id, "wrong"));

        let (joined, sealed) = ss.join(&s.id, "windows-agent", "wbkey").unwrap();
        assert_eq!(joined.members.len(), 2);
        assert!(sealed.is_empty());

        // post_sealed by creator, then take by the joiner's fp
        ss.post_sealed(
            &s.id,
            "mac-agent",
            &[("sealct".into(), "wbfingerprint".into())],
        )
        .unwrap();
        let got = ss.take_sealed(&s.id, "wbfingerprint").unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].ct, "sealct");

        // persistence across reload
        let ss2 = Sessions::load_at(&d.join("sessions.json"));
        let s2 = ss2.get(&s.id).unwrap();
        assert_eq!(s2.members.len(), 2);
        assert_eq!(s2.sealed.len(), 1);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn message_seq_and_membership() {
        let (ss, d) = temp_sessions("msgs");
        let (s, _pk) = ss.create("ops", "box-a", "k", "").unwrap();
        let r1 = ss
            .post_message(&s.id, "box-a", &"a".repeat(24), "ct1")
            .unwrap();
        assert_eq!(r1.seq, 1);
        let r2 = ss
            .post_message(&s.id, "box-a", &"b".repeat(24), "ct2")
            .unwrap();
        assert_eq!(r2.seq, 2);

        // non-member rejected
        assert!(ss
            .post_message(&s.id, "box-z", &"c".repeat(24), "ct3")
            .is_err());

        let msgs = ss.read_messages(&s.id, 0).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].seq, 1);
        let after = ss.read_messages(&s.id, 1).unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].seq, 2);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn caps_fail_closed() {
        let (ss, d) = temp_sessions("caps");
        // bad nonce lengths rejected
        assert!(ss
            .post_message(
                &ss.create("x", "a", "k", "").unwrap().0.id,
                "a",
                "short",
                "c"
            )
            .is_err());
        // oversize name
        assert!(ss.create(&"n".repeat(200), "a", "k", "").is_err());
        // registry cap: "x" from the nonce test already occupies one slot
        for i in 0..MAX_SESSIONS - 1 {
            ss.create(&format!("s{i}"), "a", "k", "").unwrap();
        }
        assert!(ss.create("overflow", "a", "k", "").is_err());
        std::fs::remove_dir_all(&d).ok();
    }
}
