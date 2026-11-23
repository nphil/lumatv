// Shared kit for the UX rework option mocks. It sits on the existing LumaGame mock kit
// (../../mock: tokens.css, lumagame.css, themes.js, icons.js, kit.js) and adds only what the
// options need: a few icons, the console tile, the quick menu, the one list row, and the Steam
// data a game page shows. Every number, name and post here is illustrative.
//
// URL: ?text=100|200  ?marks=1 outlines content whose Steam source is fragile or unconfirmed.
(function () {
  const K = window.Kit;
  const { games, engines, esc } = K;
  const BASE = "../../mock/";
  for (const g of Object.values(games)) {
    for (const f of ["cover", "hero", "logo"]) if (g[f]) g[f] = BASE + g[f];
    if (g.id) g.header = `${BASE}assets/${g.id}/header.jpg`;
  }

  // Icons the base set lacks (Lucide v1.27.0, ISC), same stroke and grid.
  const extra = {
    "star": "<path d=\"M11.525 2.295a.53.53 0 0 1 .95 0l2.31 4.679a2.123 2.123 0 0 0 1.595 1.16l5.166.756a.53.53 0 0 1 .294.904l-3.736 3.638a2.123 2.123 0 0 0-.611 1.878l.882 5.14a.53.53 0 0 1-.771.56l-4.618-2.428a2.122 2.122 0 0 0-1.973 0L6.396 21.01a.53.53 0 0 1-.77-.56l.881-5.139a2.122 2.122 0 0 0-.611-1.879L2.16 9.795a.53.53 0 0 1 .294-.906l5.165-.755a2.122 2.122 0 0 0 1.597-1.16z\"/>",
    "triangle-alert": "<path d=\"m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3\"/><path d=\"M12 9v4\"/><path d=\"M12 17h.01\"/>",
    "newspaper": "<path d=\"M15 18h-5\"/><path d=\"M18 14h-8\"/><path d=\"M4 22h16a2 2 0 0 0 2-2V4a2 2 0 0 0-2-2H8a2 2 0 0 0-2 2v16a2 2 0 0 1-4 0v-9a2 2 0 0 1 2-2h2\"/><rect width=\"8\" height=\"4\" x=\"10\" y=\"6\" rx=\"1\"/>",
    "stop": "<rect width=\"14\" height=\"14\" x=\"5\" y=\"5\" rx=\"2\" fill=\"currentColor\"/>",
    "delete": "<path d=\"M10 5a2 2 0 0 0-1.344.519l-6.328 5.74a1 1 0 0 0 0 1.481l6.328 5.741A2 2 0 0 0 10 19h10a2 2 0 0 0 2-2V7a2 2 0 0 0-2-2z\"/><path d=\"m12 9 6 6\"/><path d=\"m18 9-6 6\"/>",
    "space": "<path d=\"M22 17v1c0 .5-.5 1-1 1H3c-.5 0-1-.5-1-1v-1\"/>",
    "chevron-up": "<path d=\"m18 15-6-6-6 6\"/>",
    "activity": "<path d=\"M22 12h-2.48a2 2 0 0 0-1.93 1.46l-2.35 8.36a.25.25 0 0 1-.48 0L9.24 2.18a.25.25 0 0 0-.48 0l-2.35 8.36A2 2 0 0 1 4.49 12H2\"/>",
    "layers": "<path d=\"M12.83 2.18a2 2 0 0 0-1.66 0L2.6 6.08a1 1 0 0 0 0 1.83l8.58 3.91a2 2 0 0 0 1.66 0l8.58-3.9a1 1 0 0 0 0-1.83z\"/><path d=\"M2 12a1 1 0 0 0 .58.91l8.6 3.91a2 2 0 0 0 1.65 0l8.58-3.9A1 1 0 0 0 22 12\"/><path d=\"M2 17a1 1 0 0 0 .58.91l8.6 3.91a2 2 0 0 0 1.65 0l8.58-3.9A1 1 0 0 0 22 17\"/>",
    "message-square": "<path d=\"M22 17a2 2 0 0 1-2 2H6.828a2 2 0 0 0-1.414.586l-2.202 2.202A.71.71 0 0 1 2 21.286V5a2 2 0 0 1 2-2h16a2 2 0 0 1 2 2z\"/>",
    "thumbs-up": "<path d=\"M15 5.88 14 10h5.83a2 2 0 0 1 1.92 2.56l-2.33 8A2 2 0 0 1 17.5 22H4a2 2 0 0 1-2-2v-8a2 2 0 0 1 2-2h2.76a2 2 0 0 0 1.79-1.11L12 2a3.13 3.13 0 0 1 3 3.88Z\"/><path d=\"M7 10v12\"/>",
    "thumbs-down": "<path d=\"M9 18.12 10 14H4.17a2 2 0 0 1-1.92-2.56l2.33-8A2 2 0 0 1 6.5 2H20a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2h-2.76a2 2 0 0 0-1.79 1.11L12 22a3.13 3.13 0 0 1-3-3.88Z\"/><path d=\"M17 14V2\"/>",
    "book-open": "<path d=\"M12 7v14\"/><path d=\"M3 18a1 1 0 0 1-1-1V4a1 1 0 0 1 1-1h5a4 4 0 0 1 4 4 4 4 0 0 1 4-4h5a1 1 0 0 1 1 1v13a1 1 0 0 1-1 1h-6a3 3 0 0 0-3 3 3 3 0 0 0-3-3z\"/>",
    "pin": "<path d=\"M12 17v5\"/><path d=\"M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z\"/>",
    "eye-off": "<path d=\"M10.733 5.076a10.744 10.744 0 0 1 11.205 6.575 1 1 0 0 1 0 .696 10.747 10.747 0 0 1-1.444 2.49\"/><path d=\"M14.084 14.158a3 3 0 0 1-4.242-4.242\"/><path d=\"M17.479 17.499a10.75 10.75 0 0 1-15.417-5.151 1 1 0 0 1 0-.696 10.75 10.75 0 0 1 4.446-5.143\"/><path d=\"m2 2 20 20\"/>",
    "users-round": "<path d=\"M18 21a8 8 0 0 0-16 0\"/><circle cx=\"10\" cy=\"8\" r=\"5\"/><path d=\"M22 20c0-3.37-2-6.5-4-8a5 5 0 0 0-.45-8.3\"/>",
    "panel-right": "<rect width=\"18\" height=\"18\" x=\"3\" y=\"3\" rx=\"2\"/><path d=\"M15 3v18\"/>",
    "scan-line": "<path d=\"M3 7V5a2 2 0 0 1 2-2h2\"/><path d=\"M17 3h2a2 2 0 0 1 2 2v2\"/><path d=\"M21 17v2a2 2 0 0 1-2 2h-2\"/><path d=\"M7 21H5a2 2 0 0 1-2-2v-2\"/><path d=\"M7 12h10\"/>",
    "arrow-right": "<path d=\"M5 12h14\"/><path d=\"m12 5 7 7-7 7\"/>",
    "wrench": "<path d=\"M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.106-3.105c.32-.322.863-.22.983.218a6 6 0 0 1-8.259 7.057l-7.91 7.91a1 1 0 0 1-2.999-3l7.91-7.91a6 6 0 0 1 7.057-8.259c.438.12.54.662.219.984z\"/>",
    // Pad shoulder buttons: the key's own shape with its letters, as the pad draws them.
    "key-lb": "<path d=\"M3 16V11a5 5 0 0 1 5-5h13v10z\"/><path d=\"M8.2 9.3v4.2h2.3\" stroke-width=\"1.5\"/><path d=\"M13.4 9.3v4.2h1.9a1.1 1.1 0 0 0 0-2.2h-1.9m0 0h1.7a1 1 0 0 0 0-2h-1.7\" stroke-width=\"1.5\"/>",
    "key-rb": "<path d=\"M21 16V11a5 5 0 0 0-5-5H3v10z\"/><path d=\"M6.8 13.5V9.3h1.8a1.2 1.2 0 0 1 0 2.4H6.8m1.7 0 1.2 1.8\" stroke-width=\"1.5\"/><path d=\"M12.6 9.3v4.2h1.9a1.1 1.1 0 0 0 0-2.2h-1.9m0 0h1.7a1 1 0 0 0 0-2h-1.7\" stroke-width=\"1.5\"/>",
  };
  const base = window.Icons;
  const wrap = (inner) => `<svg class="ic" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${inner}</svg>`;
  const svg = (name) => (extra[name] ? wrap(extra[name]) : base.svg(name));
  window.Icons = {
    svg,
    fill(root = document) {
      root.querySelectorAll("i[data-ic]").forEach((el) => {
        const t = document.createElement("template");
        t.innerHTML = svg(el.dataset.ic);
        const s = t.content.firstElementChild;
        if (el.className) s.setAttribute("class", "ic " + el.className);
        el.replaceWith(s);
      });
    },
  };
  const ic = K.ic;
  const q = new URLSearchParams(location.search);
  if (q.get("marks")) document.documentElement.dataset.marks = "1";

  // Where each game can run, and the state of each way in this household.
  const state = { gfn: "ready", sunshine: "asleep", punktfunk: "offline", native: "ready" };
  // A PC that is asleep wakes on Play, so it counts as ready here; only a way that cannot start
  // right now (offline, sign-in needed) marks its game.
  const mark = { offline: "wifi-off", signin: "key-round" };

  // ── Top bar: three places and the profile. Settings, accounts and Refresh live in the quick
  // menu (Menu key, or the profile), so the bar stays this small on every screen.
  function nav(current, o = {}) {
    const item = (id, label, icon) => `<span class="nav-item${current === id ? " is-current" : ""}${o.focus === id ? " is-focused" : ""}">${ic(icon)}<span>${label}</span></span>`;
    const lb = o.pad ? `<span class="rw-bump">${ic("key-lb", "glyph")}</span>` : "";
    const rb = o.pad ? `<span class="rw-bump">${ic("key-rb", "glyph")}</span>` : "";
    return `<nav class="nav"><div class="rw-navwrap">${lb}<div class="nav-group">${item("home", "Home", "house")}${item("library", "Library", "library-big")}${item("search", "Search", "search")}</div>${rb}</div>
      <div class="nav-group"><span class="nav-item icon-only avatar${o.focus === "profile" ? " is-focused" : ""}${current === "profile" ? " is-current" : ""}" aria-label="Profile and menu"><span>N</span>${o.attention ? `<i class="rw-attn"></i>` : ""}</span></div></nav>`;
  }

  // Hints: bottom-right safe corner on every full screen. Key glyph, then what it does.
  const hints = (list) => `<div class="corner-hints rw-dwell">${K.hints(list)}</div>`;

  // ── Game tile: the game's wide art cropped to the tile, a baked bottom scrim, its logo.
  // A game is a tile on Home; services never label it. A way that is not ready shows its mark.
  function tile(key, o = {}) {
    const g = games[key];
    const st = o.state || state[g.target];
    const flag = st !== "ready" && mark[st] ? `<span class="rw-flag">${ic(mark[st])}</span>` : "";
    const body = g.hero
      ? `<img class="art" src="${g.hero}" alt="" style="object-position:${o.pos || g.pos || "62% 30%"}"><img class="logo" data-trim src="${g.logo}" alt="${esc(g.title)}">`
      : K.plate(g);
    return `<div class="rw-tile${o.cls ? " " + o.cls : ""}${o.focused ? " is-focused" : ""}" data-game="${key}"><div class="rw-tile-art">${body}${flag}</div><span class="land"></span></div>`;
  }

  // Library cover: 2:3 art. The mark appears only when the game's way to play is not ready.
  function cover(key, o = {}) {
    const g = games[key];
    const st = o.state || state[g.target];
    const body = g.cover ? `<img src="${g.cover}" alt="">` : K.plate(g);
    const flag = st !== "ready" && mark[st] ? `<span class="rw-flag">${ic(mark[st])}</span>` : "";
    const cap = o.focused ? `<div class="cover-cap"><div class="cap-name">${esc(g.title)}</div><div class="cap-where rw-capline">${esc(o.line || g.played || g.hours + " played")}</div></div>` : "";
    return `<div class="cover${o.focused ? " is-focused" : ""}"><div class="cover-art">${body}${flag}</div><span class="land"></span>${cap}</div>`;
  }

  // ── The one list row (80 high): leading mark or picture, name and one short line, trailing
  // value, state or control. Settings, accounts, ways to play, posts and achievements all use it.
  function row(o) {
    const lead = o.img ? `<img class="rw-thumb${o.imgCls ? " " + o.imgCls : ""}" src="${o.img}" alt="">` : o.icon ? `<span class="rw-gt">${ic(o.icon)}</span>` : o.lead || "";
    const src = o.src ? ` data-src="${o.src}"` : "";
    return `<div class="srow rw-row${o.focus ? " is-focused" : ""}${o.cls ? " " + o.cls : ""}"${src}>${lead}<span class="grow"><span class="rw-name">${esc(o.name)}</span>${o.sub ? `<span class="sub">${o.subHtml ? o.sub : esc(o.sub)}</span>` : ""}</span>${o.right || ""}</div>`;
  }
  const val = (v, chev) => `<span class="val">${esc(v)}${chev ? ic("chevron-right") : ""}</span>`;
  const st = (key, word) => {
    const s = K.status[key];
    const m = s.icon ? ic(s.icon, "rw-st " + s.cls) : `<span class="dot ${s.cls}"></span>`;
    return `<span class="val rw-state">${m}<span>${esc(word || s.word)}</span></span>`;
  };
  const toggle = (on) => `<span class="toggle${on ? " on" : ""}">${on ? ic("check", "tick") : ""}</span>`;
  const radio = (on) => `<span class="rw-radio${on ? " on" : ""}"></span>`;
  const bar = (pct) => `<span class="rw-bar"><b style="width:${pct}%"></b></span>`;
  const from = (stale) => `<span class="rw-from">${stale ? ic("triangle-alert", "warn") : ""}<span>${stale ? "From Steam, " + esc(stale) : "From Steam"}</span></span>`;
  const btn = (label, icon, o = {}) => `<span class="btn${o.primary ? " primary" : ""}${o.danger ? " danger" : ""}${o.focus ? " is-focused" : ""}${o.cls ? " " + o.cls : ""}">${icon ? ic(icon, icon === "play" ? "fill" : "") : ""}<span>${esc(label)}</span>${o.after || ""}</span>`;

  // ── Quick menu: one panel on the Menu key (pad Y) from Home, Library and a game page.
  // Refresh libraries is its first row and exists nowhere else.
  function quickMenu(o = {}) {
    const g = o.game ? games[o.game] : null;
    const rows = [
      row({ icon: "refresh-cw", name: "Refresh libraries", sub: o.refreshing ? "Checking GeForce NOW…" : "Updated 2 min ago", focus: o.focus === "refresh" }),
      row({ icon: "users-round", name: "Accounts and PCs", sub: o.attention || "", right: o.attention ? ic("triangle-alert", "rw-st warn") : "", focus: o.focus === "accounts" }),
      row({ icon: "settings", name: "Settings", focus: o.focus === "settings" }),
    ];
    const game = g ? `<div class="group-label">${esc(g.title)}</div>` + [
      row({ icon: "info", name: "Game page" }),
      row({ icon: "pin", name: "Pin to Home" }),
      row({ icon: "eye-off", name: "Hide" }),
    ].join("") : "";
    return `<div class="scrim rw-scrim-side"></div><aside class="sheet rw-drawer rw-quick"><div class="rw-drawer-head rw-title">Menu</div>${rows.join("")}${game}
      <div class="rw-drawer-foot">${K.hints([["key-back", "Close"]])}</div></aside>`;
  }

  // ── Steam data for the game page (Hades II). Reviews, news, rarity and store media come from
  // stable Steam interfaces; discussions, guides and hub media are read from Steam's pages and
  // may be absent; friends are unconfirmed. data-src carries that into the markup.
  const A = (f) => `assets/ach/${f}.jpg`;
  const steam = {
    hours: "31 h", recent: "4 h", last: "Today, 19:40", lastLen: "1 h 12 min", lastWhere: "GeForce NOW",
    achDone: 18, achAll: 50, players: "8,412",
    ach: [
      { n: "Witch of the Depths", d: "Clear Oceanus", img: A("witch-of-the-depths"), when: "Today", pct: "61%" },
      { n: "So Mote It Be", d: "Cast 50 Incantations", img: A("so-mote-it-be"), when: "Today", pct: "34%" },
      { n: "Soothing Soak", d: "Visit the Hot Springs", img: A("soothing-soak"), when: "Sunday", pct: "72%" },
      { n: "Witch of the Woods", d: "Clear Erebus", img: A("witch-of-the-woods"), when: "12 Nov", pct: "88%" },
    ],
    next: [
      { n: "Witch of the Plains", d: "Clear the Fields of Mourning", img: A("witch-of-the-plains"), pct: "47%" },
      { n: "The Arms of Night", d: "Fulfil the prophecy", img: A("the-arms-of-night"), pct: "29%" },
      { n: "Death to Chronos", d: "Complete the lifelong task", img: A("death-to-chronos"), pct: "9%" },
    ],
    news: [
      { t: "Noclip making-of documentary, concert video and more", d: "25 Sep", by: "Supergiant Games", x: "A full year has passed since Hades II reached v1.0. To everyone who aided Princess Melinoë on her fateful task, thank you." },
      { t: "Post-launch patch 2, hotfix 5", d: "28 Jul", by: "Supergiant Games", x: "Fixes Wells of Charon in Dream Dives and two boon interactions." },
      { t: "Watch the Hades orchestral concert, live from London", d: "16 Jun", by: "Supergiant Games", x: "The full video of this one-time performance is now available." },
    ],
    review: { word: "Overwhelmingly Positive", pct: 96, n: "81,204" },
    reviews: [
      { up: true, x: "Everything the first game did, deeper and stranger. Fifty hours in and still finding new duo boons.", by: "after 54 h", help: "1,204 found this helpful" },
      { up: true, x: "Plays beautifully on a controller. Runs are the right length for one evening.", by: "after 22 h", help: "618 found this helpful" },
      { up: false, x: "The late game leans on resource grinding more than I wanted.", by: "after 71 h", help: "240 found this helpful" },
    ],
    threads: [
      { t: "Best aspect for a first surface clear?", n: "48 replies", d: "2 h ago" },
      { t: "Hidden prophecies you nearly missed", n: "112 replies", d: "Yesterday" },
      { t: "Controller layout that suits Omega moves", n: "19 replies", d: "3 days ago" },
    ],
    guides: [
      { t: "Every Fated List prophecy and how to finish it", n: "4.9 stars", d: "32 min read" },
      { t: "Boon and duo boon reference", n: "4.8 stars", d: "18 min read" },
    ],
    media: {
      store: [1, 2, 3, 4, 5].map((i) => `${BASE}assets/1145350/media/shot-${i}.jpg`),
      trailers: [1, 2].map((i) => `${BASE}assets/1145350/media/trailer-${i}.jpg`),
      full: [1, 2, 3].map((i) => `assets/shots/full-${i}.jpg`),
    },
  };


  // ── Game page sections. Four, fixed, in this order, on every game: a section whose data is
  // missing says so in its own place, so the page never changes shape when a lookup returns.
  const SECTIONS = ["Activity", "Community", "Media", "About"];
  const big = document.documentElement.dataset.text === "200";
  // At 200% the strip shows the current section between its two arrows: four names do not fit.
  function tabs(cur, o = {}) {
    if (big) return `<div class="gp-tabs"><span class="tab gp-step${o.focus ? " is-focused" : ""}">${ic("chevron-left")}<span>${cur}</span>${ic("chevron-right")}</span><span class="rw-cap">${SECTIONS.indexOf(cur) + 1} of ${SECTIONS.length}</span></div>`;
    return `<div class="gp-tabs"><div class="tabs">${SECTIONS.map((t) => `<span class="tab${t === cur ? " is-selected" : ""}${o.focus && t === cur ? " is-focused" : ""}">${t}</span>`).join("")}</div></div>`;
  }
  const fact = (k, v, extra = "") => `<div class="fact"><div class="k">${k}</div><div class="v">${v}</div>${extra}</div>`;
  const sec = {
    activity(o = {}) {
      const a = steam.ach.slice(0, o.ach ?? 3), n = steam.news.slice(0, o.news ?? 2);
      return `<div class="facts gp-facts">
          ${fact("Played", steam.hours, `<div class="rw-cap">${steam.recent} in the last two weeks</div>`)}
          ${fact("Achievements", `${steam.achDone} of ${steam.achAll}`, bar(36))}
          ${fact("Playing now", steam.players, `<div class="rw-cap" data-src="unconfirmed">2 friends</div>`)}
        </div>
        <div class="group-label">Latest achievements</div>
        ${a.map((x, i) => row({ img: x.img, name: x.n, sub: x.d, right: val(x.when), focus: o.focus === i })).join("")}
        <div class="group-label">News</div>
        ${n.map((x, i) => row({ icon: "newspaper", name: x.t, sub: `${x.d}, ${x.by}`, right: ic("chevron-right", "rw-st"), focus: o.focus === "news" + i })).join("")}`;
    },
    community(o = {}) {
      const r = steam.review;
      return `<div class="gp-score"><div><div class="rw-title">${r.word}</div><div class="rw-cap">${r.pct}% of ${r.n} reviews</div></div>${from()}</div>${bar(r.pct)}
        <div class="gp-gap"></div>
        ${steam.reviews.slice(0, o.reviews ?? 2).map((x, i) => row({ icon: x.up ? "thumbs-up" : "thumbs-down", name: x.x, sub: `${x.by}, ${x.help}`, cls: "gp-quote", focus: o.focus === i })).join("")}
        ${o.fragile === false ? "" : `<div class="group-label">Discussions</div>
        ${steam.threads.slice(0, o.threads ?? 2).map((x) => row({ icon: "message-square", name: x.t, sub: `${x.n}, ${x.d}`, right: ic("chevron-right", "rw-st"), src: "fragile" })).join("")}
        <div class="group-label">Guides</div>
        ${steam.guides.slice(0, o.guides ?? 1).map((x) => row({ icon: "book-open", name: x.t, sub: `${x.n}, ${x.d}`, right: ic("chevron-right", "rw-st"), src: "fragile" })).join("")}`}`;
    },
    media(o = {}) {
      const kinds = o.fragile === false ? ["Official"] : ["Official", "Screenshots", "Artwork", "Videos"];
      const pics = [...steam.media.trailers.map((p) => [p, true]), ...steam.media.store.map((p) => [p, false])].slice(0, o.n ?? 6);
      return `<div class="gp-score"><span class="seg">${kinds.map((k, i) => `<span class="${i === 0 ? "is-selected" : ""}"${i ? ` data-src="fragile"` : ""}>${k}</span>`).join("")}</span>${from()}</div>
        <div class="gp-grid">${pics.map(([p, video], i) => `<div class="gp-pic${o.focus === i ? " is-focused" : ""}"><img src="${p}" alt="">${video ? `<span class="gp-playmark">${ic("play", "fill")}</span>` : ""}</div>`).join("")}</div>`;
    },
    about() {
      return `<p class="gp-body-text">${esc(games.hades2.desc || "Battle beyond the Underworld using dark sorcery to take on the Titan of Time in this bewitching sequel to the award-winning rogue-like dungeon crawler.")}</p>
        <div class="group-label">Details</div>
        ${row({ icon: "users", name: "Developer", right: val("Supergiant Games") })}
        ${row({ icon: "calendar", name: "Released", right: val("25 Sep 2025") })}
        ${row({ icon: "gamepad-2", name: "Controller", right: val("Full support") })}`;
    },
    // Reading happens in place: one post, 28 px body, about 60 characters a line, a page at a time.
    reader(o = {}) {
      const n = steam.news[0];
      return `<div class="gp-reader"><span class="chip-btn gp-crumb">${ic("chevron-left")}<span>${o.back || "Activity"}</span></span>
        <h2 class="rw-title">${esc(n.t)}</h2><div class="rw-cap">${n.d}, ${n.by}</div>
        <div class="gp-body-text"><p>A full year has passed since Hades II reached v1.0. To everyone who aided Princess Melinoë on her fateful task, thank you.</p>
        <p>The team at Noclip followed the making of the game from its first prototype. Their documentary is out now, alongside the full video of the orchestral concert recorded in London.</p>
        <p>A small patch arrives with this update. It fixes two rare boon interactions and a save issue some players met after long sessions.</p></div>
        <div class="gp-reader-foot"><span class="rw-cap">Page 1 of 3</span>${from()}</div></div>`;
    },
  };
  // The one where-to-play control: a row that names the way Play will use. OK opens the list of
  // ways in the same place; the choice sticks for this game once a session has started on it.
  function where(o = {}) {
    if (!o.open) return row({ icon: "cloud", name: "GeForce NOW", sub: "Ready", right: ic("chevron-down", "rw-st"), focus: o.focus, cls: "gp-where" });
    const way = (icon, name, sub, on, focus) => row({ icon, name, sub, right: radio(on), focus });
    return `<div class="gp-ways">${way("cloud", "GeForce NOW", "Ready", true, true)}${way("monitor", "Gaming PC", "Asleep, wakes on Play")}${way("radio", "Desk PC", "Offline")}</div>`;
  }


  // ── Accounts and PCs: one list, one word of state per row. Steam sits under its own label
  // because it adds information to games and plays none.
  function accounts(o = {}) {
    const f = o.focus;
    return `${row({ icon: "cloud", name: "GeForce NOW", sub: "nitin, Ultimate", right: st("ready"), focus: f === "gfn" })}
      ${row({ icon: "monitor", name: "Gaming PC", sub: "BEAST-PC", right: st("asleep"), focus: f === "pc" })}
      ${row({ icon: "radio", name: "Desk PC", sub: "Last seen yesterday", right: st("offline"), focus: f === "desk" })}
      ${row({ icon: "tv", name: "This TV", sub: "3 games installed", right: st("ready"), focus: f === "tv" })}
      <div class="group-label">Adds to your games</div>
      ${row({ icon: "user-round", name: "Steam", sub: o.steam === "out" ? "Play time, achievements and friends" : o.steam === "stale" ? "Sign-in has expired" : "Nitin, 118 games matched", right: o.steam === "out" ? st("signin") : o.steam === "stale" ? st("signin", "Sign in again") : st("ready", "Linked"), focus: f === "steam" })}
      ${o.add === false ? "" : row({ icon: "plus", name: "Add an account or PC", focus: f === "add" })}`;
  }
  // Steam sign-in: the code on its white plate, one instruction, and how long the code lasts.
  function qr(o = {}) {
    return `<div class="rw-qr${o.stack ? " stack" : ""}"><div class="qr"><img src="${BASE}qr-signin.svg" alt="QR code"></div>
      <div><p class="rw-qr-text">Scan with the Steam app on a phone.</p>
      <div class="rw-qr-wait"><span class="rw-ring" style="--p:84%"></span><span>New code in 4:12</span></div></div></div>`;
  }
  const crumb = (label) => `<span class="chip-btn gp-crumb">${ic("chevron-left")}<span>${esc(label)}</span></span>`;
  const stepper = (v) => `<span class="rw-stepper">${ic("chevron-left")}<span>${esc(v)}</span>${ic("chevron-right")}</span>`;


  // Settings rail: five places, each with its current state as the second line.
  function settingsRail(cur, focus) {
    const items = [["accounts", "users-round", "Accounts and PCs", "1 offline"], ["picture", "monitor-play", "Picture and stream", "4K, 120 fps"], ["pads", "gamepad-2", "Controllers", "2 connected"], ["look", "palette", "Appearance", "Rosé Pine"], ["system", "info", "System", "Version 0.9.2"]];
    return items.map(([id, icon, name, sub]) => row({ icon, name, sub, cls: id === cur ? "is-current" : "", focus: id === focus })).join("");
  }
  const settingsTitle = () => `<header class="sT-title">${ic("chevron-left", "rw-st")}<h1 class="rw-title">Settings</h1></header>`;
  // Home A's first view, also the backdrop of the drawer mocks.
  function homeSpotlight() {
    const g = games.hades2;
    const recent = ["hades2", "bg3", "cyberpunk", "clair", "balatro", "forza", "elden", "ittakestwo", "stardew"];
    return `<div class="stage full"><img src="${g.hero}" alt=""></div>
    ${nav("home")}
    <div class="hA-row">${recent.map((k, i) => tile(k, { focused: i === 0 })).join("")}</div>
    <header class="hA-hero">
      <img class="hero-logo" data-trim src="${g.logo}" alt="${esc(g.title)}">
      <div class="hero-meta"><span>Played 2 h ago</span><i class="sep"></i><span>${steam.hours}</span><i class="sep"></i><span>${steam.achDone} of ${steam.achAll} achievements</span></div>
      <div class="rw-play">${ic("key-ok", "glyph")}<span>Play</span><span class="via">${ic("cloud")}</span></div>
    </header>`;
  }

  const hintsInline = (list) => K.hints(list);
  window.RW = { settingsRail, settingsTitle, homeSpotlight, chip: K.chip, accounts, qr, crumb, stepper, hintsInline, SECTIONS, big, tabs, sec, where, fact, games, engines, state, mark, steam, ic, esc, nav, hints, tile, cover, row, val, st, toggle, radio, bar, from, btn, quickMenu, ready: K.ready, q, BASE };
})();
