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
            "ping" => self.tool_ping(),
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
