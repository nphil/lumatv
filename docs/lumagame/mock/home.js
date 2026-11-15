// Home: full-bleed still art, the floating nav, one featured game, then rows.
// ?focus=<row>:<item> moves focus; rows below the first scroll the hero away and the stage
// shows the focused game's art, dimmed (it dissolves in 200 ms after focus settles).
(function () {
  const { games, cover, nav, ic, esc, where } = window.Kit;
  const q = new URLSearchParams(location.search);
  const [focusRow, focusItem] = (q.get("focus") || "0:1").split(":").map(Number);
  const featured = games.hades2;

  const rows = [
    { title: "Continue", keys: ["bg3", "cyberpunk", "clair", "stardew", "forza", "elden", "ittakestwo"] },
    { title: "Most played", keys: ["stardew", "elden", "witcher", "bg3", "rdr2", "rocket", "hollow"] },
    { title: "Collections", kind: "collection", items: [
      ["Couch co-op", "6 games", "ittakestwo"], ["Short sessions", "9 games", "balatro"], ["Showcase in 4K HDR", "7 games", "forza"], ["Finished", "14 games", "witcher"], ["Waiting to start", "11 games", "wukong"],
    ] },
    { title: "Genres", kind: "genre", items: [
      ["RPG", "8 games", ["bg3", "disco", "clair"]], ["Action", "6 games", ["wukong", "mhwilds", "control"]], ["Platformer", "4 games", ["celeste", "ori", "hollow"]], ["Shooter", "3 games", ["quake", "doom", "cyberpunk"]], ["Racing", "2 games", ["forza", "rocket"]],
    ] },
    { title: "Native on this TV", keys: ["quake", "doom", "openttd", "celesteclassic", "supertuxkart", "freedoom"], state: "installed" },
  ];

  function tileRow(r, ri) {
    const tiles = r.items.map((it, i) => {
      const focused = ri === focusRow && i === focusItem ? " is-focused" : "";
      if (r.kind === "collection") {
        const [name, count, key] = it;
        return `<div class="coll${focused}"><div class="tile collection"><img src="${games[key].hero}" alt=""></div><span class="land"></span><div class="coll-label"><div class="tile-name">${esc(name)}</div><div class="tile-count">${esc(count)}</div></div></div>`;
      }
      const [name, count, keys] = it;
      return `<div class="tile genre${focused}"><div class="tile-name">${esc(name)}</div><div class="tile-count">${esc(count)}</div><div class="strip">${keys.map((k) => `<img src="${games[k].cover}" alt="">`).join("")}</div></div>`;
    }).join("");
    return `<section class="row tiles"><h2 class="row-head">${esc(r.title)}</h2><div class="row-track">${tiles}</div></section>`;
  }

  function coverRow(r, ri) {
    const items = r.keys.map((k, i) => cover(k, { focused: ri === focusRow && i === focusItem, state: r.state || "ready" })).join("");
    return `<section class="row${ri === focusRow ? " has-focus" : ""}"><h2 class="row-head">${esc(r.title)}</h2><div class="row-track">${items}</div></section>`;
  }

  const density = document.documentElement.dataset.density;
  const showDesc = density === "cinematic";
  const focusedKey = focusRow >= 0 && rows[focusRow].keys ? rows[focusRow].keys[focusItem] : null;
  const stageGame = focusRow > 0 && focusedKey && games[focusedKey].hero ? games[focusedKey] : featured;

  const hero = `<header class="hero on-stage">
      <img class="hero-logo" data-trim src="${featured.logo}" alt="${esc(featured.title)}">
      <div class="hero-meta"><span>${esc(featured.genre)}</span><i class="sep"></i><span>${esc(featured.played)}</span><i class="sep"></i><span>${esc(featured.hours)} played</span></div>
      ${showDesc ? `<p class="hero-desc">${esc(featured.desc)}</p>` : ""}
      <div class="actions">
        <span class="btn primary${focusRow < 0 ? " is-focused" : ""}">${ic("play", "fill")}<span>Play</span><span class="sub">GeForce NOW</span></span>
        <span class="btn">${ic("info")}<span>Game details</span></span>
      </div>
    </header>`;

  document.querySelector(".screen").innerHTML = `
    <div class="stage${focusRow > 0 ? " dim" : ""}"><img src="${stageGame.hero}" alt=""></div>
    ${nav("home")}
    <main class="scroll">${hero}${rows.map((r, i) => (r.kind ? tileRow(r, i) : coverRow(r, i))).join("")}</main>`;

  // A fixed focus line: the focused row's header always lands at y=170 under the nav; rows
  // scrolled above it leave, so nothing slides beneath the nav.
  window.Kit.ready(() => {
    if (focusRow > 0) {
      const rowsEl = [...document.querySelectorAll(".scroll > .row")];
      const y = rowsEl[focusRow].getBoundingClientRect().top;
      document.querySelector(".scroll").style.transform = `translateY(${Math.round(170 - y)}px)`;
      for (const el of [document.querySelector(".hero"), ...rowsEl]) if (el.getBoundingClientRect().bottom < 160) el.classList.add("past");
    }
  });
})();
