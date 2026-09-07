//! Router chat: talk to the operator-run local-router's OpenAI-compatible
//! endpoint (`http://127.0.0.1:11434/v1/chat/completions`) from the machine
//! running wtf. This is deliberately ALL the inference this repo does:
//! passive status probing, live model discovery, and OpenAI completions.
//!
//! Invariants (operator directives 2026-09-04 / 2026-09-07):
//! - wtf NEVER spawns, stops, restarts, or supervises `local-router`,
//!   `fcc-server`, or `fcc-dsh`. The router is operator-owned infrastructure;
//!   when it is down every entry point fails with a clear message instead of
//!   trying to fix the world.
//! - No agent-harness CLIs are launched by this repo anymore. Cross-machine
//!   agent work happens through the machines' own tooling, never through wtf.

use crate::json::Value;
use std::sync::Mutex;
use std::time::Duration;

/// OpenAI-compatible chat completions endpoint of the operator-run router.
pub const ROUTER_CHAT_URL: &str = "http://127.0.0.1:11434/v1/chat/completions";
/// OpenAI-compatible models catalog of the operator-run router.
pub const ROUTER_MODELS_URL: &str = "http://127.0.0.1:11434/v1/models";
/// Ollama-style version probe of the operator-run router.
pub const ROUTER_VERSION_URL: &str = "http://127.0.0.1:11434/api/version";
/// Router-side default brain when no preferred model is discovered.
pub const DEFAULT_MODEL: &str = "local-router/fallback-models";

static ROUTER_CACHE: Mutex<Option<(u64, Value)>> = Mutex::new(None);

pub fn invalidate_router_cache() {
    if let Ok(mut guard) = ROUTER_CACHE.lock() {
        *guard = None;
    }
}

/// PASSIVE probe of the operator-run router: version + live model catalog.
/// Read-only HTTP GETs with tight timeouts; never starts anything.
pub fn probe_router_info() -> Value {
    let mut alive = false;
    let mut version = String::new();
    let mut models_count = 0usize;
    let mut models_vec: Vec<Value> = Vec::new();

    // 1. Probe version
    if let Ok(resp) = crate::client::request_with_timeout(
        ROUTER_VERSION_URL,
        "GET",
        &[],
        b"",
        Duration::from_millis(500),
        Duration::from_millis(800),
    ) {
        if resp.status == 200 {
            alive = true;
            if let Some(json) = resp.json() {
                if let Some(v) = json.get("version").and_then(|x| x.as_str()) {
                    version = v.to_string();
                }
            }
        }
    }

    // 2. Probe models catalog
    if let Ok(resp) = crate::client::request_with_timeout(
        ROUTER_MODELS_URL,
        "GET",
        &[],
        b"",
        Duration::from_millis(500),
        Duration::from_millis(1500),
    ) {
        if resp.status == 200 {
            alive = true;
            if let Some(json) = resp.json() {
                if let Some(Value::Arr(data)) = json.get("data") {
                    models_count = data.len();
                    for item in data {
                        if let Some(Value::Str(id)) = item.get("id") {
                            models_vec.push(Value::from(id.as_str()));
                        }
                    }
                }
            }
        }
    }

    Value::obj(vec![
        ("alive", Value::from(alive)),
        ("endpoint", Value::from("http://127.0.0.1:11434/v1")),
        ("target_model", Value::from(DEFAULT_MODEL)),
        ("version", Value::from(version.as_str())),
        ("models_count", Value::from(models_count as i64)),
        ("models", Value::Arr(models_vec)),
        ("last_checked", Value::from(crate::util::now_secs() as i64)),
    ])
}

/// Cached passive probe (2 s TTL).
pub fn router_state_json() -> Value {
    let now = crate::util::now_secs();
    if let Ok(guard) = ROUTER_CACHE.lock() {
        if let Some((ts, ref val)) = *guard {
            if now.saturating_sub(ts) < 2 {
                return val.clone();
            }
        }
    }
    let fresh = probe_router_info();
    if let Ok(mut guard) = ROUTER_CACHE.lock() {
        *guard = Some((now, fresh.clone()));
    }
    fresh
}

/// Passive read-only reachability check for the operator-run router.
pub fn router_alive() -> bool {
    let state = router_state_json();
    state.get("alive").and_then(|v| v.as_bool()).unwrap_or(false)
}

/// Model discovery (operator directive 2026-09-07): pick the preferred model
/// from the operator-run router's live `/v1/models` catalog — the llama.cpp
/// qwen3 abliterated tier first, then any qwen3, then any llama.cpp build,
/// else the router's own default brain. Pure so the tier policy is testable.
pub fn preferred_model(models: &[String]) -> String {
    for m in models {
        let l = m.to_ascii_lowercase();
        if l.contains("qwen3") && l.contains("abliterat") {
            return m.clone();
        }
    }
    for m in models {
        if m.to_ascii_lowercase().contains("qwen3") {
            return m.clone();
        }
    }
    for m in models {
        if m.to_ascii_lowercase().contains("llama.cpp") {
            return m.clone();
        }
    }
    DEFAULT_MODEL.to_string()
}

/// Models currently advertised by the operator-run router (passive read of
/// the cached probe state; empty when :11434 is down).
pub fn router_models() -> Vec<String> {
    let mut out = Vec::new();
    if let Some(Value::Arr(items)) = router_state_json().get("models") {
        for it in items {
            if let Value::Str(id) = it {
                out.push(id.clone());
            }
        }
    }
    out
}

/// Build an OpenAI `/v1/chat/completions` request body. Pure + tested.
pub fn chat_request_body(model: &str, prompt: &str, system: Option<&str>) -> String {
    let mut messages: Vec<Value> = Vec::new();
    if let Some(s) = system {
        if !s.trim().is_empty() {
            messages.push(Value::obj(vec![
                ("role", Value::from("system")),
                ("content", Value::from(s)),
            ]));
        }
    }
    messages.push(Value::obj(vec![
        ("role", Value::from("user")),
        ("content", Value::from(prompt)),
    ]));
    Value::obj(vec![
        ("model", Value::from(model)),
        ("messages", Value::Arr(messages)),
        ("stream", Value::from(false)),
    ])
    .to_json()
}

/// Parse an OpenAI chat-completions reply into the assistant content. Pure +
/// tested; fails closed with a readable error on any non-200 or odd shape.
pub fn chat_parse_reply(status: u16, body: &str) -> Result<String, String> {
    if status != 200 {
        let snippet: String = body.trim().chars().take(200).collect();
        return Err(format!("router HTTP {status}: {snippet}"));
    }
    let json = crate::json::parse(body).map_err(|e| format!("router reply not JSON: {e}"))?;
    let content = json
        .get("choices")
        .and_then(|c| c.as_arr())
        .and_then(|arr| arr.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .map(str::to_string);
    match content {
        Some(text) if !text.trim().is_empty() => Ok(text),
        Some(_) => Err("router returned an empty completion".into()),
        None => Err("unexpected router reply shape (no choices[0].message.content)".into()),
    }
}

/// Chat with the operator-run local-router via OpenAI completions. Fails
/// closed when :11434 is down — wtf never tries to start it. `model` falls
/// back to `preferred_model()` discovery; `system` is an optional preamble.
pub fn chat_completion(
    prompt: &str,
    model: Option<&str>,
    system: Option<&str>,
) -> Result<String, String> {
    if prompt.trim().is_empty() {
        return Err("empty prompt".into());
    }
    if !router_alive() {
        return Err(
            "local-router offline on 127.0.0.1:11434 — start it yourself (operator-run); wtf never spawns it"
                .into(),
        );
    }
    let model = model
        .map(str::to_string)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| preferred_model(&router_models()));
    let body = chat_request_body(&model, prompt, system);
    let resp = crate::client::request_with_timeout(
        ROUTER_CHAT_URL,
        "POST",
        &[("Content-Type".to_string(), "application/json".to_string())],
        body.as_bytes(),
        Duration::from_secs(5),
        Duration::from_secs(180),
    )?;
    chat_parse_reply(resp.status, &resp.text())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Model discovery prefers the llama.cpp qwen3 abliterated tier.
    #[test]
    fn preferred_model_tiers() {
        let m =
            |ids: &[&str]| preferred_model(&ids.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(
            m(&["gpt-4o", "llama.cpp/qwen3-8b-abliterated"]),
            "llama.cpp/qwen3-8b-abliterated"
        );
        // abliterated beats plain qwen3 regardless of order
        assert_eq!(
            m(&["qwen3-4b-instruct", "llama.cpp/qwen3-8b-abliterated"]),
            "llama.cpp/qwen3-8b-abliterated"
        );
        assert_eq!(m(&["qwen3-4b", "smollm"]), "qwen3-4b");
        assert_eq!(m(&["llama.cpp/other"]), "llama.cpp/other");
        assert_eq!(m(&["random-model"]), DEFAULT_MODEL);
        assert_eq!(m(&[]), DEFAULT_MODEL);
    }

    #[test]
    fn chat_request_body_shape() {
        let body = chat_request_body("llama.cpp/qwen3-8b-abliterated", "hi", Some("be terse"));
        let v = crate::json::parse(&body).unwrap();
        assert_eq!(
            v.get("model").and_then(|x| x.as_str()),
            Some("llama.cpp/qwen3-8b-abliterated")
        );
        let msgs = v.get("messages").and_then(|x| x.as_arr()).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].get("role").and_then(|x| x.as_str()), Some("system"));
        assert_eq!(msgs[1].get("role").and_then(|x| x.as_str()), Some("user"));
        assert_eq!(msgs[1].get("content").and_then(|x| x.as_str()), Some("hi"));
        // no system preamble -> single user message
        let body2 = chat_request_body("m", "hello", None);
        let v2 = crate::json::parse(&body2).unwrap();
        assert_eq!(v2.get("messages").and_then(|x| x.as_arr()).map(|a| a.len()), Some(1));
        // streaming is off — callers read one complete completion
        assert_eq!(v2.get("stream").and_then(|x| x.as_bool()), Some(false));
    }

    #[test]
    fn chat_parse_reply_extracts_content() {
        let ok = r#"{"choices":[{"message":{"role":"assistant","content":"ship it"}}]}"#;
        assert_eq!(chat_parse_reply(200, ok).unwrap(), "ship it");
        assert!(chat_parse_reply(500, "boom").is_err());
        assert!(chat_parse_reply(200, "{\"choices\":[]}").is_err());
        assert!(chat_parse_reply(200, "not json").is_err());
        assert!(
            chat_parse_reply(200, "{\"choices\":[{\"message\":{\"content\":\"  \"}}]}").is_err()
        );
    }
}
