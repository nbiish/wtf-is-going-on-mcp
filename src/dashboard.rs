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
</style>
</head>
<body>
<header><h1>WTF IS GOING ON</h1><span id="meta" class="dim">connecting…</span><span id="conn" class="dot" title="live stream"></span></header>
<main>
<section><h2>AGENTS</h2><div id="agents" class="card"><span class="dim">no agents have checked in yet</span></div></section>
<section><h2>EVENT LOG</h2><div class="card"><ul id="feed"></ul></div></section>
</main>
<script>
"use strict";
const K = new URLSearchParams(location.search).get("k") || "";
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
    ag.innerHTML = s.agents.map(a=>{
      const cls = a.stale ? "stale" : esc(a.status);
      const label = esc(a.status)+(a.stale?" · stale":"");
      return '<div class="agent"><div class="top"><span class="who">'+esc(a.agent)+'@'+esc(a.device)+'</span>'
        +'<span class="pill '+cls+'">'+label+'</span>'
        +'<span class="age">'+ago(a.last_seen, now)+' ago</span></div>'
        +(a.task?'<div class="task">'+esc(a.task)+'</div>':"")
        +(a.details?'<div class="details">'+esc(a.details)+'</div>':"")
        +'</div>';
    }).join("");
  }
  const feed = document.getElementById("feed");
  const rows = s.events.slice().reverse().map(e=>
    '<li><span class="dim">#'+e.id+' '+hms(e.ts)+'</span> <span class="lv '+esc(e.level)+'">['+esc(e.level)+']</span> <b>'+esc(e.agent)+'@'+esc(e.device)+'</b> '+esc(e.message)+'</li>'
  ).join("");
  feed.innerHTML = rows || '<li class="dim">no events yet</li>';
}
function boot(){
  fetch("/api/v1/state?k="+encodeURIComponent(K))
    .then(r=>{if(!r.ok)throw 0;return r.json();})
    .then(render)
    .catch(()=>{document.getElementById("meta").textContent="state fetch failed — check the ?k= key";});
  const es = new EventSource("/stream?k="+encodeURIComponent(K));
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
