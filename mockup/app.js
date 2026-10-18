// Slop - desktop store mockup. Plain JS, hash routing, fake data from data.js.
// Nothing here talks to a server: installs, donations, scans, pings and reports are simulated.

/* ======================================================================
   State
   ====================================================================== */
const STORE_KEY = "slop.state.v3";
const FEE = 0.05;
const APP_BY_ID = Object.fromEntries(APPS.map(a => [a.id, a]));
const TAG_GROUP_OF = {};
TAG_GROUPS.forEach(g => g.tags.forEach(t => (TAG_GROUP_OF[t] = g.name)));
const MONTH = new Date().toLocaleDateString(undefined, { month: "long" });

function detectOS() {
  const ua = navigator.userAgent;
  if (/iPhone|iPad/.test(ua)) return "ios";
  if (/Android/.test(ua)) return "android";
  if (/Mac/i.test(ua) && !/iPhone|iPad/.test(ua)) return "mac";
  if (/Linux|X11|CrOS/i.test(ua) && !/Android/.test(ua)) return "linux";
  return "windows";
}

const defaults = () => ({
  os: detectOS(),
  donated: 0,
  library: {
    "pixel-dungeon-delver": { branch: "main", version: "1.6.2", hours: 41, installed: true },
    "cat-cafe-tycoon": { branch: "more-cats", version: "2.4.0", hours: 18, installed: true },
    "bytebench": { branch: "main", version: "5.2.1", hours: 300, installed: true },
    "neon-drift-2077": { branch: "physics-fix", version: "1.0.3", hours: 4, installed: true },
    "fractal-forge": { branch: "main", version: "4.0.0", hours: 0, installed: false },
  },
  interested: { "moonlit-orchard": true },
  user: null,
  devices: [{ name: "Pixel 8", os: "android" }],
  teams: { whiskerworks: true, "cozy-crew": true, "weekend-jam-club": true },
  myTeams: [],
  privates: JSON.parse(JSON.stringify(PRIVATE_SEED)),
  hires: { "inkwell-and-ivy": "done", loopsmith: "done" }, // jobs you've completed - only these can be rated
  humanReviews: {},
  stageChanges: {},
  myReviews: {},
  reports: [],
  submissions: [],
});
let S;
try { S = { ...defaults(), ...JSON.parse(localStorage.getItem(STORE_KEY) || "{}") }; } catch { S = defaults(); }
function save() { try { localStorage.setItem(STORE_KEY, JSON.stringify(S)); } catch { /* private mode */ } }
// Re-apply stage changes the user made as a publisher (e.g. Coming soon -> Beta).
Object.entries(S.stageChanges).forEach(([id, ch]) => APP_BY_ID[id] && Object.assign(APP_BY_ID[id], ch));

/* ======================================================================
   Helpers
   ====================================================================== */
const $ = (sel, root = document) => root.querySelector(sel);
const $$ = (sel, root = document) => [...root.querySelectorAll(sel)];
const esc = s => String(s ?? "").replace(/[&<>"']/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
const money = n => "$" + (Math.round(n * 100) / 100).toLocaleString(undefined, { minimumFractionDigits: n % 1 ? 2 : 0, maximumFractionDigits: 2 });
const num = n => n >= 1000 ? (n / 1000).toFixed(n >= 10000 ? 0 : 1).replace(/\.0$/, "") + "k" : String(n);
const date = d => new Date(d + "T12:00:00").toLocaleDateString(undefined, { month: "short", day: "numeric", year: "numeric" });
const daysSince = d => Math.floor((Date.now() - new Date(d + "T12:00:00")) / DAY);
const addDays = (d, n) => new Date(new Date(d + "T12:00:00").getTime() + n * DAY).toISOString().slice(0, 10);
const enc = encodeURIComponent;
const icon = id => `<svg><use href="#i-${id}"/></svg>`;
const isGame = app => app.type === "Games";
// Slop Pulse applies to public games only - Private Slop skips it.
const checksFor = app => SCAN_CHECKS.filter(c => !c.games || (isGame(app) && !app.isPrivate));
const isPhoneOS = os => PLATFORMS[os].group === "phone";
const allTeams = () => [...TEAMS, ...S.myTeams];
const TEAM_BY_ID = id => allTeams().find(t => t.id === id);
const scanResults = app => app.scan.results.filter(r => checksFor(app).some(c => c.id === r.id));

// Ratings are two buttons - 👍 Not Slop / 👎 Slop. `slop` in the data is the % of Slop votes;
// everything user-facing shows the positive "% Not Slop" so it's obvious which way is good.
const SLOP_TIERS = [
  { min: 95, label: "Overwhelmingly Not Slop", hint: "Almost everyone who launched it says it's the real deal." },
  { min: 80, label: "Very Not Slop", hint: "Most people who launched it say it's genuinely good." },
  { min: 70, label: "Mostly Not Slop", hint: "Good, with a few AI fingerprints." },
  { min: 40, label: "Mixed", hint: "People are split." },
  { min: 20, label: "Mostly Slop", hint: "Most people who launched it call it slop." },
  { min: 0, label: "Overwhelmingly Slop", hint: "Uncut, unedited AI output." },
];
const tier = s => SLOP_TIERS.find(t => 100 - s >= t.min);
const isSlopVote = r => (r.vote ? r.vote === "slop" : r.slop >= 50);
const voteChip = r => isSlopVote(r) ? `<span class="vote-chip slop">👎 Slop</span>` : `<span class="vote-chip not">👍 Not Slop</span>`;
// Launching through Slop is what earns a vote - same launch that counts you in Slop Pulse.
const canVote = id => !!(S.library[id] && (S.library[id].launched || S.library[id].hours > 0));
const STOPS = [[0, [74, 222, 128]], [25, [163, 230, 53]], [50, [250, 204, 21]], [72, [249, 115, 22]], [100, [124, 74, 30]]];
function slopColor(s) {
  for (let i = 1; i < STOPS.length; i++) {
    const [b, cb] = STOPS[i], [a, ca] = STOPS[i - 1];
    if (s <= b) { const t = (s - a) / (b - a); return `rgb(${ca.map((v, k) => Math.round(v + (cb[k] - v) * t)).join(",")})`; }
  }
  return "rgb(124,74,30)";
}
function slopHtml(s, size = "", withLabel = true, sub = "") {
  if (s == null) return `<span class="slop ${size}" title="Voting opens at full release"><span class="num" style="background:var(--panel-3);color:var(--muted)">–</span>${withLabel ? `<span class="lbl muted">No votes yet${sub ? `<small>${sub}</small>` : ""}</span>` : ""}</span>`;
  return `<span class="slop ${size}" title="${100 - s}% of people who launched it voted Not Slop">
    <span class="num" style="background:${slopColor(s)}">${size === "sm" ? "" : "👍 "}${100 - s}%</span>
    ${withLabel ? `<span class="lbl" style="color:${slopColor(s)}">${tier(s).label}${sub ? `<small>${sub}</small>` : ""}</span>` : ""}</span>`;
}
function artHtml(app, cls = "", withName = true) {
  return `<div class="art ${cls}" style="background:linear-gradient(135deg, ${app.art.from}, ${app.art.to})">
    <span class="em">${app.art.emoji}</span>${withName ? `<span class="nm">${esc(app.name)}</span>` : ""}</div>`;
}

/* ---- release stages -------------------------------------------------- */
const STAGE_LABEL = { soon: "Coming soon", beta: "Beta", released: "Released" };
function lifecycle(app) {
  if (app.stage === "released") return { stage: "released" };
  const limit = app.stage === "soon" ? STAGE_RULES.soonDays : STAGE_RULES.betaDays;
  const deadline = addDays(app.stageSince, limit);
  const daysLeft = Math.ceil((new Date(deadline + "T12:00:00") - Date.now()) / DAY);
  const quiet = daysSince(app.lastCommit);
  return { stage: app.stage, deadline, daysLeft, limit, used: limit - daysLeft, quiet, dead: quiet >= STAGE_RULES.deadAfterDays, overdue: daysLeft < 0 };
}
const isUpcoming = app => app.stage !== "released";
function stageBadge(app) {
  const l = lifecycle(app);
  if (l.dead) return `<span class="badge bad" title="No code changes in ${l.quiet} days">DEAD PROJECT</span>`;
  if (l.stage === "soon") return `<span class="badge soon">COMING SOON</span>`;
  if (l.stage === "beta") return `<span class="badge beta">BETA</span>`;
  return "";
}
function deadlineText(app) {
  const l = lifecycle(app);
  if (l.dead) return `No commits in ${l.quiet} days`;
  if (l.overdue) return `Missed its ${l.stage === "soon" ? "beta" : "release"} deadline`;
  if (l.stage === "soon") return `Beta due in ${l.daysLeft} day${l.daysLeft === 1 ? "" : "s"}`;
  return `Full release due by ${date(l.deadline)}`;
}
const isInterested = id => !!(S.user && S.interested[id]);
const interestCount = app => (app.interested || 0) + (isInterested(app.id) ? 1 : 0);
function playingHtml(app, long = false) {
  if (!isGame(app) || app.playing == null) return "";
  return `<span class="playing" title="Counted by Slop Pulse">${num(app.playing)} playing${long ? ` now · ${num(app.peak)} peak this month` : ""}</span>`;
}

// What can this OS install? Official build first, otherwise community branches.
function availability(app, os) {
  const official = app.builds[os] ? { branch: app.tracked, author: app.dev, build: app.builds[os], official: true } : null;
  const community = app.branches.filter(b => b.name !== app.tracked && b.builds[os]).map(b => ({ branch: b.name, author: b.author, build: b.builds[os], official: false, data: b }));
  return { official, community };
}
const runsOn = (app, os) => { const a = availability(app, os); return !!(a.official || a.community.length); };
function platsHtml(app) {
  return `<span class="plats">${Object.keys(PLATFORMS).map(os => {
    if (app.builds[os]) return `<span title="${PLATFORMS[os].label} - official build">${icon(os)}</span>`;
    if (availability(app, os).community.length) return `<span class="comm" title="${PLATFORMS[os].label} - community build">${icon(os)}</span>`;
    return "";
  }).join("")}</span>`;
}
function tileHtml(app, rank) {
  if (isUpcoming(app)) return upTileHtml(app);
  return `<a class="tile" href="#/app/${app.id}">
    <div style="position:relative">${artHtml(app)}${rank ? `<span class="rank-badge">#${rank}</span>` : ""}</div>
    <div class="tile-body">
      <span class="d">${esc(app.short)}</span>
      ${rank ? `<div class="tile-stat">+${num(app.monthAdds)} added in ${MONTH}${app.playing != null ? ` · ${playingHtml(app)}` : ""}</div>` : `<div class="chips">${app.tags.slice(0, 3).map(t => `<span class="chip">${esc(t)}</span>`).join("")}</div>`}
      <div class="tile-foot">${slopHtml(app.slop, "sm")}${platsHtml(app)}</div>
    </div></a>`;
}
function upTileHtml(app, reason = "") {
  const on = isInterested(app.id);
  return `<a class="tile" href="#/app/${app.id}">
    <div style="position:relative">${artHtml(app)}<span class="stage-corner">${stageBadge(app)}</span></div>
    <div class="tile-body">
      <span class="d">${esc(app.short)}</span>
      ${reason ? `<div class="reason">${reason}</div>` : ""}
      <div class="tile-stat">${deadlineText(app)}</div>
      <div class="tile-foot"><span class="muted" style="font-size:12px">★ ${num(interestCount(app))} interested</span>
        <button class="btn small ${on ? "" : "slime"}" data-interest="${app.id}">${on ? "✓ Interested" : "★ Interested"}</button></div>
    </div></a>`;
}
const repoUrl = app => `github.com/${app.dev}/${app.id}`;
const branchRepo = (app, b) => `github.com/${b.author}/${app.id}/tree/${b.name}`;

// Minimal markdown for branch READMEs (input is escaped first).
function md(src) {
  const inline = s => esc(s).replace(/`([^`]+)`/g, "<code>$1</code>").replace(/\*\*([^*]+)\*\*/g, "<b>$1</b>");
  let out = "", inList = false;
  for (const line of src.split("\n")) {
    const close = () => { if (inList) { out += "</ul>"; inList = false; } };
    let m;
    if ((m = line.match(/^(#{1,3}) (.*)/))) { close(); out += `<h${m[1].length}>${inline(m[2])}</h${m[1].length}>`; }
    else if ((m = line.match(/^- (.*)/))) { if (!inList) { out += "<ul>"; inList = true; } out += `<li>${inline(m[1])}</li>`; }
    else if (line.trim()) { close(); out += `<p>${inline(line)}</p>`; }
    else close();
  }
  return out + (inList ? "</ul>" : "");
}

/* ---- toast + modal --------------------------------------------------- */
function toast(html, ms = 3500) {
  const t = document.createElement("div");
  t.className = "toast";
  t.innerHTML = html;
  $("#toasts").append(t);
  setTimeout(() => t.remove(), ms);
  return t;
}
function progressToast(label, done) {
  const t = toast(`${label}<div class="progress"><i style="width:0"></i></div>`, 999999);
  const bar = $("i", t);
  let p = 0;
  const iv = setInterval(() => {
    p = Math.min(100, p + 7 + Math.random() * 14);
    bar.style.width = p + "%";
    if (p >= 100) { clearInterval(iv); setTimeout(() => { t.remove(); done(); }, 250); }
  }, 140);
}
function modal(html, { wide = false, onMount } = {}) {
  const bg = document.createElement("div");
  bg.className = "modal-bg";
  bg.innerHTML = `<div class="modal ${wide ? "wide" : ""}" role="dialog">${html}</div>`;
  const close = () => { bg.remove(); document.removeEventListener("keydown", onKey); };
  const onKey = e => e.key === "Escape" && close();
  bg.addEventListener("click", e => { if (e.target === bg || e.target.closest("[data-close]")) close(); });
  document.addEventListener("keydown", onKey);
  $("#window").append(bg); // inside the window so it stays in the phone frame
  onMount?.($(".modal", bg), close);
  return close;
}
function githubToast(url) { toast(`🔗 Would open <b>${esc(url)}</b> in your browser.<br><span class="muted">Mockup - the repos aren't real.</span>`); }

/* ======================================================================
   Router
   ====================================================================== */
let heroTimer = null;
const routes = [
  [/^\/?$/, store],
  [/^\/app\/([^/]+)\/branch\/(.+)$/, (id, b) => branchPage(id, decodeURIComponent(b))],
  [/^\/app\/([^/]+)(?:\/(\w+))?$/, appPage],
  [/^\/upcoming(?:\/(\w+))?$/, upcoming],
  [/^\/browse\/?$/, () => browse()],
  [/^\/browse\/(.+)$/, g => browse(decodeURIComponent(g))],
  [/^\/tag\/(.+)$/, t => tagPage(decodeURIComponent(t))],
  [/^\/search\/(.*)$/, q => searchPage(decodeURIComponent(q))],
  [/^\/library(?:\/([^/]+))?$/, library],
  [/^\/private\/([^/]+)$/, privatePage],
  [/^\/teams(?:\/(\w+))?$/, teamsPage],
  [/^\/team\/([^/]+)$/, teamPage],
  [/^\/humans(?:\/(.+))?$/, h => humansPage(h ? decodeURIComponent(h) : "")],
  [/^\/human\/([^/]+)$/, humanPage],
  [/^\/publish$/, publish],
  [/^\/rules$/, rules],
];
function render() {
  clearInterval(heroTimer);
  const path = location.hash.slice(1) || "/";
  for (const [re, fn] of routes) {
    const m = path.match(re);
    if (m) {
      const tab = /^\/(library|private)/.test(path) ? "library" : path.startsWith("/team") ? "teams" : path.startsWith("/human") ? "humans" : path.startsWith("/publish") ? "publish" : path.startsWith("/rules") ? "rules" : "store";
      $$("#tabs a, #bottom-tabs a").forEach(a => a.classList.toggle("on", a.dataset.tab === tab));
      const view = $("#view"), keep = view.dataset.path === path ? view.scrollTop : 0;
      view.dataset.path = path;
      fn(...m.slice(1));
      view.scrollTop = keep;
      return;
    }
  }
  $("#view").innerHTML = `<div class="page"><div class="empty-state">Page not found. <a href="#/">Back to the store</a></div></div>`;
}
function mount(html) { $("#view").innerHTML = html; return $("#view"); }

/* ======================================================================
   Store front
   ====================================================================== */
const appsWithTag = t => APPS.filter(a => a.tags.includes(t));
const appsInGroup = g => APPS.filter(a => a.type === g.name || a.tags.some(t => g.tags.includes(t)));
const released = () => APPS.filter(a => !isUpcoming(a));
const tagLink = t => { const n = appsWithTag(t).length; return `<a href="#/tag/${enc(t)}" ${n ? "" : 'class="zero" title="Nothing here yet - publish the first one!"'}>${t}<span>${n}</span></a>`; };

function sidebarHtml(active = "") {
  const up = APPS.filter(isUpcoming);
  return `<aside class="sidebar">
    <h4>Discover</h4>
    <a href="#/upcoming">🔭 Coming soon &amp; beta<span class="n">${up.filter(a => !lifecycle(a).dead).length}</span></a>
    <a href="#/search/">✨ Least slop first</a>
    <h4>Browse</h4>
    <a href="#/browse">🧭 All categories</a>
    ${TAG_GROUPS.map(g => `<a href="#/browse/${enc(g.name)}" ${active === g.name ? 'style="color:var(--slime)"' : ""}>${g.icon} ${g.name}<span class="n">${appsInGroup(g).length}</span></a>`).join("")}
    <h4>Popular tags</h4>
    ${["Roguelike", "Cozy", "Productivity", "Offline", "Music", "Retro", "Kids"].map(t => `<a href="#/tag/${enc(t)}"># ${t}<span class="n">${appsWithTag(t).length}</span></a>`).join("")}
    <h4>Creators</h4>
    <a href="#/publish">⬆️ Publish your app</a>
    <a href="#/rules">📜 The Slop rules</a>
  </aside>`;
}

// "For you": score upcoming apps by tag overlap with your library (x1) and your Interested list (x2),
// then boost by how many people are interested overall. Dead projects never get recommended.
function recommendUpcoming() {
  const weights = {};
  const bump = (app, w) => app && app.tags.forEach(t => (weights[t] = (weights[t] || 0) + w));
  Object.keys(S.library).forEach(id => bump(APP_BY_ID[id], 1));
  Object.keys(S.interested).forEach(id => bump(APP_BY_ID[id], 2));
  return APPS.filter(a => isUpcoming(a) && !lifecycle(a).dead).map(a => {
    const hits = a.tags.filter(t => weights[t]).sort((x, y) => weights[y] - weights[x]);
    const match = hits.reduce((s, t) => s + weights[t], 0);
    return { app: a, match, hits, score: match * (1 + Math.log10(1 + interestCount(a))) };
  }).filter(r => r.match > 0).sort((a, b) => b.score - a.score);
}
const mostAnticipated = () => APPS.filter(a => isUpcoming(a) && !lifecycle(a).dead).sort((a, b) => interestCount(b) - interestCount(a));

function store() {
  const featured = ["fractal-forge", "cat-cafe-tycoon", "deckbound", "bytebench", "tidy-files"].map(id => APP_BY_ID[id]);
  const rel = released();
  const byClean = [...rel].sort((a, b) => a.slop - b.slop);
  const byNew = [...rel].sort((a, b) => b.updated.localeCompare(a.updated));
  const popular = [...rel].sort((a, b) => b.monthAdds - a.monthAdds).slice(0, 4);
  const hotBranches = rel.flatMap(a => a.branches.filter(b => b.name !== a.tracked).map(b => ({ app: a, b }))).sort((x, y) => y.b.stars - x.b.stars).slice(0, 6);
  const v = mount(`<div class="page"><div class="store">
    ${sidebarHtml()}
    <div>
      <div id="hero"></div>
      <div class="hero-dots" id="hero-dots">${featured.map((_, i) => `<button data-i="${i}" aria-label="Featured ${i + 1}"></button>`).join("")}</div>

      <div class="section-h"><h2>Popular in ${MONTH}</h2><span class="sub">Most added to libraries this month</span></div>
      <div class="shelf">${popular.map((a, i) => tileHtml(a, i + 1)).join("")}</div>

      <div class="section-h"><h2>Coming soon &amp; in beta</h2>
        <div class="seg" id="up-seg"><button data-v="foryou" class="on">For you</button><button data-v="hype">Most anticipated</button></div></div>
      <div class="muted up-explain" id="up-explain"></div>
      <div class="shelf" id="up-shelf"></div>
      <div style="text-align:right;margin-top:8px"><a href="#/upcoming" style="font-size:13px">See everything upcoming →</a></div>

      <div class="section-h"><h2>Browse by category</h2><a href="#/browse" class="sub">All tags →</a></div>
      <div class="cat-grid">${TAG_GROUPS.map(g => `<a class="cat" href="#/browse/${enc(g.name)}"><span class="ic">${g.icon}</span><b>${g.name}</b><span>${g.desc} · ${appsInGroup(g).length}</span></a>`).join("")}</div>

      <div class="section-h"><h2>Not Slop right now</h2><span class="sub">Highest 👍 Not Slop share</span></div>
      <div class="shelf">${byClean.slice(0, 4).map(a => tileHtml(a)).join("")}</div>

      <div class="section-h"><h2>Community branches worth installing</h2><span class="sub">Mods, ports and fixes - built by players, scanned like everything else</span></div>
      <div class="branch-strip">${hotBranches.map(({ app, b }) => `
        <a class="panel branch-card" href="#/app/${app.id}/branch/${enc(b.name)}">
          <div style="display:flex;justify-content:space-between;align-items:center;gap:8px"><span class="bn">${esc(b.name)}</span><span class="badge community">COMMUNITY</span></div>
          <div class="muted" style="font-size:12px">for <b style="color:var(--text)">${esc(app.name)}</b> · by @${esc(b.author)}</div>
          <div style="font-size:13px;color:#c4ced9">${esc(b.desc)}</div>
          <div style="display:flex;justify-content:space-between;font-size:12px" class="muted"><span>★ ${num(b.stars)}</span>${platsHtml({ builds: b.builds, branches: [] })}</div>
        </a>`).join("")}</div>

      <div class="section-h"><h2>New &amp; updated</h2></div>
      <div class="shelf">${byNew.slice(0, 4).map(a => tileHtml(a)).join("")}</div>

      <div class="section-h"><h2>Fresh slop 🫠</h2><span class="sub">Voted 👎 Slop by most players - viewer discretion advised</span></div>
      <div class="shelf">${[...byClean].reverse().slice(0, 4).map(a => tileHtml(a)).join("")}</div>
    </div>
  </div></div>`);

  // upcoming shelf with two modes
  let mode = "foryou";
  const drawUp = () => {
    $$("#up-seg button", v).forEach(b => b.classList.toggle("on", b.dataset.v === mode));
    if (mode === "foryou") {
      const recs = recommendUpcoming().slice(0, 4);
      $("#up-explain", v).textContent = "Matched to tags in your library and things you've marked Interested, boosted by how many people are waiting for them.";
      $("#up-shelf", v).innerHTML = recs.length ? recs.map(r => upTileHtml(r.app, `Because you like ${r.hits.slice(0, 2).map(esc).join(" &amp; ")}`)).join("")
        : `<div class="empty-state" style="grid-column:1/-1">Add a few things to your library and we'll find upcoming apps you'll like.</div>`;
    } else {
      $("#up-explain", v).textContent = "Ranked by how many people have marked them Interested.";
      $("#up-shelf", v).innerHTML = mostAnticipated().slice(0, 4).map(a => upTileHtml(a)).join("");
    }
  };
  $("#up-seg", v).onclick = e => { const b = e.target.closest("button"); if (b) { mode = b.dataset.v; drawUp(); } };
  drawUp();
  v.addEventListener("slop:interest", drawUp);

  let i = 0;
  const show = n => {
    i = (n + featured.length) % featured.length;
    const a = featured[i];
    $("#hero", v).innerHTML = `<div class="hero">
      <a href="#/app/${a.id}" style="display:contents">${artHtml(a)}</a>
      <div class="hero-info">
        <div style="display:flex;gap:6px"><span class="badge official">FEATURED</span>${stageBadge(a)}</div>
        <h3>${esc(a.name)}</h3>
        <p>${esc(a.short)}</p>
        <div class="chips">${a.tags.slice(0, 4).map(t => `<a class="chip" href="#/tag/${enc(t)}">${esc(t)}</a>`).join("")}</div>
        <div class="row">${slopHtml(a.slop, "", true, isUpcoming(a) ? "in beta" : `${num(a.reviews)} reviews`)}${playingHtml(a)}</div>
        <div class="spacer"></div>
        <div class="row"><span class="muted">Free · donations optional</span><div style="flex:1"></div>${platsHtml(a)}</div>
        <div class="row">${primaryActionHtml(a)}<button class="btn" data-donate="${a.id}">♥ Donate</button></div>
      </div></div>`;
    $$("#hero-dots button", v).forEach((b, k) => b.classList.toggle("on", k === i));
  };
  show(0);
  $("#hero-dots", v).onclick = e => { const b = e.target.closest("button"); if (b) { show(+b.dataset.i); clearInterval(heroTimer); } };
  heroTimer = setInterval(() => show(i + 1), 6000);
}

/* ---- upcoming page --------------------------------------------------- */
function upcoming(filter = "all") {
  const all = APPS.filter(isUpcoming);
  const groups = { all: all.filter(a => !lifecycle(a).dead), soon: all.filter(a => a.stage === "soon" && !lifecycle(a).dead), beta: all.filter(a => a.stage === "beta" && !lifecycle(a).dead), dead: all.filter(a => lifecycle(a).dead) };
  const list = [...groups[filter] || groups.all].sort((a, b) => interestCount(b) - interestCount(a));
  mount(`<div class="page"><div class="store">${sidebarHtml()}<div>
    <div class="crumbs"><a href="#/">Store</a> › Upcoming</div>
    <h1 style="font-size:28px">Coming soon &amp; beta</h1>
    <p class="muted" style="margin:4px 0 16px">Mark things <b style="color:var(--text)">Interested</b> to add them to your library before they ship. Publishers see how many people are waiting.</p>
    ${timelineHtml()}
    <div class="seg" style="margin:18px 0">
      ${[["all", "Everything"], ["soon", "Coming soon"], ["beta", "Beta"], ["dead", "Dead projects"]].map(([k, l]) => `<a href="#/upcoming/${k}" class="${filter === k ? "on" : ""}">${l} <span class="faint">${groups[k].length}</span></a>`).join("")}
    </div>
    ${filter === "dead" ? `<div class="caveat" style="margin:0 0 16px">These projects haven't had a code change in ${STAGE_RULES.deadAfterDays}+ days. They stay visible so nobody is misled, and they come back to life with the next commit.</div>` : ""}
    <div class="shelf">${list.length ? list.map(a => upTileHtml(a)).join("") : `<div class="empty-state" style="grid-column:1/-1">Nothing here.</div>`}</div>
  </div></div></div>`);
}
function timelineHtml(app) {
  const l = app ? lifecycle(app) : null;
  const step = (key, title, sub) => {
    const order = ["soon", "beta", "released"];
    const cur = l ? order.indexOf(l.stage) : -1, me = order.indexOf(key);
    return `<div class="tl-step ${l ? (me < cur ? "done" : me === cur ? "on" : "") : ""}"><i></i><b>${title}</b><span>${sub}</span></div>`;
  };
  return `<div class="timeline">
    ${step("soon", "Coming soon", `Source + scans, no build needed · max ${STAGE_RULES.soonDays} days`)}
    ${step("beta", "Beta", `At least one build · max ${STAGE_RULES.betaDays / 365 * 12} months`)}
    ${step("released", "Released", "Reviews and slop score open")}
  </div>`;
}

/* ======================================================================
   App page
   ====================================================================== */
function primaryActionHtml(app, big = false) {
  const cls = big ? "big" : "";
  if (app.stage === "soon") {
    const on = isInterested(app.id);
    return `<button class="btn ${cls} ${on ? "" : "slime"}" data-interest="${app.id}">${on ? "✓ Interested · in your library" : "★ I'm interested"}</button>`;
  }
  const owned = S.user && S.library[app.id];
  if (owned) return `<a class="btn ${cls} slime" href="#/library/${app.id}">✓ In library${owned.installed ? "" : " · not installed"}</a>`;
  return `<button class="btn ${cls} slime" data-add="${app.id}">+ Add to library${app.stage === "beta" ? " (beta)" : ""}</button>`;
}

function appPage(id, tab = "overview") {
  const app = APP_BY_ID[id];
  if (!app) return mount(`<div class="page"><div class="empty-state">App not found.</div></div>`);
  const up = isUpcoming(app), l = lifecycle(app);
  const mine = S.myReviews[id];
  const reviews = mine ? [{ ...mine, mine: true }, ...app.reviewList] : app.reviewList;
  if (up && tab === "reviews") tab = "overview";
  const passed = scanResults(app).filter(r => r.status !== "fail").length;
  const v = mount(`<div class="page">
    <div class="crumbs"><a href="#/">Store</a> › ${up ? `<a href="#/upcoming">Upcoming</a>` : `<a href="#/browse/${enc(app.type)}">${esc(app.type)}</a>`} › ${esc(app.name)}</div>
    <div class="app-head">
      ${artHtml(app, "", false)}
      <div class="app-side">
        <div>
          <div style="display:flex;gap:8px;align-items:center;flex-wrap:wrap"><h1>${esc(app.name)}</h1>${stageBadge(app)}</div>
          <div class="muted">by <b style="color:var(--text)">@${esc(app.dev)}</b> · ${esc(app.type)}</div>
        </div>
        ${up ? `<div class="panel panel-pad" style="display:flex;gap:16px;align-items:center;flex-wrap:wrap">
            <div><div class="big-stat">★ ${num(interestCount(app))}</div><div class="muted" style="font-size:12px">interested</div></div>
            ${app.playing != null ? `<div><div class="big-stat">${num(app.playing)}</div><div class="muted" style="font-size:12px">playing the beta now</div></div>` : ""}
            <div style="flex:1;min-width:160px;font-size:13px">${l.dead ? `<b style="color:var(--bad)">Dead project</b><div class="muted">No code changes in ${l.quiet} days</div>` : `<b>${deadlineText(app)}</b><div class="muted">Last commit ${l.quiet === 0 ? "today" : `${l.quiet} day${l.quiet === 1 ? "" : "s"} ago`}</div>`}</div>
          </div>`
        : `<div>
          ${slopHtml(app.slop, "lg", true, tier(app.slop).hint)}
          ${voteBarHtml(app, mine)}
        </div>`}
        ${isGame(app) && app.playing != null && !up ? `<div>${playingHtml(app, true)}</div>` : ""}
        <p style="margin:0;color:#c4ced9">${esc(app.short)}</p>
        <div class="chips">${app.tags.map(t => `<a class="chip" href="#/tag/${enc(t)}">${esc(t)}</a>`).join("")}</div>
        <div style="display:flex;gap:8px;flex-wrap:wrap;align-items:center">${primaryActionHtml(app)}${app.stage === "beta" ? primaryInterest(app) : ""}<button class="btn" data-donate="${app.id}">♥ Donate ${money(app.donation)}</button></div>
        <dl class="kv">
          <dt>Source</dt><dd><a href="#" data-gh="${repoUrl(app)}">${repoUrl(app)}</a></dd>
          <dt>License</dt><dd>${esc(app.license)} <span class="badge official">OPEN SOURCE</span></dd>
          <dt>Version</dt><dd>${esc(app.version)} · last commit ${date(app.lastCommit)}</dd>
          <dt>Tracked branch</dt><dd><code>${esc(app.tracked)}</code></dd>
          <dt>Made with</dt><dd>${app.ai.map(esc).join(", ")}</dd>
        </dl>
      </div>
    </div>

    <nav class="subtabs">
      ${[["overview", "Overview"], ["branches", `Branches &amp; mods<span class="n">${app.branches.length}</span>`], ["security", "Security report"], ...(up ? [] : [["reviews", `Reviews<span class="n">${num(app.reviews)}</span>`]])]
        .map(([k, lb]) => `<a href="#/app/${id}/${k}" class="${tab === k ? "on" : ""}">${lb}</a>`).join("")}
    </nav>

    <div class="two-col">
      <div id="tab"></div>
      <div style="display:flex;flex-direction:column;gap:14px">
        ${installBoxHtml(app)}
        ${donateBoxHtml(app)}
        <div class="panel panel-pad trust">
          <div class="trust-row"><span class="ic">{ }</span><span><b>Open source</b> · ${esc(app.license)} · <a href="#" data-gh="${repoUrl(app)}">view code</a></span></div>
          <div class="trust-row"><span class="ic">✓</span><span><b>${passed}/${checksFor(app).length} security checks passed</b> · <a href="#/app/${id}/security">report</a></span></div>
          ${Object.keys(app.builds).length ? `<div class="trust-row"><span class="ic">⇄</span><span><b>Builds match source</b> · verified ${date(app.scan.date)}</span></div>` : ""}
          ${isGame(app) ? `<div class="trust-row"><span class="ic">📶</span><span><b>Slop Pulse</b> · player count only, nothing else</span></div>` : ""}
          <div class="trust-row"><span class="ic">🚫</span><span><b>No loot boxes, no pay-to-win</b></span></div>
          <div class="trust-row"><span class="ic">🛍</span><span><b>Nothing for sale</b> · donations unlock nothing</span></div>
        </div>
        <button class="btn ghost danger" data-report="${id}">⚑ Report this app</button>
      </div>
    </div>
  </div>`);

  const t = $("#tab", v);
  if (tab === "branches") t.innerHTML = branchesTab(app);
  else if (tab === "security") t.innerHTML = securityTab(app);
  else if (tab === "reviews") t.innerHTML = reviewsTab(app, reviews);
  else t.innerHTML = overviewTab(app);
}
function primaryInterest(app) {
  const on = isInterested(app.id);
  return `<button class="btn" data-interest="${app.id}">${on ? "✓ Interested" : "★ Interested"}</button>`;
}

function installBoxHtml(app) {
  const os = S.os, av = availability(app, os), owned = S.user && S.library[app.id], label = PLATFORMS[os].label;
  let body;
  if (app.stage === "soon") {
    body = `<div class="no-official">No builds yet - this is a <b>Coming soon</b> page. The source is public and passes every scan.</div>
      ${primaryActionHtml(app, true)}
      <div class="meta">Interested adds it to your library now. You'll get it as soon as a beta build lands.</div>`;
  } else if (owned) {
    body = `<a class="btn big slime" href="#/library/${app.id}">✓ In your library · Open</a>
      <div class="meta">${owned.installed ? `Installed from <code>${esc(owned.branch)}</code> · v${esc(owned.version)}` : "Not installed yet - install it from your library"}</div>`;
  } else if (av.official || av.community.length) {
    const b = av.official || av.community[0];
    body = `${av.official ? "" : `<div class="no-official">The publisher hasn't released a ${label} build. A community build is available.</div>`}
      <button class="btn big slime" data-add="${app.id}">+ Add to library${app.stage === "beta" ? " (beta)" : ""}</button>
      <div class="meta">Free · you'll be asked whether to install the ${b.official ? "official" : "community"} ${BUILD_KINDS[b.build.kind]} (${b.build.size})</div>
      ${b.official ? "" : `<div class="caveat"><b>Community-sourced build</b> from <code>${esc(b.branch)}</code> by @${esc(b.author)}. <a href="#/app/${app.id}/branch/${enc(b.branch)}" style="color:var(--info)">Read its README →</a></div>`}`;
  } else {
    body = `<div class="no-official">No ${label} build yet - official or community.</div>
      <button class="btn big" data-add="${app.id}">+ Add to library anyway</button>
      <div class="meta">It'll wait in your library until a ${label} build appears. Or <a href="#" data-gh="${repoUrl(app)}">build it from source</a>.</div>`;
  }
  return `<div class="install-box"><h3>${icon(os)} ${label}</h3>${body}</div>`;
}

function donateBoxHtml(app) {
  return `<div class="donate-box">
    <div class="muted" style="font-size:12px;text-transform:uppercase;letter-spacing:.08em;font-weight:700">Free forever · support the maker</div>
    <div style="margin:8px 0 2px"><span class="raised">${money(app.raised)}</span> <span class="muted">from ${num(app.donors)} donors</span></div>
    <button class="btn slime big" style="width:100%;margin-top:10px" data-donate="${app.id}">♥ Donate ${money(app.donation)}</button>
    <div class="meta muted" style="font-size:12px;margin-top:8px;text-align:center">@${esc(app.dev)} suggests ${money(app.donation)} · any amount · paid straight to them</div>
  </div>`;
}

function overviewTab(app) {
  const up = isUpcoming(app);
  const rows = Object.keys(PLATFORMS).map(os => {
    const av = availability(app, os);
    let cell;
    if (av.official) cell = `<span class="badge official">OFFICIAL</span> ${BUILD_KINDS[av.official.build.kind]} · ${av.official.build.size}`;
    else if (av.community.length) cell = av.community.map(c => `<div><span class="badge community">COMMUNITY</span> ${BUILD_KINDS[c.build.kind]} from <a href="#/app/${app.id}/branch/${enc(c.branch)}"><code>${esc(c.branch)}</code></a> by @${esc(c.author)}</div>`).join("");
    else cell = `<span class="faint">${app.stage === "soon" ? "Arrives with the beta" : "Not available yet"}</span>`;
    return `<tr class="${os === S.os ? "mine" : ""}"><td>${icon(os)}${PLATFORMS[os].label}${os === S.os ? ` <span class="faint" style="font-weight:400">(you)</span>` : ""}</td><td>${cell}</td></tr>`;
  }).join("");
  return `
    <div class="prose">${app.long.split("\n\n").map(p => `<p>${esc(p)}</p>`).join("")}</div>
    ${up ? `<div class="section-h"><h2>Release progress</h2></div>
      <div class="panel panel-pad">${timelineHtml(app)}${stageProgressHtml(app)}</div>
      <div class="section-h"><h2>Recent commits</h2><span class="sub">From the tracked branch</span></div>
      <div class="panel panel-pad"><ul style="margin:0;padding-left:18px;color:#c4ced9">${(app.devlog || []).map(d => `<li>${esc(d)}</li>`).join("")}</ul></div>` : ""}
    <div class="section-h"><h2>Platforms &amp; builds</h2><span class="sub">Missing a platform? Anyone can add one in a branch.</span></div>
    <div class="panel panel-pad"><table class="plat-table"><tr><th>Platform</th><th>Available build</th></tr>${rows}</table></div>
    <div class="section-h"><h2>Nothing for sale</h2></div>
    <div class="panel panel-pad">
      <p style="margin:0">Everything in this app is free. Apps on Slop can't sell anything, and donating doesn't unlock anything either - donors get exactly the same app as everyone else.</p>
    </div>`;
}
function stageProgressHtml(app) {
  const l = lifecycle(app);
  const pct = Math.min(100, Math.max(0, (l.used / l.limit) * 100));
  return `<div style="margin-top:16px">
    <div style="display:flex;justify-content:space-between;font-size:12px" class="muted"><span>${STAGE_LABEL[l.stage]} since ${date(app.stageSince)}</span><span>${l.stage === "soon" ? "Must enter beta" : "Must fully release"} by ${date(l.deadline)}</span></div>
    <div class="progress" style="margin-top:6px"><i style="width:${pct}%;${l.daysLeft < 7 ? "background:var(--warn)" : ""}"></i></div>
    ${l.dead ? `<div class="caveat" style="border-color:rgba(248,113,113,.4);background:rgba(248,113,113,.06);color:#f5c2c2;margin-top:12px"><b style="color:var(--bad)">Marked dead.</b> Nothing has been pushed to <code>${esc(app.tracked)}</code> in ${l.quiet} days (limit: ${STAGE_RULES.deadAfterDays}). It comes back to life with the next commit.</div>` : ""}
  </div>`;
}

function branchesTab(app) {
  const q = app.queue;
  const n = checksFor(app).length;
  return `
    ${q ? `<div class="panel queue">
      <div style="display:flex;justify-content:space-between;gap:10px;flex-wrap:wrap;align-items:center">
        <div><b>Update queued: v${esc(q.version)}</b> <span class="muted">from tracked branch <code>${esc(app.tracked)}</code></span></div>
        ${q.scan >= n ? `<span class="badge official">READY TO SHIP</span>` : `<span class="badge warn">SCANNING · ${q.scan}/${n} CHECKS</span>`}
      </div>
      <ul>${q.commits.map(c => `<li>${esc(c)}</li>`).join("")}</ul>
      <div class="progress" style="margin-top:12px"><i style="width:${Math.min(100, (q.scan / n) * 100)}%"></i></div>
      <div class="muted" style="font-size:12px;margin-top:6px">Pushes to the tracked branch are queued, scanned, then delivered to everyone who installed from it.</div>
    </div>` : ""}
    <p class="muted" style="margin-top:0">The publisher chooses which branch is <b>tracked</b> - that's the official version. Every other branch is a community mod, port or fix. Branches with their own builds can be added to your library directly.</p>
    ${app.branches.map(b => {
      const tracked = b.name === app.tracked;
      const builds = tracked ? app.builds : b.builds;
      return `<div class="panel branch ${tracked ? "tracked" : ""}">
        <div>
          <div style="display:flex;gap:10px;align-items:center;flex-wrap:wrap;margin-bottom:4px">
            <span class="bn">${esc(b.name)}</span>
            ${tracked ? `<span class="badge official">TRACKED · OFFICIAL</span>` : `<span class="badge community">COMMUNITY</span>`}
            ${!tracked && Object.keys(b.builds).length ? `<span class="badge neutral">HAS BUILDS</span>` : ""}
          </div>
          <div style="color:#c4ced9;font-size:13px;margin-bottom:6px">${esc(b.desc)}</div>
          <div class="line"><span>@${esc(b.author)}</span>${b.stars ? `<span>★ ${num(b.stars)}</span>` : ""}<span>updated ${date(b.updated)}</span>
            ${Object.keys(builds).length ? `<span>builds: ${platsHtml({ builds, branches: [] })}</span>` : `<span>source only</span>`}
            <span style="color:var(--good)">✓ scanned</span></div>
        </div>
        <div class="acts">
          ${!tracked ? `<a class="btn small" href="#/app/${app.id}/branch/${enc(b.name)}">README</a>` : ""}
          <a class="btn small" href="#" data-gh="${branchRepo(app, b)}">GitHub</a>
          ${builds[S.os] ? `<button class="btn small ${tracked ? "slime" : ""}" data-add="${app.id}" data-branch="${esc(b.name)}">+ Add to library</button>` : ""}
        </div>
      </div>`;
    }).join("")}
    <div class="panel panel-pad" style="margin-top:16px;display:flex;gap:14px;align-items:center;flex-wrap:wrap">
      <div style="flex:1;min-width:220px"><b>Make your own branch</b><div class="muted" style="font-size:13px">Fork on GitHub, push your changes, add builds if you like, then register it here. It'll be scanned before anyone can install it.</div></div>
      <button class="btn" onclick="toast('🍴 Would fork <b>${repoUrl(app)}</b> and open the register-branch form.')">Fork &amp; register branch</button>
    </div>`;
}

function scanRowsHtml(results) {
  return results.map(r => {
    const c = SCAN_CHECKS.find(x => x.id === r.id);
    const sym = { pass: "✓", warn: "!", fail: "✕", run: "◌", wait: "·" }[r.status];
    return `<div class="scan-row" data-check="${r.id}">
      <span class="st ${r.status}">${sym}</span>
      <div><b>${c.name}</b><div class="note ${r.status === "warn" ? "hit" : r.status === "fail" ? "err" : ""}">${esc(r.note || c.detail)}</div></div>
      <span class="badge ${r.status === "pass" ? "official" : r.status === "warn" ? "warn" : r.status === "fail" ? "bad" : "neutral"}">${{ pass: "PASSED", warn: "NOTE", fail: "FAILED", run: "RUNNING", wait: "QUEUED" }[r.status]}</span>
    </div>`;
  }).join("");
}

function securityTab(app) {
  const comm = app.branches.filter(b => b.name !== app.tracked && Object.keys(b.builds).length);
  const n = checksFor(app).length;
  return `
    <div class="panel panel-pad" style="margin-bottom:16px;display:flex;gap:16px;align-items:center;flex-wrap:wrap">
      <span style="font-size:34px">🛡️</span>
      <div style="flex:1;min-width:220px"><b style="font-size:16px">Passed Slop review on ${date(app.scan.date)}</b>
        <div class="muted" style="font-size:13px">Every upload - official or community, released or upcoming - goes through the same pipeline before it reaches the store, and again on every update or credible report.</div></div>
    </div>
    <div class="panel panel-pad"><div class="scan-list">${scanRowsHtml(scanResults(app))}</div></div>
    ${comm.length ? `<div class="section-h"><h2>Community builds</h2><span class="sub">Scanned separately - they're different binaries</span></div>
      <div class="panel panel-pad"><table class="plat-table"><tr><th>Branch</th><th>Builds</th><th>Result</th></tr>
      ${comm.map(b => `<tr><td><a href="#/app/${app.id}/branch/${enc(b.name)}"><code>${esc(b.name)}</code></a></td><td>${platsHtml({ builds: b.builds, branches: [] })}</td><td><span class="badge official">${n}/${n} PASSED</span> <span class="faint">${date(b.updated)}</span></td></tr>`).join("")}
      </table></div>` : ""}`;
}

// Monthly rating history (synthesised). TRENDS: how much sloppier the app was 12 months ago.
const TRENDS = { "pixel-dungeon-delver": 26, "neon-drift-2077": 18, "starfreight-idle": 14, "ultimate-clicker-ultimate": -3, "cat-cafe-tycoon": -6, "tidy-files": 10, "stepquest": 8 };
function ratingHistory(app) {
  let seed = [...app.id].reduce((a, c) => (a * 31 + c.charCodeAt(0)) >>> 0, 7);
  const rnd = () => ((seed = (seed * 1103515245 + 12345) >>> 0) % 1000) / 1000;
  const trend = TRENDS[app.id] ?? 6;
  return Array.from({ length: 12 }, (_, i) => {
    const d = new Date(); d.setDate(1); d.setMonth(d.getMonth() - 11 + i);
    const slop = Math.max(1, Math.min(99, Math.round(app.slop + trend * (1 - i / 11) + (rnd() - .5) * 8)));
    return { label: d.toLocaleDateString(undefined, { month: "short" }), slop: i === 11 ? app.slop : slop, votes: Math.round(app.reviews / 14 * (0.6 + rnd() * 0.8)) };
  });
}
function voteBarHtml(app, mine, spark = true) {
  const total = app.reviews + (mine ? 1 : 0), yes = Math.round(app.reviews * (100 - app.slop) / 100) + (mine && !isSlopVote(mine) ? 1 : 0);
  const pts = ratingHistory(app).map((m, i) => `${(i / 11) * 120},${29 - ((100 - m.slop) / 100) * 27}`).join(" ");
  return `<div class="vote-bar"><i style="flex:${yes}"></i><i style="flex:${total - yes}"></i></div>
    <div class="vote-legend"><span>👍 ${num(yes)} Not Slop</span>
      ${spark ? `<a href="#/app/${app.id}/reviews" class="spark" title="% Not Slop over the last 12 months"><svg viewBox="0 0 120 30" width="96" height="24"><polyline points="${pts}" fill="none" stroke="${slopColor(app.slop)}" stroke-width="2.5" stroke-linejoin="round"/></svg>12 mo</a>` : ""}
      <span>👎 ${num(total - yes)} Slop</span></div>`;
}
function historyChartHtml(app) {
  const h = ratingHistory(app);
  const diff = (100 - h[11].slop) - (100 - h[0].slop);
  return `<div class="hist-chart">${h.map(m => `<div class="col" title="${m.label}: ${100 - m.slop}% Not Slop · ${num(m.votes)} votes">
      <span class="pct">${100 - m.slop}</span><span class="bar-wrap"><i style="height:${100 - m.slop}%;background:${slopColor(m.slop)}"></i></span><span class="mo">${m.label}</span></div>`).join("")}</div>
    <div class="muted" style="font-size:12px;margin-top:8px">${Math.abs(diff) < 3 ? "Steady over the last year." : diff > 0 ? `<b style="color:var(--good)">▲ ${diff} points</b> more Not Slop than a year ago - updates and branches are working.` : `<b style="color:var(--warn)">▼ ${-diff} points</b> sloppier than a year ago.`}</div>`;
}

function reviewsTab(app, reviews) {
  const mine = S.myReviews[app.id];
  return `
    <div class="panel rev-summary">
      <div>
        <div class="muted" style="font-size:12px;text-transform:uppercase;letter-spacing:.08em;font-weight:700;margin-bottom:8px">Overall</div>
        ${slopHtml(app.slop, "lg", true, `${num(app.reviews)} votes`)}
        <p class="muted" style="font-size:12px;margin:12px 0">Everyone who has launched it through Slop gets one vote: <b style="color:var(--text)">👍 Not Slop</b> or <b style="color:var(--text)">👎 Slop</b>.</p>
        <button class="btn ${canVote(app.id) ? "slime" : ""}" data-review="${app.id}">${mine ? "Change your vote" : canVote(app.id) ? "Vote & review" : "🔒 Launch it to vote"}</button>
      </div>
      <div><div class="muted" style="font-size:12px;text-transform:uppercase;letter-spacing:.08em;font-weight:700;margin-bottom:8px">% Not Slop over time</div>${historyChartHtml(app)}</div>
    </div>
    ${reviews.map(r => `
      <div class="panel review" ${r.mine ? 'style="border-color:rgba(163,230,53,.4)"' : ""}>
        <div class="who"><b>${r.mine ? "You" : "@" + esc(r.user)}</b>${r.hours} hrs used<br>${date(r.date)}${r.mine ? `<br><span class="badge official" style="margin-top:6px">YOUR VOTE</span>` : ""}</div>
        <div>
          ${voteChip(r)}
          <p>${esc(r.text)}</p>
          <div class="foot">${r.mine ? "" : `<span>${num(r.helpful)} people found this helpful</span><button class="btn small ghost" onclick="this.textContent='👍 Thanks'">👍 Helpful</button><button class="btn small ghost danger" data-report="${app.id}" data-target="review:${esc(r.user)}">⚑ Report</button>`}</div>
        </div>
      </div>`).join("")}`;
}

/* ---- branch README page ---------------------------------------------- */
function branchPage(id, name) {
  const app = APP_BY_ID[id];
  const b = app?.branches.find(x => x.name === name);
  if (!b) return mount(`<div class="page"><div class="empty-state">Branch not found.</div></div>`);
  const tracked = b.name === app.tracked;
  const builds = tracked ? app.builds : b.builds;
  const n = checksFor(app).length;
  const readme = b.readme || `# ${app.name} - ${b.name}\n\n${b.desc}\n\nThis branch hasn't written a longer README yet.`;
  mount(`<div class="page">
    <div class="crumbs"><a href="#/">Store</a> › <a href="#/app/${id}">${esc(app.name)}</a> › <a href="#/app/${id}/branches">Branches</a> › ${esc(b.name)}</div>
    <div style="display:flex;gap:12px;align-items:center;flex-wrap:wrap;margin-bottom:16px">
      <h1 style="font-family:ui-monospace,Menlo,monospace;font-size:26px">${esc(b.name)}</h1>
      ${tracked ? `<span class="badge official">TRACKED · OFFICIAL</span>` : `<span class="badge community">COMMUNITY BRANCH</span>`}
    </div>
    <div class="two-col">
      <div>
        ${tracked ? "" : `<div class="caveat" style="margin:0 0 16px"><b>This is a community branch of ${esc(app.name)}.</b> It's maintained by @${esc(b.author)}, not by the publisher @${esc(app.dev)}. It passed Slop's security scan, but the publisher hasn't reviewed it and it may fall behind official updates.</div>`}
        <div class="panel panel-pad prose">${md(readme)}</div>
      </div>
      <div style="display:flex;flex-direction:column;gap:14px">
        <div class="install-box"><h3>Builds on this branch</h3>
          ${Object.keys(builds).length ? `<table class="plat-table">${Object.entries(builds).map(([os, bd]) => `<tr class="${os === S.os ? "mine" : ""}"><td>${icon(os)}${PLATFORMS[os].label}</td><td>${BUILD_KINDS[bd.kind]} · ${bd.size}</td></tr>`).join("")}</table>` : `<div class="no-official">Source only - no builds uploaded.</div>`}
          ${builds[S.os] ? `<button class="btn big ${tracked ? "slime" : ""}" style="margin-top:12px;${tracked ? "" : "border-color:var(--info);color:var(--info)"}" data-add="${id}" data-branch="${esc(b.name)}">+ Add to library</button>` : `<div class="meta">No build for ${PLATFORMS[S.os].label} on this branch.</div>`}
        </div>
        <div class="panel panel-pad trust">
          <div class="trust-row"><span class="ic">@</span><span>Maintained by <b>@${esc(b.author)}</b>${b.stars ? ` · ★ ${num(b.stars)}` : ""}</span></div>
          <div class="trust-row"><span class="ic">✓</span><span><b>${n}/${n} security checks passed</b> · ${date(b.updated)}</span></div>
          <div class="trust-row"><span class="ic">{ }</span><span><a href="#" data-gh="${branchRepo(app, b)}">${esc(branchRepo(app, b))}</a></span></div>
        </div>
        <button class="btn ghost danger" data-report="${id}" data-target="branch:${esc(b.name)}">⚑ Report this branch</button>
      </div>
    </div>
  </div>`);
}

/* ======================================================================
   Browse, tags, search
   ====================================================================== */
function browse(groupName) {
  if (!groupName) {
    return mount(`<div class="page"><div class="store">${sidebarHtml()}<div>
      <h1 style="font-size:28px;margin-bottom:6px">All categories</h1>
      <p class="muted" style="margin:0 0 24px">Broad categories on top, specific tags underneath. Every app has several tags, so it shows up in more than one place.</p>
      ${TAG_GROUPS.map(g => `<div class="browse-group">
        <h3><a href="#/browse/${enc(g.name)}">${g.icon} ${g.name}</a> <span class="muted" style="font-size:13px;font-weight:400">${appsInGroup(g).length} apps</span></h3>
        <div class="tag-cloud">${g.tags.map(tagLink).join("")}</div>
      </div>`).join("")}
    </div></div></div>`);
  }
  const g = TAG_GROUPS.find(x => x.name === groupName);
  if (!g) return mount(`<div class="page"><div class="empty-state">Unknown category.</div></div>`);
  listPage({
    crumbs: `<a href="#/">Store</a> › <a href="#/browse">Categories</a> › ${g.name}`,
    title: `${g.icon} ${g.name}`, sub: g.desc, active: g.name,
    extra: `<div class="tag-cloud" style="margin-top:14px">${g.tags.map(tagLink).join("")}</div>`,
    apps: appsInGroup(g),
  });
}
function tagPage(tag) {
  const gName = TAG_GROUP_OF[tag];
  const apps = appsWithTag(tag);
  const related = [...new Set(apps.flatMap(a => a.tags))].filter(t => t !== tag).slice(0, 10);
  listPage({
    crumbs: `<a href="#/">Store</a> › <a href="#/browse">Categories</a> › ${gName ? `<a href="#/browse/${enc(gName)}">${gName}</a> › ` : ""}${esc(tag)}`,
    title: `# ${esc(tag)}`, sub: gName ? `A ${gName.toLowerCase()} tag` : "Tag", active: gName,
    extra: related.length ? `<div style="margin-top:12px" class="chips"><span class="muted" style="font-size:12px;margin-right:4px">Related:</span>${related.map(t => `<a class="chip" href="#/tag/${enc(t)}">${esc(t)}</a>`).join("")}</div>` : "",
    apps,
  });
}
function searchPage(q) {
  const s = q.trim().toLowerCase();
  const apps = APPS.filter(a => !s || [a.name, a.short, a.dev, a.type, ...a.tags].join(" ").toLowerCase().includes(s));
  listPage({ crumbs: `<a href="#/">Store</a> › Search`, title: s ? `Results for “${esc(q)}”` : "Everything", sub: `${apps.length} apps`, apps });
}
function listPage({ crumbs, title, sub, extra = "", apps, active }) {
  const v = mount(`<div class="page"><div class="store">${sidebarHtml(active)}<div>
    <div class="crumbs">${crumbs}</div>
    <h1 style="font-size:28px">${title}</h1>
    <div class="muted">${sub}</div>
    ${extra}
    <div class="filters">
      <label>Sort <select id="sort"><option value="slop">Least slop</option><option value="popular">Popular this month</option><option value="reviews">Most reviewed</option><option value="new">Recently updated</option><option value="sloppiest">Sloppiest first</option></select></label>
      <label>Show <select id="stage"><option value="released">Released</option><option value="all">Released + upcoming</option><option value="upcoming">Upcoming only</option></select></label>
      <label><input type="checkbox" id="mine-os"> Runs on ${PLATFORMS[S.os].label}</label>
      <span class="muted" style="margin-left:auto">👍 at least</span>
      <input type="range" id="minnot" min="0" max="90" step="5" value="0" style="width:110px">
      <span id="minnot-l" class="muted" style="width:40px">0%</span>
    </div>
    <div class="shelf" id="results"></div>
  </div></div></div>`);
  const draw = () => {
    const sortBy = $("#sort", v).value, min = +$("#minnot", v).value, stage = $("#stage", v).value;
    $("#minnot-l", v).textContent = `${min}%`;
    let list = apps.filter(a => (stage === "all" || (stage === "upcoming") === isUpcoming(a)) && (a.slop == null || 100 - a.slop >= min)
      && (!$("#mine-os", v).checked || runsOn(a, S.os)));
    const sl = a => a.slop ?? 101;
    list = [...list].sort({ slop: (a, b) => sl(a) - sl(b), sloppiest: (a, b) => (b.slop ?? -1) - (a.slop ?? -1), popular: (a, b) => (b.monthAdds || interestCount(b) / 10) - (a.monthAdds || interestCount(a) / 10), reviews: (a, b) => b.reviews - a.reviews, new: (a, b) => b.lastCommit.localeCompare(a.lastCommit) }[sortBy]);
    $("#results", v).innerHTML = list.length ? list.map(a => tileHtml(a)).join("") : `<div class="empty-state" style="grid-column:1/-1">Nothing matches those filters.</div>`;
  };
  $$("select, input", $(".filters", v)).forEach(el => el.addEventListener("input", draw));
  v.addEventListener("slop:interest", draw);
  draw();
}

/* ======================================================================
   Library
   ====================================================================== */
function library(id) {
  if (!S.user) return mount(`<div class="page"><div class="empty-state">📚 Sign in to see your library.<div style="margin-top:14px"><button class="btn slime" onclick="signIn(render)">Sign in</button></div></div></div>`);
  const owned = Object.keys(S.library).filter(k => APP_BY_ID[k]);
  const wanted = Object.keys(S.interested).filter(k => APP_BY_ID[k] && !S.library[k]);
  if (!owned.length && !wanted.length) return mount(`<div class="page"><div class="empty-state">Your library is empty. <a href="#/">Find something in the store</a>.</div></div>`);
  if (!id || !(S.library[id] || S.interested[id])) id = owned[0] || wanted[0];
  const app = APP_BY_ID[id], inst = S.library[id];

  const item = a => {
    const i = S.library[a.id];
    const upd = i?.installed && a.queue && i.branch === a.tracked && i.version !== a.queue.version && a.queue.scan >= checksFor(a).length;
    return `<a class="lib-item ${a.id === id ? "on" : ""} ${i && !i.installed ? "dim" : ""}" href="#/library/${a.id}"><span class="mini" style="background:linear-gradient(135deg,${a.art.from},${a.art.to})">${a.art.emoji}</span><span class="nm">${esc(a.name)}</span>${upd ? `<span class="upd" title="Update ready"></span>` : ""}${!i ? `<span class="tag-mini">${lifecycle(a).dead ? "dead" : a.stage === "soon" ? "soon" : "beta"}</span>` : i.installed ? "" : `<span class="tag-mini">not installed</span>`}</a>`;
  };
  const games = owned.filter(k => isGame(APP_BY_ID[k])).map(k => APP_BY_ID[k]);
  const privHtml = () => {
    const mine = S.privates.filter(p => p.owner === "you"), shared = visiblePrivates().filter(p => p.owner !== "you");
    const it = p => `<a class="lib-item" href="#/private/${p.id}"><span class="mini" style="background:linear-gradient(135deg,${p.art.from},${p.art.to})">${p.art.emoji}</span><span class="nm">${esc(p.name)}</span><span class="tag-mini">${p.owner === "you" ? "🔒" : "@" + esc(p.owner)}</span></a>`;
    return `<h4>Private Slop (${mine.length + shared.length})</h4>${mine.map(it).join("")}${shared.map(it).join("")}<a class="lib-item faint" href="#/publish" style="font-size:12px">+ Add Private Slop</a>`;
  };
  const other = owned.filter(k => !isGame(APP_BY_ID[k])).map(k => APP_BY_ID[k]);

  mount(`<div class="library">
    <aside class="lib-list">
      ${games.length ? `<h4>Games (${games.length})</h4>${games.map(item).join("")}` : ""}
      ${other.length ? `<h4>Apps &amp; tools (${other.length})</h4>${other.map(item).join("")}` : ""}
      ${wanted.length ? `<h4>Interested (${wanted.length})</h4>${wanted.map(k => item(APP_BY_ID[k])).join("")}` : ""}
      ${privHtml()}
    </aside>
    <section class="lib-main" id="lib-main"></section>
  </div>`);
  const main = $("#lib-main");
  const hero = `<div class="lib-hero" style="background:linear-gradient(135deg, ${app.art.from}, ${app.art.to})"><span class="em">${app.art.emoji}</span><div><div style="margin-bottom:6px">${stageBadge(app)}</div><h1>${esc(app.name)}</h1></div></div>`;

  if (!inst) { // Interested, not in the library as a build
    const l = lifecycle(app);
    main.innerHTML = `${hero}
      <div class="lib-bar">
        ${app.stage === "beta" && runsOn(app, S.os) ? `<button class="btn play" data-add="${id}">+ GET THE BETA</button>` : `<button class="btn play" disabled style="opacity:.5">${l.dead ? "PROJECT DEAD" : "NOT RELEASED YET"}</button>`}
        <div class="stat"><b>★ ${num(interestCount(app))}</b>interested</div>
        <div class="stat"><b>${deadlineText(app)}</b>${STAGE_LABEL[app.stage]}</div>
        <div style="flex:1"></div>
        <a class="btn" href="#/app/${id}">Store page</a>
        <button class="btn" data-interest="${id}">Remove interest</button>
      </div>
      <div class="lib-body"><div style="display:flex;flex-direction:column;gap:14px">
        <div class="panel panel-pad"><b>You're on the list</b><div class="muted" style="font-size:13px;margin-top:4px">${l.dead ? "This project has gone quiet. If the publisher starts committing again it will come back to life and you'll be told." : app.stage === "soon" ? "No build yet. You'll get a notification the moment it enters beta, and it'll be ready to install from here." : "The beta is out. Add it to install now, or wait for the full release."}</div>
          <div style="margin-top:14px">${timelineHtml(app)}${stageProgressHtml(app)}</div></div>
        <div class="panel panel-pad"><b>Recent commits</b><ul style="margin:8px 0 0;padding-left:18px;color:#c4ced9">${(app.devlog || []).map(d => `<li>${esc(d)}</li>`).join("")}</ul></div>
      </div><div><div class="panel panel-pad"><b>About</b><p style="color:#c4ced9;margin:6px 0 0">${esc(app.short)}</p></div></div></div>`;
    return;
  }

  const q = inst.installed && app.queue && inst.branch === app.tracked && inst.version !== app.queue.version ? app.queue : null;
  const n = checksFor(app).length;
  const installable = app.branches.filter(b => (b.name === app.tracked ? app.builds : b.builds)[S.os]);
  const onBranch = app.branches.find(b => b.name === inst.branch);
  main.innerHTML = `${hero}
    <div class="lib-bar">
      ${inst.installed ? `<button class="btn play" id="play">▶ ${isGame(app) ? "PLAY" : "LAUNCH"}</button>` : `<button class="btn play install" data-install="${id}">⬇ INSTALL</button>`}
      <div class="stat"><b>${inst.hours} hours</b>used</div>
      <div class="stat"><b>${inst.installed ? "v" + esc(inst.version) : "Not installed"}</b>${inst.installed ? "installed" : `${runsOn(app, S.os) ? "ready to install" : `no ${PLATFORMS[S.os].label} build yet`}`}</div>
      ${inst.installed ? `<div class="stat"><b><code>${esc(inst.branch)}</code> ${inst.branch === app.tracked ? `<span class="badge official">OFFICIAL</span>` : `<span class="badge community">COMMUNITY</span>`}</b>source branch</div>` : ""}
      ${isGame(app) && app.playing != null ? `<div class="stat"><b>${playingHtml(app)}</b>right now</div>` : ""}
      <div style="flex:1"></div>
      <a class="btn" href="#/app/${id}">Store page</a>
      <button class="btn" data-donate="${id}">♥ Donate</button>
    </div>
    <div class="lib-body">
      <div style="display:flex;flex-direction:column;gap:14px">
        ${!inst.installed ? `<div class="panel panel-pad" style="display:flex;gap:12px;align-items:center;flex-wrap:wrap"><div style="flex:1;min-width:200px"><b>In your library, not installed</b><div class="muted" style="font-size:13px">It's yours - install whenever you like. Nothing is downloaded until you do.</div></div><button class="btn slime" data-install="${id}">Install</button></div>` : ""}
        ${q ? `<div class="panel queue" style="margin:0">
          <div style="display:flex;justify-content:space-between;align-items:center;gap:10px;flex-wrap:wrap">
            <b>Update v${esc(q.version)} from <code>${esc(app.tracked)}</code></b>
            ${q.scan >= n ? `<button class="btn slime small" id="update">Update now</button>` : `<span class="badge warn">SECURITY SCAN ${q.scan}/${n}</span>`}
          </div>
          <ul>${q.commits.map(c => `<li>${esc(c)}</li>`).join("")}</ul>
          ${q.scan < n ? `<div class="progress" style="margin-top:10px"><i style="width:${(q.scan / n) * 100}%"></i></div><div class="muted" style="font-size:12px;margin-top:6px">The publisher pushed this to the tracked branch. It installs automatically once every check passes.</div>` : ""}
        </div>` : inst.installed ? `<div class="panel panel-pad"><b>✓ Up to date</b><div class="muted" style="font-size:13px">${inst.branch === app.tracked ? `You're following the publisher's tracked branch <code>${esc(app.tracked)}</code>.` : `You're on community branch <code>${esc(inst.branch)}</code> by @${esc(onBranch?.author || "?")}. Updates come from its author.`}</div></div>` : ""}
        <div class="panel panel-pad"><b>About</b><p style="color:#c4ced9;margin:6px 0 0">${esc(app.short)}</p></div>
        ${S.myReviews[id] || !inst.installed || isUpcoming(app) ? "" : canVote(id) ? `<div class="panel panel-pad" style="display:flex;gap:10px;align-items:center;flex-wrap:wrap"><div style="flex:1;min-width:200px"><b>Slop or not?</b><div class="muted" style="font-size:13px">You've launched it, so your vote counts.</div></div><button class="btn" data-review="${id}" data-vote="not">👍 Not Slop</button><button class="btn" data-review="${id}" data-vote="slop">👎 Slop</button></div>`
          : `<div class="panel panel-pad muted" style="font-size:13px">🔒 Launch it once to vote 👍 Not Slop or 👎 Slop.</div>`}
      </div>
      <div style="display:flex;flex-direction:column;gap:14px">
        ${inst.installed && installable.length ? `<div class="panel panel-pad">
          <b>Switch branch</b>
          <div class="muted" style="font-size:12px">Install a mod or port instead of the official version.</div>
          <select id="branch">${installable.map(b => `<option value="${esc(b.name)}" ${b.name === inst.branch ? "selected" : ""}>${esc(b.name)} ${b.name === app.tracked ? "(official)" : `(community · @${esc(b.author)})`}</option>`).join("")}</select>
          <a href="#/app/${id}/branches" style="font-size:12px">Compare branches →</a>
        </div>` : ""}
        <div class="panel panel-pad trust">
          <div class="trust-row"><span class="ic">✓</span><span>Scanned ${date(app.scan.date)}</span></div>
          <div class="trust-row"><span class="ic">{ }</span><a href="#" data-gh="${repoUrl(app)}">Source on GitHub</a></div>
          <div class="trust-row"><span class="ic" style="background:rgba(248,113,113,.12);color:var(--bad)">⚑</span><a href="#" data-report="${id}" style="color:var(--bad)">Report a problem</a></div>
          ${inst.installed ? `<div class="trust-row"><span class="ic" style="background:var(--panel-3);color:var(--muted)">⏏</span><a href="#" id="uninstall" style="color:var(--muted)">Uninstall (keep in library)</a></div>` : ""}
          <div class="trust-row"><span class="ic" style="background:var(--panel-3);color:var(--muted)">🗑</span><a href="#" id="remove" style="color:var(--muted)">Remove from library</a></div>
        </div>
      </div>
    </div>`;

  $("#play")?.addEventListener("click", () => { if (!inst.launched) { inst.launched = true; save(); setTimeout(render, 50); } toast(`▶ Launching <b>${esc(app.name)}</b>…${isGame(app) ? `<br><span class="muted">Slop handed it a one-time Pulse token - you now count as 1 of ${num((app.playing || 0) + 1)} playing.</span>` : ""}`, 5000); });
  $("#update")?.addEventListener("click", () => progressToast(`Updating ${esc(app.name)} to v${esc(q.version)}…`, () => { inst.version = q.version; save(); toast(`✓ ${esc(app.name)} updated to v${esc(q.version)}`); render(); }));
  $("#branch")?.addEventListener("change", e => {
    const nb = e.target.value, b = app.branches.find(x => x.name === nb);
    const go = () => progressToast(`Switching to <code>${esc(nb)}</code>…`, () => { inst.branch = nb; inst.version = nb === app.tracked ? app.version : app.version + "-" + nb; save(); render(); });
    if (nb === app.tracked) go(); else communityConfirm(app, b, go, () => (e.target.value = inst.branch));
  });
  $("#uninstall")?.addEventListener("click", e => { e.preventDefault(); inst.installed = false; save(); toast(`${esc(app.name)} uninstalled. It stays in your library.`); render(); });
  $("#remove").onclick = e => { e.preventDefault(); if (confirm(`Remove ${app.name} from your library?`)) { delete S.library[id]; save(); location.hash = "#/library"; render(); } };
}

/* ======================================================================
   Add to library, install, interest, donate, report, review
   ====================================================================== */
function communityConfirm(app, b, onOk, onCancel = () => {}) {
  modal(`<div class="modal-h"><h2>Switch to a community build?</h2><button class="x" data-close>×</button></div>
    <p style="margin:0 0 10px"><code>${esc(b.name)}</code> is made by <b>@${esc(b.author)}</b>, not by ${esc(app.name)}'s publisher.</p>
    ${caveatHtml(app, b.name, b.author, b.updated)}
    <label style="display:flex;gap:8px;margin-top:14px;font-size:13px;align-items:flex-start"><input type="checkbox" id="ack" style="accent-color:var(--slime);margin-top:3px"> I understand this is a community-sourced build</label>
    <div class="actions"><button class="btn" data-close id="cancel">Cancel</button><button class="btn slime" id="ok" disabled>Switch</button></div>`,
  { onMount: (m, close) => {
      $("#ack", m).onchange = e => ($("#ok", m).disabled = !e.target.checked);
      $("#ok", m).onclick = () => { close(); onOk(); };
      $("#cancel", m).onclick = onCancel;
    } });
}
function caveatHtml(app, branch, author, updated) {
  return `<div class="caveat" style="margin:0"><ul style="margin:0">
    <li>Passed all ${checksFor(app).length} Slop security checks on ${date(updated)}</li>
    <li>Not reviewed or supported by @${esc(app.dev)}</li>
    <li>Updates come from @${esc(author)} and may lag behind official releases</li></ul>
    <div style="margin-top:8px"><a href="#/app/${app.id}/branch/${enc(branch)}" data-close style="color:var(--info)">Read its README first →</a></div></div>`;
}

// Step 1: add to library (free, instant). Step 2: ask whether to install now.
function addToLibrary(id, branch) {
  const app = APP_BY_ID[id];
  if (!S.library[id]) {
    S.library[id] = { branch: branch || app.tracked, version: app.version, hours: 0, installed: false };
    save();
  }
  installPrompt(id, branch, true);
  render();
}
function installPrompt(id, prefer, justAdded = false) {
  const app = APP_BY_ID[id], av = availability(app, S.os), label = PLATFORMS[S.os].label;
  const opts = [...(av.official ? [av.official] : []), ...av.community];
  const head = justAdded ? `✓ ${esc(app.name)} is in your library` : `Install ${esc(app.name)}`;
  const phone = sendToPhoneHtml(app);
  if (!opts.length) {
    return modal(`<div class="modal-h"><h2>${head}</h2><button class="x" data-close>×</button></div>
      ${phone ? `<p class="muted" style="margin:0 0 12px">${esc(app.name)} is a phone app - there's no ${label} build. Send it to your phone instead:</p>${phone}`
        : `<p class="muted" style="margin:0">There's no ${label} build yet - official or community. It'll wait in your library and you'll be told when one appears.</p>`}
      <div class="actions"><button class="btn ${phone ? "" : "slime"}" data-close>${phone ? "Later" : "OK"}</button></div>`, { onMount: wirePhoneSend(app) });
  }
  const sel = opts.find(o => o.branch === prefer) || opts[0];
  modal(`<div class="modal-h"><h2>${head}</h2><button class="x" data-close>×</button></div>
    <p class="muted" style="margin:0 0 12px">${justAdded ? "Want to install it now? You can always do it later from your library." : `Choose a ${label} build.`}</p>
    <div class="build-opts">${opts.map(o => `<label><input type="radio" name="b" value="${esc(o.branch)}" ${o === sel ? "checked" : ""}>
      <span><b>${o.official ? "Official" : "Community"} ${BUILD_KINDS[o.build.kind]}</b> ${o.official ? `<span class="badge official">OFFICIAL</span>` : `<span class="badge community">COMMUNITY</span>`}${app.stage === "beta" ? ` <span class="badge beta">BETA</span>` : ""}
      <small><code>${esc(o.branch)}</code> · @${esc(o.author)} · ${o.build.size}</small></span></label>`).join("")}</div>
    <div id="cav" style="margin-top:12px"></div>
    ${phone ? `<div class="muted" style="font-size:12px;margin:16px 0 6px;text-transform:uppercase;letter-spacing:.08em;font-weight:700">Also on your phone</div>${phone}` : ""}
    <div class="actions"><button class="btn" data-close>${justAdded ? "Not now" : "Cancel"}</button><button class="btn slime" id="go">⬇ Install on this ${isPhoneOS(S.os) ? "phone" : "computer"}</button></div>`,
  { onMount: (m, close) => {
      wirePhoneSend(app)(m);
      const upd = () => {
        const o = opts.find(x => x.branch === $("input[name=b]:checked", m).value);
        $("#cav", m).innerHTML = o.official ? "" : `${caveatHtml(app, o.branch, o.author, o.data.updated)}
          <label style="display:flex;gap:8px;margin-top:12px;font-size:13px;align-items:flex-start"><input type="checkbox" id="ack" style="accent-color:var(--slime);margin-top:3px"> I understand this is a community-sourced build</label>`;
        $("#go", m).disabled = !o.official;
        $("#ack", m)?.addEventListener("change", e => ($("#go", m).disabled = !e.target.checked));
      };
      $(".build-opts", m).onchange = upd; upd();
      $("#go", m).onclick = () => {
        const o = opts.find(x => x.branch === $("input[name=b]:checked", m).value);
        close();
        progressToast(`⬇ Installing <b>${esc(app.name)}</b> (${esc(o.branch)}) - verifying signature…`, () => {
          Object.assign(S.library[id], { branch: o.branch, installed: true, version: o.official ? app.version : app.version + "-" + o.branch });
          save();
          toast(`✓ <b>${esc(app.name)}</b> installed. <a href="#/library/${id}">Open in library</a>`);
          render();
        });
      };
    } });
}
function toggleInterest(id) {
  const app = APP_BY_ID[id];
  if (S.interested[id]) { delete S.interested[id]; toast(`Removed interest in ${esc(app.name)}`); }
  else { S.interested[id] = true; toast(`★ <b>${esc(app.name)}</b> added to your library as Interested. We'll tell you when it ${app.stage === "soon" ? "enters beta" : "releases"}.`, 4500); }
  save();
  // Pages with live shelves listen for this instead of a full re-render (keeps scroll + tab state).
  const v = $("#view");
  if (location.hash.startsWith("#/library") || location.hash.startsWith("#/app/")) render();
  else { v.dispatchEvent(new Event("slop:interest")); $$(`[data-interest="${id}"]`).forEach(b => { b.classList.toggle("slime", !S.interested[id]); b.textContent = S.interested[id] ? "✓ Interested" : "★ Interested"; }); }
}

// Card processing is added on top so the maker gets the amount chosen (flat published rate: 2.9% + $0.30).
const donorTotal = d => Math.round(((d + 0.30) / (1 - 0.029)) * 100) / 100;

function donate(id) {
  const app = APP_BY_ID[id];
  const presets = [...new Set([1, 3, app.donation, 10, 25])].sort((a, b) => a - b).slice(0, 5);
  modal(`<div class="modal-h"><h2>♥ Support ${esc(app.name)}</h2><button class="x" data-close>×</button></div>
    <p class="muted" style="margin:0 0 14px">Everything on Slop is free. Donations are optional, unlock nothing, and are paid straight into <b style="color:var(--text)">@${esc(app.dev)}</b>'s own Stripe account. Choose how much they get:</p>
    <div class="amounts">${presets.map(p => `<button data-amt="${p}" class="${p === app.donation ? "on" : ""}">${money(p)}${p === app.donation ? "<small>suggested</small>" : "<small>&nbsp;</small>"}</button>`).join("")}</div>
    <div class="money-in"><span>$</span><input id="amt" type="number" min="0.5" step="0.5" value="${app.donation}" aria-label="Amount"></div>
    <div class="split-bar"><i></i><i></i></div>
    <div class="breakdown" id="bd"></div>
    <p class="muted" style="font-size:12px;margin:12px 0 0">Slop never holds the money. It charges the maker a flat 5% platform fee for hosting and security scanning. Card processing is added to your total so the maker isn't charged for it. Donations are final and aren't tax-deductible. Your donation is anonymous.</p>
    <div class="actions"><button class="btn" data-close>Not now</button><button class="btn slime" id="give">Donate</button></div>`,
  { onMount: (m, close) => {
      const inp = $("#amt", m);
      const upd = () => {
        const a = Math.max(0, +inp.value || 0);
        const t = donorTotal(a);
        $("#bd", m).innerHTML = `<span>@${esc(app.dev)} receives (95%)</span><b>${money(a * (1 - FEE))}</b><span class="muted">Slop platform fee (5%)</span><b class="muted">${money(a * FEE)}</b><span class="muted">Card processing</span><b class="muted">${money(t - a)}</b><span style="border-top:1px solid var(--line);padding-top:4px">You pay</span><b style="border-top:1px solid var(--line);padding-top:4px">${money(t)}</b>`;
        $("#give", m).textContent = `Donate ${money(t)}`;
        $("#give", m).disabled = a < 0.5;
        $$(".amounts button", m).forEach(b => b.classList.toggle("on", +b.dataset.amt === a));
      };
      $(".amounts", m).onclick = e => { const b = e.target.closest("button"); if (b) { inp.value = b.dataset.amt; upd(); } };
      inp.oninput = upd;
      upd();
      $("#give", m).onclick = () => {
        const a = +inp.value;
        close();
        progressToast("Processing donation…", () => {
          S.donated += a; app.raised += a; app.donors += 1; save(); updateMe();
          toast(`♥ Thank you! <b>${money(a * (1 - FEE))}</b> paid to @${esc(app.dev)} via Stripe.<br><span class="muted">Mockup - no real money moved.</span>`, 5000);
          if (!location.hash.includes("/publish")) render();
        });
      };
    } });
}

function report(id, target = "") {
  const app = APP_BY_ID[id];
  const opts = [["app", `The whole app (${app.name})`], ...app.branches.filter(b => b.name !== app.tracked).map(b => [`branch:${b.name}`, `Community branch: ${b.name}`]), ...app.reviewList.map(r => [`review:${r.user}`, `Review by @${r.user}`])];
  modal(`<div class="modal-h"><h2>⚑ Report a problem</h2><button class="x" data-close>×</button></div>
    <label class="muted" style="font-size:12px">What are you reporting?</label>
    <select id="target">${opts.map(([v, l]) => `<option value="${esc(v)}" ${v === target ? "selected" : ""}>${esc(l)}</option>`).join("")}</select>
    <div class="muted" style="font-size:12px;margin:14px 0 6px">What's wrong?</div>
    <div class="reasons">${REPORT_REASONS.map(r => `<label><input type="radio" name="reason" value="${r.id}"><span>${r.label}${r.hint ? `<small>${r.hint}</small>` : ""}</span></label>`).join("")}</div>
    <div class="muted" style="font-size:12px;margin:14px 0 0">Details (optional) - file names, links, what happened</div>
    <textarea id="details" placeholder="e.g. the installer asked for admin rights and then opened a browser window to a casino site"></textarea>
    <div class="actions"><button class="btn" data-close>Cancel</button><button class="btn" id="send" disabled style="background:var(--bad);border-color:var(--bad);color:#2a0606">Send report</button></div>`,
  { onMount: (m, close) => {
      $(".reasons", m).onchange = () => ($("#send", m).disabled = false);
      $("#send", m).onclick = () => {
        const reason = $("input[name=reason]:checked", m).value;
        const rid = "SR-" + Math.floor(10000 + Math.random() * 89999);
        S.reports.push({ id: rid, app: id, target: $("#target", m).value, reason, at: Date.now() }); save();
        close();
        const serious = ["malware", "illegal", "scam", "notopen"].includes(reason);
        toast(`✓ Report <b>${rid}</b> filed.<br>${serious ? "An automated re-scan has started and a moderator will look within 24 hours." : "A moderator will review it. Thanks for keeping Slop clean."}`, 6000);
      };
    } });
}

// Two-button vote modal, shared by apps (gated on launching) and Humans (gated on a completed job).
function voteModal({ title, question, gate, prev, placeholder, extra = "", onPost }) {
  if (gate) return modal(`<div class="modal-h"><h2>${title}</h2><button class="x" data-close>×</button></div><p class="muted" style="margin:0">${gate}</p><div class="actions"><button class="btn slime" data-close>OK</button></div>`);
  modal(`<div class="modal-h"><h2>${title}</h2><button class="x" data-close>×</button></div>
    <p class="muted" style="margin:0 0 12px">${question}</p>
    <div class="vote-btns">
      <button data-v="not" class="${prev && !isSlopVote(prev) ? "on" : ""}"><span>👍</span><b>Not Slop</b><small>Made with care. Genuinely good.</small></button>
      <button data-v="slop" class="${prev && isSlopVote(prev) ? "on" : ""}"><span>👎</span><b>Slop</b><small>Low-effort AI output.</small></button>
    </div>
    ${extra}
    <textarea id="text" placeholder="${placeholder}">${esc(prev?.text || "")}</textarea>
    <div class="actions"><button class="btn" data-close>Cancel</button><button class="btn slime" id="post" ${prev ? "" : "disabled"}>Post</button></div>`,
  { onMount: (m, close) => {
      let vote = prev ? (isSlopVote(prev) ? "slop" : "not") : null;
      $(".vote-btns", m).onclick = e => { const b = e.target.closest("[data-v]"); if (!b) return; vote = b.dataset.v; $$(".vote-btns button", m).forEach(x => x.classList.toggle("on", x === b)); $("#post", m).disabled = false; };
      $("#post", m).onclick = () => {
        const text = $("#text", m).value.trim();
        if (text.length < 10) { $("#text", m).focus(); return toast("Add a sentence so others know why."); }
        onPost({ vote, slop: vote === "slop" ? 100 : 0, text }, m); close();
      };
    } });
}
function review(id, preset) {
  const app = APP_BY_ID[id], prev = S.myReviews[id] || (preset ? { vote: preset } : null);
  voteModal({
    title: `Is ${esc(app.name)} slop?`,
    question: "One vote per person. You can change it any time.",
    gate: canVote(id) ? "" : `Only people who've <b style="color:var(--text)">launched ${esc(app.name)} through Slop</b> can vote - the same launch that counts you in Slop Pulse. ${S.library[id] ? "Install it and press Play from your library." : "Add it to your library, install it and launch it first."}`,
    prev, placeholder: "What's good? What's sloppy? Which branch should people install?",
    onPost: ({ vote, slop, text }) => {
      S.myReviews[id] = { user: "you", vote, slop, text, hours: S.library[id]?.hours ?? 1, date: new Date().toISOString().slice(0, 10) };
      save();
      toast(`✓ Voted ${vote === "slop" ? "👎 Slop" : "👍 Not Slop"}`);
      location.hash = `#/app/${id}/reviews`; render();
    },
  });
}

/* ======================================================================
   Publish wizard
   ====================================================================== */
const PUB_STEPS = ["Source & stage", "Store page", "Builds & Pulse", "Donations & rules", "Scan & submit"];
const pub = {
  step: 0, repo: "", repoOk: false, branch: "main", stage: "released", visibility: "public",
  builds: { windows: null, mac: null, linux: null, ios: null, android: null },
  name: "", short: "", type: "Games", tags: new Set(), ai: "",
  pulse: null, donation: 3, attest: {}, demo: "none",
};
const FILE_EXT = { windows: { installer: "-Setup.exe", executable: ".exe", runner: "-portable.zip" }, mac: { installer: ".dmg", executable: ".app.zip", runner: "-portable.zip" }, linux: { installer: ".deb", executable: ".bin", runner: ".AppImage" }, ios: { ipa: ".ipa" }, android: { apk: ".apk" } };
const ATTESTS = [
  ["owns", "I wrote this or have the right to publish all its code and assets under the declared license"],
  ["noloot", "No loot boxes or paid random rewards"],
  ["nop2w", "No pay-to-win - paying never gives a gameplay or functional advantage"],
  ["nosales", "Nothing for sale - no in-app purchases, supporter packs, subscriptions or paid items"],
  ["nodonorperks", "Donations never unlock anything - no perks, items, ranks or access for donors"],
  ["noscam", "No fake currencies, countdown pressure or misleading offers"],
  ["nohidden", "No hidden telemetry, ads or network calls that aren't disclosed in the README"],
];
const isPriv = () => pub.visibility === "private";
const needsBuild = () => isPriv() || pub.stage !== "soon";
const pubIsGame = () => pub.type === "Games";
const needsPulse = () => pubIsGame() && !isPriv();

function publish() {
  const v = mount(`<div class="page">
    <h1 style="font-size:28px">Publish to Slop</h1>
    <p class="muted" style="margin:4px 0 22px">Free to publish. Your code must be public on GitHub, and everything gets scanned before it goes live. Start with a Coming soon page if you don't have a build yet.</p>
    <div class="pub">
      <div class="steps" id="steps"></div>
      <div id="step" class="form"></div>
    </div>
    <div class="section-h"><h2>Your projects &amp; submissions</h2><span class="sub">Release deadlines and what the scanners caught</span></div>
    <div class="subs" id="subs"></div>
  </div>`);
  drawSteps(v); drawStep(v); drawSubs(v);
}
function drawSubs(v) {
  const all = [...S.submissions, ...MY_SUBMISSIONS];
  const badge = { live: ["official", "LIVE"], rejected: ["bad", "REJECTED"], held: ["warn", "ON HOLD"], blocked: ["bad", "BLOCKED"] };
  $("#subs", v).innerHTML = all.map(s => {
    const app = s.appId && APP_BY_ID[s.appId];
    const l = app && lifecycle(app);
    const stageRow = app && isUpcoming(app) ? `<div class="note">
        <div style="display:flex;justify-content:space-between;gap:10px;flex-wrap:wrap;align-items:center"><span>${stageBadge(app)} <b style="color:var(--text)">${deadlineText(app)}</b> · ${num(interestCount(app))} interested</span>
        ${app.stage === "soon" ? `<button class="btn small slime" data-promote="${app.id}">Move to Beta →</button>` : `<button class="btn small slime" data-release="${app.id}">Fully release →</button>`}</div>
        <div class="progress" style="margin-top:8px"><i style="width:${Math.min(100, (l.used / l.limit) * 100)}%"></i></div></div>` : "";
    return `<div class="panel sub-row">
      <div><b>${esc(s.name)}</b>${s.check ? ` <span class="muted" style="font-size:12px">· failed: ${SCAN_CHECKS.find(c => c.id === s.check).name}</span>` : ""}</div>
      <span class="badge ${badge[s.status][0]}">${badge[s.status][1]}</span>
      <div class="note">${esc(s.note)}${s.appId ? ` · <a href="#/app/${s.appId}">View store page</a>` : ""}</div>
      ${stageRow}
    </div>`;
  }).join("");
  $("#subs", v).onclick = e => {
    const p = e.target.closest("[data-promote]"), r = e.target.closest("[data-release]");
    if (p) promoteToBeta(p.dataset.promote, () => drawSubs(v));
    if (r) { const app = APP_BY_ID[r.dataset.release]; setStage(app, { stage: "released", slop: null }); toast(`🎉 ${esc(app.name)} is fully released. Reviews are open.`); drawSubs(v); }
  };
}
function setStage(app, ch) {
  Object.assign(app, ch);
  S.stageChanges[app.id] = { ...(S.stageChanges[app.id] || {}), ...ch };
  save();
}
// Coming soon -> Beta requires at least one build (and, for games, a live Pulse handshake).
function promoteToBeta(id, done) {
  const app = APP_BY_ID[id];
  modal(`<div class="modal-h"><h2>Move ${esc(app.name)} to Beta</h2><button class="x" data-close>×</button></div>
    <p class="muted" style="margin:0 0 12px">Beta needs at least one installer, executable or portable runner. You'll then have ${STAGE_RULES.betaDays / 365 * 12} months to fully release.</p>
    <div class="build-opts">${Object.keys(PLATFORMS).map(os => `<label><input type="checkbox" value="${os}"><span><b>${PLATFORMS[os].label}</b><small>${os === "linux" ? "AppImage runner" : "Installer"}</small></span></label>`).join("")}</div>
    ${isGame(app) ? `<div class="rule-box" style="margin-top:12px">📶 Slop Pulse will be tested live against the build when you continue.</div>` : ""}
    <div class="actions"><button class="btn" data-close>Cancel</button><button class="btn slime" id="go" disabled>Attach &amp; move to Beta</button></div>`,
  { onMount: (m, close) => {
      m.onchange = () => ($("#go", m).disabled = !$$("input:checked", m).length);
      $("#go", m).onclick = () => {
        const builds = {};
        $$("input:checked", m).forEach(i => (builds[i.value] = { kind: i.value === "linux" ? "runner" : "installer", size: "64 MB" }));
        close();
        progressToast(`Scanning builds${isGame(app) ? " and testing Slop Pulse" : ""}…`, () => {
          setStage(app, { stage: "beta", stageSince: new Date().toISOString().slice(0, 10), builds, playing: isGame(app) ? 0 : undefined, peak: 0, version: "0.9.0-beta" });
          toast(`🧪 ${esc(app.name)} is in Beta. Everyone marked Interested has been notified.`, 5000);
          done();
        });
      };
    } });
}
function drawSteps(v) {
  $("#steps", v).innerHTML = PUB_STEPS.map((s, i) => `<div class="s ${i === pub.step ? "on" : i < pub.step ? "done" : ""}"><i>${i < pub.step ? "✓" : i + 1}</i><span>${s}</span></div>`).join("");
}
function stepValid() {
  switch (pub.step) {
    case 0: return pub.repoOk;
    case 1: return pub.name.trim().length >= 2 && pub.short.trim().length >= 10 && pub.tags.size >= 3;
    case 2: return (!needsBuild() || Object.values(pub.builds).some(Boolean)) && (!needsPulse() || pub.pulse === "ok");
    case 3: return ATTESTS.every(([k]) => pub.attest[k]) && pub.donation >= 0;
    default: return true;
  }
}
function navHtml(nextLabel = "Continue") {
  return `<div class="form-nav">
    <button class="btn" id="back" ${pub.step === 0 ? "disabled" : ""}>← Back</button>
    <button class="btn slime" id="next" ${stepValid() ? "" : "disabled"}>${nextLabel} →</button></div>`;
}
function drawStep(v) {
  // Fresh element per step so listeners from earlier steps don't pile up.
  const old = $("#step", v), el = old.cloneNode(false);
  old.replaceWith(el);
  const refreshNav = () => { const n = $("#next", el); if (n) n.disabled = !stepValid(); };
  const s = pub.step;

  if (s === 0) {
    el.innerHTML = `
      <div class="field"><label>Source repository</label>
        <input type="text" id="repo" placeholder="github.com/you/your-app" value="${esc(pub.repo)}">
        <div id="repo-msg"></div>
        <div class="hint">A public repo on GitHub, GitLab or Bitbucket with an open-source license - even for Coming soon pages. Private Slop can use a private repo you give Slop read access to.</div></div>
      <div class="field"><label>Visibility</label>
        <div class="radio-cards">
          <label><input type="radio" name="vis" value="public" ${!isPriv() ? "checked" : ""}><b>🌍 Public</b><span>On the store for everyone.</span></label>
          <label><input type="radio" name="vis" value="private" ${isPriv() ? "checked" : ""}><b>🔒 Private Slop</b><span>Only you can see it. Share with friends or player teams if you like. Same rules, no Slop Pulse.</span></label>
        </div></div>
      <div class="field ${isPriv() ? "hidden" : ""}" id="stage-field"><label>Release stage</label>
        <div class="radio-cards">
          <label><input type="radio" name="stage" value="soon" ${pub.stage === "soon" ? "checked" : ""}><b>Coming soon</b><span>No build needed. Must move to Beta within ${STAGE_RULES.soonDays} days.</span></label>
          <label><input type="radio" name="stage" value="beta" ${pub.stage === "beta" ? "checked" : ""}><b>Beta</b><span>At least one build. Must fully release within ${STAGE_RULES.betaDays / 365 * 12} months.</span></label>
          <label><input type="radio" name="stage" value="released" ${pub.stage === "released" ? "checked" : ""}><b>Full release</b><span>At least one build. Reviews and slop score open.</span></label>
        </div>
        <div class="hint">Upcoming projects with no commits for ${STAGE_RULES.deadAfterDays} days are labelled <b>dead</b>.</div></div>
      <div class="field"><label>Tracked branch</label>
        <select id="branch">${["main", "release", "stable", "dev"].map(b => `<option ${b === pub.branch ? "selected" : ""}>${b}</option>`).join("")}</select>
        <div class="hint">Your <b>official</b> version. Pushes here queue an update, get scanned, then reach everyone who installed it.</div></div>
      ${navHtml()}`;
    const check = () => {
      pub.repo = $("#repo", el).value.trim();
      const ok = /^(https?:\/\/)?(www\.)?(github\.com|gitlab\.com|bitbucket\.org)\/[\w.-]+\/[\w.-]+\/?$/i.test(pub.repo);
      pub.repoOk = ok;
      $("#repo-msg", el).innerHTML = !pub.repo ? "" : ok
        ? `<div class="ok">✓ Public repository found · License: <b>MIT</b> (OSI-approved) · 4 branches · last commit 2 hours ago <span class="faint">(simulated)</span></div>`
        : `<div class="err">${/drive\.google|dropbox/i.test(pub.repo) ? "Slop needs a Git repository, not a file-sharing link." : "Enter a repo like github.com/you/your-app (GitLab and Bitbucket work too)"}</div>`;
      refreshNav();
    };
    $("#repo", el).oninput = check; check();
    el.addEventListener("change", e => {
      if (e.target.name === "stage") { pub.stage = e.target.value; pub.pulse = null; }
      if (e.target.name === "vis") { pub.visibility = e.target.value; pub.pulse = null; $("#stage-field", el).classList.toggle("hidden", isPriv()); }
    });
    $("#branch", el).onchange = e => (pub.branch = e.target.value);
  }

  if (s === 1) {
    el.innerHTML = `
      <div class="field"><label>Name</label><input type="text" id="name" value="${esc(pub.name)}" placeholder="My Great App"></div>
      <div class="field"><label>One-line pitch</label><input type="text" id="short" value="${esc(pub.short)}" maxlength="140" placeholder="What is it, and why would someone want it?"></div>
      <div class="field"><label>Main category</label><select id="type">${TAG_GROUPS.slice(0, 5).map(g => `<option ${g.name === pub.type ? "selected" : ""}>${g.name}</option>`).join("")}</select>
        <div class="hint" id="type-hint"></div></div>
      <div class="field"><label>Tags <span class="muted" id="tagcount" style="font-weight:400"></span></label>
        <div class="hint" style="margin:0 0 10px">Pick 3-10. Mix broad and specific - they decide which store categories you appear in, and who sees you in “For you”.</div>
        <div class="tag-pick" id="tags">${TAG_GROUPS.map(g => `<div class="grp"><b>${g.icon} ${g.name}</b><div class="chips">${g.tags.map(t => `<button class="chip ${pub.tags.has(t) ? "on" : ""}" data-tag="${esc(t)}">${esc(t)}</button>`).join("")}</div></div>`).join("")}</div></div>
      <div class="field"><label>AI tools used</label><input type="text" id="ai" value="${esc(pub.ai)}" placeholder="e.g. Claude, Stable Diffusion, Copilot">
        <div class="hint">Shown on your store page as “Made with”. Honesty here helps reviewers.</div></div>
      ${navHtml()}`;
    const upd = () => {
      $("#tagcount", el).textContent = `(${pub.tags.size} selected)`;
      $("#type-hint", el).innerHTML = needsPulse() ? "📶 Public games must include <b>Slop Pulse</b> - the player-count ping. You'll set it up in the next step." : pubIsGame() ? "🔒 Private Slop games don't need Slop Pulse." : "";
      refreshNav();
    };
    $("#name", el).oninput = e => { pub.name = e.target.value; upd(); };
    $("#short", el).oninput = e => { pub.short = e.target.value; upd(); };
    $("#type", el).onchange = e => { pub.type = e.target.value; pub.pulse = null; upd(); };
    $("#ai", el).oninput = e => (pub.ai = e.target.value);
    $("#tags", el).onclick = e => {
      const b = e.target.closest("[data-tag]"); if (!b) return;
      const t = b.dataset.tag;
      if (pub.tags.has(t)) pub.tags.delete(t); else if (pub.tags.size < 10) pub.tags.add(t); else return toast("10 tags max");
      b.classList.toggle("on", pub.tags.has(t)); upd();
    };
    upd();
  }

  if (s === 2) {
    const slug = (pub.repo.split("/").pop() || "my-app").replace(/\.git$/, "");
    el.innerHTML = `
      <div class="field"><label>Builds ${needsBuild() ? "" : `<span class="badge soon">OPTIONAL FOR COMING SOON</span>`}</label>
        <div class="hint" style="margin:0 0 10px">${needsBuild() ? "Slop Build makes these for you from your repo: accept one ready-made file and every push builds, signs and attaches them to your repo's release page. Or attach your own. We check the signature and that each build came from your tagged commit, and Slop never stores the files." : "Coming soon pages don't need a build. You'll need at least one to move to Beta."}</div>
        <div class="build-slots" id="slots"></div></div>
      <div id="need"></div>
      ${needsPulse() ? `<div class="panel panel-pad pulse-box" id="pulse"></div>` : ""}
      ${navHtml()}`;
    const drawSlots = () => {
      const count = Object.values(pub.builds).filter(Boolean).length;
      $("#slots", el).innerHTML = Object.keys(PLATFORMS).map(os => {
        const b = pub.builds[os];
        return `<div class="slot ${b ? "filled" : ""}">
          <h4>${icon(os)} ${PLATFORMS[os].label}</h4>
          ${b ? `<select data-kind="${os}">${KINDS_FOR[os].map(k => `<option value="${k}" ${k === b.kind ? "selected" : ""}>${BUILD_KINDS[k]}</option>`).join("")}</select>
                 <div class="file">📦 ${esc(b.file)}</div><div class="muted" style="font-size:12px">${b.size} · SHA-256 recorded</div>
                 <button class="btn small ghost danger" data-remove="${os}">Remove</button>`
              : `<div class="muted" style="font-size:12px">${isPhoneOS(os) ? (os === "ios" ? "iOS app - EU marketplace or App Store/TestFlight link" : "Android APK") : "Installer, executable or portable runner"}</div>
                 <button class="btn small" data-attach="${os}">+ Attach build</button>`}
        </div>`;
      }).join("");
      $("#need", el).className = `rule-box ${count || !needsBuild() ? "met" : "need"}`;
      $("#need", el).innerHTML = !needsBuild() && !count ? `✓ <b>No build needed for Coming soon.</b> Your source still goes through every scan.`
        : count ? `✓ <b>${count} of 5 platforms covered.</b> ${count < 5 ? "Missing platforms can be filled later by you - or by the community through branches." : "Full coverage - nice."}`
        : `<b>At least one build is required.</b> Attach an installer, an executable or a portable runner for any platform.`;
      drawPulse();
      refreshNav();
    };
    const drawPulse = () => {
      const box = $("#pulse", el); if (!box) return;
      const hasBuild = Object.values(pub.builds).some(Boolean);
      const mode = hasBuild ? "live" : "source";
      box.innerHTML = `<div style="display:flex;gap:12px;align-items:flex-start;flex-wrap:wrap">
        <span style="font-size:26px">📶</span>
        <div style="flex:1;min-width:240px"><b>Slop Pulse - required for games</b>
          <div class="muted" style="font-size:13px;margin:4px 0 10px">Lets Slop show “N playing now”. It's a tiny open-source library that sends one thing: a signed “still playing” heartbeat. No device IDs, no gameplay data, nothing else.</div>
          <pre class="code">import { pulse } from "slop-pulse";
pulse.start();   // reads the one-time token Slop passes at launch</pre>
          <div class="muted" style="font-size:12px;margin-top:8px">${mode === "live" ? "We'll launch your build in a sandbox and wait for a signed heartbeat." : "No build yet, so we'll check the SDK is in your source. The live handshake runs when you add your first build."}</div>
          <div id="pulse-res" style="margin-top:10px">${pub.pulse === "ok" ? `<div class="ok">✓ ${mode === "live" ? "Signed heartbeat received · payload: app id + launch token only" : "slop-pulse found in package.json · pulse.start() called in src/main.ts"}</div>` : ""}</div>
        </div>
        <button class="btn ${pub.pulse === "ok" ? "" : "slime"}" id="pulse-test">${pub.pulse === "ok" ? "Re-test" : mode === "live" ? "Run handshake test" : "Check source"}</button>
      </div>`;
      $("#pulse-test", box).onclick = () => {
        const res = $("#pulse-res", box), btn = $("#pulse-test", box);
        btn.disabled = true;
        res.innerHTML = `<span class="muted">${mode === "live" ? "Launching build in sandbox · waiting for heartbeat…" : "Scanning source for slop-pulse…"}</span>`;
        setTimeout(() => { pub.pulse = "ok"; drawPulse(); refreshNav(); }, 1600);
      };
    };
    el.addEventListener("click", e => {
      const a = e.target.closest("[data-attach]"), r = e.target.closest("[data-remove]");
      if (a) { const os = a.dataset.attach, kind = os === "linux" ? "runner" : KINDS_FOR[os][0]; pub.builds[os] = { kind, file: slug + "-1.0.0" + FILE_EXT[os][kind], size: 20 + Math.floor(Math.random() * 60) + " MB" }; pub.pulse = null; drawSlots(); }
      if (r) { pub.builds[r.dataset.remove] = null; pub.pulse = null; drawSlots(); }
    });
    el.addEventListener("change", e => {
      const k = e.target.closest("[data-kind]");
      if (k) { const os = k.dataset.kind; pub.builds[os].kind = k.value; pub.builds[os].file = slug + "-1.0.0" + FILE_EXT[os][k.value]; drawSlots(); }
    });
    drawSlots();
  }

  if (s === 3) {
    el.innerHTML = `
      ${isPriv() ? `<div class="rule-box met">🔒 <b>Private Slop has no store page or Donate button.</b> It still has to follow every rule below.</div>` : ""}
      <div class="field ${isPriv() ? "hidden" : ""}"><label>Suggested donation</label>
        <div style="display:flex;gap:12px;align-items:center"><span class="muted">$</span><input type="number" id="don" min="0" step="0.5" value="${pub.donation}" style="max-width:120px"><span class="muted" style="font-size:13px">This is the default on your Donate button. People can always choose any amount - including nothing.</span></div>
        <div class="panel panel-pad" style="margin-top:12px;display:flex;gap:14px;align-items:center;flex-wrap:wrap">
          <button class="btn slime" id="don-prev">♥ Donate $${pub.donation}</button>
          <div class="breakdown" style="flex:1;min-width:220px" id="don-bd"></div>
        </div></div>
      <div class="field ${isPriv() ? "hidden" : ""}"><label>Where donations go</label>
        <div class="rule-box met">💳 Donations are paid <b>straight into your own Stripe account</b> - Slop never holds the money. You'll connect Stripe after publishing (simulated). Splitting with collaborators is up to you. Donations may be taxable income for you.</div></div>
      <div class="field ${isPriv() ? "hidden" : ""}"><label>In-app purchases</label>
        <div class="rule-box">🚫 <b>Not allowed.</b> Apps on Slop can't sell anything - no items, supporter packs, subscriptions or loot boxes. Detected automatically.</div></div>
      <div class="field"><label>Publisher agreement</label>
        <div class="checks">${ATTESTS.map(([k, l]) => `<label><input type="checkbox" data-att="${k}" ${pub.attest[k] ? "checked" : ""}><span>${l}</span></label>`).join("")}</div>
        <div class="hint">These are checked automatically too - see <a href="#/rules">the rules</a>.</div></div>
      ${navHtml("Review")}`;
    const upd = () => {
      const d = pub.donation;
      $("#don-prev", el).textContent = `♥ Donate ${money(d)}`;
      $("#don-bd", el).innerHTML = `<span>You receive per donation</span><b>${money(d * (1 - FEE))}</b><span class="muted">Slop's 5% platform fee</span><b class="muted">${money(d * FEE)}</b><span class="muted">Card processing (paid by the donor)</span><b class="muted">${money(donorTotal(d) - d)}</b>`;
      refreshNav();
    };
    $("#don", el).oninput = e => { pub.donation = Math.max(0, +e.target.value || 0); upd(); };
    el.addEventListener("change", e => {
      if (e.target.dataset.att) { pub.attest[e.target.dataset.att] = e.target.checked; refreshNav(); }
    });
    upd();
  }

  if (s === 4) {
    const builds = Object.entries(pub.builds).filter(([, b]) => b);
    const checks = SCAN_CHECKS.filter(c => !c.games || needsPulse());
    el.innerHTML = `
      <div class="panel panel-pad">
        <div style="display:flex;gap:16px;align-items:center;flex-wrap:wrap">
          <div class="art" style="width:120px;height:68px;background:linear-gradient(135deg,#365314,#0ea5e9)"><span class="em" style="font-size:30px">🆕</span></div>
          <div style="flex:1;min-width:200px"><b style="font-size:17px">${esc(pub.name)}</b> ${isPriv() ? `<span class="badge private">🔒 PRIVATE SLOP</span>` : `<span class="badge ${pub.stage === "soon" ? "soon" : pub.stage === "beta" ? "beta" : "official"}">${STAGE_LABEL[pub.stage].toUpperCase()}</span>`}<div class="muted" style="font-size:13px">${esc(pub.short)}</div></div>
        </div>
        <dl class="kv" style="margin-top:14px">
          <dt>Source</dt><dd>${esc(pub.repo)} · tracked <code>${esc(pub.branch)}</code></dd>
          <dt>Builds</dt><dd>${builds.length ? builds.map(([os, b]) => `${PLATFORMS[os].label} (${BUILD_KINDS[b.kind]})`).join(", ") : "None yet (Coming soon)"}</dd>
          ${needsPulse() ? `<dt>Slop Pulse</dt><dd>✓ ${builds.length ? "live handshake passed" : "SDK found in source"}</dd>` : pubIsGame() ? `<dt>Slop Pulse</dt><dd class="muted">Not required for Private Slop</dd>` : ""}
          <dt>Tags</dt><dd>${[...pub.tags].map(esc).join(", ")}</dd>
          ${isPriv() ? `<dt>Visible to</dt><dd>Only you (share it after it passes)</dd>` : `<dt>Donation</dt><dd>${money(pub.donation)} suggested · paid to your Stripe account · nothing for sale</dd>`}
        </dl>
      </div>
      <div class="demo-toggle">🧪 <b>Mockup demo:</b> plant a problem for the scanner to find
        <select id="demo">
          <option value="none">Nothing - clean submission</option>
          <option value="malware">Installer phones home to an unknown server</option>
          <option value="license">Copied GPL code into an MIT project</option>
          <option value="monetization">Sneaky loot box in the shop code</option>
          <option value="secrets">API key committed to the repo</option>
          ${needsPulse() ? `<option value="pulse">Pulse also sends a device fingerprint</option>` : ""}
        </select></div>
      <div class="panel panel-pad" id="scan-panel">
        <div style="display:flex;justify-content:space-between;align-items:center;margin-bottom:10px;gap:10px"><b>Security &amp; policy scan</b><span class="muted" id="scan-count">${checks.length} checks</span></div>
        <div class="progress"><i id="scan-bar" style="width:0"></i></div>
        <div class="scan-list" id="scan-list" style="margin-top:6px">${scanRowsHtml(checks.map(c => ({ id: c.id, status: "wait" })))}</div>
      </div>
      <div id="result"></div>
      <div class="form-nav"><button class="btn" id="back">← Back</button><button class="btn slime" id="submit">Submit for scanning</button></div>`;
    $("#demo", el).value = $("#demo", el).querySelector(`option[value="${pub.demo}"]`) ? pub.demo : "none";
    $("#demo", el).onchange = e => (pub.demo = e.target.value);
    $("#submit", el).onclick = () => runScan(v, checks);
  }

  $("#back", el)?.addEventListener("click", () => { pub.step--; drawSteps(v); drawStep(v); });
  $("#next", el)?.addEventListener("click", () => { if (!stepValid()) return; pub.step++; drawSteps(v); drawStep(v); });
}

const FAIL_NOTES = {
  malware: { check: "behavior", note: "Sandbox: installer opened a connection to 185.xx.xx.12:4444 (undisclosed) and wrote to the startup folder.", status: "blocked", summary: "Your installer behaves differently from your source code. It's been blocked and the account flagged for a human review." },
  license: { check: "license", note: "14 files in src/render/ match 'pixelkit' (GPL-3.0). This repo is licensed MIT.", status: "held", summary: "Relicense to GPL-3.0, or remove/replace the matching files, then resubmit. Nothing else needs to change." },
  monetization: { check: "monetization", note: "src/shop/crate.ts:88 sells a 'mystery crate' with randomized contents for real money.", status: "rejected", summary: "Loot boxes and in-app purchases aren't allowed on Slop. Remove the shop and resubmit." },
  secrets: { check: "secrets", note: "A live-looking API key was found in config/prod.env (commit a41f9c2).", status: "held", summary: "Revoke the key, remove it from git history, and resubmit. We never publish apps with leaked credentials." },
  pulse: { check: "pulse", note: "Heartbeat payload includes a hardware fingerprint (machine-id). Pulse may only send the app id and launch token.", status: "rejected", summary: "Slop Pulse is a player count, not tracking. Remove the extra fields from the heartbeat and resubmit." },
};
function runScan(v, checks) {
  const sub = $("#submit", v), back = $("#back", v);
  sub.disabled = back.disabled = true; $("#demo", v).disabled = true;
  $("#result", v).innerHTML = "";
  const fail = FAIL_NOTES[pub.demo];
  const results = checks.map(c => ({ id: c.id, status: "wait" }));
  let i = 0;
  const step = () => {
    if (i > 0) {
      const prev = results[i - 1];
      if (fail && fail.check === prev.id) { prev.status = "fail"; prev.note = fail.note; }
      else prev.status = "pass";
    }
    if (i < results.length && !(fail && results[i - 1]?.status === "fail")) {
      results[i].status = "run";
      $("#scan-list", v).innerHTML = scanRowsHtml(results);
      $("#scan-bar", v).style.width = (i / results.length) * 100 + "%";
      $("#scan-count", v).textContent = `${i}/${results.length} checks`;
      i++;
      setTimeout(step, 450 + Math.random() * 450);
      return;
    }
    results.forEach(r => { if (r.status === "wait") r.note = "Skipped - an earlier check failed"; });
    $("#scan-list", v).innerHTML = scanRowsHtml(results);
    $("#scan-bar", v).style.width = "100%";
    $("#scan-count", v).textContent = fail ? "stopped" : `${results.length}/${results.length} checks`;
    const name = pub.name || "Untitled";
    if (fail) {
      S.submissions.unshift({ name, status: fail.status, check: fail.check, note: fail.note });
      $("#result", v).innerHTML = `<div class="result-banner bad"><span class="big-ic">${fail.status === "blocked" ? "⛔" : "✋"}</span><div><b style="font-size:16px">${fail.status === "blocked" ? "Blocked" : fail.status === "held" ? "On hold" : "Rejected"}: ${SCAN_CHECKS.find(c => c.id === fail.check).name}</b><div class="muted" style="font-size:13px">${esc(fail.summary)}</div></div></div>`;
    } else if (isPriv()) {
      const id = (pub.repo.split("/").pop() || "private").replace(/\W+/g, "-") + "-" + Date.now().toString(36);
      S.privates.unshift({ id, name, owner: "you", type: pub.type, art: { emoji: "🔒", from: "#1e293b", to: "#64748b" }, short: pub.short, repo: pub.repo,
        builds: Object.fromEntries(Object.entries(pub.builds).filter(([, b]) => b).map(([os, b]) => [os, { kind: b.kind, size: b.size }])), shared: { friends: [], teams: [] }, scanned: new Date().toISOString().slice(0, 10) });
      S.submissions.unshift({ name, status: "live", note: `Private Slop · passed all ${results.length} checks (simulated)` });
      $("#result", v).innerHTML = `<div class="result-banner ok"><span class="big-ic">🔒</span><div style="flex:1"><b style="font-size:16px">${esc(name)} is in your Private Slop</b><div class="muted" style="font-size:13px">Only you can see it. Share it with friends or player teams whenever you like.</div></div><button class="btn slime" data-share="${id}">Share…</button></div>`;
    } else {
      const what = { soon: `Your Coming soon page is live. You have ${STAGE_RULES.soonDays} days to add a build and move to Beta.`, beta: `Your beta is live. You have ${STAGE_RULES.betaDays / 365 * 12} months to fully release.`, released: "It's in the store, unrated until the first reviews come in." }[pub.stage];
      S.submissions.unshift({ name, status: "live", note: `${STAGE_LABEL[pub.stage]} · passed all ${results.length} checks (simulated)` });
      $("#result", v).innerHTML = `<div class="result-banner ok"><span class="big-ic">🎉</span><div><b style="font-size:16px">Approved - ${esc(name)} is live!</b><div class="muted" style="font-size:13px">${what} Pushes to <code>${esc(pub.branch)}</code> queue updates automatically.</div></div></div>`;
    }
    save(); drawSubs(v);
    back.disabled = false; $("#demo", v).disabled = false;
    sub.disabled = false; sub.textContent = "Run again";
  };
  step();
}

/* ======================================================================
   Rules
   ====================================================================== */
function rules() {
  let n = 0;
  const R = (title, body) => `<div class="panel rule"><div class="n">RULE ${++n}</div><h3>${title}</h3>${body}</div>`;
  mount(`<div class="page">
    <h1 style="font-size:28px">The Slop rules</h1>
    <p class="muted" style="margin:4px 0 22px;max-width:760px">Slop is a store for AI-generated apps and games. We don't care how something was made - we care that it's honest, scanned, open and free. These rules are enforced by automated scanning on every upload and by people like you reporting problems.</p>
    <div class="rules">
      ${R("Everything is open source", `<p>Every app - including Coming soon pages - must link a public repository (GitHub, GitLab or Bitbucket) under an OSI-approved license. The code in the repo must be the code that runs - builds are signed by the publisher and checked against the tagged commit.</p>`)}
      ${R("Release stages", `<p>Projects move forward or get labelled:</p><ul><li><b>Coming soon</b> - source + scans, no build. Max ${STAGE_RULES.soonDays} days.</li><li><b>Beta</b> - at least one build. Max ${STAGE_RULES.betaDays / 365 * 12} months.</li><li><b>Released</b> - reviews and slop score open.</li><li><b>Dead</b> - any upcoming project with no commits for ${STAGE_RULES.deadAfterDays} days.</li></ul><p style="margin-top:8px">Missing a deadline delists the page until it moves to the next stage. People can mark upcoming projects <b>Interested</b> to add them to their library.</p>`)}
      ${R("Ship something runnable", `<p>Beta and full releases need at least one of these, for at least one platform:</p><ul><li>An installer</li><li>An executable</li><li>A portable runner (AppImage, zip…)</li></ul>`)}
      ${R("Slop Pulse (games only)", `<p>Games must include Slop Pulse so the store can show how many people are playing. It sends <b>only</b> a signed heartbeat - no device IDs, no gameplay data.</p><ul><li>Slop gives the game a one-time token when it launches it</li><li>Only tokens from real, signed-in installs count; one per account per game</li><li>Copies launched outside Slop simply aren't counted</li><li>Community builds of games need Pulse too</li></ul>`)}
      ${R("Branches are encouraged", `<p>Publishers choose one <b>tracked</b> branch as the official version; pushes there queue updates. Anyone can register a branch with mods, fixes or ports.</p><p>If a branch adds a build for a platform the publisher doesn't support, players see it as a <b>community build</b> with clear caveats and a link to its README.</p>`)}
      ${R("Security before anything", `<p>Before anything reaches the store - and on every update - it goes through:</p><ul>${SCAN_CHECKS.map(c => `<li><b>${c.name}</b> - ${c.detail[0].toLowerCase() + c.detail.slice(1)}</li>`).join("")}</ul>`)}
      ${R("Nothing illegal", `<p>No illegal content of any kind, no sexually explicit content, no pirated or license-violating code or assets, no malware. Matches are blocked automatically and escalated to human moderators.</p>`)}
      ${R("Free to download. Always.", `<p>Nobody pays to download anything. Adding to your library is free; installing is a separate step. Each app has a <b>Donate</b> button: the publisher sets a suggested amount, and people can give whatever they like - or nothing. Donations unlock nothing, and apps can't tell who donated. Apps that run servers can also offer a monthly <b>server donation</b> you can cancel any time - it never controls who can play.</p>`)}
      ${R("A flat 5% platform fee. That's it.", `<p>Donations are paid straight into the publisher's own Stripe account - Slop never holds the money. Slop charges the publisher a 5% platform fee for hosting and security scanning, and card processing is added to the donor's total, so the publisher gets 95% of what the donor chose. No listing fees, no surprises.</p><div class="split-bar" style="margin-top:12px"><i></i><i></i></div><div class="slop-scale"><span>95% publisher</span><span>5% Slop</span></div>`)}
      ${R("Nothing for sale", `<p>Apps on Slop don't sell anything. Not allowed:</p><ul><li>In-app purchases, cosmetic items or supporter packs</li><li>Subscriptions or paid access</li><li>Loot boxes / paid random rewards</li><li>Pay-to-win or paid power</li><li>Premium currencies</li><li>Perks, ranks or items for donors</li></ul>`)}
      ${R("Anti-scam detection", `<p>Scanners look for fake giveaways, phishing links, credential prompts, undisclosed ads, crypto miners and "too good to be true" offers. Anything that misleads players about what they're getting is removed.</p>`)}
      ${R("Slop or Not Slop", `<p>Every app has two buttons: <b>👍 Not Slop</b> and <b>👎 Slop</b>. Only people who've <b>launched it through Slop</b> can vote, one vote each. Voting opens at full release, and the store shows the share of Not Slop votes and how it changes month to month.</p><div style="display:flex;flex-direction:column;gap:6px;margin-top:8px">${SLOP_TIERS.map(t => `<div style="display:flex;gap:10px;align-items:center">${slopHtml(100 - Math.max(t.min, 5), "sm", false)}<b style="width:190px">${t.label}</b></div>`).join("")}</div><p style="margin-top:8px">Humans profiles use the same two buttons - but only clients who finished a job can vote.</p>`)}
      ${R("One account, any sign-in", `<p>Sign in with Google, Apple, Microsoft, GitHub or Discord - Slop never stores a password. One account means one vote per app and one Slop Pulse count per game. Accounts are for ages 16 and up. In the UK, 18+ games stay hidden until you pass an age check.</p>`)}
      ${R("Desktop and phone", `<p>Slop runs on Windows, macOS, Linux, iOS and Android, and publishers can ship builds for any of them. From a computer you can send a phone app to a linked phone.</p><p class="muted">iOS: how Slop works on iPhone is still being decided.</p>`)}
      ${R("Private Slop", `<p>Publish something just for you - same rules (at least one build, every scan) except Slop Pulse, and the repo can be private as long as Slop can read it. It never appears in the store or search.</p><p>Share it with specific friends or <b>player teams</b>. It can't be shared with or advertised to <b>official teams</b>.</p>`)}
      ${R("Teams", `<p><b>Player teams</b> are open to anyone - for game nights, picks and sharing Private Slop. <b>Official teams</b> are developer studios verified through their GitHub organization, and show the apps they publish.</p>`)}
      ${R("Humans", `<p>A separate directory of artists, modellers, musicians, writers, voice actors and testers developers can hire. Every profile must disclose how it uses AI, and only clients who finished a job can vote 👍 Not Slop or 👎 Slop.</p><p>You hire and pay the creative directly - the agreement is between the two of you, and Slop doesn't settle disputes.</p>`)}
      ${R("Report anything", `<p>Every app, branch and review has a <b>Report</b> button. Reports about malware, illegal content, scams or source mismatches trigger an immediate automated re-scan and a human review within 24 hours.</p>`)}
      ${R("Tags you can actually browse", `<p>Every app carries several tags, from broad (Games, Tools, Creative) to specific (Roguelike, Note-taking, Pixel Art). The store has a page for every category and every tag.</p><p><a href="#/browse">Browse all categories →</a></p>`)}
    </div>
  </div>`);
}

/* ======================================================================
   Accounts: sign in with an account you already have
   ====================================================================== */
const PROVIDERS = [
  ["Google", "G", "#ea4335"], ["Apple", "", "#e5e7eb"], ["Microsoft", "⊞", "#00a4ef"], ["GitHub", "", "#a78bfa"], ["Discord", "◉", "#5865f2"],
];
function requireAuth(fn) { if (S.user) fn(); else signIn(fn); }
function signIn(then) {
  modal(`<div class="signin">
      <svg viewBox="0 0 32 32" class="signin-logo"><use href="#logo"/></svg>
      <h2>Sign in to Slop</h2>
      <p class="muted">Use an account you already have. Slop only receives your name and email - never a password.</p>
      <div class="providers">${PROVIDERS.map(([n, g, c]) => `<button class="btn provider" data-p="${n}"><span class="g" style="color:${c}">${g || n[0]}</span>Continue with ${n}</button>`).join("")}</div>
      <div id="step2" class="hidden">
        <p class="muted" style="margin:14px 0 6px;font-size:13px" id="via"></p>
        <input type="text" id="uname" maxlength="24" value="slopfan" aria-label="Username">
        <button class="btn slime" id="finish" style="width:100%;margin-top:10px">Create account</button>
      </div>
      <p class="faint" style="font-size:11px;margin:14px 0 0">Your account is also what makes Slop Pulse player counts hard to fake - one account counts once.</p>
    </div>`,
  { onMount: (m, close) => {
      let provider;
      $(".providers", m).onclick = e => {
        const b = e.target.closest("[data-p]"); if (!b) return;
        provider = b.dataset.p;
        $(".providers", m).classList.add("hidden");
        $("#step2", m).classList.remove("hidden");
        $("#via", m).textContent = `✓ Signed in with ${provider}. Pick a username:`;
        $("#uname", m).select();
      };
      $("#finish", m).onclick = () => {
        const name = $("#uname", m).value.trim().replace(/[^\w.-]/g, "") || "slopfan";
        S.user = { name, provider }; save(); close(); updateMe();
        toast(`👋 Welcome, <b>${esc(name)}</b>! Signed in with ${provider}.`);
        render();
        then?.();
      };
    } });
}
function accountModal() {
  const u = S.user;
  modal(`<div class="modal-h"><h2>@${esc(u.name)}</h2><button class="x" data-close>×</button></div>
    <dl class="kv"><dt>Signed in with</dt><dd>${esc(u.provider)}</dd><dt>Donated</dt><dd>${money(S.donated)}</dd>
      <dt>Teams</dt><dd>${Object.keys(S.teams).map(id => TEAM_BY_ID(id)?.name).filter(Boolean).map(esc).join(", ") || "None"}</dd>
      <dt>Friends</dt><dd>${FRIENDS.map(f => "@" + esc(f)).join(", ")}</dd></dl>
    <div class="section-h" style="margin:20px 0 8px"><h2 style="font-size:12px">Your devices</h2></div>
    <div class="devices">${devicesHtml()}</div>
    <div class="actions"><button class="btn" id="link">📱 Link a phone</button><button class="btn danger" id="out">Sign out</button></div>`,
  { onMount: (m, close) => {
      $("#out", m).onclick = () => { S.user = null; save(); close(); updateMe(); toast("Signed out."); render(); };
      $("#link", m).onclick = () => { close(); linkPhone(); };
    } });
}
function devicesHtml() {
  const here = { name: `This ${isPhoneOS(S.os) ? "phone" : "computer"}`, os: S.os };
  return [here, ...S.devices].map((d, i) => `<div class="device">${icon(d.os)}<b>${esc(d.name)}</b><span class="muted">${PLATFORMS[d.os].label}${i === 0 ? " · current" : ""}</span></div>`).join("");
}
function linkPhone(then) {
  modal(`<div class="modal-h"><h2>Link a phone</h2><button class="x" data-close>×</button></div>
    <p class="muted" style="margin:0 0 14px">Open Slop on your phone, go to <b>Account → Link this device</b>, and scan this code.</p>
    <div class="qr">${Array.from({ length: 121 }, (_, i) => `<i style="opacity:${(i * 7919 % 13) > 5 || [0, 1, 2, 10, 11, 12, 22, 110, 120, 98, 108, 109].includes(i) ? 1 : 0}"></i>`).join("")}</div>
    <p class="faint" style="font-size:12px;text-align:center">(Mockup - pick a phone to pretend you scanned it)</p>
    <div class="actions"><button class="btn" data-os="ios">iPhone</button><button class="btn" data-os="android">Android phone</button></div>`,
  { onMount: (m, close) => {
      m.addEventListener("click", e => {
        const b = e.target.closest(".actions [data-os]"); if (!b) return;
        const os = b.dataset.os, name = os === "ios" ? "iPhone 16" : "Galaxy S25";
        S.devices.push({ name, os }); save(); close();
        toast(`📱 <b>${name}</b> linked to your account.`);
        then?.();
      });
    } });
}
// "Send to phone" lets a desktop user install phone builds remotely on a linked device.
function sendToPhoneHtml(app) {
  if (isPhoneOS(S.os)) return "";
  const phoneBuilds = Object.keys(PLATFORMS).filter(os => isPhoneOS(os) && runsOn(app, os));
  if (!phoneBuilds.length) return "";
  const devices = S.devices.filter(d => phoneBuilds.includes(d.os));
  return `<div class="send-phone">
    ${devices.map(d => `<div class="device">${icon(d.os)}<b>${esc(d.name)}</b><span class="muted">${PLATFORMS[d.os].label}</span><button class="btn small" data-send="${esc(d.name)}">Send to phone</button></div>`).join("")}
    ${devices.length ? "" : `<div class="muted" style="font-size:13px">Available for ${phoneBuilds.map(os => PLATFORMS[os].label).join(" and ")}, but you haven't linked a matching phone.</div>`}
    <button class="btn small ghost" data-linkphone>+ Link ${devices.length ? "another" : "a"} phone</button>
    ${phoneBuilds.includes("ios") ? `<div class="faint" style="font-size:11px">iOS: installs through Slop's EU marketplace, or opens App Store/TestFlight elsewhere.</div>` : ""}
  </div>`;
}
function wirePhoneSend(app) {
  return (m, close) => {
    m.addEventListener("click", e => {
      const send = e.target.closest("[data-send]"), link = e.target.closest("[data-linkphone]");
      if (send) { send.disabled = true; send.textContent = "✓ Sent"; toast(`📲 Sent <b>${esc(app.name)}</b> to ${esc(send.dataset.send)}. It'll install next time the phone is online.`, 4500); }
      if (link) { $(".modal-bg:last-of-type [data-close]")?.click(); linkPhone(() => installPrompt(app.id, null)); }
    });
  };
}

/* ======================================================================
   Private Slop
   ====================================================================== */
// Visible: mine, or shared with me directly, or shared with a player team I'm in.
const visiblePrivates = () => S.privates.filter(p => p.owner === "you" || p.shared.friends.includes("you") || p.shared.teams.some(t => S.teams[t]));
function sharedLabel(p) {
  const n = p.shared.friends.length + p.shared.teams.length;
  if (!n) return "Only you";
  return [...p.shared.friends.map(f => "@" + f), ...p.shared.teams.map(t => TEAM_BY_ID(t)?.name || t)].join(", ");
}
function privatePage(id) {
  const p = S.privates.find(x => x.id === id);
  if (!p || !visiblePrivates().includes(p)) return mount(`<div class="page"><div class="empty-state">🔒 This Private Slop doesn't exist, or hasn't been shared with you.</div></div>`);
  const mine = p.owner === "you";
  const fake = { ...p, isPrivate: true, scan: { date: p.scanned, results: SCAN_CHECKS.map(c => ({ id: c.id, status: "pass" })) } };
  const builds = Object.entries(p.builds);
  mount(`<div class="page">
    <div class="crumbs"><a href="#/library">Library</a> › Private Slop › ${esc(p.name)}</div>
    <div class="app-head">
      ${artHtml(p, "", false)}
      <div class="app-side">
        <div><div style="display:flex;gap:8px;align-items:center;flex-wrap:wrap"><h1>${esc(p.name)}</h1><span class="badge private">🔒 PRIVATE SLOP</span></div>
          <div class="muted">by <b style="color:var(--text)">@${esc(p.owner)}</b> · ${esc(p.type)}</div></div>
        <p style="margin:0;color:#c4ced9">${esc(p.short)}</p>
        <div class="panel panel-pad"><b>Visible to</b><div class="muted" style="font-size:13px;margin-top:2px">${mine ? "You" : `@${esc(p.owner)}`}${p.shared.friends.length || p.shared.teams.length ? ` + ${esc(sharedLabel(p))}` : " only"}</div>
          ${mine ? `<button class="btn small" style="margin-top:10px" data-share="${p.id}">Change sharing…</button>` : `<div class="faint" style="font-size:12px;margin-top:6px">${p.shared.friends.includes("you") ? "Shared with you as a friend" : "Shared with a team you're in"}</div>`}</div>
        <div style="display:flex;gap:8px;flex-wrap:wrap">
          ${p.builds[S.os] ? `<button class="btn slime" onclick="progressToast('⬇ Installing <b>${esc(p.name)}</b>…', () => toast('✓ Installed'))">⬇ Install for ${PLATFORMS[S.os].label}</button>` : `<button class="btn" disabled>No ${PLATFORMS[S.os].label} build</button>`}
          <a class="btn" href="#" data-gh="${esc(p.repo)}">Source on GitHub</a>
        </div>
        <dl class="kv"><dt>Source</dt><dd>${esc(p.repo)} <span class="badge official">OPEN SOURCE</span></dd>
          <dt>Builds</dt><dd>${builds.map(([os, b]) => `${PLATFORMS[os].label} (${BUILD_KINDS[b.kind]})`).join(", ")}</dd>
          <dt>Slop Pulse</dt><dd class="muted">Not required for Private Slop</dd></dl>
      </div>
    </div>
    <div class="section-h"><h2>Security report</h2><span class="sub">Private Slop goes through the same scans as public apps</span></div>
    <div class="panel panel-pad"><div class="scan-list">${scanRowsHtml(fake.scan.results.filter(r => checksFor(fake).some(c => c.id === r.id)))}</div></div>
    <p class="faint" style="font-size:12px;margin-top:14px">Private Slop never appears in the store, search, recommendations or on official team pages.</p>
  </div>`);
}
function shareModal(id) {
  const p = S.privates.find(x => x.id === id);
  const myTeams = allTeams().filter(t => S.teams[t.id]);
  modal(`<div class="modal-h"><h2>Share “${esc(p.name)}”</h2><button class="x" data-close>×</button></div>
    <p class="muted" style="margin:0 0 12px">Private Slop is only visible to you until you share it. People you share with can see the page and install it - it never shows up in the store.</p>
    <div class="share-h">Friends</div>
    <div class="checks">${FRIENDS.map(f => `<label><input type="checkbox" data-f="${f}" ${p.shared.friends.includes(f) ? "checked" : ""}><span>@${f}</span></label>`).join("")}</div>
    <div class="share-h">Your teams</div>
    <div class="checks">${myTeams.map(t => t.kind === "official"
      ? `<label class="disabled" title="Private Slop can't be shared with official teams"><input type="checkbox" disabled><span>${t.icon} ${esc(t.name)} <span class="badge official">OFFICIAL</span><small>Private Slop can't be advertised to official teams</small></span></label>`
      : `<label><input type="checkbox" data-t="${t.id}" ${p.shared.teams.includes(t.id) ? "checked" : ""}><span>${t.icon} ${esc(t.name)} <span class="faint">· ${num(t.members)} members</span></span></label>`).join("")}</div>
    <div class="actions"><button class="btn" data-close>Cancel</button><button class="btn slime" id="save">Save</button></div>`,
  { onMount: (m, close) => {
      $("#save", m).onclick = () => {
        p.shared.friends = $$("[data-f]:checked", m).map(i => i.dataset.f);
        p.shared.teams = $$("[data-t]:checked", m).map(i => i.dataset.t);
        save(); close();
        toast(`🔒 ${esc(p.name)} is now visible to: <b>${esc(sharedLabel(p))}</b>`);
        render();
      };
    } });
}

/* ======================================================================
   Teams
   ====================================================================== */
function teamsPage(filter = "mine") {
  const list = allTeams().filter(t => filter === "mine" ? S.teams[t.id] : filter === "official" ? t.kind === "official" : filter === "player" ? t.kind === "player" : true);
  mount(`<div class="page">
    <div style="display:flex;justify-content:space-between;align-items:flex-end;gap:12px;flex-wrap:wrap">
      <div><h1 style="font-size:28px">Teams</h1><p class="muted" style="margin:4px 0 0">Find people who play what you play, or follow the studios that make it.</p></div>
      <button class="btn slime" id="create">+ Create a team</button>
    </div>
    <div class="team-kinds">
      <div class="panel panel-pad"><b>👥 Player teams</b><div class="muted" style="font-size:13px">Anyone can start one. Share picks, organise game nights, and share Private Slop with each other.</div></div>
      <div class="panel panel-pad"><b>🏷️ Official teams</b><div class="muted" style="font-size:13px">Verified developer studios. They show the apps they publish. Private Slop can't be shared with them.</div></div>
    </div>
    <div class="seg" style="margin:18px 0">
      ${[["mine", "Your teams"], ["player", "Player teams"], ["official", "Official teams"], ["all", "All"]].map(([k, l]) => `<a href="#/teams/${k}" class="${filter === k ? "on" : ""}">${l}</a>`).join("")}
    </div>
    <div class="team-grid">${list.length ? list.map(teamCardHtml).join("") : `<div class="empty-state" style="grid-column:1/-1">You're not in any teams yet.</div>`}</div>
  </div>`);
  $("#create").onclick = () => requireAuth(createTeam);
}
function teamCardHtml(t) {
  const on = S.teams[t.id];
  return `<a class="panel team-card" href="#/team/${t.id}">
    <div class="team-ic">${t.icon}</div>
    <div style="flex:1;min-width:0">
      <div style="display:flex;gap:8px;align-items:center;flex-wrap:wrap"><b>${esc(t.name)}</b>${t.kind === "official" ? `<span class="badge official">✓ OFFICIAL</span>` : `<span class="badge neutral">PLAYER TEAM</span>`}</div>
      <div class="muted" style="font-size:13px;margin:4px 0">${esc(t.desc)}</div>
      <div class="faint" style="font-size:12px">${num(t.members + (on && !t.mine ? 0 : 0))} member${t.members === 1 ? "" : "s"} · ${t.kind === "official" ? `${(t.apps || []).length} published` : t.open ? "open to join" : "invite only"}</div>
    </div>
    ${on ? `<span class="badge official">JOINED</span>` : ""}
  </a>`;
}
function teamPage(id) {
  const t = TEAM_BY_ID(id);
  if (!t) return mount(`<div class="page"><div class="empty-state">Team not found.</div></div>`);
  const on = S.teams[t.id];
  const sharedHere = t.kind === "player" && on ? S.privates.filter(p => p.shared.teams.includes(t.id)) : [];
  const apps = (t.kind === "official" ? t.apps : t.picks || []).map(a => APP_BY_ID[a]).filter(Boolean);
  const members = t.kind === "official" ? ["whiskerworks", "nightcoder", "linenfold"].includes(t.id.split("-")[0]) ? [t.id.split("-")[0], ...USERS.slice(0, t.members - 1)] : USERS.slice(0, t.members) : USERS.slice(0, Math.min(10, t.members));
  mount(`<div class="page">
    <div class="crumbs"><a href="#/teams">Teams</a> › ${esc(t.name)}</div>
    <div class="team-head panel">
      <div class="team-ic big">${t.icon}</div>
      <div style="flex:1;min-width:200px">
        <div style="display:flex;gap:8px;align-items:center;flex-wrap:wrap"><h1 style="font-size:26px">${esc(t.name)}</h1>${t.kind === "official" ? `<span class="badge official">✓ OFFICIAL · VERIFIED DEVELOPER</span>` : `<span class="badge neutral">PLAYER TEAM</span>`}</div>
        <p class="muted" style="margin:6px 0 0">${esc(t.desc)}</p>
        <div class="faint" style="font-size:12px;margin-top:6px">${num(t.members)} members · ${t.open ? "open to join" : "invite only"}</div>
      </div>
      <button class="btn ${on ? "" : "slime"}" id="join">${on ? "Leave team" : t.open ? "Join team" : t.kind === "official" ? "Request to join" : "Request invite"}</button>
    </div>
    ${t.kind === "official" ? `<div class="section-h"><h2>Published by ${esc(t.name)}</h2></div><div class="shelf">${apps.map(a => tileHtml(a)).join("")}</div>`
      : `${on ? `<div class="section-h"><h2>🔒 Private Slop shared with this team</h2><span class="sub">Only members can see these</span></div>
          <div class="shelf">${sharedHere.length ? sharedHere.map(p => `<a class="tile" href="#/private/${p.id}">${artHtml(p)}<div class="tile-body"><span class="d">${esc(p.short)}</span><div class="tile-foot"><span class="muted" style="font-size:12px">by @${esc(p.owner)}</span><span class="badge private">PRIVATE</span></div></div></a>`).join("")
            : `<div class="empty-state" style="grid-column:1/-1">Nothing shared yet. Share your Private Slop from its page.</div>`}</div>` : ""}
        ${apps.length ? `<div class="section-h"><h2>Team picks</h2></div><div class="shelf">${apps.map(a => tileHtml(a)).join("")}</div>` : ""}
        ${t.tags ? `<div class="section-h"><h2>What we play</h2></div><div class="chips">${t.tags.map(x => `<a class="chip" href="#/tag/${enc(x)}">${esc(x)}</a>`).join("")}</div>` : ""}`}
    <div class="section-h"><h2>Members</h2><span class="sub">${num(t.members)} total</span></div>
    <div class="chips">${(on ? ["you", ...members] : members).slice(0, 12).map(u => `<span class="chip">@${esc(u)}</span>`).join("")}${t.members > 12 ? `<span class="chip faint">+${num(t.members - 12)} more</span>` : ""}</div>
  </div>`);
  $("#join").onclick = () => requireAuth(() => {
    if (on) { delete S.teams[t.id]; toast(`Left ${esc(t.name)}`); }
    else if (t.open) { S.teams[t.id] = true; toast(`👥 Joined <b>${esc(t.name)}</b>`); }
    else return toast(`Request sent to ${esc(t.name)}'s admins.`);
    save(); render();
  });
}
function createTeam() {
  modal(`<div class="modal-h"><h2>Create a team</h2><button class="x" data-close>×</button></div>
    <div class="field"><label>Name</label><input type="text" id="tn" placeholder="e.g. Midnight Speedrunners"></div>
    <div class="field" style="margin-top:12px"><label>Type</label>
      <div class="radio-cards">
        <label><input type="radio" name="k" value="player" checked><b>👥 Player team</b><span>For players. Can see Private Slop shared with it.</span></label>
        <label><input type="radio" name="k" value="official"><b>🏷️ Official team</b><span>For developers. Needs a verified publisher account - you're verified as a publisher of Cat Café Tycoon.</span></label>
      </div></div>
    <div class="field" style="margin-top:12px"><label>Description</label><textarea id="td" placeholder="What's this team about?"></textarea></div>
    <label style="display:flex;gap:8px;font-size:13px;margin-top:10px"><input type="checkbox" id="open" checked style="accent-color:var(--slime)"> Anyone can join (otherwise invite only)</label>
    <div class="actions"><button class="btn" data-close>Cancel</button><button class="btn slime" id="mk">Create team</button></div>`,
  { onMount: (m, close) => {
      $("#mk", m).onclick = () => {
        const name = $("#tn", m).value.trim();
        if (name.length < 3) return toast("Give the team a name (3+ characters).");
        const kind = $("input[name=k]:checked", m).value;
        const t = { id: name.toLowerCase().replace(/\W+/g, "-") + "-" + Date.now().toString(36).slice(-3), name, kind, icon: kind === "official" ? "🏷️" : "✨", members: 1, open: $("#open", m).checked,
          desc: $("#td", m).value.trim() || "A brand new team.", apps: kind === "official" ? ["cat-cafe-tycoon"] : [], picks: [], mine: true };
        S.myTeams.push(t); S.teams[t.id] = true; save(); close();
        toast(`${t.icon} Created <b>${esc(name)}</b>${kind === "official" ? " - verification pending" : ""}`);
        location.hash = `#/team/${t.id}`;
      };
    } });
}

/* ======================================================================
   Humans: hire real people for art, music, writing and more
   ====================================================================== */
const HUMAN_BY_ID = Object.fromEntries(HUMANS.map(h => [h.id, h]));
const humanRating = h => { const mine = S.humanReviews[h.id]; return mine ? Math.round((h.slop * h.clients + mine.slop) / (h.clients + 1)) : h.slop; };
function aiBadge(h) {
  const [label] = AI_USE[h.ai];
  return `<span class="badge ${h.ai === "none" ? "official" : h.ai === "assisted" ? "warn" : "bad"}" title="${AI_USE[h.ai][1]}">${h.ai === "none" ? "✋ " : "🤖 "}${label.toUpperCase()}</span>`;
}
function humanCardHtml(h) {
  return `<a class="panel human-card" href="#/human/${h.id}">
    <div class="human-top">
      <div class="h-avatar" style="background:linear-gradient(135deg,${h.art[0]},${h.art[1]})">${h.avatar}</div>
      <div style="min-width:0;flex:1"><b>${esc(h.name)}</b><div class="muted" style="font-size:12px">${h.kind === "studio" ? `Studio · ${h.team} people` : "Individual"} · ${esc(h.location)}</div></div>
      ${slopHtml(humanRating(h), "sm", false)}
    </div>
    <div class="h-portfolio">${h.portfolio.slice(0, 4).map(e => `<span style="background:linear-gradient(135deg,${h.art[0]}88,${h.art[1]}88)">${e}</span>`).join("")}</div>
    <div style="font-size:13px;color:#c4ced9">${esc(h.tagline)}</div>
    <div class="chips">${h.skills.map(x => `<span class="chip">${esc(x)}</span>`).join("")}</div>
    <div class="h-foot"><span><b>${esc(h.rate)}</b></span>${aiBadge(h)}</div>
    <div class="h-foot muted" style="font-size:12px"><span><span class="avail ${h.available ? "on" : ""}"></span>${h.available ? "Taking work" : "Fully booked"}</span><span>${h.clients} verified clients</span></div>
  </a>`;
}
function humansPage(skill = "") {
  const v = mount(`<div class="page">
    <div class="humans-hero">
      <div><span class="badge neutral">SEPARATE FROM THE STORE</span>
        <h1>Humans</h1>
        <p>Artists, modellers, musicians, writers and studios you can hire to make the parts of your game that shouldn't be slop. Rated by the developers who've actually worked with them.</p></div>
      <button class="btn slime big" id="list-me">+ List yourself or your studio</button>
    </div>
    <div class="seg wrap" style="margin:18px 0 10px">
      <a href="#/humans" class="${!skill ? "on" : ""}">Everyone</a>
      ${HUMAN_SKILLS.map(x => `<a href="#/humans/${enc(x)}" class="${skill === x ? "on" : ""}">${x} <span class="faint">${HUMANS.filter(h => h.skills.includes(x)).length}</span></a>`).join("")}
    </div>
    <div class="filters">
      <label>Sort <select id="hsort"><option value="slop">Least slop</option><option value="clients">Most clients</option></select></label>
      <label><input type="checkbox" id="havail"> Taking work now</label>
      <label>AI use <select id="hai"><option value="">Any</option><option value="none">No AI only</option><option value="assisted">No AI or AI-assisted</option></select></label>
      <label><input type="checkbox" id="hstudio"> Studios only</label>
    </div>
    <div class="human-grid" id="hgrid"></div>
    <div class="panel panel-pad" style="margin-top:20px;font-size:13px"><b>How human slop scores work</b>
      <div class="muted" style="margin-top:4px">Only developers who completed a job through Slop can vote - so every score comes from a real client. <b style="color:var(--text)">👍 Not Slop</b> means hand-crafted, on time, exactly what was asked for. <b style="color:var(--text)">👎 Slop</b> means you paid for something a generator could've made in a minute. AI use must be disclosed on every profile.</div></div>
  </div>`);
  const draw = () => {
    const sort = $("#hsort", v).value, ai = $("#hai", v).value;
    let list = HUMANS.filter(h => (!skill || h.skills.includes(skill)) && (!$("#havail", v).checked || h.available) && (!$("#hstudio", v).checked || h.kind === "studio")
      && (!ai || h.ai === "none" || (ai === "assisted" && h.ai === "assisted")));
    list.sort(sort === "clients" ? (a, b) => b.clients - a.clients : (a, b) => humanRating(a) - humanRating(b));
    $("#hgrid", v).innerHTML = list.length ? list.map(humanCardHtml).join("") : `<div class="empty-state" style="grid-column:1/-1">Nobody matches those filters.</div>`;
  };
  $$("select, input", $(".filters", v)).forEach(el => el.addEventListener("input", draw));
  $("#list-me", v).onclick = () => requireAuth(listYourself);
  draw();
}
function humanPage(id) {
  const h = HUMAN_BY_ID[id];
  if (!h) return mount(`<div class="page"><div class="empty-state">Not found.</div></div>`);
  const hire = S.hires[id], mine = S.humanReviews[id];
  const reviews = mine ? [{ ...mine, mine: true }, ...h.reviews] : h.reviews;
  const rating = humanRating(h);
  mount(`<div class="page">
    <div class="crumbs"><a href="#/humans">Humans</a> › ${esc(h.name)}</div>
    <div class="human-head panel">
      <div class="h-avatar big" style="background:linear-gradient(135deg,${h.art[0]},${h.art[1]})">${h.avatar}</div>
      <div style="flex:1;min-width:220px">
        <div style="display:flex;gap:8px;align-items:center;flex-wrap:wrap"><h1 style="font-size:28px">${esc(h.name)}</h1><span class="badge neutral">${h.kind === "studio" ? `STUDIO · ${h.team} PEOPLE` : "INDIVIDUAL"}</span>${aiBadge(h)}</div>
        <p style="margin:6px 0;color:#c4ced9">${esc(h.tagline)}</p>
        <div class="muted" style="font-size:13px">${esc(h.location)} · <span class="avail ${h.available ? "on" : ""}"></span>${h.available ? "Taking work" : "Fully booked"} · turnaround ${esc(h.turnaround)}</div>
        <div class="chips" style="margin-top:10px">${h.skills.map(x => `<a class="chip" href="#/humans/${enc(x)}">${esc(x)}</a>`).join("")}</div>
      </div>
      <div style="display:flex;flex-direction:column;gap:10px;align-items:flex-end">
        ${slopHtml(rating, "lg", true, `${h.clients + (mine ? 1 : 0)} verified clients`)}
        <div style="width:240px">${voteBarHtml({ id: h.id, slop: rating, reviews: h.clients }, mine, false)}</div>
        <button class="btn slime" id="quote" ${h.available ? "" : "disabled"}>${h.available ? "Request a quote" : "Fully booked"}</button>
      </div>
    </div>
    <div class="two-col" style="margin-top:20px">
      <div>
        <div class="section-h" style="margin-top:0"><h2>Portfolio</h2></div>
        <div class="h-portfolio big">${h.portfolio.map(e => `<span style="background:linear-gradient(135deg,${h.art[0]},${h.art[1]})">${e}</span>`).join("")}</div>
        ${h.worked.length ? `<div class="section-h"><h2>Worked on</h2><span class="sub">Apps on Slop that credit them</span></div><div class="shelf">${h.worked.map(a => tileHtml(APP_BY_ID[a])).join("")}</div>` : ""}
        <div class="section-h"><h2>Reviews from clients</h2><span class="sub">Only people who completed a job can rate</span></div>
        ${reviews.map(r => `<div class="panel review">
          <div class="who"><b>${r.mine ? "You" : "@" + esc(r.user)}</b><span class="badge official" style="margin-top:4px">✓ VERIFIED CLIENT</span></div>
          <div>${voteChip(r)}<div class="faint" style="font-size:12px;margin-top:6px">${esc(r.project)}</div><p>${esc(r.text)}</p></div>
        </div>`).join("")}
      </div>
      <div style="display:flex;flex-direction:column;gap:14px">
        <div class="panel panel-pad"><b>Services</b>
          <table class="plat-table" style="margin-top:8px">${h.services.map(([n, pr]) => `<tr><td style="font-weight:400;white-space:normal">${esc(n)}</td><td style="text-align:right">${esc(pr)}</td></tr>`).join("")}</table></div>
        <div class="panel panel-pad"><b>AI use</b><div style="margin:8px 0 4px">${aiBadge(h)}</div><div class="muted" style="font-size:13px">${AI_USE[h.ai][1]}</div></div>
        <div class="panel panel-pad" id="hire-box">${hireBoxHtml(h)}</div>
        <button class="btn ghost danger" id="hreport">⚑ Report this profile</button>
      </div>
    </div>
  </div>`);
  $("#quote").onclick = () => requireAuth(() => quoteModal(h));
  $("#hreport").onclick = () => requireAuth(() => toast(`✓ Report filed about ${esc(h.name)}. A moderator will review the profile.`));
  $("#hire-box").onclick = e => {
    if (e.target.id === "done") { S.hires[id] = "done"; save(); toast("✓ Job marked complete - you can now vote on their work."); render(); }
    if (e.target.id === "rate") requireAuth(() => rateHuman(h));
  };
}
function hireBoxHtml(h) {
  const hire = S.hires[h.id];
  if (hire === "done") return `<b>You've worked with ${esc(h.name)}</b><div class="muted" style="font-size:13px;margin:4px 0 10px">Verified client - your rating counts.</div><button class="btn slime small" id="rate">${S.humanReviews[h.id] ? "Change your vote" : "👍 / 👎 Vote on their work"}</button>`;
  if (hire === "requested") return `<b>Quote requested</b><div class="muted" style="font-size:13px;margin:4px 0 10px">When it's delivered you have 7 days to mark it complete (after that it completes automatically) - then you can rate them. Your agreement is with them, not Slop.</div><button class="btn small" id="done">Mark job complete (demo)</button>`;
  return `<b>Vote after hiring</b><div class="muted" style="font-size:13px;margin-top:4px">Only developers who complete a job through Slop can vote 👍 Not Slop or 👎 Slop. No drive-by reviews.</div>`;
}
function quoteModal(h) {
  const projects = [...Object.keys(S.library).map(id => APP_BY_ID[id]?.name), ...S.privates.filter(p => p.owner === "you").map(p => p.name + " (Private Slop)"), "Tiny Tavern", "Something new"].filter(Boolean);
  modal(`<div class="modal-h"><h2>Request a quote from ${esc(h.name)}</h2><button class="x" data-close>×</button></div>
    <div class="field"><label>For which project?</label><select id="qp">${[...new Set(projects)].map(x => `<option>${esc(x)}</option>`).join("")}</select></div>
    <div class="field" style="margin-top:12px"><label>Service</label><select id="qs">${h.services.map(([n, pr]) => `<option>${esc(n)} - ${esc(pr)}</option>`).join("")}</select></div>
    <div class="field" style="margin-top:12px"><label>What do you need?</label><textarea id="qm" placeholder="Style, quantity, deadline, references…"></textarea></div>
    <label style="display:flex;gap:8px;font-size:13px;margin-top:10px"><input type="checkbox" checked disabled> Credit them on my store page when it ships</label>
    <div class="actions"><button class="btn" data-close>Cancel</button><button class="btn slime" id="send">Send request</button></div>`,
  { onMount: (m, close) => {
      $("#send", m).onclick = () => {
        if (!S.hires[h.id]) S.hires[h.id] = "requested";
        save(); close();
        toast(`✉️ Request sent to <b>${esc(h.name)}</b>. You'll hear back in your Slop inbox.`, 4500);
        render();
      };
    } });
}
function rateHuman(h) {
  voteModal({
    title: `Was ${esc(h.name)}'s work slop?`,
    question: "You're a verified client, so your vote counts.",
    prev: S.humanReviews[h.id],
    extra: `<input type="text" id="proj" placeholder="What did they make for you?" value="${esc(S.humanReviews[h.id]?.project || "")}" style="width:100%;margin-top:12px;background:var(--panel-2);border:1px solid var(--line);border-radius:8px;padding:9px 12px">`,
    placeholder: "Quality, communication, deadlines, honesty about AI use…",
    onPost: ({ vote, slop, text }, m) => {
      S.humanReviews[h.id] = { user: "you", vote, slop, project: $("#proj", m).value.trim() || "Commissioned work", text };
      save(); toast(`✓ Voted ${vote === "slop" ? "👎 Slop" : "👍 Not Slop"}`); render();
    },
  });
}
function listYourself() {
  modal(`<div class="modal-h"><h2>List yourself on Humans</h2><button class="x" data-close>×</button></div>
    <div class="radio-cards"><label><input type="radio" name="hk" value="individual" checked><b>🙋 Individual</b><span>Just you</span></label><label><input type="radio" name="hk" value="studio"><b>🏢 Studio</b><span>A team taking work together</span></label></div>
    <div class="field" style="margin-top:12px"><label>What do you do?</label><div class="chips" id="sk">${HUMAN_SKILLS.map(x => `<button class="chip" data-s="${esc(x)}">${esc(x)}</button>`).join("")}</div></div>
    <div class="field" style="margin-top:12px"><label>Starting rate</label><input type="text" placeholder="e.g. $40 / sprite sheet"></div>
    <div class="field" style="margin-top:12px"><label>How do you use AI?</label>
      <div class="radio-cards">${Object.entries(AI_USE).map(([k, [l, d]], i) => `<label><input type="radio" name="ai" value="${k}" ${i === 0 ? "checked" : ""}><b>${l}</b><span>${d}</span></label>`).join("")}</div>
      <div class="hint">Required, shown on your profile, and checked against client reviews.</div></div>
    <div class="actions"><button class="btn" data-close>Cancel</button><button class="btn slime" id="go">Create profile</button></div>`,
  { onMount: (m, close) => {
      $("#sk", m).onclick = e => { const b = e.target.closest("[data-s]"); if (b) { e.preventDefault(); b.classList.toggle("on"); } };
      $("#go", m).onclick = () => { close(); toast("🙋 Profile created (mockup). It goes live after a quick portfolio check."); };
    } });
}

/* ======================================================================
   Global wiring
   ====================================================================== */
function updateMe() {
  const me = $("#me");
  if (!S.user) { me.innerHTML = `<button class="btn slime" id="signin">Sign in</button>`; $("#signin").onclick = () => signIn(); return; }
  me.innerHTML = `<button class="me-btn" id="acct"><div class="who"><b>${esc(S.user.name)}</b><span class="muted">${money(S.donated)} donated</span></div><div class="avatar">${esc(S.user.name[0].toUpperCase())}</div></button>`;
  $("#acct").onclick = accountModal;
}
function setOS(os) {
  S.os = os; save();
  $("#window").className = `window os-${os}`;
  document.body.classList.toggle("phone-mode", isPhoneOS(os));
  $$("#os-switch button").forEach(b => b.classList.toggle("on", b.dataset.os === os));
}

document.addEventListener("click", e => {
  const t = e.target.closest("[data-add],[data-install],[data-interest],[data-donate],[data-report],[data-review],[data-share],[data-gh]");
  if (!t) return;
  e.preventDefault();
  e.stopPropagation();
  if (t.dataset.gh) return githubToast(t.dataset.gh);
  // Everything else is tied to an account.
  requireAuth(() => {
    if (t.dataset.add) addToLibrary(t.dataset.add, t.dataset.branch);
    else if (t.dataset.install) installPrompt(t.dataset.install, S.library[t.dataset.install]?.branch);
    else if (t.dataset.interest) toggleInterest(t.dataset.interest);
    else if (t.dataset.donate) donate(t.dataset.donate);
    else if (t.dataset.report) report(t.dataset.report, t.dataset.target);
    else if (t.dataset.review) review(t.dataset.review, t.dataset.vote);
    else if (t.dataset.share) shareModal(t.dataset.share);
  });
}, true);
$("#os-switch").addEventListener("click", e => {
  const b = e.target.closest("button[data-os]");
  if (!b) return;
  setOS(b.dataset.os);
  toast(`Now previewing Slop on <b>${PLATFORMS[b.dataset.os].label}</b>${isPhoneOS(b.dataset.os) ? " - the phone app" : ""}. Install options update to match.`);
  render();
});
$("#search").addEventListener("submit", e => { e.preventDefault(); location.hash = `#/search/${enc($("input", e.target).value)}`; });
window.addEventListener("hashchange", render);

setOS(S.os);
updateMe();
render();
