// Game hub builder, shared by hub.html, chooser.html and states.html.
// Hub.html(key, { focus, target, state, notice, caps, dim, tab, ways, media }) returns the hub's markup.
(function () {
  const { games, engines, ic, esc } = window.Kit;

  // Where the selected target lives, per engine (illustrative accounts and hosts).
  const place = {
    gfn: "nitin · Ultimate",
    sunshine: "BEAST-PC · Sunshine",
    punktfunk: "Desk PC · Punktfunk",
    native: "Installed",
  };
  const stateMark = {
    ready: ["dot ok", "Ready"],
    asleep: ["moon", "Asleep, wakes when you play"],
    offline: ["wifi-off", "Offline"],
    installed: ["dot ok", "Ready"],
  };

  function targetLine(game, target, state) {
    const e = engines[target];
    const [mark, word] = stateMark[state] || stateMark.ready;
    const m = mark.startsWith("dot") ? `<span class="${mark}"></span>` : ic(mark, `st-${state}`);
    const where = target === "native" ? "" : `<i class="sep"></i><span>${esc(place[target])}</span>`;
    return `<div class="target-line">${ic(e.icon)}<span>${esc(e.name)}</span>${where}<i class="sep"></i>${m}<span class="ink">${esc(word)}</span></div>`;
  }

  // A notice replaces the target line when a launch failed or the target cannot be reached.
  function noticeCard(n) {
    return `<div class="notice ${n.tone}">${ic(n.icon)}<div><div class="notice-title">${esc(n.title)}</div><div class="notice-body">${esc(n.body)}</div></div></div>`;
  }

  function facts(game, target) {
    const list = [
      ["Last played", (game.played || "Last month").replace(/^Played /, "")],
      ["Play time", (game.hours || "0 h").replace(" h", " hours")],
      ["Achievements", game.ach || "18 of 40"],
      ["Saves", target === "native" ? "On this TV" : "Steam Cloud"],
    ];
    return `<div class="facts">${list.map(([k, v]) => `<div class="fact"><div class="k">${esc(k)}</div><div class="v">${esc(v)}</div></div>`).join("")}</div>`;
  }

  const saves = {
    shared: ["badge-check", "Shared"],
    separate: ["unlink", "Separate"],
    unknown: ["circle-question-mark", "Unknown"],
  };
  const wayHead = `<div class="crow-head"><span></span><span>Where</span><span>Saves</span><span>Status</span><span></span></div>`;

  // One row per way to play, in the chooser and the Ways to play tab. The radio marks the target
  // Play uses; status and save compatibility come from the plugins.
  function wayRow(r) {
    const e = engines[r.e];
    const [sIc, sWord] = saves[r.saves];
    return `<div class="crow${r.focus ? " is-focused" : ""}">
      <span class="eng">${ic(e.icon)}</span>
      <span class="who">${esc(e.name)}<small>${esc(r.sub)}</small></span>
      <span class="meta"><span class="saves ${r.saves}">${ic(sIc)}<span>${sWord}</span></span>
      <span class="status">${r.status}</span></span>
      <span class="radio${r.on ? " on" : ""}"></span>
    </div>`;
  }

  // Trailers are named like collections; screenshots are pictures and carry no label. A trailer
  // plays only when selected.
  function mediaRow(head, items, focusAt) {
    const tile = (m, i) => `<div class="coll${i === focusAt ? " is-focused" : ""}">
      <div class="tile collection"><img src="${m.src}" alt="">${m.name ? `<span class="play-badge on-stage">${ic("play", "fill")}</span>` : ""}</div>
      <span class="land"></span>
      ${m.name ? `<div class="coll-label"><div class="tile-name">${esc(m.name)}</div><div class="tile-count">${esc(m.sub)}</div></div>` : ""}
    </div>`;
    return `<div class="media-head">${esc(head)}</div><div class="media-track">${items.map(tile).join("")}</div>`;
  }

  function pane(game, target, o, tab) {
    if (tab === "ways") {
      const add = `<div class="crow add${o.focus === "add" ? " is-focused" : ""}"><span class="eng">${ic("plus")}</span><span class="who">Add a way to play<small>Sign in to a cloud service or pair a PC</small></span></div>`;
      return `<div class="way-list">${wayHead}${o.ways.map(wayRow).join("")}${add}</div>`;
    }
    if (tab === "media") return mediaRow("Trailers", o.media.trailers, o.focus === "media" ? 0 : -1) + mediaRow("Screenshots", o.media.shots, -1);
    return facts(game, target);
  }

  const tabs = [["overview", "Overview"], ["ways", "Ways to play"], ["media", "Media"]];

  function html(key, o = {}) {
    const game = games[key];
    const target = o.target || game.target;
    const state = o.state || "ready";
    const e = engines[target];
    const tab = o.tab || "overview";
    const deep = tab !== "overview";
    const F = (id) => (o.focus === id ? " is-focused" : "");
    const caps = o.caps || game.caps || ["4K HDR", "Full controller support", "Cloud saves"];
    const btn = (b, id, cls = "") => `<span class="btn${cls}${F(id)}">${ic(b.icon, b.icon === "play" ? "fill" : "")}<span>${esc(b.label)}</span></span>`;
    const primary = o.notice && o.notice.primary
      ? btn(o.notice.primary, "primary", " primary")
      : `<span class="btn primary${F("play")}">${ic("play", "fill")}<span>Play</span><span class="sub">${esc(e.name)}</span></span>`;
    const secondary = btn((o.notice && o.notice.secondary) || { icon: "arrow-left-right", label: "Choose where to play" }, "choose");
    // Below Overview the art stays only as a dim backdrop (none in light themes), so the top bar
    // takes the theme's own roles there.
    return `
      <div class="stage hub-stage${o.dim || deep ? " dim" : ""}"><img src="${game.hero}" alt=""></div>
      <header class="topbar${deep ? "" : " on-stage"}">
        <span class="btn icon-only${F("back")}" aria-label="Back">${ic("arrow-left")}</span>
        <span class="crumb">${esc(game.title)}</span>
        <span class="btn icon-only${F("fav")}" aria-label="Favourite">${ic("heart")}</span>
        <span class="btn icon-only${F("more")}" aria-label="More">${ic("ellipsis")}</span>
      </header>
      <main class="hub${deep ? " is-deep" : ""}">
        <section class="hub-top on-stage">
          <img class="hub-logo" data-trim src="${game.logo}" alt="${esc(game.title)}">
          <div class="hero-meta"><span>${esc(game.genre)}</span><i class="sep"></i><span>${game.year}</span><i class="sep"></i><span>${esc(game.players)}</span></div>
          ${game.desc && !o.notice ? `<p class="hero-desc">${esc(game.desc)}</p>` : ""}
          <div class="cap-row">${caps.map((c) => `<span class="cap">${ic("check")}<span>${esc(c)}</span></span>`).join("")}</div>
          <div class="actions">
            ${primary}
            ${secondary}
          </div>
          ${o.notice ? noticeCard(o.notice) : targetLine(game, target, state)}
        </section>
        <section class="hub-body">
          <div class="tabs">${tabs.map(([id, label]) => `<span class="tab${id === tab ? " is-selected" + F("tab") : ""}">${label}</span>`).join("")}</div>
          <div class="tab-pane">${pane(game, target, o, tab)}</div>
        </section>
      </main>`;
  }

  function settle() {
    const hub = document.querySelector(".hub");
    const top = document.querySelector(".hub-top");
    const stage = document.querySelector(".hub-stage");
    if (hub && hub.classList.contains("is-deep")) {
      // Home's focus line: the tabs land at y=170 under the top bar and the block above leaves.
      const y = document.querySelector(".hub-body").getBoundingClientRect().top;
      hub.style.transform = `translateY(${Math.round(170 - y)}px)`;
      top.classList.add("past");
      return;
    }
    // Light themes end the art band right under the on-stage block (see .hub-stage).
    if (top && stage) stage.style.setProperty("--hub-band", Math.round(top.getBoundingClientRect().bottom + 6) + "px");
  }

  window.Hub = { html, settle, wayRow, wayHead };
})();
