// Game hub builder, shared by hub.html, chooser.html and states.html.
// Hub.html(key, { focus, target, state, notice, caps, dim }) returns the hub's markup; the Overview tab is shown.
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

  function html(key, o = {}) {
    const game = games[key];
    const target = o.target || game.target;
    const state = o.state || "ready";
    const e = engines[target];
    const F = (id) => (o.focus === id ? " is-focused" : "");
    const caps = o.caps || game.caps || ["4K HDR", "Full controller support", "Cloud saves"];
    const btn = (b, id, cls = "") => `<span class="btn${cls}${F(id)}">${ic(b.icon, b.icon === "play" ? "fill" : "")}<span>${esc(b.label)}</span></span>`;
    const primary = o.notice && o.notice.primary
      ? btn(o.notice.primary, "primary", " primary")
      : `<span class="btn primary${F("play")}">${ic("play", "fill")}<span>Play</span><span class="sub">${esc(e.name)}</span></span>`;
    const secondary = btn((o.notice && o.notice.secondary) || { icon: "arrow-left-right", label: "Choose where to play" }, "choose");
    return `
      <div class="stage hub-stage${o.dim ? " dim" : ""}"><img src="${game.hero}" alt=""></div>
      <header class="topbar on-stage">
        <span class="btn icon-only${F("back")}" aria-label="Back">${ic("arrow-left")}</span>
        <span class="crumb">${esc(game.title)}</span>
        <span class="btn icon-only${F("fav")}" aria-label="Favourite">${ic("heart")}</span>
        <span class="btn icon-only${F("more")}" aria-label="More">${ic("ellipsis")}</span>
      </header>
      <main class="hub">
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
          <div class="tabs"><span class="tab is-selected${F("tab")}">Overview</span><span class="tab">Ways to play</span><span class="tab">Media</span></div>
          <div class="tab-pane">${facts(game, target)}</div>
        </section>
      </main>`;
  }

  // Light themes end the art band right under the on-stage block (see .hub-stage).
  function settle() {
    const top = document.querySelector(".hub-top");
    const stage = document.querySelector(".hub-stage");
    if (top && stage) stage.style.setProperty("--hub-band", Math.round(top.getBoundingClientRect().bottom + 6) + "px");
  }

  window.Hub = { html, settle };
})();
