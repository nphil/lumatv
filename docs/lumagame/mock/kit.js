// Shared data and builders for the LumaGame mocks. The catalogue is illustrative: real titles
// with Steam store art (fetch-assets.sh), invented play history and target availability.
(function () {
  const art = (id, f) => `assets/${id}/${f}`;

  // Ways to play. Plugins bring the data; every surface styles it from theme tokens.
  const engines = {
    gfn: { name: "GeForce NOW", icon: "cloud" },
    sunshine: { name: "Gaming PC", via: "Sunshine", icon: "monitor" },
    punktfunk: { name: "Desk PC", via: "Punktfunk", icon: "radio" },
    native: { name: "This TV", icon: "tv" },
  };

  const g = (id, title, genre, year, extra = {}) => ({ id, title, genre, year, ...extra });
  const games = {
    hades2: g(1145350, "Hades II", "Roguelike", 2025, { players: "1 player", target: "gfn", played: "Played 2 h ago", hours: "31 h", desc: "Battle beyond the Underworld as Melinoë, Princess of the Underworld, against the Titan of Time." }),
    bg3: g(1086940, "Baldur's Gate 3", "RPG", 2023, { players: "1–4 players", target: "gfn", played: "Yesterday", hours: "142 h" }),
    cyberpunk: g(1091500, "Cyberpunk 2077", "RPG", 2020, { players: "1 player", target: "gfn", played: "3 days ago", hours: "64 h", desc: "An open-world action adventure in Night City, a megalopolis obsessed with power, glamour and body modification." }),
    clair: g(1903340, "Clair Obscur: Expedition 33", "RPG", 2025, { players: "1 player", target: "sunshine", played: "Last week", hours: "48 h" }),
    stardew: g(413150, "Stardew Valley", "Farming", 2016, { players: "1–4 players", target: "sunshine", played: "Last week", hours: "212 h" }),
    forza: g(1551360, "Forza Horizon 5", "Racing", 2021, { players: "1–12 players", target: "gfn", played: "2 weeks ago", hours: "37 h" }),
    elden: g(1245620, "Elden Ring", "Action RPG", 2022, { players: "1–4 players", target: "sunshine", hours: "188 h" }),
    witcher: g(292030, "The Witcher 3: Wild Hunt", "RPG", 2015, { players: "1 player", target: "gfn", hours: "171 h" }),
    rdr2: g(1174180, "Red Dead Redemption 2", "Open world", 2019, { players: "1 player", target: "gfn", hours: "96 h" }),
    hollow: g(367520, "Hollow Knight", "Metroidvania", 2017, { players: "1 player", target: "sunshine", hours: "77 h" }),
    portal2: g(620, "Portal 2", "Puzzle", 2011, { players: "1–2 players", target: "punktfunk", hours: "22 h" }),
    ittakestwo: g(1426210, "It Takes Two", "Co-op", 2021, { players: "2 players", target: "sunshine", hours: "19 h" }),
    ori: g(1057090, "Ori and the Will of the Wisps", "Platformer", 2020, { players: "1 player", target: "sunshine", hours: "14 h" }),
    celeste: g(504230, "Celeste", "Platformer", 2018, { players: "1 player", target: "punktfunk", hours: "26 h" }),
    disco: g(632470, "Disco Elysium", "RPG", 2019, { players: "1 player", target: "gfn", hours: "41 h" }),
    wukong: g(2358720, "Black Myth: Wukong", "Action", 2024, { players: "1 player", target: "gfn", hours: "29 h" }),
    balatro: g(2379780, "Balatro", "Card game", 2024, { players: "1 player", target: "punktfunk", hours: "58 h" }),
    tunic: g(553420, "Tunic", "Adventure", 2022, { players: "1 player", target: "sunshine", hours: "12 h" }),
    dave: g(1868140, "Dave the Diver", "Adventure", 2023, { players: "1 player", target: "punktfunk", hours: "33 h" }),
    mhwilds: g(2246340, "Monster Hunter Wilds", "Action", 2025, { players: "1–4 players", target: "gfn", hours: "52 h" }),
    rocket: g(252950, "Rocket League", "Sports", 2015, { players: "1–8 players", target: "gfn", hours: "90 h" }),
    control: g(870780, "Control", "Action", 2019, { players: "1 player", target: "gfn", hours: "21 h" }),
    quake: g(2310, "Quake", "Shooter", 1996, { players: "1–4 players", target: "native", hours: "9 h", desc: "The 1996 shooter, with both expansions. Runs on this TV, or streams from either PC." }),
    doom: g(2280, "DOOM + DOOM II", "Shooter", 1993, { players: "1–4 players", target: "native", hours: "15 h" }),
    openttd: g(1536610, "OpenTTD", "Strategy", 2004, { players: "1 player", target: "native", hours: "40 h" }),
    // Native titles without store art: they get a typeset title plate.
    celesteclassic: g(null, "Celeste Classic", "Platformer", 2015, { target: "native" }),
    supertuxkart: g(null, "SuperTuxKart", "Racing", 2007, { target: "native" }),
    freedoom: g(null, "Freedoom", "Shooter", 2003, { target: "native" }),
  };
  for (const [key, v] of Object.entries(games)) {
    v.key = key;
    if (v.id) Object.assign(v, { cover: art(v.id, "library_600x900_2x.jpg"), hero: art(v.id, "library_hero.jpg"), logo: art(v.id, "logo.png") });
  }

  const ic = (name, cls = "") => `<i data-ic="${name}" class="${cls}"></i>`;
  const esc = (s) => String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;");

  // Status: a coloured dot or glyph carries the state; the word is always ink.
  const status = {
    ready: { word: "Ready", cls: "ok" },
    asleep: { word: "Asleep", cls: "sleep", icon: "moon" },
    offline: { word: "Offline", cls: "bad", icon: "wifi-off" },
    signin: { word: "Sign in", cls: "warn" },
    unknown: { word: "Unknown", cls: "unknown" },
    installed: { word: "Installed", cls: "ok" },
    install: { word: "Install", cls: "neutral", icon: "download" },
    queue: { word: "Queue", cls: "warn", icon: "hourglass" },
  };
  function chip(key, word) {
    const s = status[key];
    const mark = s.icon ? ic(s.icon, "chip-ic") : `<span class="dot"></span>`;
    return `<span class="chip ${s.cls}">${mark}<span>${esc(word || s.word)}</span></span>`;
  }

  function where(game, state = "ready") {
    const e = engines[game.target];
    const s = status[state];
    const mark = s.icon ? ic(s.icon, "where-ic") : `<span class="dot ${s.cls}"></span>`;
    return `<span class="where">${ic(e.icon, "where-ic")}<span>${esc(e.name)}</span>${mark}<span>${esc(s.word)}</span></span>`;
  }

  function plate(game) {
    return `<div class="plate"><span class="plate-mono" data-mono="${esc(game.title[0])}" aria-hidden="true"></span><span class="plate-title">${esc(game.title)}</span></div>`;
  }

  // One card per game. Focus adds the landing light and the caption naming where it plays.
  function cover(key, o = {}) {
    const game = games[key];
    const body = game.cover ? `<img src="${game.cover}" alt="">` : plate(game);
    const cap = o.focused ? `<div class="cover-cap"><div class="cap-name">${esc(game.title)}</div><div class="cap-where">${where(game, o.state)}</div></div>` : "";
    return `<div class="cover${o.focused ? " is-focused" : ""}" data-game="${key}"><div class="cover-art">${body}</div><span class="land"></span>${cap}</div>`;
  }

  function row(title, keys, o = {}) {
    const items = keys.map((k, i) => cover(k, { focused: o.focus === i, state: o.state })).join("");
    return `<section class="row${o.focus != null ? " has-focus" : ""}"><h2 class="row-head">${esc(title)}</h2><div class="row-track">${items}</div></section>`;
  }

  function nav(current, o = {}) {
    const item = (id, label, icon) => `<span class="nav-item${current === id ? " is-current" : ""}${o.focus === id ? " is-focused" : ""}">${ic(icon)}<span>${label}</span></span>`;
    const iconItem = (id, icon, label) => `<span class="nav-item icon-only${current === id ? " is-current" : ""}${o.focus === id ? " is-focused" : ""}" aria-label="${label}">${ic(icon)}</span>`;
    return `<nav class="nav"><div class="nav-group">${item("home", "Home", "house")}${item("library", "Library", "library-big")}${item("search", "Search", "search")}</div><div class="nav-group">${`<span class="nav-item icon-only avatar${o.focus === "profile" ? " is-focused" : ""}" aria-label="Profile"><span>N</span></span>`}${iconItem("settings", "settings", "Settings")}</div></nav>`;
  }

  // Remote hints: key glyph in ink, the action in ink-2. Never the words OK or Back.
  function hints(list) {
    return `<div class="hints">${list.map(([k, a]) => `<span class="hint">${ic(k, "glyph")}<span>${esc(a)}</span></span>`).join("")}</div>`;
  }

  // Store logos carry wide transparent margins. The native decoder trims them to the alpha
  // bounds once; here object-view-box does the same so logos size by their ink, not their canvas.
  function trimLogo(im) {
    const c = document.createElement("canvas");
    c.width = im.naturalWidth; c.height = im.naturalHeight;
    const x = c.getContext("2d");
    x.drawImage(im, 0, 0);
    const d = x.getImageData(0, 0, c.width, c.height).data;
    let t = c.height, b = 0, l = c.width, r = 0;
    for (let y = 0; y < c.height; y++) for (let xx = 0; xx < c.width; xx++) {
      if (d[(y * c.width + xx) * 4 + 3] > 24) { if (y < t) t = y; if (y > b) b = y; if (xx < l) l = xx; if (xx > r) r = xx; }
    }
    if (r > l && b > t) im.style.objectViewBox = `inset(${t}px ${c.width - r - 1}px ${c.height - b - 1}px ${l}px)`;
  }

  // The left scrim is fitted per image, as the decoder would do natively: it measures the art
  // under the text column and darkens until ink-2 clears 7:1 on the brightest tenth of it.
  // Black over sRGB scales linear luminance by about (1 - a)^2.2; ink-2 needs L <= 0.021 behind it.
  function fitScrim(stage) {
    const im = stage.querySelector(":scope > img");
    if (!im || !im.naturalWidth) return;
    const k = 8, W = im.clientWidth, H = im.clientHeight;
    const c = document.createElement("canvas");
    c.width = Math.ceil(W / k); c.height = Math.ceil(H / k);
    const g = c.getContext("2d", { willReadFrequently: true });
    const s = Math.max(W / im.naturalWidth, H / im.naturalHeight);
    const dw = im.naturalWidth * s, dh = im.naturalHeight * s;
    g.drawImage(im, (W - dw) * 0.5 / k, (H - dh) * 0.32 / k, dw / k, dh / k);
    const d = g.getImageData(0, 0, Math.ceil(1000 / k), c.height).data;
    const lin = (v) => (v /= 255) <= 0.04045 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4);
    const lum = [];
    for (let i = 0; i < d.length; i += 4) lum.push(0.2126 * lin(d[i]) + 0.7152 * lin(d[i + 1]) + 0.0722 * lin(d[i + 2]));
    lum.sort((p, q) => p - q);
    const p90 = Math.max(lum[Math.floor(lum.length * 0.9)], 0.021);
    const a = 1 - Math.pow(0.021 / p90, 1 / 2.2);
    stage.style.setProperty("--scrim-a", Math.min(0.9, Math.max(0.72, a)).toFixed(2));
  }

  async function ready(after) {
    window.Icons.fill();
    await document.fonts.ready;
    await Promise.all([...document.images].map((im) => (im.complete ? 0 : new Promise((r) => { im.onload = im.onerror = r; }))));
    document.querySelectorAll("img[data-trim]").forEach((im) => { try { trimLogo(im); } catch (e) { console.warn("trim", e.message); } });
    document.querySelectorAll(".stage").forEach((st) => { try { fitScrim(st); } catch (e) { console.warn("scrim", e.message); } });
    // A title whose longest word does not fit steps down one type size; words never break.
    document.querySelectorAll(".plate-title").forEach((t) => { if (t.scrollWidth > t.clientWidth + 1) t.classList.add("fit"); });
    if (after) after();
    document.documentElement.dataset.ready = "1";
  }

  window.Kit = { games, engines, status, chip, where, plate, cover, row, nav, hints, ic, esc, ready };
})();
