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
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

/// OpenAI-compatible chat completions endpoint of the operator-run router.
pub const ROUTER_CHAT_URL: &str = "http://127.0.0.1:11434/v1/chat/completions";
/// OpenAI-compatible models catalog of the operator-run router.
pub const ROUTER_MODELS_URL: &str = "http://127.0.0.1:11434/v1/models";
/// Ollama-style version probe of the operator-run router.
pub const ROUTER_VERSION_URL: &str = "http://127.0.0.1:11434/api/version";
/// Default base endpoint for the local router proxy.
pub const DEFAULT_ROUTER_BASE: &str = "http://127.0.0.1:11434";
/// Router-side default brain when no preferred model is discovered.
pub const DEFAULT_MODEL: &str = "local-router/fallback-models";

pub const AGENT_HERMES: &str = "hermes";
pub const AGENT_PI: &str = "pi";
pub const SUPPORTED_AGENTS: &[&str] = &[AGENT_HERMES, AGENT_PI];

pub fn normalize_agent(agent: &str) -> &'static str {
    let lower = agent.trim().to_ascii_lowercase();
    if lower == "pi" || lower.starts_with("pi-") {
        AGENT_PI
    } else {
        AGENT_HERMES
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CompletionsConfig {
    pub provider: String,       // "openai" | "anthropic"
    pub endpoint: String,       // e.g. "http://127.0.0.1:11434"
    pub api_key: String,        // optional auth token
    pub model: String,          // target model or empty for discovery
    pub selected_agent: String, // "hermes" | "pi"
    pub system_prompt: String,  // optional default system prompt
}

impl Default for CompletionsConfig {
    fn default() -> Self {
        CompletionsConfig {
            provider: "openai".to_string(),
            endpoint: DEFAULT_ROUTER_BASE.to_string(),
            api_key: String::new(),
            model: String::new(),
            selected_agent: AGENT_HERMES.to_string(),
            system_prompt: String::new(),
        }
    }
}

impl CompletionsConfig {
    pub fn to_json(&self) -> Value {
        Value::obj(vec![
            ("provider", Value::from(self.provider.as_str())),
            ("endpoint", Value::from(self.endpoint.as_str())),
            ("api_key", Value::from(self.api_key.as_str())),
            ("model", Value::from(self.model.as_str())),
            ("selected_agent", Value::from(self.selected_agent.as_str())),
            ("system_prompt", Value::from(self.system_prompt.as_str())),
        ])
    }

    pub fn from_json(v: &Value) -> Self {
        let mut cfg = CompletionsConfig::default();
        if let Some(p) = v.get("provider").and_then(|x| x.as_str()) {
            let p_lower = p.trim().to_ascii_lowercase();
            if p_lower == "anthropic" {
                cfg.provider = "anthropic".to_string();
            } else {
                cfg.provider = "openai".to_string();
            }
        }
        if let Some(e) = v.get("endpoint").and_then(|x| x.as_str()) {
            if !e.trim().is_empty() {
                cfg.endpoint = e.trim().to_string();
            }
        }
        if let Some(k) = v.get("api_key").and_then(|x| x.as_str()) {
            cfg.api_key = k.trim().to_string();
        }
        if let Some(m) = v.get("model").and_then(|x| x.as_str()) {
            cfg.model = m.trim().to_string();
        }
        if let Some(a) = v.get("selected_agent").and_then(|x| x.as_str()) {
            cfg.selected_agent = normalize_agent(a).to_string();
        }
        if let Some(s) = v.get("system_prompt").and_then(|x| x.as_str()) {
            cfg.system_prompt = s.to_string();
        }
        cfg
    }
}

pub fn completions_config_path() -> PathBuf {
    crate::config::home().join("completions.json")
}

static CURRENT_COMPLETIONS_CONFIG: Mutex<Option<CompletionsConfig>> = Mutex::new(None);

pub fn load_completions_config() -> CompletionsConfig {
    if let Ok(guard) = CURRENT_COMPLETIONS_CONFIG.lock() {
        if let Some(ref c) = *guard {
            return c.clone();
        }
    }
    let loaded = match crate::config::load_json(&completions_config_path()) {
        Ok(Some(v)) => CompletionsConfig::from_json(&v),
        _ => CompletionsConfig::default(),
    };
    if let Ok(mut guard) = CURRENT_COMPLETIONS_CONFIG.lock() {
        *guard = Some(loaded.clone());
    }
    loaded
}

pub fn save_completions_config(cfg: &CompletionsConfig) -> Result<(), String> {
    crate::config::save_json(&completions_config_path(), &cfg.to_json(), 0o600)?;
    if let Ok(mut guard) = CURRENT_COMPLETIONS_CONFIG.lock() {
        *guard = Some(cfg.clone());
    }
    Ok(())
}

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

/// Build an Anthropic `/v1/messages` request body. Pure + tested.
pub fn anthropic_request_body(model: &str, prompt: &str, system: Option<&str>) -> String {
    let mut fields: Vec<(&str, Value)> = vec![
        ("model", Value::from(model)),
        ("max_tokens", Value::from(4096i64)),
        (
            "messages",
            Value::Arr(vec![Value::obj(vec![
                ("role", Value::from("user")),
                ("content", Value::from(prompt)),
            ])]),
        ),
    ];
    if let Some(s) = system {
        if !s.trim().is_empty() {
            fields.push(("system", Value::from(s)));
        }
    }
    Value::obj(fields).to_json()
}

/// Parse an Anthropic messages reply into the assistant text content. Pure + tested.
pub fn anthropic_parse_reply(status: u16, body: &str) -> Result<String, String> {
    if status != 200 {
        let snippet: String = body.trim().chars().take(200).collect();
        return Err(format!("anthropic HTTP {status}: {snippet}"));
    }
    let json = crate::json::parse(body).map_err(|e| format!("anthropic reply not JSON: {e}"))?;
    if let Some(Value::Arr(content_arr)) = json.get("content") {
        let mut out = String::new();
        for block in content_arr {
            if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                out.push_str(text);
            }
        }
        if !out.trim().is_empty() {
            return Ok(out);
        }
    }
    Err("unexpected anthropic reply shape (no content[0].text)".into())
}

/// URL resolution for OpenAI chat endpoint.
pub fn resolve_openai_chat_url(base: &str) -> String {
    let clean = base.trim().trim_end_matches('/');
    if clean.ends_with("/chat/completions") {
        clean.to_string()
    } else if clean.ends_with("/v1") {
        format!("{clean}/chat/completions")
    } else {
        format!("{clean}/v1/chat/completions")
    }
}

/// URL resolution for Anthropic messages endpoint.
pub fn resolve_anthropic_messages_url(base: &str) -> String {
    let clean = base.trim().trim_end_matches('/');
    if clean.ends_with("/messages") {
        clean.to_string()
    } else if clean.ends_with("/v1") {
        format!("{clean}/messages")
    } else {
        format!("{clean}/v1/messages")
    }
}

/// Chat completion using the loaded/active CompletionsConfig or custom overrides.
/// Returns Ok((response_content, model_used)).
pub fn chat_completion_with_config(
    prompt: &str,
    cfg: &CompletionsConfig,
    model_override: Option<&str>,
    system_override: Option<&str>,
) -> Result<(String, String), String> {
    if prompt.trim().is_empty() {
        return Err("empty prompt".into());
    }

    let is_local = cfg.endpoint.contains("127.0.0.1") || cfg.endpoint.contains("localhost");
    if is_local && !router_alive() {
        return Err(
            "local-router offline on 127.0.0.1:11434 — start it yourself (operator-run); wtf never spawns it"
                .into(),
        );
    }

    let model = model_override
        .map(str::to_string)
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            if !cfg.model.trim().is_empty() {
                Some(cfg.model.clone())
            } else {
                None
            }
        })
        .unwrap_or_else(|| preferred_model(&router_models()));

    let system = system_override.or_else(|| {
        if !cfg.system_prompt.trim().is_empty() {
            Some(cfg.system_prompt.as_str())
        } else {
            None
        }
    });

    if cfg.provider == "anthropic" {
        let url = resolve_anthropic_messages_url(&cfg.endpoint);
        let body = anthropic_request_body(&model, prompt, system);
        let mut headers = vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            ("anthropic-version".to_string(), "2023-06-01".to_string()),
        ];
        if !cfg.api_key.trim().is_empty() {
            headers.push(("x-api-key".to_string(), cfg.api_key.clone()));
        } else {
            headers.push(("x-api-key".to_string(), "local-router".to_string()));
        }
        let resp = crate::client::request_with_timeout(
            &url,
            "POST",
            &headers,
            body.as_bytes(),
            Duration::from_secs(5),
            Duration::from_secs(180),
        )?;
        let content = anthropic_parse_reply(resp.status, &resp.text())?;
        Ok((content, model))
    } else {
        let url = resolve_openai_chat_url(&cfg.endpoint);
        let body = chat_request_body(&model, prompt, system);
        let mut headers = vec![("Content-Type".to_string(), "application/json".to_string())];
        if !cfg.api_key.trim().is_empty() {
            headers.push((
                "Authorization".to_string(),
                format!("Bearer {}", cfg.api_key.trim()),
            ));
        }
        let resp = crate::client::request_with_timeout(
            &url,
            "POST",
            &headers,
            body.as_bytes(),
            Duration::from_secs(5),
            Duration::from_secs(180),
        )?;
        let content = chat_parse_reply(resp.status, &resp.text())?;
        Ok((content, model))
    }
}

/// Chat with the configured completions backend (defaults to local-router :11434).
pub fn chat_completion(
    prompt: &str,
    model: Option<&str>,
    system: Option<&str>,
) -> Result<String, String> {
    let cfg = load_completions_config();
    chat_completion_with_config(prompt, &cfg, model, system).map(|(content, _)| content)
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

    #[test]
    fn anthropic_request_body_shape() {
        let body = anthropic_request_body("claude-3-5-sonnet", "hi", Some("be terse"));
        let v = crate::json::parse(&body).unwrap();
        assert_eq!(v.get("model").and_then(|x| x.as_str()), Some("claude-3-5-sonnet"));
        assert_eq!(v.get("system").and_then(|x| x.as_str()), Some("be terse"));
        let msgs = v.get("messages").and_then(|x| x.as_arr()).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].get("role").and_then(|x| x.as_str()), Some("user"));
        assert_eq!(msgs[0].get("content").and_then(|x| x.as_str()), Some("hi"));
    }

    #[test]
    fn anthropic_parse_reply_extracts_content() {
        let ok = r#"{"content":[{"type":"text","text":"hello from anthropic"}]}"#;
        assert_eq!(anthropic_parse_reply(200, ok).unwrap(), "hello from anthropic");
        assert!(anthropic_parse_reply(500, "err").is_err());
        assert!(anthropic_parse_reply(200, "{\"content\":[]}").is_err());
    }

    #[test]
    fn completions_config_roundtrip() {
        let cfg = CompletionsConfig {
            provider: "anthropic".to_string(),
            endpoint: "http://127.0.0.1:11434".to_string(),
            api_key: "test-token".to_string(),
            model: "claude-test".to_string(),
            selected_agent: "pi".to_string(),
            system_prompt: "act as pi".to_string(),
        };
        let val = cfg.to_json();
        let loaded = CompletionsConfig::from_json(&val);
        assert_eq!(loaded, cfg);
        assert_eq!(normalize_agent("pi"), "pi");
        assert_eq!(normalize_agent("hermes"), "hermes");
        assert_eq!(normalize_agent("unknown"), "hermes");
    }
}
