//! Embedded dashboard: single dark-theme HTML page, no external assets.
//! Bootstraps from /api/v1/state?k=… or /w/<capability> then live-updates via SSE (/stream).
//! Integrates User Chat & SWE-bench Agent Orchestration paired with a Federated Shell.

pub const PAGE: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>WTF is going on? · Studio & Federated Shell</title>
<style>
:root{--bg:#0b0e14;--panel:#131824;--panel2:#0e131d;--edge:#232b3d;--edge2:#2d374d;--ink:#d7dde8;--dim:#8a93a6;--ok:#38d17c;--warn:#f0b429;--err:#ff5c5c;--info:#4aa3ff;--accent:#00ffc8;--term-bg:#05070c}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--ink);font:14px/1.5 ui-monospace,SFMono-Regular,Menlo,Consolas,monospace}
header{display:flex;align-items:center;gap:14px;padding:12px 20px;border-bottom:1px solid var(--edge);background:var(--panel2);flex-wrap:wrap}
h1{font-size:15px;margin:0;letter-spacing:2px;font-weight:700}
#meta{color:var(--dim);font-size:12px}
.dot{width:10px;height:10px;border-radius:50%;background:var(--dim);margin-left:auto}
.dot.on{background:var(--ok);box-shadow:0 0 8px rgba(56,209,124,.6)}
.cap-badge{font-size:11px;background:#162238;color:var(--accent);border:1px solid #234;border-radius:6px;padding:2px 8px;cursor:pointer}
.cap-badge:hover{border-color:var(--accent)}
main{padding:16px 20px;display:flex;flex-direction:column;gap:18px}
h2{font-size:12px;color:var(--dim);letter-spacing:1px;margin:0 0 8px;text-transform:uppercase}
.card{background:var(--panel);border:1px solid var(--edge);border-radius:8px;padding:12px}

/* Studio: Chat & Federated Shell Split View */
#studio{display:grid;grid-template-columns:1fr 1fr;gap:16px}
@media(max-width:1024px){#studio{grid-template-columns:1fr}}

.studio-pane{display:flex;flex-direction:column;gap:8px}
.bar{display:flex;gap:8px;align-items:center;flex-wrap:wrap}
select,input[type="text"],textarea{background:var(--bg);color:var(--ink);border:1px solid var(--edge2);border-radius:6px;padding:6px 8px;font:inherit}
select:focus,input[type="text"]:focus,textarea:focus{outline:none;border-color:var(--accent)}
button{background:#162947;color:var(--ink);border:1px solid var(--edge2);border-radius:6px;padding:5px 12px;font:inherit;cursor:pointer;transition:all .15s}
button:hover{border-color:var(--dim);background:#1d355c}
button.primary{background:var(--accent);color:#0b0e14;font-weight:700;border:none}
button.primary:hover{opacity:.9}

.feed-box{background:var(--term-bg);border:1px solid var(--edge);border-radius:6px;padding:10px;height:340px;overflow-y:auto;font-size:13px;display:flex;flex-direction:column;gap:8px}
.m{border-bottom:1px dashed var(--edge);padding-bottom:6px}
.m:last-child{border-bottom:0}
.m .mtop{display:flex;gap:8px;font-size:11px;color:var(--dim)}
.m pre{margin:4px 0 0;white-space:pre-wrap;word-break:break-word;color:var(--ink);font-family:inherit}
.m.agent-msg pre{color:var(--accent)}

/* Federated Shell styling */
#shell-out{background:var(--term-bg);border:1px solid var(--edge);border-radius:6px;padding:10px;height:340px;overflow-y:auto;color:#a8d18d;font-size:12px;white-space:pre-wrap;word-break:break-word}
.cmd-prompt-line{color:var(--accent);font-weight:bold}
.cmd-badge-local{color:#38d17c}
.cmd-badge-remote{color:#f0b429}

.pill{font-size:11px;padding:1px 8px;border-radius:10px;border:1px solid var(--dim);color:var(--dim)}
.pill.working{color:var(--ok);border-color:var(--ok)}
.pill.blocked{color:var(--warn);border-color:var(--warn)}
.pill.done{color:var(--info);border-color:var(--info)}
.pill.stale{opacity:.5}
.ochip{font-size:11px;padding:1px 7px;border-radius:10px;border:1px solid var(--info);color:var(--info);margin-right:4px}
.repo{font-size:11px;padding:1px 7px;border-radius:10px;border:1px solid var(--warn);color:var(--warn);margin:0 6px}
.dim{color:var(--dim)}

/* Overview Grid */
#overview{display:grid;grid-template-columns:1fr 1fr;gap:16px}
@media(max-width:900px){#overview{grid-template-columns:1fr}}
#feed{list-style:none;margin:0;padding:0;max-height:380px;overflow-y:auto}
#feed li{padding:4px 0;border-bottom:1px dashed var(--edge);white-space:pre-wrap;word-break:break-word}
.lv{font-size:11px}
.lv.error{color:var(--err)}
.lv.warn{color:var(--warn)}
.lv.info{color:var(--info)}

/* Bins */
#bins{display:grid;grid-template-columns:repeat(3,minmax(220px,1fr));gap:16px;padding:0}
@media(max-width:1100px){#bins{grid-template-columns:1fr}}
.bin textarea{width:100%;min-height:120px;margin-top:6px;background:var(--bg);color:var(--ink);border:1px solid var(--edge);border-radius:6px;padding:8px;font:inherit;resize:vertical}
.bhead{display:flex;gap:8px;align-items:baseline}
.bmeta{margin-left:auto;font-size:11px}
.bmeta .who{color:var(--info)}
.bbtns{display:flex;gap:8px;margin-top:6px}
.bbtns button.dirty{border-color:var(--warn);color:var(--warn)}

/* Sessions card */
.sess{padding:8px 0;border-bottom:1px dashed var(--edge);cursor:pointer}
.sess:hover{background:rgba(74,163,255,.06)}
.sess:last-child{border-bottom:0}
</style>
</head>
<body>
<header>
  <h1>WTF IS GOING ON</h1>
  <span id="meta" class="dim">connecting…</span>
  <span id="cap-url-chip" class="cap-badge" title="Click to copy singular dashboard URL">📋 copy url</span>
  <span class="pill" style="border-color:var(--accent);color:var(--accent)" title="Loopback router proxy">proxy: 11434</span>
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
      <div id="chat-feed" class="feed-box">
        <div class="dim">select or create a chat lane above to begin</div>
      </div>
      <div class="bar">
        <select id="agent-select" style="font-size:11px">
          <option value="auto">⚡ Auto Fallback Cascade</option>
          <option value="fleet">🤖 SWE-bench Fleet (Trae + Mini)</option>
          <option value="free-claude-code">Claude Code / FCC</option>
          <option value="omp">OhMyPy (omp)</option>
          <option value="hermes">Hermes Agent</option>
          <option value="trae-cli">Trae-CLI (AST Refactoring Master)</option>
          <option value="mini">Mini-SWE (TDD Reproduction Engineer)</option>
          <option value="codex">Codex CLI</option>
          <option value="opencode">OpenCode CLI</option>
          <option value="aider">Aider CLI</option>
          <option value="cline">Cline CLI</option>
          <option value="pi">Pi Coding Agent</option>
        </select>
        <label style="font-size:11px;display:flex;align-items:center;gap:4px;cursor:pointer"><input type="checkbox" id="fleet-toggle" checked/> Fleet</label>
        <span class="pill" style="border-color:var(--accent);color:var(--accent);font-size:10px" title="Single-source router model via 11434">model: local-router/fallback-models</span>
        <button id="btn-run-agent" class="primary" style="margin-left:auto;font-size:11px">⚡ Run Agent</button>
        <button id="btn-post-msg" style="font-size:11px">💬 Post</button>
      </div>
      <textarea id="chat-input" rows="2" placeholder="Type instructions for agents across machines, or message to lane... (Enter dispatches agent)"></textarea>
    </div>

    <!-- Federated Shell -->
    <div class="card studio-pane">
      <div class="bar">
        <h2>FEDERATED SHELL</h2>
        <span class="dim" style="font-size:11px">Root: <b style="color:var(--accent)">~/</b> (cluster machines)</span>
        <span id="shell-cwd-badge" class="pill working" style="margin-left:auto">~/</span>
      </div>
      <div id="shell-lkgl-info" class="dim" style="font-size:10px;margin:2px 0 4px 0">Cluster Root · Cross-Architecture Compound Terminal</div>
      <div class="bar" id="machine-chips-bar" style="font-size:11px">
        <span class="dim">Quick Jump:</span>
        <button class="mach-chip" data-dir="~/" style="padding:1px 8px;font-size:11px">~/ (all)</button>
      </div>
      <div id="shell-out">=== WTF FEDERATED SHELL & DISTRIBUTED OMP ===
Virtual cluster root (~/) anchors to connected machines with persistent architecture LKGL.
Dispatched tasks & 'omp' inherit synchronized local-router proxy (:11434) and fallback cascades.
Supports cross-architecture compound orchestration in one prompt:
  cd ~/mac && echo "mac ok" && cd ~/windows && echo "win ok"
Type 'ls ~' or 'cd <machine>' to navigate.</div>
      <div class="bar" style="margin-top:2px">
        <span id="shell-prompt-tag" style="color:var(--ok);font-weight:bold;font-size:12px">[~/]$</span>
        <input type="text" id="shell-input" style="flex:1;font-size:12px" placeholder="ls ~  OR  cd mac && npm test  OR  cd ~/windows && cargo test"/>
        <button id="btn-shell-exec" class="primary" style="font-size:11px">Exec</button>
        <button id="btn-shell-clear" style="font-size:11px">Clear</button>
      </div>
    </div>
  </section>

  <!-- OVERVIEW: Agents & Event Stream -->
  <section id="overview">
    <div>
      <h2>AGENTS</h2>
      <div id="agents" class="card"><span class="dim">no agents have checked in yet</span></div>
    </div>
    <div>
      <h2>EVENT LOG</h2>
      <div class="card"><ul id="feed"><li class="dim">no events yet</li></ul></div>
    </div>
  </section>

  <!-- SESSIONS & BINS -->
  <section>
    <h2>SESSIONS · federated agent chats</h2>
    <div id="sessions" class="card"><span class="dim">no chats yet</span></div>
  </section>

  <section id="bins-sec">
    <h2>SHARED BINS · paste here for agents · agents publish back with write_bin</h2>
    <div id="bins"></div>
  </section>
</main>

<script>
"use strict";
const Q = new URLSearchParams(location.search);
const K = Q.get("k") || "";
const CAP = Q.get("cap")
  || (location.pathname.match(/^\/w\/([0-9a-f]{64})$/) || [])[1]
  || "";
const AUTH = CAP ? ("cap="+encodeURIComponent(CAP)) : (K ? ("k="+encodeURIComponent(K)) : "");

function esc(s){return String(s).replace(/[&<>"']/g, c=>({"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;","'":"&#39;"}[c]));}
function ago(ts, now){const d=Math.max(0,now-ts);if(d<60)return d+"s";if(d<3600)return Math.floor(d/60)+"m";if(d<86400)return Math.floor(d/3600)+"h";return Math.floor(d/86400)+"d";}
function hms(ts){const d=ts%86400,p=n=>String(n).padStart(2,"0");return p(Math.floor(d/3600))+":"+p(Math.floor(d%3600/60))+":"+p(d%60);}

// Active Studio State
let currentLaneId = "";
let currentCwd = "~/";
let currentSessions = [];
let machinesList = [];

// Copy capability URL
document.getElementById("cap-url-chip").addEventListener("click", ()=>{
  const url = location.origin + (CAP ? ("/w/" + CAP) : (location.pathname + (K ? ("?k=" + K) : "")));
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
          html += '<div class="agent" style="margin-left:14px;padding:6px 0;border-bottom:1px dashed var(--edge)"><div class="top"><span class="who">'+esc(a.agent)+'@'+esc(a.device)+'</span>'
            +(a.repo?'<span class="repo">'+esc(a.repo)+'</span>':"")
            +'<span class="pill '+cls+'">'+label+'</span>'
            +'<span class="dim" style="margin-left:auto;font-size:11px">'+ago(a.last_seen, now)+' ago</span></div>'
            +(a.task?'<div class="task" style="margin:2px 0 0">'+esc(a.task)+'</div>':"")
            +(a.details?'<div class="dim" style="font-size:12px">'+esc(a.details)+'</div>':"")
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
  renderBins(s.bins, now);
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
  } else {
    document.getElementById("lane-scope-chip").textContent = "-";
    document.getElementById("lane-members-text").textContent = "-";
    document.getElementById("chat-feed").innerHTML = '<div class="dim">select or create a chat lane above to begin</div>';
  }
}

document.getElementById("lane-select").addEventListener("change", updateActiveLaneUI);

document.getElementById("btn-new-lane").addEventListener("click", async ()=>{
  const name = prompt("Enter name for the new federated lane:", "cross-arch-release");
  if(!name) return;
  const repo = prompt("Enter repo/scope label (e.g. frontend+backend@mac+windows):", "main");
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
        feed.innerHTML = '<div class="dim">no messages in this lane yet. Type below to dispatch agents or post messages.</div>';
      } else {
        feed.innerHTML = msgs.map(m=>{
          const isAgent = m.sender && (m.sender.includes("agent") || m.sender.includes("cli"));
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

// Run Agent Dispatch
async function runAgent(){
  const input = document.getElementById("chat-input");
  const promptText = input.value.trim();
  if(!promptText) return;
  if(!currentLaneId){alert("Please select or create a lane first.");return;}
  
  const sess = currentSessions.find(s=>s.id === currentLaneId);
  const slug = (sess && sess.name ? sess.name : currentLaneId).toLowerCase().replace(/[^a-z0-9]+/g,"-").replace(/^-+|-+$/g,"").slice(0,24)||"task";
  const termName = "wtf-chat-" + slug;
  
  const agentChoice = document.getElementById("agent-select").value;
  const fleetOn = document.getElementById("fleet-toggle").checked;
  const stat = document.getElementById("chat-stat");
  stat.textContent = "running " + agentChoice + "…";
  
  try{
    const r = await fetch("/api/v1/term/"+termName+"?"+AUTH, {
      method: "POST",
      headers: {"Content-Type": "application/json"},
      body: JSON.stringify({prompt: promptText, agent: agentChoice, fleet: fleetOn})
    });
    const j = await r.json();
    stat.textContent = j.ok ? ("completed (" + j.cli + ")") : ("failed (" + (j.error || j.cli) + ")");
    input.value = "";
    if(j.output){
      const feed = document.getElementById("chat-feed");
      const msgDiv = document.createElement("div");
      msgDiv.className = "m agent-msg";
      msgDiv.innerHTML = '<div class="mtop"><b>'+esc(j.cli)+'</b><span>(exit '+(j.ok?'0':'fail')+')</span><span style="margin-left:auto">'+new Date().toISOString().slice(11,19)+'Z</span></div><pre>'+esc(j.output)+'</pre>';
      feed.appendChild(msgDiv);
      feed.scrollTop = feed.scrollHeight;
    }
    setTimeout(()=>loadLaneChat(currentLaneId), 600);
  }catch(e){
    stat.textContent = "error: " + e.message;
  }
}

document.getElementById("btn-run-agent").addEventListener("click", runAgent);

document.getElementById("btn-post-msg").addEventListener("click", async ()=>{
  const input = document.getElementById("chat-input");
  const text = input.value.trim();
  if(!text || !currentLaneId) return;
  const sess = currentSessions.find(s=>s.id === currentLaneId);
  const slug = (sess && sess.name ? sess.name : currentLaneId).toLowerCase().replace(/[^a-z0-9]+/g,"-").replace(/^-+|-+$/g,"").slice(0,24)||"task";
  const termName = "wtf-chat-" + slug;
  try{
    await fetch("/api/v1/term/"+termName+"?"+AUTH, {
      method: "POST",
      headers: {"Content-Type": "application/json"},
      body: JSON.stringify({keys: text})
    });
    input.value = "";
    setTimeout(()=>loadLaneChat(currentLaneId), 500);
  }catch(e){alert(e.message);}
});

document.getElementById("chat-input").addEventListener("keydown", (e)=>{
  if(e.key === "Enter" && !e.shiftKey){
    e.preventDefault();
    runAgent();
  }
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
    btn.addEventListener("click", ()=>switchCwd("~/" + m.name));
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
      +'<div class="dim" style="font-size:11px">'+esc(x.id)+'</div></div>';
  }
  host.innerHTML = rows;
}

// BINS
const binIds = [1,2,3];
const dirty = {};
function buildBins(){
  const host = document.getElementById("bins");
  host.innerHTML = "";
  for(const id of binIds){
    const card = document.createElement("div");card.className = "card bin";
    const head = document.createElement("div");head.className = "bhead";
    const title = document.createElement("b");title.textContent = "BIN " + id;
    const meta = document.createElement("span");meta.className = "bmeta dim";meta.id = "bmeta-" + id;meta.textContent = "loading…";
    head.appendChild(title);head.appendChild(meta);
    const ta = document.createElement("textarea");ta.id = "ta-" + id;ta.spellcheck = false;ta.placeholder = "paste content for agents here — agents publish via write_bin too";
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
}

function renderBins(bins, now){
  if(!bins) return;
  for(const b of bins){
    const ta = document.getElementById("ta-" + b.id);
    if(!ta) continue;
    if(!dirty[b.id] && ta.value !== b.content){ta.value = b.content;}
    const m = document.getElementById("bmeta-" + b.id);
    if(b.updated_by){
      const byAgent = b.updated_by !== "dashboard";
      m.innerHTML = b.size+" chars · updated "+ago(b.updated_at,now)+" ago by "
        +(byAgent?'<span class="who">✎ '+esc(b.updated_by)+'</span>':esc(b.updated_by));
    } else {
      m.textContent = b.size + " chars · empty";
    }
  }
}

async function saveBin(id){
  const ta = document.getElementById("ta-" + id);
  const btn = document.getElementById("save-" + id);
  btn.disabled = true;btn.textContent = "Saving…";
  try{
    const r = await fetch("/api/v1/bins/"+id+"?"+AUTH, {method:"PUT", headers:{"Content-Type":"application/json"}, body:JSON.stringify({content:ta.value})});
    const j = await r.json();
    if(!r.ok||!j.ok) throw new Error(j.error||("HTTP "+r.status));
    dirty[id] = false;btn.classList.remove("dirty");btn.textContent = "Saved";
    setTimeout(()=>{btn.textContent = "Save";}, 1500);
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

async function loadAvailableAgents(){
  try{
    const r = await fetch("/api/v1/agents/available?"+AUTH);
    if(r.ok){
      const j = await r.json();
      const agents = j.agents || [];
      const sel = document.getElementById("agent-select");
      for(const opt of sel.options){
        const found = agents.find(a=>a.id === opt.value);
        if(found){
          if(found.installed){
            opt.textContent = "✓ " + found.name;
          } else {
            opt.textContent = "· " + found.name + " (not detected)";
          }
        }
      }
    }
  }catch(e){}
}

function boot(){
  buildBins();
  loadMachines();
  loadAvailableAgents();
  fetch("/api/v1/state?"+AUTH)
    .then(r=>{if(!r.ok)throw 0;return r.json();})
    .then(render)
    .catch(()=>{document.getElementById("meta").textContent="state fetch failed — open the link printed by wtf dashboard-url";});
  
  const es = new EventSource("/stream?"+AUTH);
  const dot = document.getElementById("conn");
  es.addEventListener("state", ev=>{try{render(JSON.parse(ev.data));}catch(e){}});
  es.onopen = ()=>{dot.classList.add("on");};
  es.onerror = ()=>{dot.classList.remove("on");};

  // Poll active lane chat periodically
  setInterval(()=>{
    if(currentLaneId) loadLaneChat(currentLaneId);
  }, 4000);
}
boot();
</script>
</body>
</html>
"#;
