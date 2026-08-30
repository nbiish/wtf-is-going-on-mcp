//! MCP stdio bridge: the process MCP clients launch. Speaks newline-delimited
//! JSON-RPC 2.0 on stdin/stdout and forwards tool calls to the hub over
//! HMAC-signed HTTP. Tool failures become `isError:true` results, never
//! protocol errors.
//!
//! Heartbeats: a background thread pings the hub every 60 s so the dashboard
//! can show machine liveness between agent activities.

use crate::client;
use crate::config::BridgeConfig;
use crate::json::{self, Value};
use crate::rand::nonce_hex;
use crate::store::{LEVELS, STATUSES, rel_age};
use crate::util::now_secs;
use std::io::{BufRead, Write};
use std::time::Duration;

const HEARTBEAT_SECS: u64 = 60;

pub struct Bridge {
    pub cfg: BridgeConfig,
}

// ---------- signed HTTP helpers ----------

impl Bridge {
    fn signed_headers(
        &self,
        method: &str,
        path_and_query: &str,
        body: &[u8],
    ) -> Result<Vec<(String, String)>, String> {
        let ts = now_secs();
        let nonce = nonce_hex();
        let sig = crate::auth::sign(&self.cfg.device_key, method, path_and_query, ts, &nonce, body)
            .ok_or_else(|| "device key is not valid hex".to_string())?;
        Ok(vec![
            ("X-Wtf-Device".into(), self.cfg.device_name.clone()),
            ("X-Wtf-Timestamp".into(), ts.to_string()),
            ("X-Wtf-Nonce".into(), nonce),
            ("X-Wtf-Signature".into(), sig),
        ])
    }

    fn decode(&self, resp: client::ClientResponse, path: &str) -> Result<Value, String> {
        if resp.status == 401 {
            return Err(format!("hub rejected credentials for {path} (HTTP 401) — key revoked or wrong?"));
        }
        if resp.status != 200 {
            return Err(format!("hub returned HTTP {} for {path}", resp.status));
        }
        json::parse(&resp.text()).map_err(|e| format!("hub sent invalid JSON for {path}: {e}"))
    }

    pub fn api_post(&self, path: &str, body: &Value) -> Result<Value, String> {
        let body_str = body.to_json();
        let headers = self.signed_headers("POST", path, body_str.as_bytes())?;
        let url = format!("{}{}", self.cfg.hub_url, path);
        let resp = client::request(&url, "POST", &headers, body_str.as_bytes())?;
        self.decode(resp, path)
    }

    pub fn api_get(&self, path: &str) -> Result<Value, String> {
        let headers = self.signed_headers("GET", path, b"")?;
        let url = format!("{}{}", self.cfg.hub_url, path);
        let resp = client::request(&url, "GET", &headers, b"")?;
        self.decode(resp, path)
    }

    pub fn api_put(&self, path: &str, body: &Value) -> Result<Value, String> {
        let body_str = body.to_json();
        let headers = self.signed_headers("PUT", path, body_str.as_bytes())?;
        let url = format!("{}{}", self.cfg.hub_url, path);
        let resp = client::request(&url, "PUT", &headers, body_str.as_bytes())?;
        self.decode(resp, path)
    }
}

/// Signed state fetch, shared with `wtf status`.
pub fn fetch_state(cfg: &BridgeConfig) -> Result<Value, String> {
    Bridge { cfg: cfg.clone() }.api_get("/api/v1/state")
}

// ---------- state text formatting (dashboard-independent) ----------

fn hms(ts: u64) -> String {
    let d = ts % 86400;
    format!("{:02}:{:02}:{:02}", d / 3600, (d % 3600) / 60, d % 60)
}

/// Render hub state as plain text (used by `wtf_is_going_on` and `wtf status`).
pub fn format_state(state: &Value, hub_label: &str) -> String {
    let now = state
        .get("server")
        .and_then(|s| s.get("now"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0) as u64;
    let mut out = String::new();
    out.push_str(&format!("WTF IS GOING ON — hub {hub_label}\n"));
    let agents = state.get("agents").and_then(|v| v.as_arr()).unwrap_or(&[]);
    let events = state.get("events").and_then(|v| v.as_arr()).unwrap_or(&[]);
    let bins = state.get("bins").and_then(|v| v.as_arr()).unwrap_or(&[]);
    if bins
        .iter()
        .any(|b| b.get("size").and_then(|x| x.as_i64()).unwrap_or(0) > 0)
    {
        out.push_str("\nBINS (shared; read_bin to fetch, write_bin to publish)\n");
        for b in bins {
            let size = b.get("size").and_then(|x| x.as_i64()).unwrap_or(0);
            if size == 0 {
                continue;
            }
            let id = b.get("id").and_then(|x| x.as_i64()).unwrap_or(0);
            let by = b.get("updated_by").and_then(|x| x.as_str()).unwrap_or("?");
            let at = b.get("updated_at").and_then(|x| x.as_i64()).unwrap_or(0) as u64;
            out.push_str(&format!(
                "  BIN {id} — {size} chars — updated {} ago by {by}\n",
                rel_age(now, at)
            ));
        }
    }
    out.push_str(&format!(
        "{} agent(s) tracked · showing {} recent event(s)\n",
        agents.len(),
        events.len()
    ));
    out.push_str("\nAGENTS\n");
    if agents.is_empty() {
        out.push_str("  (none have checked in yet)\n");
    }
    for a in agents {
        let agent = a.get("agent").and_then(|v| v.as_str()).unwrap_or("?");
        let device = a.get("device").and_then(|v| v.as_str()).unwrap_or("?");
        let status = a.get("status").and_then(|v| v.as_str()).unwrap_or("?");
        let task = a.get("task").and_then(|v| v.as_str()).unwrap_or("");
        let details = a.get("details").and_then(|v| v.as_str()).unwrap_or("");
        let last = a.get("last_seen").and_then(|v| v.as_i64()).unwrap_or(0) as u64;
        let stale = a.get("stale").and_then(|v| v.as_bool()).unwrap_or(false);
        let mark = if stale { "○" } else { "●" };
        out.push_str(&format!(
            "  {mark} {agent}@{device} [{status}] ({}, utc {})\n",
            rel_age(now, last),
            hms(last)
        ));
        if !task.is_empty() {
            out.push_str(&format!("      task: {task}\n"));
        }
        if !details.is_empty() {
            out.push_str(&format!("      details: {details}\n"));
        }
    }
    out.push_str("\nEVENTS (newest first)\n");
    if events.is_empty() {
        out.push_str("  (no events yet)\n");
    }
    for e in events.iter().rev() {
        let id = e.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
        let ts = e.get("ts").and_then(|v| v.as_i64()).unwrap_or(0) as u64;
        let agent = e.get("agent").and_then(|v| v.as_str()).unwrap_or("?");
        let device = e.get("device").and_then(|v| v.as_str()).unwrap_or("?");
        let level = e.get("level").and_then(|v| v.as_str()).unwrap_or("info");
        let message = e.get("message").and_then(|v| v.as_str()).unwrap_or("");
        out.push_str(&format!(
            "  #{id} {} [{level}] {agent}@{device}: {message}\n",
            hms(ts)
        ));
    }
    out
}

// ---------- JSON-RPC ----------

const ERR_PARSE: i64 = -32700;
const ERR_REQUEST: i64 = -32600;
const ERR_METHOD: i64 = -32601;
const ERR_PARAMS: i64 = -32602;

fn error_resp(id: Value, code: i64, msg: &str) -> Value {
    Value::obj(vec![
        ("jsonrpc", Value::from("2.0")),
        ("id", id),
        (
            "error",
            Value::obj(vec![("code", Value::from(code)), ("message", Value::from(msg))]),
        ),
    ])
}

fn ok_resp(id: Value, result: Value) -> Value {
    Value::obj(vec![
        ("jsonrpc", Value::from("2.0")),
        ("id", id),
        ("result", result),
    ])
}

fn text_result(text: String, is_error: bool) -> Value {
    Value::obj(vec![
        (
            "content",
            Value::arr(vec![Value::obj(vec![
                ("type", Value::from("text")),
                ("text", Value::from(text)),
            ])]),
        ),
        ("isError", Value::from(is_error)),
    ])
}

fn arg_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(|v| v.as_str())
}

fn int_prop(desc: &str) -> Value {
    Value::obj(vec![
        ("type", Value::from("integer")),
        ("description", Value::from(desc)),
    ])
}

impl Bridge {
    /// Handle one stdin line. Returns Some(response JSON) or None (silent).
    pub fn handle_line(&self, line: &str) -> Option<Value> {
        let v = match json::parse(line) {
            Ok(v) => v,
            Err(e) => return Some(error_resp(Value::Null, ERR_PARSE, &format!("parse error: {e}"))),
        };
        let id = v.get("id").cloned();
        let method = v.get("method").and_then(|m| m.as_str()).map(|s| s.to_string());
        match (method, id) {
            // notification (no id): never answered
            (Some(_), None) => None,
            (Some(method), Some(id)) => match self.dispatch(&method, v.get("params")) {
                Ok(result) => Some(ok_resp(id, result)),
                Err((code, msg)) => Some(error_resp(id, code, &msg)),
            },
            (None, Some(id)) => Some(error_resp(id, ERR_REQUEST, "request missing 'method'")),
            (None, None) => None,
        }
    }

    fn dispatch(&self, method: &str, params: Option<&Value>) -> Result<Value, (i64, String)> {
        match method {
            "initialize" => {
                // Echo the client's protocol version when provided.
                let pv = params
                    .and_then(|p| p.get("protocolVersion"))
                    .and_then(|v| v.as_str())
                    .map(|s| Value::from(s))
                    .unwrap_or_else(|| Value::from("2025-06-18"));
                Ok(Value::obj(vec![
                    ("protocolVersion", pv),
                    (
                        "instructions",
                        Value::from(
                            "All status reporting MUST use chain-of-draft style: terse fragments of <=5 words each, no prose paragraphs, no secrets (e.g. \"fixing auth replay bug; hub restarted; blocked on sshd\"). The user reads these updates on the dashboard to see what the fuck is going on — keep them scannable and frequent.",
                        ),
                    ),
                    (
                        "capabilities",
                        Value::obj(vec![(
                            "tools",
                            Value::obj(vec![("listChanged", Value::from(false))]),
                        )]),
                    ),
                    (
                        "serverInfo",
                        Value::obj(vec![
                            ("name", Value::from("wtf")),
                            ("title", Value::from("WTF Is Going On")),
                            ("version", Value::from(crate::VERSION)),
                        ]),
                    ),
                ]))
            }
            "ping" => Ok(Value::obj(vec![])),
            "tools/list" => Ok(self.tools_list()),
            "tools/call" => self.tools_call(params),
            other => Err((ERR_METHOD, format!("method not found: {other}"))),
        }
    }

    fn prop(desc: &str) -> Value {
        Value::obj(vec![
            ("type", Value::from("string")),
            ("description", Value::from(desc)),
        ])
    }

    fn tools_list(&self) -> Value {
        let status_schema = Value::obj(vec![
            ("type", Value::from("string")),
            (
                "enum",
                Value::arr(STATUSES.iter().map(|s| Value::from(*s)).collect()),
            ),
            ("description", Value::from("working | blocked | done | idle")),
        ]);
        let level_schema = Value::obj(vec![
            ("type", Value::from("string")),
            (
                "enum",
                Value::arr(LEVELS.iter().map(|l| Value::from(*l)).collect()),
            ),
            ("description", Value::from("info | warn | error")),
        ]);
        let tools = vec![
            Value::obj(vec![
                ("name", Value::from("check_in")),
                (
                    "description",
                    Value::from(
                        "Report your current status to the team hub so the user can see what the fuck is going on. task and details MUST be chain-of-draft: terse fragments, <=5 words each, no prose.",
                    ),
                ),
                (
                    "inputSchema",
                    Value::obj(vec![
                        ("type", Value::from("object")),
                        (
                            "properties",
                            Value::obj(vec![
                                ("status", status_schema),
                                ("task", Self::prop("chain-of-draft fragment: what you are doing right now, <=5 words")),
                                ("details", Self::prop("optional chain-of-draft line: blockers or progress, <=5 words per fragment")),
                                (
                                    "agent",
                                    Self::prop("optional agent name; defaults to this device"),
                                ),
                            ]),
                        ),
                        (
                            "required",
                            Value::arr(vec![Value::from("status"), Value::from("task")]),
                        ),
                        ("additionalProperties", Value::from(false)),
                    ]),
                ),
            ]),
            Value::obj(vec![
                ("name", Value::from("log_event")),
                (
                    "description",
                    Value::from(
                        "Append a log line to the shared team event feed. message MUST be chain-of-draft: one terse line, <=5 words per fragment, no prose.",
                    ),
                ),
                (
                    "inputSchema",
                    Value::obj(vec![
                        ("type", Value::from("object")),
                        (
                            "properties",
                            Value::obj(vec![
                                ("message", Self::prop("chain-of-draft one-liner: what happened, <=5 words")),
                                ("level", level_schema),
                                (
                                    "agent",
                                    Self::prop("optional agent name; defaults to this device"),
                                ),
                            ]),
                        ),
                        ("required", Value::arr(vec![Value::from("message")])),
                        ("additionalProperties", Value::from(false)),
                    ]),
                ),
            ]),
            Value::obj(vec![
                ("name", Value::from("wtf_is_going_on")),
                (
                    "description",
                    Value::from(
                        "Answer the eternal question: a snapshot of every agent's status and recent events across machines.",
                    ),
                ),
                (
                    "inputSchema",
                    Value::obj(vec![
                        ("type", Value::from("object")),
                        (
                            "properties",
                            Value::obj(vec![(
                                "agent",
                                Self::prop("optional: filter to one agent name"),
                            )]),
                        ),
                        ("additionalProperties", Value::from(false)),
                    ]),
                ),
            ]),
            Value::obj(vec![
                ("name", Value::from("read_bin")),
                (
                    "description",
                    Value::from(
                        "Read a shared paste-bin (BIN 1-3): prompts, notes, or knowledge placed there by the user or other agents/machines. When the user or a peer says 'work from bin N', fetch it with this tool before starting.",
                    ),
                ),
                (
                    "inputSchema",
                    Value::obj(vec![
                        ("type", Value::from("object")),
                        (
                            "properties",
                            Value::obj(vec![(
                                "bin",
                                int_prop("bin number: 1, 2, or 3"),
                            )]),
                        ),
                        ("required", Value::arr(vec![Value::from("bin")])),
                        ("additionalProperties", Value::from(false)),
                    ]),
                ),
            ]),
            Value::obj(vec![
                ("name", Value::from("write_bin")),
                (
                    "description",
                    Value::from(
                        "Write your content to a shared paste-bin (BIN 1-3) so other agents on other machines/harnesses can read it with read_bin — cross-agent handoff of prompts, findings, and knowledge. Replaces the whole bin (last writer wins); read it first, keep writes purposeful, never put secrets in a bin.",
                    ),
                ),
                (
                    "inputSchema",
                    Value::obj(vec![
                        ("type", Value::from("object")),
                        (
                            "properties",
                            Value::obj(vec![
                                ("bin", int_prop("bin number: 1, 2, or 3")),
                                (
                                    "content",
                                    Self::prop("full text to place in the bin (replaces existing content; max 65,536 chars)"),
                                ),
                            ]),
                        ),
                        (
                            "required",
                            Value::arr(vec![Value::from("bin"), Value::from("content")]),
                        ),
                        ("additionalProperties", Value::from(false)),
                    ]),
                ),
            ]),
            Value::obj(vec![
                ("name", Value::from("list_bins")),
                (
                    "description",
                    Value::from(
                        "List the shared paste-bins (sizes, last writer, age) without full content; fetch a specific bin with read_bin.",
                    ),
                ),
                (
                    "inputSchema",
                    Value::obj(vec![
                        ("type", Value::from("object")),
                        ("properties", Value::obj(vec![])),
                        ("additionalProperties", Value::from(false)),
                    ]),
                ),
            ]),
            Value::obj(vec![
                ("name", Value::from("ping")),
                (
                    "description",
                    Value::from("Check hub connectivity (unsigned /healthz probe)."),
                ),
                (
                    "inputSchema",
                    Value::obj(vec![
                        ("type", Value::from("object")),
                        ("properties", Value::obj(vec![])),
                        ("additionalProperties", Value::from(false)),
                    ]),
                ),
            ]),
            Value::obj(vec![
                ("name", Value::from("hub_info")),
                (
                    "description",
                    Value::from(
                        "Query which hub this bridge is connected to: the hub URL (localhost/LAN address), this device's identity, hub version and uptime. Call this when the operator asks where the hub is or wants the dashboard link — the dashboard key itself is never exposed over MCP; the operator runs `wtf dashboard-url` on the hub machine for the clickable URL.",
                    ),
                ),
                (
                    "inputSchema",
                    Value::obj(vec![
                        ("type", Value::from("object")),
                        ("properties", Value::obj(vec![])),
                        ("additionalProperties", Value::from(false)),
                    ]),
                ),
            ]),
            Value::obj(vec![
                ("name", Value::from("session_create")),
                (
                    "description",
                    Value::from(
                        "Create an encrypted agent-to-agent session channel (dedicated chat). Generates a fresh 256-bit session key, seals it to your identity with ML-KEM-768 (FIPS 203), and registers the channel on the hub. Other agents join with session_join; the hub stores only ciphertext — it cannot read messages.",
                    ),
                ),
                (
                    "inputSchema",
                    Value::obj(vec![
                        ("type", Value::from("object")),
                        (
                            "properties",
                            Value::obj(vec![(
                                "name",
                                Self::prop("channel name, e.g. 'auth-refactor chat'"),
                            )]),
                        ),
                        ("required", Value::arr(vec![Value::from("name")])),
                        ("additionalProperties", Value::from(false)),
                    ]),
                ),
            ]),
            Value::obj(vec![
                ("name", Value::from("session_list")),
                (
                    "description",
                    Value::from(
                        "List the encrypted session channels on the hub with member counts and message counts.",
                    ),
                ),
                (
                    "inputSchema",
                    Value::obj(vec![
                        ("type", Value::from("object")),
                        ("properties", Value::obj(vec![])),
                        ("additionalProperties", Value::from(false)),
                    ]),
                ),
            ]),
            Value::obj(vec![
                ("name", Value::from("session_join")),
                (
                    "description",
                    Value::from(
                        "Join an encrypted session channel with your ML-KEM-768 identity and decapsulate the shared session key from any sealed package addressed to you. Run this after the creator seals the key for you (they do that with session_seal).",
                    ),
                ),
                (
                    "inputSchema",
                    Value::obj(vec![
                        ("type", Value::from("object")),
                        (
                            "properties",
                            Value::obj(vec![(
                                "session",
                                Self::prop("session id from session_create or session_list"),
                            )]),
                        ),
                        ("required", Value::arr(vec![Value::from("session")])),
                        ("additionalProperties", Value::from(false)),
                    ]),
                ),
            ]),
            Value::obj(vec![
                ("name", Value::from("session_seal")),
                (
                    "description",
                    Value::from(
                        "Creator only: seal the session key to a member's ML-KEM-768 identity so they can decapsulate it. Run after the member joins with session_join.",
                    ),
                ),
                (
                    "inputSchema",
                    Value::obj(vec![
                        ("type", Value::from("object")),
                        (
                            "properties",
                            Value::obj(vec![
                                ("session", Self::prop("session id")),
                                ("member", Self::prop("device name of the member to seal for")),
                            ]),
                        ),
                        (
                            "required",
                            Value::arr(vec![Value::from("session"), Value::from("member")]),
                        ),
                        ("additionalProperties", Value::from(false)),
                    ]),
                ),
            ]),
            Value::obj(vec![
                ("name", Value::from("session_send")),
                (
                    "description",
                    Value::from(
                        "Send an encrypted message to a session channel. AES-256-GCM with a per-(session, sender) subkey; the AAD binds the hub-assigned sequence number, so replay across sessions/senders/positions fails closed.",
                    ),
                ),
                (
                    "inputSchema",
                    Value::obj(vec![
                        ("type", Value::from("object")),
                        (
                            "properties",
                            Value::obj(vec![
                                ("session", Self::prop("session id")),
                                ("message", Self::prop("plaintext message to encrypt and send")),
                            ]),
                        ),
                        (
                            "required",
                            Value::arr(vec![Value::from("session"), Value::from("message")]),
                        ),
                        ("additionalProperties", Value::from(false)),
                    ]),
                ),
            ]),
            Value::obj(vec![
                ("name", Value::from("session_read")),
                (
                    "description",
                    Value::from(
                        "Read and decrypt new messages in a session channel (optionally after a sequence number). Messages from other members are verified against their sender binding before display.",
                    ),
                ),
                (
                    "inputSchema",
                    Value::obj(vec![
                        ("type", Value::from("object")),
                        (
                            "properties",
                            Value::obj(vec![
                                ("session", Self::prop("session id")),
                                ("after", int_prop("only messages with seq greater than this (default 0 = all stored)")),
                            ]),
                        ),
                        ("required", Value::arr(vec![Value::from("session")])),
                        ("additionalProperties", Value::from(false)),
                    ]),
                ),
            ]),
        ];
        Value::obj(vec![("tools", Value::Arr(tools))])
    }

    fn tools_call(&self, params: Option<&Value>) -> Result<Value, (i64, String)> {
        let params = params.ok_or((ERR_PARAMS, "missing params".to_string()))?;
        let name = match params.get("name").and_then(|v| v.as_str()) {
            Some(n) => n,
            None => return Err((ERR_PARAMS, "missing tool name".to_string())),
        };
        let empty = Value::Obj(vec![]);
        let args = params.get("arguments").unwrap_or(&empty);
        let (text, is_error) = self.call_tool(name, args);
        Ok(text_result(text, is_error))
    }

    fn call_tool(&self, name: &str, args: &Value) -> (String, bool) {
        match name {
            "check_in" => self.tool_check_in(args),
            "log_event" => self.tool_log_event(args),
            "wtf_is_going_on" => self.tool_state(args),
            "read_bin" => self.tool_read_bin(args),
            "write_bin" => self.tool_write_bin(args),
            "list_bins" => self.tool_list_bins(),
            "ping" => self.tool_ping(),
            "hub_info" => self.tool_hub_info(),
            "session_create" => self.tool_session_create(args),
            "session_list" => self.tool_session_list(),
            "session_join" => self.tool_session_join(args),
            "session_seal" => self.tool_session_seal(args),
            "session_send" => self.tool_session_send(args),
            "session_read" => self.tool_session_read(args),
            other => (
                format!("unknown tool: {other}"),
                true,
            ),
        }
    }

    fn tool_check_in(&self, args: &Value) -> (String, bool) {
        let status = match arg_str(args, "status") {
            Some(s) if STATUSES.contains(&s) => s.to_string(),
            Some(other) => {
                return (
                    format!(
                        "invalid status '{other}'; must be one of: {}",
                        STATUSES.join(", ")
                    ),
                    true,
                )
            }
            None => return ("missing required argument: status".into(), true),
        };
        let task = match arg_str(args, "task") {
            Some(t) if !t.trim().is_empty() => t.to_string(),
            _ => return ("missing required argument: task".into(), true),
        };
        let details = arg_str(args, "details").unwrap_or("");
        let agent = arg_str(args, "agent").unwrap_or(&self.cfg.device_name);
        let body = Value::obj(vec![
            ("status", Value::from(status.as_str())),
            ("task", Value::from(task.as_str())),
            ("details", Value::from(details)),
            ("agent", Value::from(agent)),
        ]);
        match self.api_post("/api/v1/checkin", &body) {
            Ok(_) => {
                let suffix = if details.is_empty() {
                    String::new()
                } else {
                    format!(" — {details}")
                };
                (format!("checked in: [{status}] {task}{suffix}"), false)
            }
            Err(e) => (e, true),
        }
    }

    fn tool_log_event(&self, args: &Value) -> (String, bool) {
        let message = match arg_str(args, "message") {
            Some(m) if !m.trim().is_empty() => m.to_string(),
            _ => return ("missing required argument: message".into(), true),
        };
        let level = match arg_str(args, "level") {
            Some(l) if LEVELS.contains(&l) => l.to_string(),
            Some(other) => {
                return (
                    format!("invalid level '{other}'; must be one of: {}", LEVELS.join(", ")),
                    true,
                )
            }
            None => "info".to_string(),
        };
        let agent = arg_str(args, "agent").unwrap_or(&self.cfg.device_name);
        let body = Value::obj(vec![
            ("message", Value::from(message.as_str())),
            ("level", Value::from(level.as_str())),
            ("agent", Value::from(agent)),
        ]);
        match self.api_post("/api/v1/event", &body) {
            Ok(v) => {
                let id = v.get("id").and_then(|x| x.as_i64()).unwrap_or(0);
                (format!("logged event #{id}"), false)
            }
            Err(e) => (e, true),
        }
    }

    fn tool_state(&self, args: &Value) -> (String, bool) {
        match fetch_state(&self.cfg) {
            Ok(state) => {
                let mut state = state;
                if let Some(filter) = arg_str(args, "agent") {
                    state = filter_state(&state, filter);
                }
                (format_state(&state, &self.cfg.hub_url), false)
            }
            Err(e) => (e, true),
        }
    }

    fn tool_read_bin(&self, args: &Value) -> (String, bool) {
        let id = match args.get("bin").and_then(|v| v.as_i64()) {
            Some(n) if crate::bins::Bins::valid_id(n) => n as u8,
            Some(other) => {
                return (format!("invalid bin {other}; must be 1, 2, or 3"), true)
            }
            None => return ("missing required argument: bin".into(), true),
        };
        match self.api_get(&format!("/api/v1/bins/{id}")) {
            Ok(v) => {
                let size = v.get("size").and_then(|x| x.as_i64()).unwrap_or(0);
                if size == 0 {
                    return (format!("BIN {id} is empty"), false);
                }
                let content = v.get("content").and_then(|x| x.as_str()).unwrap_or("");
                let by = v.get("updated_by").and_then(|x| x.as_str()).unwrap_or("?");
                let at = v.get("updated_at").and_then(|x| x.as_i64()).unwrap_or(0) as u64;
                (
                    format!(
                        "BIN {id} — {size} chars — updated {} ago by {by}\n\n{content}",
                        rel_age(now_secs(), at)
                    ),
                    false,
                )
            }
            Err(e) => (e, true),
        }
    }

    /// Agents publish to shared bins with a device-signed PUT; the hub
    /// records the device as `updated_by`, so cross-machine writes are
    /// attributable in the dashboard.
    fn tool_write_bin(&self, args: &Value) -> (String, bool) {
        let id = match args.get("bin").and_then(|v| v.as_i64()) {
            Some(n) if crate::bins::Bins::valid_id(n) => n as u8,
            Some(other) => {
                return (format!("invalid bin {other}; must be 1, 2, or 3"), true)
            }
            None => return ("missing required argument: bin".into(), true),
        };
        let content = match arg_str(args, "content") {
            Some(c) if !c.is_empty() => c.to_string(),
            Some(_) => return ("content must not be empty (bins have no delete; leave that to the operator)".into(), true),
            None => return ("missing required argument: content".into(), true),
        };
        if content.chars().count() > crate::bins::MAX_BIN_CHARS {
            return (
                format!(
                    "bin content too large (max {} chars)",
                    crate::bins::MAX_BIN_CHARS
                ),
                true,
            );
        }
        let body = Value::obj(vec![("content", Value::from(content.as_str()))]);
        match self.api_put(&format!("/api/v1/bins/{id}"), &body) {
            Ok(v) => {
                let bid = v.get("id").and_then(|x| x.as_i64()).unwrap_or(id as i64);
                let ev = v.get("event").and_then(|x| x.as_i64()).unwrap_or(0);
                (
                    format!(
                        "BIN {bid} updated ({} chars, by {}) — event #{ev}; peers can fetch it with read_bin",
                        content.chars().count(),
                        self.cfg.device_name
                    ),
                    false,
                )
            }
            Err(e) => (e, true),
        }
    }

    fn tool_list_bins(&self) -> (String, bool) {
        match self.api_get("/api/v1/bins") {
            Ok(v) => {
                let bins = v.get("bins").and_then(|x| x.as_arr()).unwrap_or(&[]).to_vec();
                let mut out = String::from("shared paste-bins (read_bin to fetch, write_bin to publish):\n");
                for b in &bins {
                    let id = b.get("id").and_then(|x| x.as_i64()).unwrap_or(0);
                    let size = b.get("size").and_then(|x| x.as_i64()).unwrap_or(0);
                    let by = b.get("updated_by").and_then(|x| x.as_str()).unwrap_or("?");
                    let at = b.get("updated_at").and_then(|x| x.as_i64()).unwrap_or(0) as u64;
                    if size == 0 {
                        out.push_str(&format!("  BIN {id}: (empty)\n"));
                    } else {
                        let content = b.get("content").and_then(|x| x.as_str()).unwrap_or("");
                        let mut preview: String = content.chars().take(60).collect();
                        if content.chars().count() > 60 {
                            preview.push('…');
                        }
                        out.push_str(&format!(
                            "  BIN {id}: {size} chars, updated {} ago by {by} — {preview}\n",
                            rel_age(now_secs(), at)
                        ));
                    }
                }
                (out, false)
            }
            Err(e) => (e, true),
        }
    }

    /// Where is the hub? Answers the operator's "what's the address"
    /// question without ever exposing the dashboard key: agents get the
    /// hub URL and version; the clickable `?k=` link is printed only by
    /// `wtf dashboard-url` on the hub machine itself.
    fn tool_hub_info(&self) -> (String, bool) {
        let mut out = format!(
            "hub address: {}\nthis device: {} (bridge {})\n",
            self.cfg.hub_url, self.cfg.device_name, crate::VERSION
        );
        match client::get_text(&format!("{}/healthz", self.cfg.hub_url)) {
            Ok((200, body)) => {
                if let Ok(v) = json::parse(&body) {
                    let hv = v.get("version").and_then(|x| x.as_str()).unwrap_or("?");
                    let started = v.get("started_at").and_then(|x| x.as_i64()).unwrap_or(0) as u64;
                    out.push_str(&format!(
                        "hub version: {hv} · uptime {}s\n",
                        now_secs().saturating_sub(started)
                    ));
                }
                out.push_str(
                    "dashboard link: operator runs `wtf dashboard-url` on the hub machine (the dashboard key never travels over MCP)\n",
                );
                (out, false)
            }
            Ok((status, _)) => (format!("hub responded HTTP {status}"), true),
            Err(e) => (format!("hub unreachable: {e}"), true),
        }
    }

    fn tool_ping(&self) -> (String, bool) {
        match client::get_text(&format!("{}/healthz", self.cfg.hub_url)) {
            Ok((200, body)) => {
                let uptime = json::parse(&body)
                    .ok()
                    .and_then(|v| {
                        let started = v.get("started_at")?.as_i64()? as u64;
                        Some(now_secs().saturating_sub(started))
                    })
                    .unwrap_or(0);
                (format!("pong: hub is up (uptime {uptime}s)"), false)
            }
            Ok((status, _)) => (format!("hub responded HTTP {status}"), true),
            Err(e) => (format!("hub unreachable: {e}"), true),
        }
    }
}

fn filter_state(state: &Value, agent: &str) -> Value {
    let keep = |v: &Value| v.get("agent").and_then(|a| a.as_str()) == Some(agent);
    let mut out = state.clone();
    if let Some(arr) = state.get("agents").and_then(|v| v.as_arr()) {
        let pairs: Vec<(String, Value)> = state
            .as_obj()
            .unwrap()
            .iter()
            .map(|(k, v)| {
                if k == "agents" {
                    (
                        k.clone(),
                        Value::arr(arr.iter().filter(|a| keep(a)).cloned().collect()),
                    )
                } else if k == "events" {
                    let ev = v.as_arr().unwrap_or(&[]);
                    (
                        k.clone(),
                        Value::arr(ev.iter().filter(|e| keep(e)).cloned().collect()),
                    )
                } else {
                    (k.clone(), v.clone())
                }
            })
            .collect();
        out = Value::Obj(pairs);
    }
    out
}

// ---------- entry point ----------

/// Run the stdio MCP server until stdin closes.
pub fn run(cfg: BridgeConfig) {
    // Heartbeat thread: keeps device liveness visible between tool calls.
    {
        let hb_cfg = cfg.clone();
        std::thread::spawn(move || {
            let hb = Bridge { cfg: hb_cfg };
            loop {
                std::thread::sleep(Duration::from_secs(HEARTBEAT_SECS));
                let body = Value::obj(vec![("agent", Value::from(hb.cfg.device_name.as_str()))]);
                let _ = hb.api_post("/api/v1/heartbeat", &body);
            }
        });
    }

    let bridge = Bridge { cfg };
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }
        if let Some(resp) = bridge.handle_line(&line) {
            let mut out = stdout.lock();
            if writeln!(out, "{resp}").and_then(|_| out.flush()).is_err() {
                break; // client closed stdout; nothing left to do
            }
        }
    }
}

// ---------- encrypted agent-to-agent session tools ----------

impl Bridge {
    fn identity(&self) -> Result<crate::identity::Identity, String> {
        crate::identity::load_or_create()
    }

    fn ek_hex(&self) -> Result<String, String> {
        Ok(crate::util::hex_encode(&self.identity()?.ek))
    }

    /// POST helper for session endpoints (device-signed).
    fn api_post_session(&self, path: &str, body: &Value) -> Result<Value, String> {
        self.api_post(path, body)
    }

    fn api_get_session(&self, path: &str) -> Result<Value, String> {
        self.api_get(path)
    }

    /// session_create { name }: create a channel, generate the session
    /// key, seal it to our own identity, register as creator-member.
    fn tool_session_create(&self, args: &Value) -> (String, bool) {
        let Some(name) = arg_str(args, "name") else {
            return ("missing required argument: name".into(), true);
        };
        let id = self.identity();
        let Ok(identity) = id else {
            return (format!("identity error: {}", id.unwrap_err()), true);
        };
        let ek_hex = crate::util::hex_encode(&identity.ek);
        // Register identity first (session create requires it).
        if let Err(e) = self.api_post_session(
            "/api/v1/identity",
            &Value::obj(vec![("ek", Value::from(ek_hex.as_str()))]),
        ) {
            return (format!("identity registration failed: {e}"), true);
        }
        let created = self.api_post_session(
            "/api/v1/sessions",
            &Value::obj(vec![("name", Value::from(name))]),
        );
        let Ok(session) = created else {
            return (format!("session create failed: {}", created.unwrap_err()), true);
        };
        let Some(sid) = session.get("id").and_then(|v| v.as_str()) else {
            return ("hub returned no session id".into(), true);
        };
        // Generate + seal the session key to ourselves (creator-member).
        let session_key: [u8; 32] = crate::rand::bytes(32).try_into().unwrap();
        let pkg = match crate::session_crypto::seal_session_key(&ek_hex, &session_key, sid) {
            Ok(p) => p,
            Err(e) => return (format!("seal failed: {e}"), true),
        };
        let fp = crate::session_crypto::ek_fp(&ek_hex);
        if let Err(e) = self.api_post_session(
            &format!("/api/v1/sessions/{sid}/seal"),
            &Value::obj(vec![(
                "pkgs",
                Value::arr(vec![Value::obj(vec![
                    ("ct", Value::from(pkg.as_str())),
                    ("ek_fp", Value::from(fp.as_str())),
                ])]),
            )]),
        ) {
            return (format!("seal post failed: {e}"), true);
        }
        // Persist the session key locally, bound to the session id.
        if let Err(e) = store_session_key(sid, &session_key) {
            return (format!("session key persist failed: {e}"), true);
        }
        (
            format!(
                "session created: {sid} '{name}' — you are the creator; peers join with session_join {sid}"
            ),
            false,
        )
    }

    fn tool_session_list(&self) -> (String, bool) {
        match self.api_get_session("/api/v1/sessions") {
            Ok(v) => {
                let sessions = v.get("sessions").and_then(|x| x.as_arr()).unwrap_or(&[]);
                let mut out = String::from("sessions:\n");
                for s in sessions {
                    let id = s.get("id").and_then(|x| x.as_str()).unwrap_or("?");
                    let name = s.get("name").and_then(|x| x.as_str()).unwrap_or("?");
                    let members = s.get("members").and_then(|x| x.as_arr()).map(|a| a.len()).unwrap_or(0);
                    let msgs = s.get("msg_count").and_then(|x| x.as_i64()).unwrap_or(0);
                    out.push_str(&format!("  {id} '{name}' — {members} member(s), {msgs} message(s)\n"));
                }
                (out, false)
            }
            Err(e) => (format!("session list failed: {e}"), true),
        }
    }

    /// session_join { session }: join, fetch sealed packages, decapsulate
    /// the session key to local storage.
    fn tool_session_join(&self, args: &Value) -> (String, bool) {
        let Some(sid) = arg_str(args, "session") else {
            return ("missing required argument: session".into(), true);
        };
        let Ok(identity) = self.identity() else {
            return ("identity load failed".into(), true);
        };
        let ek_hex = crate::util::hex_encode(&identity.ek);
        let joined = self.api_post_session(
            &format!("/api/v1/sessions/{sid}/join"),
            &Value::obj(vec![("ek", Value::from(ek_hex.as_str()))]),
        );
        // Re-join to pick up a sealed package is fine: the hub rejects
        // duplicate membership, but the seal fetch below still runs.
        if let Err(e) = &joined {
            if !e.contains("HTTP 400") {
                return (format!("join failed: {e}"), true);
            }
        }
        // Fetch sealed packages addressed to our ek fingerprint.
        let fp = crate::session_crypto::ek_fp(&ek_hex);
        let seals = self.api_get_session(&format!("/api/v1/sessions/{sid}/seals"));
        let Ok(sv) = seals else {
            return (format!("seal fetch failed: {}", seals.unwrap_err()), true);
        };
        let mut recovered = 0;
        if let Some(pkgs) = sv.get("sealed").and_then(|x| x.as_arr()) {
            for p in pkgs {
                let Some(pkg_hex) = p.get("ct").and_then(|v| v.as_str()) else {
                    continue;
                };
                match crate::session_crypto::open_sealed_package(pkg_hex, &identity.dk, sid) {
                    Ok(key) => {
                        if let Err(e) = store_session_key(sid, &key) {
                            return (format!("session key persist failed: {e}"), true);
                        }
                        recovered += 1;
                    }
                    Err(e) => {
                        // Packages sealed to other members fail here; skip.
                        let _ = e;
                    }
                }
            }
        }
        if recovered == 0 {
            return (
                format!("joined {sid}; no sealed package for us yet — ask the creator to run session_seal (they need to seal the key to our new key)"),
                false,
            );
        }
        (format!("joined {sid}; session key recovered from {recovered} sealed package(s)"), false)
    }

    /// session_seal { session, member }: creator seals the session key to
    /// the member's registered ek and posts the package.
    fn tool_session_seal(&self, args: &Value) -> (String, bool) {
        let Some(sid) = arg_str(args, "session") else {
            return ("missing required argument: session".into(), true);
        };
        let Some(member) = arg_str(args, "member") else {
            return ("missing required argument: member (device name of the member to seal for)".into(), true);
        };
        // Session key must be local (creator holds it).
        let Some(key) = load_session_key(sid) else {
            return (format!("no local session key for {sid} — only the creator can seal"), true);
        };
        // Fetch the member's registered ek from the identity registry.
        let devices = self.api_get_session("/api/v1/devices");
        let Ok(dv) = devices else {
            return (format!("device list failed: {}", devices.unwrap_err()), true);
        };
        let member_ek_fp = dv
            .get("devices")
            .and_then(|x| x.as_arr())
            .and_then(|arr| {
                arr.iter()
                    .find(|d| d.get("device").and_then(|v| v.as_str()) == Some(member))
                    .and_then(|d| d.get("ek_fp").and_then(|v| v.as_str()))
                    .map(|s| s.to_string())
            });
        let Some(fp) = member_ek_fp else {
            return (format!("member '{member}' has no registered identity — they must run session_join first"), true);
        };
        // Fetch the member's ek: the registry only carries fingerprints;
        // the sealed package must target the member's REAL ek. The hub
        // returns full eks to members via the seals endpoint trick — no:
        // simplest correct flow: ask the hub for the member's ek via the
        // member-list of the session (join stored it).
        let sess = self.api_get_session(&format!("/api/v1/sessions/{sid}"));
        let Ok(sv) = sess else {
            return (format!("session fetch failed: {}", sess.unwrap_err()), true);
        };
        let member_ek = sv
            .get("members")
            .and_then(|x| x.as_arr())
            .and_then(|arr| {
                arr.iter()
                    .find(|mm| mm.get("device").and_then(|v| v.as_str()) == Some(member))
                    .and_then(|mm| mm.get("ek").and_then(|v| v.as_str()))
                    .map(|s| s.to_string())
            });
        let Some(member_ek) = member_ek else {
            return (format!("member '{member}' not in session {sid}"), true);
        };
        let pkg = match crate::session_crypto::seal_session_key(&member_ek, &key, sid) {
            Ok(p) => p,
            Err(e) => return (format!("seal failed: {e}"), true),
        };
        if let Err(e) = self.api_post_session(
            &format!("/api/v1/sessions/{sid}/seal"),
            &Value::obj(vec![(
                "pkgs",
                Value::arr(vec![Value::obj(vec![
                    ("ct", Value::from(pkg.as_str())),
                    ("ek_fp", Value::from(fp.as_str())),
                ])]),
            )]),
        ) {
            return (format!("seal post failed: {e}"), true);
        }
        (format!("session key sealed for '{member}'; they run session_join again or session_read to pick it up"), false)
    }

    /// session_send { session, message }: encrypt + post. The AAD binds
    /// the seq returned by the hub (hub-assigned monotonic).
    fn tool_session_send(&self, args: &Value) -> (String, bool) {
        let Some(sid) = arg_str(args, "session") else {
            return ("missing required argument: session".into(), true);
        };
        let Some(message) = arg_str(args, "message") else {
            return ("missing required argument: message".into(), true);
        };
        let Some(key) = load_session_key(sid) else {
            return (format!("no local session key for {sid} — join the session first"), true);
        };
        let sender = &self.cfg.device_name;
        // seq: ask the hub for the next seq by listing the session.
        let sess = self.api_get_session(&format!("/api/v1/sessions/{sid}"));
        let Ok(sv) = sess else {
            return (format!("session fetch failed: {}", sess.unwrap_err()), true);
        };
        let next_seq = sv
            .get("next_seq")
            .and_then(|v| v.as_i64())
            .unwrap_or(1) as u64;
        let (nonce, ct) =
            match crate::session_crypto::seal_message(&key, sid, sender, next_seq, message) {
                Ok(v) => v,
                Err(e) => return (format!("encrypt failed: {e}"), true),
            };
        let sent = self.api_post_session(
            &format!("/api/v1/sessions/{sid}/send"),
            &Value::obj(vec![
                ("nonce", Value::from(nonce.as_str())),
                ("ct", Value::from(ct.as_str())),
            ]),
        );
        let Ok(sv) = sent else {
            return (format!("send failed: {}", sent.unwrap_err()), true);
        };
        let seq = sv.get("seq").and_then(|v| v.as_i64()).unwrap_or(0);
        (
            format!("sent to {sid} as seq {seq} (encrypted, sender-bound)"),
            false,
        )
    }

    /// session_read { session, after? }: poll, decrypt all new messages.
    fn tool_session_read(&self, args: &Value) -> (String, bool) {
        let Some(sid) = arg_str(args, "session") else {
            return ("missing required argument: session".into(), true);
        };
        let after = args
            .get("after")
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            .max(0) as u64;
        let Some(key) = load_session_key(sid) else {
            return (format!("no local session key for {sid} — join the session first"), true);
        };
        let msgs = self.api_get_session(&format!("/api/v1/sessions/{sid}/recv?after={after}"));
        let Ok(mv) = msgs else {
            return (format!("read failed: {}", msgs.unwrap_err()), true);
        };
        let arr = mv.get("msgs").and_then(|x| x.as_arr()).unwrap_or(&[]);
        if arr.is_empty() {
            return (format!("no new messages in {sid} after seq {after}"), false);
        }
        let sender = &self.cfg.device_name;
        let mut out = String::new();
        for msg in arr {
            let seq = msg.get("seq").and_then(|v| v.as_i64()).unwrap_or(0) as u64;
            let from = msg.get("sender").and_then(|v| v.as_str()).unwrap_or("?");
            let nonce = msg.get("nonce").and_then(|v| v.as_str()).unwrap_or("");
            let ct = msg.get("ct").and_then(|v| v.as_str()).unwrap_or("");
            match crate::session_crypto::open_message(&key, sid, from, seq, nonce, ct) {
                Ok(pt) => out.push_str(&format!("#{seq} {from}: {pt}\n")),
                Err(e) => out.push_str(&format!("#{seq} {from}: <decrypted failed: {e}>\n")),
            }
        }
        let _ = sender;
        (out, false)
    }
}

// ---------- local session key store ----------

/// Per-device session key cache: `$WTF_HOME/session_keys.json` (0600).
/// Keys are session-scoped secrets — same protection class as bridge.json.
fn session_keys_path() -> std::path::PathBuf {
    crate::config::home().join("session_keys.json")
}

fn store_session_key(session_id: &str, key: &[u8; 32]) -> Result<(), String> {
    let path = session_keys_path();
    let mut map = load_session_keys(&path);
    map.insert(
        session_id.to_string(),
        crate::util::hex_encode(key),
    );
    save_session_keys(&path, &map)
}

pub fn load_session_key(session_id: &str) -> Option<[u8; 32]> {
    let map = load_session_keys(&session_keys_path());
    let hex = map.get(session_id)?;
    let bytes = crate::util::hex_decode(hex)?;
    bytes.try_into().ok()
}

fn load_session_keys(path: &std::path::Path) -> std::collections::HashMap<String, String> {
    let parsed = match crate::config::load_json(path) {
        Ok(Some(v)) => v,
        _ => return std::collections::HashMap::new(),
    };
    let Some(pairs) = parsed.get("keys").and_then(|x| x.as_obj()) else {
        return std::collections::HashMap::new();
    };
    pairs
        .iter()
        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
        .collect()
}

fn save_session_keys(
    path: &std::path::Path,
    map: &std::collections::HashMap<String, String>,
) -> Result<(), String> {
    let mut pairs: Vec<(&str, Value)> = Vec::new();
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort();
    for k in keys {
        pairs.push((k.as_str(), Value::from(map[k].as_str())));
    }
    crate::config::save_json(
        path,
        &Value::obj(vec![("keys", Value::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()))]),
        0o600,
    )
}
