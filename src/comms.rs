//! COMMS — structured ledger entries carried inside encrypted session
//! channels (bridge side).
//!
//! A COMMS entry is a small JSON envelope sent as the plaintext of an
//! ordinary encrypted session message (see `session_crypto`). The session
//! layer already provides confidentiality (AES-256-GCM), integrity +
//! replay protection (AAD binds session id / sender / seq), and membership
//! (ML-KEM-768 sealed session keys; the hub stores ciphertext only). COMMS
//! adds structure on top: ledger-vocabulary event types and a scope field
//! so agents can run the `AGENTS/{date}.COMMS.md` protocol across repos,
//! worktrees, subagents, and machines — fast, without git commits, and
//! without the user relaying messages.
//!
//! Secrets mandate: bins and the event feed are PUBLIC surfaces. Anything
//! confidential between agents — credentials, keys, private findings —
//! travels ONLY inside session/COMMS channels, which are encrypted at rest
//! (hub stores ciphertext under 0600) and in transit (ciphertext on the
//! wire); only members hold the keys to decrypt.
//!
//! The durable audit trail remains the git ledger; the encrypted channel is
//! the fast in-flight lane (the hub ring keeps the last
//! `sessions::MAX_MSGS_IN_MEMORY` messages per session).

use crate::json;
use crate::util::now_secs;

/// Envelope type marker (versioned).
pub const ENVELOPE_TYPE: &str = "wtf-comms-v1";

/// Note size cap (chars). The session ciphertext cap is the hard ceiling.
pub const MAX_NOTE_CHARS: usize = 2000;
/// Scope size cap (chars).
pub const MAX_SCOPE_CHARS: usize = 160;

/// Ledger vocabulary — mirrors the `AGENTS/{date}.COMMS.md` lifecycle
/// (checkin → update → intent-merge → checkout) plus cross-agent extras.
pub const EVENTS: &[&str] = &[
    "checkin",
    "update",
    "intent-merge",
    "checkout",
    "blocked",
    "announce",
    "handoff",
];

pub fn valid_event(event: &str) -> bool {
    EVENTS.contains(&event)
}

/// Build + validate one envelope, serialized for encryption. Fails closed
/// on unknown event types, empty notes, and oversize fields.
pub fn build(event: &str, scope: &str, note: &str) -> Result<String, String> {
    if !valid_event(event) {
        return Err(format!(
            "invalid event '{event}'; must be one of: {}",
            EVENTS.join(", ")
        ));
    }
    let note = note.trim();
    if note.is_empty() {
        return Err("note must not be empty".into());
    }
    if note.chars().count() > MAX_NOTE_CHARS {
        return Err(format!("note too large (max {MAX_NOTE_CHARS} chars)"));
    }
    let scope = scope.trim();
    if scope.chars().count() > MAX_SCOPE_CHARS {
        return Err(format!("scope too large (max {MAX_SCOPE_CHARS} chars)"));
    }
    let env = json::Value::obj(vec![
        ("t", json::Value::from(ENVELOPE_TYPE)),
        ("event", json::Value::from(event)),
        ("scope", json::Value::from(scope)),
        ("note", json::Value::from(note)),
        ("ts", json::Value::from(now_secs() as i64)),
    ]);
    Ok(env.to_json())
}

/// A parsed COMMS envelope.
pub struct Entry {
    pub event: String,
    pub scope: String,
    pub note: String,
    pub ts: u64,
}

/// Parse decrypted plaintext into an envelope. Returns `None` when the
/// message is not a COMMS envelope (e.g. a plain `session_send` on the
/// same channel) — callers render those as raw lines, never crash.
pub fn parse(plaintext: &str) -> Option<Entry> {
    let v = json::parse(plaintext).ok()?;
    if v.get("t").and_then(|x| x.as_str()) != Some(ENVELOPE_TYPE) {
        return None;
    }
    let event = v.get("event").and_then(|x| x.as_str())?.to_string();
    if !valid_event(&event) {
        return None;
    }
    let note = v.get("note").and_then(|x| x.as_str())?.to_string();
    let scope = v
        .get("scope")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let ts = v.get("ts").and_then(|x| x.as_i64()).unwrap_or(0).max(0) as u64;
    Some(Entry {
        event,
        scope,
        note,
        ts,
    })
}

/// Render one decrypted message as a ledger line, e.g.
/// `#12 [update] mac-agent (wtf/feat/x) (34s ago): rebased; gates green`.
pub fn render_line(seq: u64, sender: &str, entry: &Entry, now: u64) -> String {
    let age = now.saturating_sub(entry.ts);
    if entry.scope.is_empty() {
        format!(
            "#{seq} [{event}] {sender} ({age}s ago): {note}",
            seq = seq,
            event = entry.event,
            sender = sender,
            age = age,
            note = entry.note
        )
    } else {
        format!(
            "#{seq} [{event}] {sender} ({scope}) ({age}s ago): {note}",
            seq = seq,
            event = entry.event,
            sender = sender,
            scope = entry.scope,
            age = age,
            note = entry.note
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_parse_roundtrip() {
        let env = build(
            "update",
            "wtf-is-going-on-mcp/feat/comms",
            "gates green; merging",
        )
        .unwrap();
        let entry = parse(&env).expect("envelope parses");
        assert_eq!(entry.event, "update");
        assert_eq!(entry.scope, "wtf-is-going-on-mcp/feat/comms");
        assert_eq!(entry.note, "gates green; merging");
        assert!(entry.ts > 0);
    }

    #[test]
    fn build_fails_closed() {
        assert!(
            build("bogus", "", "note").is_err(),
            "unknown event rejected"
        );
        assert!(build("update", "", "   ").is_err(), "empty note rejected");
        let long = "n".repeat(MAX_NOTE_CHARS + 1);
        assert!(
            build("update", "", &long).is_err(),
            "oversize note rejected"
        );
        let long_scope = "s".repeat(MAX_SCOPE_CHARS + 1);
        assert!(
            build("update", &long_scope, "note").is_err(),
            "oversize scope rejected"
        );
    }

    #[test]
    fn parse_rejects_non_envelopes() {
        assert!(parse("plain session chat").is_none());
        assert!(parse("{\"t\":\"other-v1\",\"event\":\"update\"}").is_none());
        // Known marker but unknown event → not a valid entry.
        assert!(parse("{\"t\":\"wtf-comms-v1\",\"event\":\"meh\",\"note\":\"x\"}").is_none());
        // Known marker + event but missing note → not a valid entry.
        assert!(parse("{\"t\":\"wtf-comms-v1\",\"event\":\"update\"}").is_none());
    }

    #[test]
    fn render_includes_scope_and_age() {
        let now = 1_000_000;
        let e = Entry {
            event: "checkin".into(),
            scope: "repo/branch".into(),
            note: "starting".into(),
            ts: now - 30,
        };
        let line = render_line(7, "mac-agent", &e, now);
        assert!(
            line.starts_with("#7 [checkin] mac-agent (repo/branch) (30s ago): starting"),
            "{line}"
        );
        let bare = Entry {
            scope: String::new(),
            ..e
        };
        let line = render_line(8, "box-b", &bare, now);
        assert!(
            line.starts_with("#8 [checkin] box-b (30s ago): starting"),
            "{line}"
        );
    }
}
