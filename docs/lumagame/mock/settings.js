// Settings shell shared by settings.html, integrations.html, pairing.html and signin.html.
// Settings.html(section, { focus }) returns the nav, the section rail and that section's pane.
(function () {
  const { ic, esc, nav } = window.Kit;
  const { families } = window.LumaThemes;

  const sections = [
    ["appearance", "Appearance", "palette"],
    ["streaming", "Streaming", "monitor-play"],
    ["integrations", "Integrations", "plug"],
    ["controllers", "Controllers", "gamepad-2"],
    ["about", "About", "info"],
  ];

  const row = (label, right, o = {}) =>
    `<div class="srow${o.focus ? " is-focused" : ""}${o.disabled ? " is-disabled" : ""}">${o.icon ? ic(o.icon) : ""}<span class="grow">${esc(label)}${o.sub ? `<span class="sub">${esc(o.sub)}</span>` : ""}</span>${right || ""}</div>`;
  const seg = (opts, sel) => `<span class="seg">${opts.map((x) => `<span${x === sel ? ' class="is-selected"' : ""}>${esc(x)}</span>`).join("")}</span>`;
  // The tick is a glyph, not a rotated stroke: the native layer has no rotation.
  const toggle = (on) => `<span class="toggle${on ? " on" : ""}">${on ? ic("check", "tick") : ""}</span>`;
  // Left and right arrows show only on the focused row: they say which keys change the value.
  const stepper = (v) => `<span class="val"><span class="stepper">${ic("chevron-left")}<span>${esc(v)}</span>${ic("chevron-right")}</span></span>`;
  const slider = (v, max, unit) => {
    const at = (v / max) * 100;
    return `<span class="val"><span class="slider"><b style="width:${at}%"></b><i style="left:${at}%"></i></span><span class="num">${v} ${esc(unit)}</span></span>`;
  };
  const value = (v, chev) => `<span class="val">${esc(v)}${chev ? ic("chevron-right") : ""}</span>`;
  const group = (label) => `<div class="group-label">${esc(label)}</div>`;

  // Swatches preview each family in the current mode: its canvas with accent, positive,
  // warning and ink. These are the only literal colours in the kit, because they are data.
  function swatches(focus) {
    const root = document.documentElement.dataset;
    return `<div class="swatches">${Object.entries(families).map(([id, f]) => {
      const p = f[root.theme || "dark"];
      const dots = [p.accent, p.positive, p.warning, p.ink].map((c) => `<i style="background:${c}"></i>`).join("");
      return `<div class="swatch${id === root.family ? " is-selected" : ""}${id === focus ? " is-focused" : ""}">
        <span class="chipset" style="--chip-bg:${p.canvas};background:${p.canvas}">${dots}</span>
        <span class="name"><span>${esc(f.name)}</span>${ic("circle-check", "sel")}</span>
      </div>`;
    }).join("")}</div>`;
  }

  const panes = {
    appearance: (f) => {
      const dark = (document.documentElement.dataset.theme || "dark") === "dark";
      return swatches(f === "swatch" ? "nordic-fjord" : null) +
        row("Mode", seg(["Dark", "Light"], dark ? "Dark" : "Light"), { focus: f === "mode" }) +
        row("Featured game", seg(["Compact", "Standard", "Cinematic"], "Standard")) +
        row("Covers", seg(["Compact", "Standard", "Large"], "Standard")) +
        row("Text size", seg(["100%", "150%", "200%"], "100%")) +
        row("Reduce motion", toggle(false), { sub: "No lift, zoom or fades" });
    },
    streaming: (f) =>
      group("Picture") +
      row("Resolution", stepper("4K")) +
      row("Frame rate", stepper("120 fps")) +
      row("HDR", toggle(true)) +
      row("Bitrate", slider(80, 150, "Mb/s"), { focus: f === "bitrate" }) +
      row("Codec", seg(["Automatic", "HEVC", "AV1", "H.264"], "Automatic")) +
      group("Playing") +
      row("Preferred way to play", stepper("GeForce NOW"), { focus: f === "preferred" }) +
      row("Stats while playing", seg(["Off", "Compact", "Full"], "Off")),
    // One row per plugin; each opens its own page, where it is switched on and connected.
    integrations: (f) =>
      group("Cloud") +
      row("GeForce NOW", value("On", true), { icon: "cloud", sub: "Signed in as nitin · Ultimate" }) +
      group("Your PCs") +
      row("Sunshine and Apollo", value("On", true), { icon: "monitor", sub: "BEAST-PC ready · Living room PC asleep", focus: f === "sunshine" }) +
      row("Punktfunk", value("On", true), { icon: "radio", sub: "Desk PC ready" }) +
      group("On this TV") +
      row("Native games", value("On", true), { icon: "tv", sub: "6 installed" }) +
      group("Later") +
      row("Emulators", value("Coming later"), { icon: "joystick", disabled: true }) +
      row("Steam Remote Play", value("Not available yet"), { icon: "gamepad-2", disabled: true }),
  };

  // The rail carries the page title; its current item names the section, so the pane has none.
  function html(section, o = {}) {
    const rail = sections.map(([id, label, icon]) => `<div class="srow${id === section ? " is-current" : ""}${o.focus === "rail-" + id ? " is-focused" : ""}">${ic(icon)}<span class="grow">${esc(label)}</span></div>`).join("");
    return nav("settings") + `
      <main class="settings">
        <aside class="rail"><h1 class="t-title">Settings</h1>${rail}</aside>
        <section class="pane">${panes[section](o.focus)}</section>
      </main>`;
  }

  window.Settings = { html, row, seg, toggle, stepper, slider, value, group };
})();
