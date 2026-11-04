//! Embedded dashboard: single dark-theme HTML page, no external assets.
//! Bootstraps from /api/v1/state?k=… or /w/<capability> then live-updates via SSE (/stream).
//! Integrates User Chat & Agent Orchestration (Hermes / Pi) paired with a Federated Shell
//! and Cross-Machine Federated Pastebins.

pub const PAGE: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>WTF is going on? · Studio &amp; Federated Mesh</title>
<style>
:root{
  --bg:#080c14;--panel:#0f1523;--panel2:#0b101c;--panel-elev:#141c2e;
  --edge:#1e293b;--edge2:#2a3852;--edge-focus:#00f0ff;
  --ink:#e2e8f0;--dim:#7e8b9f;--dim2:#536177;
  --ok:#10b981;--warn:#f59e0b;--err:#ef4444;--info:#38bdf8;
  --accent:#00f0ff;--accent2:#818cf8;--term-bg:#04070d;
}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--ink);font:13px/1.5 ui-monospace,SFMono-Regular,Menlo,Consolas,"Liberation Mono",monospace;overflow-x:hidden}
header{display:flex;align-items:center;gap:12px;padding:10px 18px;border-bottom:1px solid var(--edge);background:var(--panel2);flex-wrap:wrap;position:sticky;top:0;z-index:100}
h1{font-size:14px;margin:0;letter-spacing:2px;font-weight:800;color:var(--ink);display:flex;align-items:center;gap:8px}
h1 span{color:var(--accent)}
#meta{color:var(--dim);font-size:11px}
.dot{width:9px;height:9px;border-radius:50%;background:var(--dim2);margin-left:auto;transition:all .2s}
.dot.on{background:var(--ok);box-shadow:0 0 10px rgba(16,185,129,.7)}
.cap-badge{font-size:11px;background:#132035;color:var(--accent);border:1px solid #1e3352;border-radius:6px;padding:2px 8px;cursor:pointer;transition:all .15s}
.cap-badge:hover{border-color:var(--accent);background:#1a2b47}
main{padding:14px 18px;display:flex;flex-direction:column;gap:16px;max-width:1600px;margin:0 auto}
h2{font-size:11px;color:var(--dim);letter-spacing:1px;margin:0 0 6px;text-transform:uppercase;font-weight:700;display:flex;align-items:center;gap:6px}
.card{background:var(--panel);border:1px solid var(--edge);border-radius:8px;padding:12px;box-shadow:0 4px 12px rgba(0,0,0,.25)}

/* Studio: Chat & Federated Shell Split View */
#studio{display:grid;grid-template-columns:1fr 1fr;gap:16px}
@media(max-width:1024px){#studio{grid-template-columns:1fr}}

.studio-pane{display:flex;flex-direction:column;gap:8px}
.bar{display:flex;gap:8px;align-items:center;flex-wrap:wrap}
select,input[type="text"],input[type="password"],textarea{background:var(--bg);color:var(--ink);border:1px solid var(--edge2);border-radius:6px;padding:6px 9px;font:inherit;font-size:12px}
select:focus,input[type="text"]:focus,input[type="password"]:focus,textarea:focus{outline:none;border-color:var(--edge-focus);box-shadow:0 0 0 1px var(--edge-focus)}
button{background:#162438;color:var(--ink);border:1px solid var(--edge2);border-radius:6px;padding:5px 12px;font:inherit;font-size:12px;cursor:pointer;transition:all .15s;display:inline-flex;align-items:center;gap:5px}
button:hover{border-color:var(--dim);background:#1e3250}
button.primary{background:var(--accent);color:#050811;font-weight:700;border:none}
button.primary:hover{opacity:.9;box-shadow:0 0 10px rgba(0,240,255,.3)}
button.active{background:#233a60;border-color:var(--accent);color:var(--accent)}

.feed-box{background:var(--term-bg);border:1px solid var(--edge);border-radius:6px;padding:10px;height:300px;overflow-y:auto;font-size:12px;display:flex;flex-direction:column;gap:8px}
.m{border-bottom:1px dashed var(--edge);padding-bottom:6px}
.m:last-child{border-bottom:0}
.m .mtop{display:flex;gap:8px;font-size:11px;color:var(--dim)}
.m pre{margin:4px 0 0;white-space:pre-wrap;word-break:break-word;color:var(--ink);font-family:inherit}
.m.agent-msg pre{color:var(--accent)}

/* Team Chat Pastebins bar in Studio */
#team-chat-bins-box{background:var(--panel2);border:1px solid var(--edge2);border-radius:6px;padding:8px;display:flex;flex-direction:column;gap:6px}
.chat-bin-tabs{display:flex;gap:6px;align-items:center}
.chat-bin-tab{padding:2px 8px;font-size:11px;border-radius:4px;cursor:pointer;background:var(--bg);border:1px solid var(--edge)}
.chat-bin-tab.active{background:var(--accent);color:#050811;font-weight:bold;border-color:var(--accent)}
#chat-bin-editor{width:100%;min-height:75px;font-size:11px;resize:vertical}

/* Federated Shell styling */
#shell-out{background:var(--term-bg);border:1px solid var(--edge);border-radius:6px;padding:10px;height:340px;overflow-y:auto;color:#a8d18d;font-size:12px;white-space:pre-wrap;word-break:break-word}
.cmd-prompt-line{color:var(--accent);font-weight:bold}
.cmd-badge-local{color:#10b981}
.cmd-badge-remote{color:#f59e0b}

.pill{font-size:11px;padding:1px 8px;border-radius:10px;border:1px solid var(--dim2);color:var(--dim)}
.pill.ok{color:var(--ok);border-color:var(--ok)}
.pill.warn{color:var(--warn);border-color:var(--warn)}
.pill.info{color:var(--info);border-color:var(--info)}
.pill.accent{color:var(--accent);border-color:var(--accent)}
.pill.stale{opacity:.5}
.ochip{font-size:11px;padding:1px 7px;border-radius:10px;border:1px solid var(--info);color:var(--info);margin-right:4px}
.repo{font-size:11px;padding:1px 7px;border-radius:10px;border:1px solid var(--warn);color:var(--warn);margin:0 6px}
.dim{color:var(--dim)}

/* Overview Grid */
#overview{display:grid;grid-template-columns:1fr 1fr;gap:16px}
@media(max-width:900px){#overview{grid-template-columns:1fr}}
#feed{list-style:none;margin:0;padding:0;max-height:360px;overflow-y:auto}
#feed li{padding:4px 0;border-bottom:1px dashed var(--edge);white-space:pre-wrap;word-break:break-word}
.lv{font-size:11px}
.lv.error{color:var(--err)}
.lv.warn{color:var(--warn)}
.lv.info{color:var(--info)}

/* Cross-Machine Federated Bins */
#bins-sec{display:flex;flex-direction:column;gap:8px}
#bins{display:grid;grid-template-columns:repeat(3,minmax(240px,1fr));gap:16px;padding:0}
@media(max-width:1100px){#bins{grid-template-columns:1fr}}
.bin{display:flex;flex-direction:column}
.bin textarea{width:100%;min-height:140px;margin-top:6px;background:var(--bg);color:var(--ink);border:1px solid var(--edge2);border-radius:6px;padding:8px;font:inherit;resize:vertical}
.bhead{display:flex;gap:8px;align-items:baseline}
.bmeta{margin-left:auto;font-size:11px}
.bmeta .who{color:var(--info)}
.bbtns{display:flex;gap:8px;margin-top:6px;align-items:center}
.bbtns button.dirty{border-color:var(--warn);color:var(--warn)}

/* Sessions card */
.sess{padding:8px 0;border-bottom:1px dashed var(--edge);cursor:pointer}
.sess:hover{background:rgba(74,163,255,.04)}
.sess:last-child{border-bottom:0}
.scope-tab{background:var(--bg);border:1px solid var(--edge2);color:var(--ink);border-radius:4px;padding:3px 10px;font-size:11px;cursor:pointer;transition:all 0.15s ease}
.scope-tab:hover{border-color:var(--accent)}
.scope-tab.active{background:var(--accent);color:#050811;font-weight:bold;border-color:var(--accent)}
.sess-bin-btn{padding:1px 7px;font-size:10px;margin-left:6px;border-radius:3px;cursor:pointer}

/* Completions Config Modal */
#config-modal{display:none;position:fixed;top:0;left:0;right:0;bottom:0;background:rgba(0,0,0,.75);backdrop-filter:blur(3px);z-index:200;align-items:center;justify-content:center}
#config-modal.open{display:flex}
.modal-box{background:var(--panel);border:1px solid var(--edge2);border-radius:10px;width:90%;max-width:560px;padding:18px;display:flex;flex-direction:column;gap:12px;box-shadow:0 10px 30px rgba(0,0,0,.5)}
.field-row{display:flex;flex-direction:column;gap:4px}
.field-row label{font-size:11px;color:var(--dim);text-transform:uppercase;letter-spacing:1px}
.segmented-opts{display:flex;gap:6px}
.segmented-opts button{flex:1;text-align:center;justify-content:center}
</style>
</head>
<body>
<header>
  <h1>WTF IS GOING ON</h1>
  <span id="port-pill" class="pill accent" style="cursor:pointer" title="Auto-fallback port (click to copy)">● :47820</span>
  <span id="meta" class="dim">connecting…</span>
  <span id="agent-toggle-pill" class="pill ok" style="cursor:pointer" title="Active Tooling Agent: click to switch between Hermes (OpenAI) and Pi (Anthropic)">⚡ Agent: hermes</span>
  <span id="router-status-pill" class="pill ok" style="cursor:pointer" title="Completions Endpoint / Local Router :11434 (click to re-probe)">● :11434</span>
  <button id="btn-open-config" title="Configure completions endpoint, models, and agents" style="font-size:11px;padding:2px 8px">⚙ Config</button>
  <span id="cap-url-chip" class="cap-badge" title="Click to copy singular dashboard URL">📋 copy url</span>
  <span id="conn" class="dot" title="live stream"></span>
</header>

<main>
  <!-- STUDIO: Paired Chat Studio & Federated Multi-Machine Shell -->
  <section id="studio">
    <!-- Chat Studio -->
    <div class="card studio-pane">
      <div class="bar">
        <h2>CHAT &amp; AGENT ORCHESTRATION</h2>
        <span id="chat-stat" class="dim" style="margin-left:auto;font-size:11px">idle</span>
      </div>

      <!-- Quick status indicator bar -->
      <div class="bar" id="router-status-bar" style="align-items:center;font-size:11px;background:rgba(0,240,255,0.04);padding:4px 8px;border-radius:4px;border:1px solid var(--edge)">
        <span id="active-agent-badge" class="pill ok" style="font-size:10px">Hermes</span>
        <span id="active-provider-badge" class="pill info" style="font-size:10px">OpenAI</span>
        <span id="router-model-info" class="dim" style="margin-left:6px;font-size:10px">http://localhost:11434 · auto-model</span>
        <span class="dim" style="margin-left:auto;font-size:10px" title="Operator-run completions proxy">localhost:11434</span>
      </div>

      <!-- Lane selection -->
      <div class="bar">
        <label class="dim" style="font-size:11px">Lane:</label>
        <select id="lane-select" style="flex:1;min-width:140px;font-size:12px"></select>
        <button id="btn-new-lane" style="font-size:11px">+ New Lane</button>
      </div>
      <div class="bar" style="font-size:11px">
        <span class="dim">Scope:</span>
        <span id="lane-scope-chip" class="repo">-</span>
        <span class="dim" style="margin-left:auto">Members: <span id="lane-members-text" style="color:var(--info)">-</span></span>
      </div>

      <!-- Dedicated Team Chat Pastebins (3 bins per team chat) -->
      <div id="team-chat-bins-box">
        <div class="chat-bin-tabs">
          <span style="font-size:11px;font-weight:bold;color:var(--dim);margin-right:4px">💬 Team Chat Bins:</span>
          <button class="chat-bin-tab active" data-bin="1">Bin 1</button>
          <button class="chat-bin-tab" data-bin="2">Bin 2</button>
          <button class="chat-bin-tab" data-bin="3">Bin 3</button>
          <span id="chat-bin-meta" class="dim" style="font-size:10px;margin-left:auto">0 chars</span>
        </div>
        <textarea id="chat-bin-editor" placeholder="Dedicated pastebin for this agent team chat (scope: chat:<id>). Agents read/write via read_bin/write_bin."></textarea>
        <div style="display:flex;gap:6px;align-items:center">
          <button id="btn-save-chat-bin" style="padding:2px 10px;font-size:11px">Save Bin</button>
          <button id="btn-copy-chat-bin" style="padding:2px 10px;font-size:11px">Copy</button>
          <span id="chat-bin-status" class="dim" style="font-size:10px;margin-left:auto"></span>
        </div>
      </div>

      <!-- Chat Feed -->
      <div id="chat-feed" class="feed-box">
        <div class="dim">select or create a chat lane above to begin</div>
      </div>

      <!-- Chat input -->
      <div class="bar">
        <span id="chat-agent-label" class="pill" style="border-color:var(--accent);color:var(--accent);font-size:10px">agent: hermes</span>
        <button id="btn-run-agent" class="primary" style="margin-left:auto;font-size:11px">⚡ Send Chat</button>
      </div>
      <textarea id="chat-input" rows="2" placeholder="Message team or prompt selected agent… (Enter sends; Shift+Enter for newline)"></textarea>
    </div>

    <!-- Federated Shell -->
    <div class="card studio-pane">
      <div class="bar">
        <h2>FEDERATED SHELL</h2>
        <span class="dim" style="font-size:11px">Root: <b style="color:var(--accent)">~/</b> (cluster mesh)</span>
        <span id="shell-cwd-badge" class="pill ok" style="margin-left:auto">~/</span>
      </div>
      <div id="shell-lkgl-info" class="dim" style="font-size:10px;margin:2px 0 4px 0">Cluster Root · Cross-Architecture Multi-Machine Terminal</div>
      <div class="bar" id="machine-chips-bar" style="font-size:11px">
        <span class="dim">Quick Jump:</span>
        <button class="mach-chip" data-dir="~/" style="padding:1px 8px;font-size:11px">~/ (all)</button>
      </div>
      <div id="shell-out">=== WTF FEDERATED SHELL &amp; MULTI-MACHINE MESH ===
Virtual cluster root (~/) anchors to connected machines with persistent architecture LKGL.
Dispatched tasks inherit synchronized local-router proxy (:11434) and fallback cascades.
Supports cross-architecture compound orchestration in one command:
  cd ~/mac &amp;&amp; echo "mac ok" &amp;&amp; cd ~/windows &amp;&amp; echo "win ok"
Type 'ls ~' or 'cd &lt;machine&gt;' to navigate.</div>
      <div class="bar" style="margin-top:2px">
        <span id="shell-prompt-tag" style="color:var(--ok);font-weight:bold;font-size:12px">[~/]$</span>
        <input type="text" id="shell-input" style="flex:1;font-size:12px" placeholder="ls ~  OR  cd mac &amp;&amp; npm test  OR  cd ~/windows &amp;&amp; cargo test"/>
        <button id="btn-shell-exec" class="primary" style="font-size:11px">Exec</button>
        <button id="btn-shell-clear" style="font-size:11px">Clear</button>
      </div>
    </div>
  </section>

  <!-- OVERVIEW: Agents & Event Stream -->
  <section id="overview">
    <div>
      <h2>AGENTS (ACTIVE MESH)</h2>
      <div id="agents" class="card"><span class="dim">no agents have checked in yet</span></div>
    </div>
    <div>
      <h2>EVENT LOG (CROSS-MACHINE)</h2>
      <div class="card"><ul id="feed"><li class="dim">no events yet</li></ul></div>
    </div>
  </section>

  <!-- SESSIONS LIST -->
  <section>
    <h2>FEDERATED AGENT CHAT LANES</h2>
    <div id="sessions" class="card"><span class="dim">no chats yet</span></div>
  </section>

  <!-- CROSS-MACHINE FEDERATED PASTEBINS (GLOBAL) -->
  <section id="bins-sec">
    <div class="bar" style="align-items:center;flex-wrap:wrap;gap:8px">
      <h2>🌐 CROSS-MACHINE FEDERATED PASTEBIN (GLOBAL · BIN 1-3)</h2>
      <span id="bins-scope-pill" class="pill ok" style="margin-left:auto">scope: user (global mesh)</span>
    </div>
    <div class="dim" style="font-size:11px;margin-top:-4px">
      Pastebins replicated across all connected machines in the federated cluster. Agents across all harnesses and terminals access via <code>read_bin</code> and <code>write_bin</code>.
    </div>
    <div class="bar" id="bin-scope-bar" style="margin:4px 0 6px 0;flex-wrap:wrap;gap:6px">
      <span class="dim" style="font-size:11px;align-self:center">Scope:</span>
      <button class="scope-tab active" id="btn-scope-user" data-scope="user">👤 Global Mesh Bins</button>
      <button class="scope-tab" id="btn-scope-active-chat" style="display:none" data-scope="">💬 Current Chat Bins</button>
      <button class="scope-tab" id="btn-scope-active-mach" style="display:none" data-scope="">💻 Machine Bins</button>
      <select id="scope-dropdown" style="font-size:11px;padding:2px 6px;margin-left:auto;max-width:240px">
        <option value="user">user (global mesh)</option>
      </select>
    </div>
    <div id="bins"></div>
  </section>
</main>

<!-- Completions & Agent Config Modal -->
<div id="config-modal">
  <div class="modal-box">
    <div style="display:flex;align-items:center;border-bottom:1px solid var(--edge);padding-bottom:8px">
      <h2 style="margin:0;font-size:13px;color:var(--ink)">⚙ COMPLETIONS &amp; AGENT CONFIG</h2>
      <button id="btn-close-config" style="margin-left:auto;padding:2px 8px;font-size:11px">✕</button>
    </div>

    <div class="field-row">
      <label>Active Tooling Agent</label>
      <div class="segmented-opts">
        <button id="opt-agent-hermes" type="button" class="active">Hermes (OpenAI protocol)</button>
        <button id="opt-agent-pi" type="button">Pi (Anthropic protocol)</button>
      </div>
    </div>

    <div class="field-row">
      <label>Completions Provider</label>
      <div class="segmented-opts">
        <button id="opt-prov-openai" type="button" class="active">OpenAI Compatible</button>
        <button id="opt-prov-anthropic" type="button">Anthropic Compatible</button>
      </div>
    </div>

    <div class="field-row">
      <label>Endpoint URL (Default: local-router :11434)</label>
      <input type="text" id="cfg-endpoint" placeholder="http://localhost:11434" value="http://localhost:11434"/>
      <div style="display:flex;gap:6px;margin-top:2px">
        <button type="button" onclick="setEndpointPreset('http://localhost:11434')" style="font-size:10px;padding:1px 6px">local-router :11434</button>
        <button type="button" onclick="setEndpointPreset('https://api.openai.com/v1')" style="font-size:10px;padding:1px 6px">OpenAI Official</button>
        <button type="button" onclick="setEndpointPreset('https://api.anthropic.com')" style="font-size:10px;padding:1px 6px">Anthropic Official</button>
      </div>
    </div>

    <div class="field-row">
      <label>Model</label>
      <input type="text" id="cfg-model" placeholder="qwen3:abliterated (leave blank for auto-discovered preferred model)"/>
      <select id="cfg-model-select" style="margin-top:2px;font-size:11px">
        <option value="">-- Discovered Models on Router --</option>
      </select>
    </div>

    <div class="field-row">
      <label>API Key (Optional / Hub-stored)</label>
      <input type="password" id="cfg-apikey" placeholder="sk-… (optional for local router; required for official endpoints)"/>
    </div>

    <div class="field-row">
      <label>System Preamble</label>
      <textarea id="cfg-sysprompt" rows="2" placeholder="Optional system prompt preamble for chat completions"></textarea>
    </div>

    <div style="display:flex;gap:8px;margin-top:6px;align-items:center">
      <button id="btn-save-config" class="primary">Save Configuration</button>
      <span id="config-save-status" class="dim" style="font-size:11px"></span>
    </div>
  </div>
</div>

<script>
"use strict";
const Q = new URLSearchParams(location.search);
const CAP = Q.get("cap")
  || (location.pathname.match(/^\/w\/([0-9a-f]{64})$/) || [])[1]
  || "";
const AUTH = CAP ? ("cap="+encodeURIComponent(CAP)) : "";

function esc(s){return String(s).replace(/[&<>"']/g, c=>({"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;","'":"&#39;"}[c]));}
function ago(ts, now){const d=Math.max(0,now-ts);if(d<60)return d+"s";if(d<3600)return Math.floor(d/60)+"m";if(d<86400)return Math.floor(d/3600)+"h";return Math.floor(d/86400)+"d";}
function hms(ts){const d=ts%86400,p=n=>String(n).padStart(2,"0");return p(Math.floor(d/3600))+":"+p(Math.floor(d%3600/60))+":"+p(d%60);}

// Active Studio State
let currentLaneId = "";
let currentCwd = "~/";
let currentSessions = [];
let machinesList = [];
let activeChatBinId = 1;
let chatBinDirty = false;

// Completions config state
let completionsCfg = {
  provider: "openai",
  endpoint: "http://localhost:11434",
  api_key: "",
  model: "",
  selected_agent: "hermes",
  system_prompt: ""
};

// Set port pill text dynamically
const portPill = document.getElementById("port-pill");
if(portPill){
  const p = location.port || (location.protocol === "https:" ? "443" : "80");
  portPill.textContent = "● :" + p;
  portPill.addEventListener("click", ()=>{
    navigator.clipboard.writeText(location.host).then(()=>{
      portPill.textContent = "✓ copied!";
      setTimeout(()=>portPill.textContent = "● :" + p, 1200);
    });
  });
}

// Copy capability URL
document.getElementById("cap-url-chip").addEventListener("click", ()=>{
  const url = location.origin + (CAP ? ("/w/" + CAP) : location.pathname);
  navigator.clipboard.writeText(url).then(()=>{
    const chip = document.getElementById("cap-url-chip");
    chip.textContent = "✓ copied!";
    setTimeout(()=>chip.textContent = "📋 copy url", 1500);
  });
});

function render(s){
  const now = s.server.now;
  document.getElementById("meta").textContent =
    "hub v"+esc(s.server.version)+" · up "+ago(s.server.started_at, now)+" · "+s.agents.length+" agent(s) · "+s.events.length+" event(s) · utc "+hms(now);

  // Render Agents
  const ag = document.getElementById("agents");
  if(!s.agents.length){ag.innerHTML = '<span class="dim">no agents have checked in yet</span>';}
  else{
    const groups = {};
    for(const a of s.agents){
      const o = a.origin || "local";
      ((groups[o] = groups[o] || {})[a.device] = groups[o][a.device] || []).push(a);
    }
    let html = "";
    for(const [origin, devs] of Object.entries(groups)){
      const count = Object.values(devs).reduce((n,d)=>n+d.length,0);
      html += '<div style="padding:6px 0 2px"><span class="ochip">'+esc(origin)+'</span><span class="dim"> · '+count+' agent(s)</span></div>';
      for(const [device, list] of Object.entries(devs)){
        for(const a of list){
          const cls = a.stale ? "stale" : esc(a.status);
          const label = esc(a.status)+(a.stale?" · stale":"");
          html += '<div class="agent" style="margin-left:14px;padding:6px 0;border-bottom:1px dashed var(--edge)"><div class="bar"><span class="who" style="font-weight:bold">'+esc(a.agent)+'@'+esc(a.device)+'</span>'
            +(a.repo?'<span class="repo">'+esc(a.repo)+'</span>':"")
            +'<span class="pill '+cls+'">'+label+'</span>'
            +'<span class="dim" style="margin-left:auto;font-size:11px">'+ago(a.last_seen, now)+' ago</span></div>'
            +(a.task?'<div class="task" style="margin:2px 0 0;color:var(--accent)">'+esc(a.task)+'</div>':"")
            +(a.details?'<div class="dim" style="font-size:11px">'+esc(a.details)+'</div>':"")
            +'</div>';
        }
      }
    }
    ag.innerHTML = html;
  }

  // Render Feed
  const feed = document.getElementById("feed");
  const rows = s.events.slice().reverse().map(e=>
    '<li><span class="dim">#'+e.id+' '+hms(e.ts)+'</span> <span class="lv '+esc(e.level)+'">['+esc(e.level)+']</span> '
    +(e.origin?'<span class="ochip">'+esc(e.origin)+'</span> ':"")
    +'<b>'+esc(e.agent)+'@'+esc(e.device)+'</b> '
    +(e.repo?'<span class="repo">'+esc(e.repo)+'</span> ':"")
    +esc(e.message)+'</li>'
  ).join("");
  feed.innerHTML = rows || '<li class="dim">no events yet</li>';

  // Sessions and Bins
  currentSessions = s.sessions || [];
  renderLaneSelector(currentSessions);
  renderSessionsList(currentSessions, now);
  if(s.bin_scopes) updateScopeDropdown(s.bin_scopes);
  if(currentBinScope === "user") renderBins(s.bins, now);

  // Router probe status
  if(s.router){
    const alive = !!s.router.alive;
    const rp = document.getElementById("router-status-pill");
    const count = s.router.models_count || 0;
    const pillText = alive ? ("● :11434 (" + count + "m)") : "○ :11434";
    const pillCls = "pill " + (alive ? "ok" : "stale");
    if(rp){
      rp.className = pillCls;
      rp.textContent = pillText;
    }
    const rm = document.getElementById("router-model-info");
    if(rm){
      const ep = completionsCfg.endpoint || "http://localhost:11434";
      const mod = completionsCfg.model || (s.router.target_model || "auto");
      rm.textContent = ep + " · " + mod;
    }
    if(s.router.models && Array.isArray(s.router.models)){
      populateDiscoveredModels(s.router.models);
    }
  }
}

function renderLaneSelector(sessions){
  const sel = document.getElementById("lane-select");
  const prevVal = sel.value;
  sel.innerHTML = "";
  if(!sessions.length){
    const opt = document.createElement("option");
    opt.value = "";
    opt.textContent = "(no lanes — click + New Lane)";
    sel.appendChild(opt);
    currentLaneId = "";
    return;
  }
  for(const s of sessions){
    const opt = document.createElement("option");
    opt.value = s.id;
    opt.textContent = (s.name || s.id) + (s.repo ? (" (" + s.repo + ")") : "");
    sel.appendChild(opt);
  }
  if(prevVal && sessions.some(s=>s.id === prevVal)){
    sel.value = prevVal;
  } else if(!currentLaneId && sessions.length){
    sel.value = sessions[0].id;
    currentLaneId = sessions[0].id;
  }
  updateActiveLaneUI();
}

function updateActiveLaneUI(){
  const sel = document.getElementById("lane-select");
  const sid = sel.value;
  currentLaneId = sid;
  const sess = currentSessions.find(s=>s.id === sid);
  if(sess){
    document.getElementById("lane-scope-chip").textContent = sess.repo || "none";
    document.getElementById("lane-members-text").textContent = (sess.members || 0) + " member(s)";
    loadLaneChat(sid);
    loadActiveChatBin();
    const chatScope = "chat:" + sid;
    const chatName = sess.name || sid.slice(0, 12);
    const chatBtn = document.getElementById("btn-scope-active-chat");
    if(chatBtn){
      chatBtn.style.display = "inline-block";
      chatBtn.setAttribute("data-scope", chatScope);
      chatBtn.textContent = "💬 Chat Bins (" + chatName + ")";
    }
  } else {
    document.getElementById("lane-scope-chip").textContent = "-";
    document.getElementById("lane-members-text").textContent = "-";
    document.getElementById("chat-feed").innerHTML = '<div class="dim">select or create a chat lane above to begin</div>';
  }
}

document.getElementById("lane-select").addEventListener("change", updateActiveLaneUI);

document.getElementById("btn-new-lane").addEventListener("click", async ()=>{
  const name = prompt("Enter name for the new federated lane:", "team-collab");
  if(!name) return;
  const repo = prompt("Enter repo/scope label (e.g. backend@cluster):", "main");
  try{
    const r = await fetch("/api/v1/sessions?"+AUTH, {
      method: "POST",
      headers: {"Content-Type": "application/json"},
      body: JSON.stringify({name: name.trim(), repo: (repo||"").trim()})
    });
    const j = await r.json();
    if(!r.ok) throw new Error(j.error || ("HTTP " + r.status));
    alert("Lane created! ID: " + (j.session ? j.session.id : j.id));
    fetch("/api/v1/state?"+AUTH).then(r=>r.json()).then(render);
  }catch(e){
    alert("Create lane failed: " + e.message);
  }
});

async function loadLaneChat(id){
  const feed = document.getElementById("chat-feed");
  try{
    const r = await fetch("/api/v1/sessions/"+encodeURIComponent(id)+"/view?"+AUTH);
    if(r.ok){
      const j = await r.json();
      const msgs = j.msgs || [];
      if(!msgs.length){
        feed.innerHTML = '<div class="dim">no messages in this lane yet. Send a prompt below to converse with the team and active agent.</div>';
      } else {
        feed.innerHTML = msgs.map(m=>{
          const isAgent = m.sender && (m.sender.includes("agent") || m.sender.includes("cli") || m.sender.includes("hermes") || m.sender.includes("pi"));
          return '<div class="m '+(isAgent?'agent-msg':'')+'">'
            +'<div class="mtop"><b>'+esc(m.sender||'unknown')+'</b><span>#'+esc(m.seq)+'</span><span style="margin-left:auto">'+new Date(m.ts*1000).toISOString().slice(11,19)+'Z</span></div>'
            +'<pre>'+esc(m.text)+'</pre></div>';
        }).join("");
        feed.scrollTop = feed.scrollHeight;
      }
    } else {
      feed.innerHTML = '<div class="dim">Chat bodies encrypted or unavailable: HTTP '+r.status+'</div>';
    }
  }catch(e){
    feed.innerHTML = '<div class="dim">Fetch error: '+esc(String(e))+'</div>';
  }
}

// TEAM CHAT PASTEBINS LOGIC (3 Dedicated Bins per Team Chat)
document.querySelectorAll(".chat-bin-tab").forEach(tab=>{
  tab.addEventListener("click", function(){
    document.querySelectorAll(".chat-bin-tab").forEach(t=>t.classList.remove("active"));
    this.classList.add("active");
    activeChatBinId = parseInt(this.getAttribute("data-bin"), 10) || 1;
    loadActiveChatBin();
  });
});

async function loadActiveChatBin(){
  if(!currentLaneId) return;
  const scope = "chat:" + currentLaneId;
  const ed = document.getElementById("chat-bin-editor");
  const meta = document.getElementById("chat-bin-meta");
  meta.textContent = "loading bin " + activeChatBinId + "…";
  try{
    const r = await fetch("/api/v1/bins?scope="+encodeURIComponent(scope)+"&"+AUTH);
    if(r.ok){
      const j = await r.json();
      const b = (j.bins || []).find(x=>x.id === activeChatBinId);
      if(b){
        if(!chatBinDirty) ed.value = b.content || "";
        meta.textContent = (b.size || 0) + " chars" + (b.updated_by ? (" · " + b.updated_by) : "");
      } else {
        if(!chatBinDirty) ed.value = "";
        meta.textContent = "0 chars · empty";
      }
    }
  }catch(e){
    meta.textContent = "load error";
  }
}

document.getElementById("chat-bin-editor").addEventListener("input", ()=>{
  chatBinDirty = true;
  document.getElementById("btn-save-chat-bin").classList.add("dirty");
});

document.getElementById("btn-save-chat-bin").addEventListener("click", async ()=>{
  if(!currentLaneId) return;
  const scope = "chat:" + currentLaneId;
  const ed = document.getElementById("chat-bin-editor");
  const btn = document.getElementById("btn-save-chat-bin");
  const stat = document.getElementById("chat-bin-status");
  btn.disabled = true;
  stat.textContent = "saving…";
  try{
    const r = await fetch("/api/v1/bins/" + activeChatBinId + "?scope=" + encodeURIComponent(scope) + "&" + AUTH, {
      method: "PUT",
      headers: {"Content-Type": "application/json"},
      body: JSON.stringify({content: ed.value, scope: scope})
    });
    const j = await r.json();
    if(r.ok && j.ok){
      chatBinDirty = false;
      btn.classList.remove("dirty");
      stat.textContent = "✓ saved";
      setTimeout(()=>stat.textContent = "", 1500);
      loadActiveChatBin();
    } else {
      stat.textContent = "err: " + (j.error || r.status);
    }
  }catch(e){
    stat.textContent = "err: " + e.message;
  }
  btn.disabled = false;
});

document.getElementById("btn-copy-chat-bin").addEventListener("click", ()=>{
  const ed = document.getElementById("chat-bin-editor");
  const stat = document.getElementById("chat-bin-status");
  navigator.clipboard.writeText(ed.value).then(()=>{
    stat.textContent = "✓ copied!";
    setTimeout(()=>stat.textContent = "", 1200);
  }).catch(()=>{
    ed.select();
    document.execCommand("copy");
    stat.textContent = "✓ copied!";
    setTimeout(()=>stat.textContent = "", 1200);
  });
});

// Chat Execution (passes active agent and provider)
async function sendChat(){
  const input = document.getElementById("chat-input");
  const promptText = input.value.trim();
  if(!promptText) return;
  const stat = document.getElementById("chat-stat");
  const feed = document.getElementById("chat-feed");
  stat.textContent = "sending to " + completionsCfg.selected_agent + "…";
  const msgDiv = document.createElement("div");
  msgDiv.className = "m agent-msg";
  msgDiv.innerHTML = '<div class="mtop"><b>you</b><span style="margin-left:auto">'+new Date().toISOString().slice(11,19)+'Z</span></div><pre>'+esc(promptText)+'</pre>';
  feed.appendChild(msgDiv);
  feed.scrollTop = feed.scrollHeight;
  try{
    const r = await fetch("/api/v1/chat/completions?"+AUTH, {
      method: "POST",
      headers: {"Content-Type": "application/json"},
      body: JSON.stringify({
        prompt: promptText,
        agent: completionsCfg.selected_agent,
        provider: completionsCfg.provider,
        model: completionsCfg.model || undefined
      })
    });
    const j = await r.json();
    input.value = "";
    if(j.ok){
      stat.textContent = "replied via " + (j.agent || completionsCfg.selected_agent) + " (" + (j.model || "model") + ")";
      const rep = document.createElement("div");
      rep.className = "m agent-msg";
      rep.innerHTML = '<div class="mtop"><b>' + esc(j.agent || completionsCfg.selected_agent) + ' (' + esc(j.model||"") + ')' + '</b><span style="margin-left:auto">'+new Date().toISOString().slice(11,19)+'Z</span></div><pre>'+esc(j.content||"")+'</pre>';
      feed.appendChild(rep);
      feed.scrollTop = feed.scrollHeight;
    } else {
      stat.textContent = "failed: " + (j.error || ("HTTP "+r.status));
      const rep = document.createElement("div");
      rep.className = "m";
      rep.innerHTML = '<div class="mtop"><b>router error</b></div><pre>'+esc(j.error||("HTTP "+r.status))+'</pre>';
      feed.appendChild(rep);
      feed.scrollTop = feed.scrollHeight;
    }
  }catch(e){
    stat.textContent = "error: " + e.message;
  }
}

document.getElementById("btn-run-agent").addEventListener("click", sendChat);

document.getElementById("chat-input").addEventListener("keydown", (e)=>{
  if(e.key === "Enter" && !e.shiftKey){
    e.preventDefault();
    sendChat();
  }
});

// COMPLETIONS & AGENT CONFIG MODAL
const configModal = document.getElementById("config-modal");
document.getElementById("btn-open-config").addEventListener("click", ()=>{
  loadConfigIntoModal();
  configModal.classList.add("open");
});
document.getElementById("btn-close-config").addEventListener("click", ()=>configModal.classList.remove("open"));
configModal.addEventListener("click", (e)=>{
  if(e.target === configModal) configModal.classList.remove("open");
});

// Quick toggle active agent from header
document.getElementById("agent-toggle-pill").addEventListener("click", async ()=>{
  const next = completionsCfg.selected_agent === "hermes" ? "pi" : "hermes";
  completionsCfg.selected_agent = next;
  if(next === "pi" && completionsCfg.provider !== "openai") completionsCfg.provider = "anthropic";
  updateAgentUI();
  await saveCompletionsConfig();
});

function updateAgentUI(){
  const ag = completionsCfg.selected_agent || "hermes";
  const prov = completionsCfg.provider || "openai";
  const pill = document.getElementById("agent-toggle-pill");
  if(pill) pill.textContent = "⚡ Agent: " + ag;
  const ab = document.getElementById("active-agent-badge");
  if(ab) ab.textContent = ag.toUpperCase();
  const pb = document.getElementById("active-provider-badge");
  if(pb) pb.textContent = prov === "anthropic" ? "Anthropic" : "OpenAI";
  const cal = document.getElementById("chat-agent-label");
  if(cal) cal.textContent = "agent: " + ag + " (" + prov + ")";
}

document.getElementById("opt-agent-hermes").addEventListener("click", function(){
  this.classList.add("active");
  document.getElementById("opt-agent-pi").classList.remove("active");
  completionsCfg.selected_agent = "hermes";
});

document.getElementById("opt-agent-pi").addEventListener("click", function(){
  this.classList.add("active");
  document.getElementById("opt-agent-hermes").classList.remove("active");
  completionsCfg.selected_agent = "pi";
});

document.getElementById("opt-prov-openai").addEventListener("click", function(){
  this.classList.add("active");
  document.getElementById("opt-prov-anthropic").classList.remove("active");
  completionsCfg.provider = "openai";
});

document.getElementById("opt-prov-anthropic").addEventListener("click", function(){
  this.classList.add("active");
  document.getElementById("opt-prov-openai").classList.remove("active");
  completionsCfg.provider = "anthropic";
});

function setEndpointPreset(url){
  document.getElementById("cfg-endpoint").value = url;
}

function loadConfigIntoModal(){
  if(completionsCfg.selected_agent === "pi"){
    document.getElementById("opt-agent-pi").classList.add("active");
    document.getElementById("opt-agent-hermes").classList.remove("active");
  } else {
    document.getElementById("opt-agent-hermes").classList.add("active");
    document.getElementById("opt-agent-pi").classList.remove("active");
  }
  if(completionsCfg.provider === "anthropic"){
    document.getElementById("opt-prov-anthropic").classList.add("active");
    document.getElementById("opt-prov-openai").classList.remove("active");
  } else {
    document.getElementById("opt-prov-openai").classList.add("active");
    document.getElementById("opt-prov-anthropic").classList.remove("active");
  }
  document.getElementById("cfg-endpoint").value = completionsCfg.endpoint || "http://localhost:11434";
  document.getElementById("cfg-model").value = completionsCfg.model || "";
  document.getElementById("cfg-apikey").value = completionsCfg.api_key || "";
  document.getElementById("cfg-sysprompt").value = completionsCfg.system_prompt || "";
}

function populateDiscoveredModels(models){
  const sel = document.getElementById("cfg-model-select");
  if(!sel) return;
  sel.innerHTML = '<option value="">-- Discovered Models on Router (' + models.length + ') --</option>';
  for(const m of models){
    const opt = document.createElement("option");
    opt.value = m;
    opt.textContent = m;
    sel.appendChild(opt);
  }
  sel.onchange = function(){
    if(this.value) document.getElementById("cfg-model").value = this.value;
  };
}

async function fetchCompletionsConfig(){
  try{
    const r = await fetch("/api/v1/completions/config?"+AUTH);
    if(r.ok){
      const j = await r.json();
      completionsCfg.provider = j.provider || "openai";
      completionsCfg.endpoint = j.endpoint || "http://localhost:11434";
      completionsCfg.api_key = j.api_key || "";
      completionsCfg.model = j.model || "";
      completionsCfg.selected_agent = j.selected_agent || "hermes";
      completionsCfg.system_prompt = j.system_prompt || "";
      updateAgentUI();
      if(j.router && j.router.models){
        populateDiscoveredModels(j.router.models);
      }
    }
  }catch(e){}
}

async function saveCompletionsConfig(){
  const status = document.getElementById("config-save-status");
  if(status) status.textContent = "saving…";
  try{
    const r = await fetch("/api/v1/completions/config?"+AUTH, {
      method: "POST",
      headers: {"Content-Type": "application/json"},
      body: JSON.stringify(completionsCfg)
    });
    const j = await r.json();
    if(r.ok && j.ok){
      if(status) status.textContent = "✓ saved!";
      setTimeout(()=>{if(status) status.textContent = "";}, 1500);
      updateAgentUI();
    } else {
      if(status) status.textContent = "err: " + (j.error || r.status);
    }
  }catch(e){
    if(status) status.textContent = "err: " + e.message;
  }
}

document.getElementById("btn-save-config").addEventListener("click", async ()=>{
  completionsCfg.endpoint = document.getElementById("cfg-endpoint").value.trim() || "http://localhost:11434";
  completionsCfg.model = document.getElementById("cfg-model").value.trim();
  completionsCfg.api_key = document.getElementById("cfg-apikey").value.trim();
  completionsCfg.system_prompt = document.getElementById("cfg-sysprompt").value.trim();
  await saveCompletionsConfig();
  setTimeout(()=>configModal.classList.remove("open"), 800);
});

// FEDERATED SHELL LOGIC
async function loadMachines(){
  try{
    const r = await fetch("/api/v1/shell/machines?"+AUTH);
    if(r.ok){
      const j = await r.json();
      machinesList = j.machines || [];
      renderMachineChips(machinesList);
    }
  }catch(e){}
}

function renderMachineChips(machines){
  const bar = document.getElementById("machine-chips-bar");
  bar.innerHTML = '<span class="dim">Quick Jump:</span>';
  const allBtn = document.createElement("button");
  allBtn.textContent = "~/ (root)";
  allBtn.style.padding = "2px 8px";
  allBtn.style.fontSize = "11px";
  allBtn.addEventListener("click", ()=>switchCwd("~/"));
  bar.appendChild(allBtn);

  for(const m of machines){
    const btn = document.createElement("button");
    const tier = m.compute_tier ? (" [" + m.compute_tier.toUpperCase() + "]") : "";
    btn.textContent = "~/" + m.name + tier + (m.is_local ? " *" : "");
    btn.style.padding = "2px 8px";
    btn.style.fontSize = "11px";
    if(m.is_local) btn.style.borderColor = "var(--ok)";
    if(m.lkgl) btn.title = "Arch: " + (m.arch||"unknown") + " | LKGL: " + m.lkgl;
    btn.addEventListener("click", ()=>{
      switchCwd("~/" + m.name);
      const machScope = "machine:" + m.name;
      const machBtn = document.getElementById("btn-scope-active-mach");
      if(machBtn){
        machBtn.style.display = "inline-block";
        machBtn.setAttribute("data-scope", machScope);
        machBtn.textContent = "💻 Machine Bins (~/" + m.name + ")";
      }
    });
    bar.appendChild(btn);
  }
}

function switchCwd(newCwd){
  currentCwd = newCwd;
  document.getElementById("shell-cwd-badge").textContent = currentCwd;
  document.getElementById("shell-prompt-tag").textContent = "["+currentCwd+"]$";
  const machName = currentCwd.replace(/^~\/?/, "").split("/")[0];
  const found = machinesList.find(m => m.name === machName || (m.aliases && m.aliases.includes(machName)));
  const infoEl = document.getElementById("shell-lkgl-info");
  if(infoEl){
    if(found && found.lkgl){
      infoEl.textContent = "Arch: " + (found.arch||"unknown") + " · Tier: " + (found.compute_tier||"standard") + " · LKGL: " + found.lkgl;
    } else {
      infoEl.textContent = "Cluster Root · Cross-Architecture Compound Terminal";
    }
  }
}

async function execShell(){
  const inp = document.getElementById("shell-input");
  const cmd = inp.value.trim();
  if(!cmd) return;
  const out = document.getElementById("shell-out");
  out.textContent += "\n[" + currentCwd + "]$ " + cmd + "\n";
  inp.value = "";
  out.scrollTop = out.scrollHeight;

  try{
    const r = await fetch("/api/v1/shell/exec?"+AUTH, {
      method: "POST",
      headers: {"Content-Type": "application/json"},
      body: JSON.stringify({cmd: cmd, cwd: currentCwd, timeout_secs: 45})
    });
    const j = await r.json();
    if(j.new_cwd){
      switchCwd(j.new_cwd);
    }
    if(j.output){
      out.textContent += j.output;
    } else {
      out.textContent += (j.ok ? "[ok]\n" : "[exit " + j.exit_code + "]\n");
    }
    out.scrollTop = out.scrollHeight;
  }catch(e){
    out.textContent += "Execution error: " + e.message + "\n";
    out.scrollTop = out.scrollHeight;
  }
}

document.getElementById("btn-shell-exec").addEventListener("click", execShell);
document.getElementById("btn-shell-clear").addEventListener("click", ()=>{
  document.getElementById("shell-out").textContent = "=== WTF FEDERATED SHELL (cleared) ===\n";
});
document.getElementById("shell-input").addEventListener("keydown", (e)=>{
  if(e.key === "Enter") execShell();
});

// Render Sessions Overview
function renderSessionsList(sessions, now){
  const host = document.getElementById("sessions");
  if(!sessions.length){
    host.innerHTML = '<span class="dim">no chats yet — create one above or via session_create</span>';
    return;
  }
  let rows = "";
  for(const x of sessions){
    rows += '<div class="sess" style="padding:6px 0;border-bottom:1px dashed var(--edge)">'
      +'<div class="bar"><b style="color:var(--info)">'+esc(x.name||x.id)+'</b>'
      +(x.repo?'<span class="repo">'+esc(x.repo)+'</span>':"")
      +'<span class="dim" style="margin-left:auto;font-size:11px">'+esc(x.msg_count)+' msg(s) · '+esc(x.members)+' member(s)</span></div>'
      +'<div class="bar" style="font-size:11px"><span class="dim">'+esc(x.id)+'</span><button class="sess-bin-btn" onclick="openChatBins(\''+esc(x.id)+'\', \''+esc(x.name||x.id)+'\')" style="margin-left:auto">📦 Chat Bins</button></div></div>';
  }
  host.innerHTML = rows;
}

// CROSS-MACHINE FEDERATED BINS ARCHITECTURE
const binIds = [1,2,3];
const dirty = {};
let currentBinScope = "user";
let currentBinScopeLabel = "user (global mesh)";

function buildBins(){
  const host = document.getElementById("bins");
  host.innerHTML = "";
  for(const id of binIds){
    const card = document.createElement("div");card.className = "card bin";
    const head = document.createElement("div");head.className = "bhead";
    const title = document.createElement("b");title.id = "btitle-" + id;title.textContent = "BIN " + id;
    const meta = document.createElement("span");meta.className = "bmeta dim";meta.id = "bmeta-" + id;meta.textContent = "loading…";
    head.appendChild(title);head.appendChild(meta);
    const ta = document.createElement("textarea");ta.id = "ta-" + id;ta.spellcheck = false;ta.placeholder = "paste content for cross-machine agents here (keys, specs, instructions) — replicated across all machines";
    const btns = document.createElement("div");btns.className = "bbtns";
    const save = document.createElement("button");save.id = "save-" + id;save.textContent = "Save";
    const copy = document.createElement("button");copy.id = "copy-" + id;copy.textContent = "Copy";
    save.addEventListener("click", ()=>saveBin(id));
    copy.addEventListener("click", ()=>copyBin(id));
    ta.addEventListener("input", ()=>{dirty[id] = true;save.classList.add("dirty");});
    btns.appendChild(save);btns.appendChild(copy);
    card.appendChild(head);card.appendChild(ta);card.appendChild(btns);
    host.appendChild(card);
  }

  // Bind scope tabs
  document.getElementById("btn-scope-user").addEventListener("click", ()=>switchBinScope("user", "user (global mesh)"));
  document.getElementById("btn-scope-active-chat").addEventListener("click", function(){
    const sc = this.getAttribute("data-scope");
    if(sc) switchBinScope(sc, this.textContent.replace("💬 ", ""));
  });
  document.getElementById("btn-scope-active-mach").addEventListener("click", function(){
    const sc = this.getAttribute("data-scope");
    if(sc) switchBinScope(sc, this.textContent.replace("💻 ", ""));
  });
  document.getElementById("scope-dropdown").addEventListener("change", function(){
    switchBinScope(this.value, this.value);
  });
}

function updateScopeDropdown(scopes){
  const sel = document.getElementById("scope-dropdown");
  if(!sel || !scopes) return;
  let html = '<option value="user">user (global mesh)</option>';
  for(const sc of scopes){
    if(sc === "user") continue;
    let label = sc;
    if(sc.startsWith("chat:")) label = "💬 " + sc;
    else if(sc.startsWith("machine:")) label = "💻 " + sc;
    html += '<option value="'+esc(sc)+'">'+esc(label)+'</option>';
  }
  sel.innerHTML = html;
  if(scopes.includes(currentBinScope)){
    sel.value = currentBinScope;
  }
}

async function switchBinScope(scope, optLabel){
  currentBinScope = scope || "user";
  currentBinScopeLabel = optLabel || currentBinScope;

  const pill = document.getElementById("bins-scope-pill");
  if(pill) pill.textContent = "scope: " + currentBinScopeLabel;

  document.querySelectorAll(".scope-tab").forEach(t=>{
    if(t.getAttribute("data-scope") === currentBinScope){
      t.classList.add("active");
    } else {
      t.classList.remove("active");
    }
  });

  const sel = document.getElementById("scope-dropdown");
  if(sel && Array.from(sel.options).some(o=>o.value === currentBinScope)){
    sel.value = currentBinScope;
  }

  for(const id of binIds){
    const t = document.getElementById("btitle-" + id);
    if(t) t.textContent = "BIN " + id + (currentBinScope === "user" ? "" : " [" + currentBinScope + "]");
    const m = document.getElementById("bmeta-" + id);
    if(m) m.textContent = "loading " + currentBinScope + "…";
    delete dirty[id];
  }

  try{
    const r = await fetch("/api/v1/bins?scope="+encodeURIComponent(currentBinScope)+"&"+AUTH);
    if(r.ok){
      const j = await r.json();
      renderBins(j.bins, Math.floor(Date.now()/1000));
      if(j.scopes) updateScopeDropdown(j.scopes);
    }
  }catch(e){}
}

function openChatBins(sessionId, sessionName){
  const chatScope = "chat:" + sessionId;
  const chatBtn = document.getElementById("btn-scope-active-chat");
  if(chatBtn){
    chatBtn.style.display = "inline-block";
    chatBtn.setAttribute("data-scope", chatScope);
    chatBtn.textContent = "💬 Chat Bins (" + (sessionName||sessionId.slice(0,12)) + ")";
  }
  switchBinScope(chatScope, "chat:" + (sessionName||sessionId));
  document.getElementById("bins-sec").scrollIntoView({behavior:"smooth"});
}

function renderBins(bins, now){
  if(!bins) return;
  for(const b of bins){
    const ta = document.getElementById("ta-" + b.id);
    if(!ta) continue;
    if(!dirty[b.id] && ta.value !== (b.content || "")){ta.value = b.content || "";}
    const m = document.getElementById("bmeta-" + b.id);
    if(b.content && b.content.length > 0){
      const byAgent = b.updated_by && b.updated_by !== "dashboard";
      m.innerHTML = (b.size||b.content.length)+" chars · updated "+ago(b.updated_at,now)+" ago by "
        +(byAgent?'<span class="who">✎ '+esc(b.updated_by)+'</span>':esc(b.updated_by||"unknown"));
    } else {
      m.textContent = "0 chars · empty (" + esc(b.scope||currentBinScope) + ")";
    }
  }
}

async function saveBin(id){
  const ta = document.getElementById("ta-" + id);
  const btn = document.getElementById("save-" + id);
  btn.disabled = true;btn.textContent = "Saving…";
  try{
    const r = await fetch("/api/v1/bins/"+id+"?scope="+encodeURIComponent(currentBinScope)+"&"+AUTH, {
      method:"PUT",
      headers:{"Content-Type":"application/json"},
      body:JSON.stringify({content:ta.value, scope:currentBinScope})
    });
    const j = await r.json();
    if(!r.ok||!j.ok) throw new Error(j.error||("HTTP "+r.status));
    dirty[id] = false;btn.classList.remove("dirty");btn.textContent = "Saved";
    setTimeout(()=>{btn.textContent = "Save";}, 1500);
    fetch("/api/v1/bins?"+AUTH).then(res=>res.json()).then(data=>{
      if(data.scopes) updateScopeDropdown(data.scopes);
    }).catch(()=>{});
  }catch(e){
    document.getElementById("bmeta-" + id).textContent = "save failed: " + esc(String(e.message||e));
    btn.textContent = "Save";
  }
  btn.disabled = false;
}

function copyBin(id){
  const ta = document.getElementById("ta-" + id);
  const btn = document.getElementById("copy-" + id);
  const done = ()=>{btn.textContent = "Copied";setTimeout(()=>{btn.textContent = "Copy";}, 1200);};
  if(navigator.clipboard && navigator.clipboard.writeText){navigator.clipboard.writeText(ta.value).then(done).catch(()=>{ta.select();document.execCommand("copy");done();});}
  else{ta.select();document.execCommand("copy");done();}
}

function boot(){
  buildBins();
  loadMachines();
  fetchCompletionsConfig();
  fetch("/api/v1/state?"+AUTH)
    .then(r=>{if(!r.ok)throw 0;return r.json();})
    .then(render)
    .catch(()=>{document.getElementById("meta").textContent="state fetch failed — open the link printed by wtf dashboard-url";});
  
  const es = new EventSource("/stream?"+AUTH);
  const dot = document.getElementById("conn");
  es.addEventListener("state", ev=>{try{render(JSON.parse(ev.data));}catch(e){}});
  es.onopen = ()=>{dot.classList.add("on");};
  es.onerror = ()=>{dot.classList.remove("on");};

  setInterval(()=>{
    if(currentLaneId) loadLaneChat(currentLaneId);
  }, 4000);
}
boot();
</script>
</body>
</html>
"#;
