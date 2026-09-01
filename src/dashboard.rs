//! Embedded dashboard: single dark-theme HTML page, no external assets.
//! Bootstraps from /api/v1/state?k=… then live-updates via SSE (/stream?k=…).

pub const PAGE: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>WTF is going on?</title>
<style>
:root{--bg:#0b0e14;--panel:#131824;--edge:#232b3d;--ink:#d7dde8;--dim:#8a93a6;--ok:#38d17c;--warn:#f0b429;--err:#ff5c5c;--info:#4aa3ff}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--ink);font:14px/1.5 ui-monospace,SFMono-Regular,Menlo,Consolas,monospace}
header{display:flex;align-items:center;gap:16px;padding:14px 20px;border-bottom:1px solid var(--edge)}
h1{font-size:15px;margin:0;letter-spacing:2px}
#meta{color:var(--dim);font-size:12px}
.dot{width:10px;height:10px;border-radius:50%;background:var(--dim);margin-left:auto}
.dot.on{background:var(--ok)}
main{display:grid;grid-template-columns:minmax(320px,1fr) minmax(320px,1fr);gap:16px;padding:16px 20px}
@media(max-width:900px){main{grid-template-columns:1fr}}
h2{font-size:12px;color:var(--dim);letter-spacing:1px;margin:0 0 8px}
.card{background:var(--panel);border:1px solid var(--edge);border-radius:8px;padding:12px}
.agent{padding:8px 0;border-bottom:1px dashed var(--edge)}
.agent:last-child{border-bottom:0}
.agent .top{display:flex;gap:8px;align-items:baseline}
.agent .who{font-weight:bold}
.pill{font-size:11px;padding:1px 8px;border-radius:10px;border:1px solid var(--dim);color:var(--dim)}
.pill.working{color:var(--ok);border-color:var(--ok)}
.pill.blocked{color:var(--warn);border-color:var(--warn)}
.pill.done{color:var(--info);border-color:var(--info)}
.pill.stale{opacity:.5}
.task{margin:2px 0 0 0}
.details{margin:0;color:var(--dim)}
.age{margin-left:auto;color:var(--dim);font-size:12px}
#feed{list-style:none;margin:0;padding:0;max-height:72vh;overflow-y:auto}
#feed li{padding:4px 0;border-bottom:1px dashed var(--edge);white-space:pre-wrap;word-break:break-word}
.lv{font-size:11px}
.lv.error{color:var(--err)}
.lv.warn{color:var(--warn)}
.lv.info{color:var(--info)}
.dim{color:var(--dim)}
#bins-sec{padding:0 20px 16px}
#bins{display:grid;grid-template-columns:repeat(3,minmax(220px,1fr));gap:16px;padding:0}
@media(max-width:1100px){#bins{grid-template-columns:1fr}}
.bin textarea{width:100%;min-height:150px;margin-top:8px;background:var(--bg);color:var(--ink);border:1px solid var(--edge);border-radius:6px;padding:8px;font:inherit;resize:vertical;white-space:pre;overflow-wrap:normal;overflow-x:auto}
.bhead{display:flex;gap:8px;align-items:baseline}
.bin b{letter-spacing:1px}
.bmeta{margin-left:auto;font-size:12px}
.bmeta .who{color:var(--info)}
.bbtns{display:flex;gap:8px;margin-top:8px}
.bbtns button{background:var(--bg);color:var(--ink);border:1px solid var(--edge);border-radius:6px;padding:4px 12px;font:inherit;cursor:pointer}
.bbtns button:hover{border-color:var(--dim)}
.bbtns button.dirty{border-color:var(--warn);color:var(--warn)}
.ochip{font-size:11px;padding:1px 7px;border-radius:10px;border:1px solid var(--info);color:var(--info);margin-right:4px}
.repo{font-size:11px;padding:1px 7px;border-radius:10px;border:1px solid var(--warn);color:var(--warn);margin:0 6px}
.origin{padding:10px 0 2px;font-size:13px}
.origin:first-child{padding-top:0}
.sess{padding:8px 0;border-bottom:1px dashed var(--edge);cursor:pointer}
.sess:hover{background:rgba(74,163,255,.06)}
.sess:last-child{border-bottom:0}
.sess .top{display:flex;gap:8px;align-items:baseline}
.sess .sid{font-size:11px;word-break:break-all}
</style>
</head>
<body>
<header><h1>WTF IS GOING ON</h1><span id="meta" class="dim">connecting…</span><span id="conn" class="dot" title="live stream"></span></header>
<main>
<section><h2>AGENTS</h2><div id="agents" class="card"><span class="dim">no agents have checked in yet</span></div></section>
<section><h2>EVENT LOG</h2><div class="card"><ul id="feed"></ul></div></section>
<section><h2>SESSIONS · federated agent chats — click to open</h2><div id="sessions" class="card"><span class="dim">no chats yet</span></div></section>
<section id="bins-sec"><h2>SHARED BINS · paste here and tell any agent “work from bin N” · agents publish back with write_bin</h2><div id="bins"></div></section>
<script>
"use strict";
const Q = new URLSearchParams(location.search);
const K = Q.get("k") || "";
// Capability self-discovery: on a loopback hub the page is served at
// /w/<capability> with no query string, so derive the token from the path.
const CAP = Q.get("cap")
  || (location.pathname.match(/^\/w\/([0-9a-f]{64})$/) || [])[1]
  || "";
const AUTH = K ? ("k="+encodeURIComponent(K)) : (CAP ? ("cap="+encodeURIComponent(CAP)) : "");
function esc(s){return String(s).replace(/[&<>"']/g, c=>({"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;","'":"&#39;"}[c]));}
function ago(ts, now){const d=Math.max(0,now-ts);if(d<60)return d+"s";if(d<3600)return Math.floor(d/60)+"m";if(d<86400)return Math.floor(d/3600)+"h";return Math.floor(d/86400)+"d";}
function hms(ts){const d=ts%86400,p=n=>String(n).padStart(2,"0");return p(Math.floor(d/3600))+":"+p(Math.floor(d%3600/60))+":"+p(d%60);}
function render(s){
  const now = s.server.now;
  document.getElementById("meta").textContent =
    "hub v"+esc(s.server.version)+" · up "+ago(s.server.started_at, now)+" · "+s.agents.length+" agent(s) · "+s.events.length+" event(s) · utc "+hms(now);
  const ag = document.getElementById("agents");
  if(!s.agents.length){ag.innerHTML = '<span class="dim">no agents have checked in yet</span>';}
  else{
    // group by origin hub, then device; repo chips per agent
    const groups = {};
    for(const a of s.agents){
      const o = a.origin || "local";
      ((groups[o] = groups[o] || {})[a.device] = groups[o][a.device] || []).push(a);
    }
    let html = "";
    for(const [origin, devs] of Object.entries(groups)){
      const count = Object.values(devs).reduce((n,d)=>n+d.length,0);
      html += '<div class="origin"><span class="ochip">'+esc(origin)+'</span><span class="dim"> · '+count+' agent(s)</span></div>';
      for(const [device, list] of Object.entries(devs)){
        for(const a of list){
          const cls = a.stale ? "stale" : esc(a.status);
          const label = esc(a.status)+(a.stale?" · stale":"");
          html += '<div class="agent" style="margin-left:14px"><div class="top"><span class="who">'+esc(a.agent)+'@'+esc(a.device)+'</span>'
            +(a.repo?'<span class="repo">'+esc(a.repo)+'</span>':"")
            +'<span class="pill '+cls+'">'+label+'</span>'
            +'<span class="age">'+ago(a.last_seen, now)+' ago</span></div>'
            +(a.task?'<div class="task">'+esc(a.task)+'</div>':"")
            +(a.details?'<div class="details">'+esc(a.details)+'</div>':"")
            +'</div>';
        }
      }
    }
    ag.innerHTML = html;
  }
  const feed = document.getElementById("feed");
  const rows = s.events.slice().reverse().map(e=>
    '<li><span class="dim">#'+e.id+' '+hms(e.ts)+'</span> <span class="lv '+esc(e.level)+'">['+esc(e.level)+']</span>'
    +(e.origin?'<span class="ochip">'+esc(e.origin)+'</span> ':"")
    +'<b>'+esc(e.agent)+'@'+esc(e.device)+'</b> '
    +(e.repo?'<span class="repo">'+esc(e.repo)+'</span> ':"")
    +esc(e.message)+'</li>'
  ).join("");
  feed.innerHTML = rows || '<li class="dim">no events yet</li>';
  renderSessions(s.sessions || [], now);
  renderBins(s.bins, now);
}
function renderSessions(sessions, now){
  const host = document.getElementById("sessions");
  if(!sessions.length){host.innerHTML = '<span class="dim">no chats yet — an agent creates one via session_create, or `wtf sessions` on the hub machine</span>';return;}
  // Multi-machine canvas (operator directive): group chats by hub origin
  // (x.origin when replicated; "this hub" otherwise) so the operator sees
  // which machine hosts which project chat at a glance. Scope labels
  // (repo-a+repo-b@mac+win) render as chips.
  const groups = {};
  for(const x of sessions){
    const o = x.origin || "this hub";
    ((groups[o] = groups[o] || []).push(x));
  }
  let rows = "";
  for(const [origin, list] of Object.entries(groups)){
    rows += '<div class="origin"><span class="ochip">'+esc(origin)+'</span><span class="dim"> · '+list.length+' chat(s)</span></div>';
    for(const x of list){
      const scopes = (x.repo||"").split(/[+@]/).filter(Boolean);
      const chips = scopes.map(sc=>'<span class="repo">'+esc(sc)+'</span>').join(" ");
      rows += '<div class="sess" data-id="'+esc(x.id)+'" title="chat '+esc(x.name)+' · scope '+esc(x.repo||'-')+'">'
        +'<div class="top"><span class="who">'+esc(x.name)+'</span>'
        +(chips?'<span>'+chips+'</span>':"")
        +'<span class="age">'+esc(x.msg_count)+' msg(s) · '+esc(x.members)+' member(s)</span></div>'
        +'<div class="sid dim">'+esc(x.id)+'</div></div>';
    }
  }
  host.innerHTML = rows;
  for(const el of host.querySelectorAll(".sess")){
    el.addEventListener("click", ()=>openSession(el.dataset.id));
  }
}
async function openSession(id){
  // Operator chat viewer (v0.15.0): decrypted bodies via the ?k=/?cap=
  // gated /view endpoint + a live terminal pane on the executor tmux
  // session. Falls back to metadata-only when this machine holds no
  // session key for the chat.
  let w = window.open("", "_blank");
  try{
    const meta = await fetch("/api/v1/sessions/"+encodeURIComponent(id)+"?"+AUTH);
    const mj = await meta.json();
    if(!meta.ok){throw new Error(mj.error||("HTTP "+meta.status));}
    const s = mj.session || mj;
    const slug = (s.name||id).toLowerCase().replace(/[^a-z0-9]+/g,"-").replace(/^-+|-+$/g,"").slice(0,24)||"task";
    const termName = "wtf-chat-"+slug;
    let viewRows = "";
    let viewNote = "";
    const vr = await fetch("/api/v1/sessions/"+encodeURIComponent(id)+"/view?"+AUTH);
    if(vr.ok){
      const vj = await vr.json();
      viewRows = (vj.msgs||[]).map(m=>'<div class="m"><span class="dim">#'+esc(m.seq)+' '+esc(m.sender)+' · '+new Date(m.ts*1000).toISOString().slice(11,19)+'Z</span><pre>'+esc(m.text)+'</pre></div>').join("");
      if(!viewRows) viewRows = '<div class="dim">no messages yet</div>';
    } else {
      const ej = await vr.json().catch(()=>({}));
      viewNote = ej.error || ("HTTP "+vr.status);
      viewRows = '<div class="dim">message bodies unavailable on this machine: '+esc(viewNote)+'</div>';
    }
    const doc = '<!doctype html><html><head><meta charset="utf-8"><title>'+esc(s.name||id)+'</title>'
      +'<style>body{background:#0b0e14;color:#d7dde8;font:14px/1.5 ui-monospace,monospace;margin:0;padding:16px}'
      +'h1{font-size:15px;letter-spacing:1px}.dim{color:#8a93a6}.m{border-bottom:1px dashed #232b3d;padding:6px 0}.m pre{white-space:pre-wrap;word-break:break-word;margin:4px 0 0}'
      +'.repo{color:#4aa3ff;font-size:12px;border:1px solid #234;border-radius:10px;padding:1px 8px}'
      +'#term{background:#05070c;border:1px solid #1d2637;border-radius:8px;height:320px;overflow:auto;padding:8px;white-space:pre-wrap;font:12px/1.45 ui-monospace,monospace;color:#9fd08a;margin:10px 0 6px}'
      +'input#cmd{width:70%;background:#0d1320;color:#d7dde8;border:1px solid #233152;border-radius:6px;padding:6px 8px;font:13px ui-monospace,monospace}'
      +'button{background:#16345c;color:#d7dde8;border:0;border-radius:6px;padding:6px 14px;cursor:pointer}'
      +'.row{display:flex;gap:8px;align-items:center}</style></head><body>'
      +'<h1>💬 '+esc(s.name||id)+'</h1>'
      +'<div class="row"><span class="dim">scope</span><input id="scope" value="'+esc(s.repo||'')+'" placeholder="repo-a+repo-b@machine1+machine2" style="width:40%;background:#0d1320;color:#4aa3ff;border:1px solid #234;border-radius:10px;padding:2px 8px;font:12px ui-monospace"/>'
      +'<button id="scopesave" style="padding:2px 10px;font-size:12px">save scope</button><span id="scopestat" class="dim"></span></div>'
      +'<div class="dim">members: '+esc((s.members||[]).map(m=>m.device).join(", ")||"-")+'</div>'
      +'<div id="feed">'+viewRows+'</div>'
      +'<h2 style="font-size:13px;margin:14px 0 4px">terminal — '+esc(termName)+' (this machine)</h2>'
      +'<div class="row"><input id="cmd" placeholder="type a command for the agent terminal, Enter sends"/><button id="send">send</button><span id="tstat" class="dim"></span></div>'
      +'<div id="term">loading pane…</div>'
      +'<script>'
      +'const TERM='+JSON.stringify(termName)+';const AUTH='+JSON.stringify(AUTH)+';'
      +'async function poll(){try{const r=await fetch("/api/v1/term/"+TERM+"?lines=400&"+AUTH);if(r.ok){const j=await r.json();document.getElementById("term").textContent=j.pane||"(empty pane)";document.getElementById("tstat").textContent="live";}else{const e=await r.json().catch(()=>({}));document.getElementById("tstat").textContent=e.error||("HTTP "+r.status);}}catch(e){document.getElementById("tstat").textContent=String(e);}setTimeout(poll,2000);}'
      +'poll();'
      +'document.getElementById("cmd").addEventListener("keydown",e=>{if(e.key==="Enter")sendCmd();});'
      +'document.getElementById("scopesave").addEventListener("click",async()=>{const v=document.getElementById("scope").value;const r=await fetch("/api/v1/sessions/'+id+'/scope?"+AUTH,{method:"POST",headers:{"Content-Type":"application/json"},body:JSON.stringify({repo:v})});const j=await r.json().catch(()=>({}));document.getElementById("scopestat").textContent=r.ok?"saved":(j.error||("HTTP "+r.status));});'
      +'<\/script></body></html>';
    w.document.write(doc);
    w.document.close();
  }catch(e){
    try{w.document.write('<body style="background:#0b0e14;color:#d7dde8;font:14px ui-monospace">chat open failed: '+esc(String(e.message||e))+' <a style="color:#4aa3ff" href="javascript:location.reload()">retry</a></body>');w.document.close();}catch(_){}
  }
}
const binIds=[1,2,3];
const dirty={};
function buildBins(){
  const host=document.getElementById("bins");
  host.innerHTML="";
  for(const id of binIds){
    const card=document.createElement("div");card.className="card bin";
    const head=document.createElement("div");head.className="bhead";
    const title=document.createElement("b");title.textContent="BIN "+id;
    const meta=document.createElement("span");meta.className="bmeta dim";meta.id="bmeta-"+id;meta.textContent="loading…";
    head.appendChild(title);head.appendChild(meta);
    const ta=document.createElement("textarea");ta.id="ta-"+id;ta.spellcheck=false;ta.placeholder="paste content for agents here — agents publish via write_bin too";
    const btns=document.createElement("div");btns.className="bbtns";
    const save=document.createElement("button");save.id="save-"+id;save.textContent="Save";save.title="Save this bin (agents see it immediately)";
    const copy=document.createElement("button");copy.id="copy-"+id;copy.textContent="Copy";copy.title="Copy this bin's content to the clipboard";
    save.addEventListener("click",()=>saveBin(id));
    copy.addEventListener("click",()=>copyBin(id));
    ta.addEventListener("input",()=>{dirty[id]=true;save.classList.add("dirty");});
    btns.appendChild(save);btns.appendChild(copy);
    card.appendChild(head);card.appendChild(ta);card.appendChild(btns);
    host.appendChild(card);
  }
}
function renderBins(bins, now){
  if(!bins)return;
  for(const b of bins){
    const ta=document.getElementById("ta-"+b.id);
    if(!ta)continue;
    if(!dirty[b.id] && ta.value!==b.content){ta.value=b.content;}
    const m=document.getElementById("bmeta-"+b.id);
    if(b.updated_by){
      const byAgent = b.updated_by !== "dashboard";
      m.innerHTML = b.size+" chars · updated "+ago(b.updated_at,now)+" ago by "
        +(byAgent?'<span class="who" title="written by an agent via write_bin">✎ '+esc(b.updated_by)+'</span>':esc(b.updated_by));
    }else{
      m.textContent = b.size+" chars · empty";
    }
  }
}
async function saveBin(id){
  const ta=document.getElementById("ta-"+id);
  const btn=document.getElementById("save-"+id);
  btn.disabled=true;btn.textContent="Saving…";
  try{
    const r=await fetch("/api/v1/bins/"+id+"?"+AUTH,{method:"PUT",headers:{"Content-Type":"application/json"},body:JSON.stringify({content:ta.value})});
    const j=await r.json();
    if(!r.ok||!j.ok)throw new Error(j.error||("HTTP "+r.status));
    dirty[id]=false;btn.classList.remove("dirty");btn.textContent="Saved";
    setTimeout(()=>{btn.textContent="Save";},1500);
  }catch(e){
    document.getElementById("bmeta-"+id).textContent="save failed: "+esc(String(e.message||e));
    btn.textContent="Save";
  }
  btn.disabled=false;
}
function copyBin(id){
  const ta=document.getElementById("ta-"+id);
  const btn=document.getElementById("copy-"+id);
  const done=()=>{btn.textContent="Copied";setTimeout(()=>{btn.textContent="Copy";},1200);};
  const fallback=()=>{ta.select();document.execCommand("copy");done();};
  if(navigator.clipboard&&navigator.clipboard.writeText){navigator.clipboard.writeText(ta.value).then(done).catch(fallback);}else{fallback();}
}
function boot(){
  buildBins();
  fetch("/api/v1/state?"+AUTH)
    .then(r=>{if(!r.ok)throw 0;return r.json();})
    .then(render)
    .catch(()=>{document.getElementById("meta").textContent="state fetch failed — open the link printed by wtf dashboard-url";});
  const es = new EventSource("/stream?"+AUTH);
  const dot = document.getElementById("conn");
  es.addEventListener("state", ev=>{try{render(JSON.parse(ev.data));}catch(e){}});
  es.onopen = ()=>{dot.classList.add("on");};
  es.onerror = ()=>{dot.classList.remove("on");};
}
boot();
</script>
</body>
</html>
"#;
