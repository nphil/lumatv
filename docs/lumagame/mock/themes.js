// Theme families for the LumaGame mocks. Each family carries the 20 Lucent palette roles in a
// dark and a light variant; the page swaps all of them in one style write so a theme change is
// atomic. Rosé Pine uses the contrast-fitted values the native shell ships
// (rustui/tokens/src/palettes.json). The other three exist to prove the token layer.
//
// Derived roles:
//   primary   filled-button colour; the accent unless accentInk on it misses 7:1
//   stage*    the dark stage game art sits on, taken from the family's dark variant
//
// URL: ?family=rose-pine|graphite-chalk|nordic-fjord|copper-kiln &theme=dark|light
//      &density=compact|standard|cinematic &text=100|200 &motion=reduced
(function () {
  const families = {
    "rose-pine": {
      name: "Rosé Pine",
      dark: { canvas: "#191724", sheet: "#1F1D2EE8", scrim: "#120E1A6B", glass: "#120E1A66", glassRaised: "#2C263A8F", edge: "#FFFFFF1F", edgeRaised: "#FFFFFF33", ink: "#F2EFF6", ink2: "#F2EFF6B1", ink3: "#F2EFF66B", focus: "#F2EFF6FA", focusInk: "#17131F", trackOff: "#FFFFFF29", accent: "#A99DF5", accentInk: "#17131F", positive: "#9CCFD8", warning: "#F6C177", danger: "#EF8AA6", info: "#9CCFD8", live: "#EF8AA6" },
      light: { canvas: "#FAF4ED", sheet: "#FFFAF3EB", scrim: "#FAF4ED4D", glass: "#FFFFFF85", glassRaised: "#FFFFFFB8", edge: "#2E2A451F", edgeRaised: "#2E2A4533", ink: "#2E2A45", ink2: "#2E2A45D5", ink3: "#2E2A4587", focus: "#FFFFFFF2", focusInk: "#2E2A45", trackOff: "#2E2A451F", accent: "#765E92", accentInk: "#FFFFFF", positive: "#286983", warning: "#925B0F", danger: "#A14D65", info: "#416F77", live: "#A14D65", primary: "#5E4B75" },
    },
    "graphite-chalk": {
      name: "Graphite & Chalk",
      dark: { canvas: "#141517", sheet: "#1D1E21E8", scrim: "#0A0B0C6B", glass: "#0A0B0C66", glassRaised: "#2B2D318F", edge: "#FFFFFF1F", edgeRaised: "#FFFFFF33", ink: "#F1F0EC", ink2: "#F1F0ECB4", ink3: "#F1F0EC6B", focus: "#F1F0ECFA", focusInk: "#141517", trackOff: "#FFFFFF29", accent: "#A9C8E0", accentInk: "#141517", positive: "#A6D3AE", warning: "#E8CF8A", danger: "#F0A3A3", info: "#A9C8E0", live: "#F0A3A3" },
      light: { canvas: "#ECEBE7", sheet: "#F8F7F4EB", scrim: "#ECEBE74D", glass: "#FFFFFF85", glassRaised: "#FFFFFFB8", edge: "#1B1C1F1F", edgeRaised: "#1B1C1F33", ink: "#1B1C1F", ink2: "#1B1C1FCC", ink3: "#1B1C1F80", focus: "#FFFFFFF2", focusInk: "#1B1C1F", trackOff: "#1B1C1F1F", accent: "#2E4C66", accentInk: "#FFFFFF", positive: "#2F6B3E", warning: "#7A5A12", danger: "#9B3434", info: "#2F5A7A", live: "#9B3434" },
    },
    "nordic-fjord": {
      name: "Nordic Fjord",
      dark: { canvas: "#0E1A20", sheet: "#14242CE8", scrim: "#070F136B", glass: "#070F1366", glassRaised: "#1F35408F", edge: "#FFFFFF1F", edgeRaised: "#FFFFFF33", ink: "#EAF3F4", ink2: "#EAF3F4B4", ink3: "#EAF3F46B", focus: "#EAF3F4FA", focusInk: "#0B161B", trackOff: "#FFFFFF29", accent: "#7DCFC4", accentInk: "#0B161B", positive: "#9AD7A8", warning: "#EBCB8B", danger: "#F2A1A8", info: "#8FC1E3", live: "#F2A1A8" },
      light: { canvas: "#E5EDEE", sheet: "#F5FAFAEB", scrim: "#E5EDEE4D", glass: "#FFFFFF85", glassRaised: "#FFFFFFB8", edge: "#0F2A321F", edgeRaised: "#0F2A3233", ink: "#0F2A32", ink2: "#0F2A32CC", ink3: "#0F2A3282", focus: "#FFFFFFF2", focusInk: "#0F2A32", trackOff: "#0F2A321F", accent: "#1A5F5A", accentInk: "#FFFFFF", positive: "#2A6B3F", warning: "#7D5A0E", danger: "#A23B4A", info: "#275F86", live: "#A23B4A" },
    },
    "copper-kiln": {
      name: "Copper Kiln",
      dark: { canvas: "#1B1411", sheet: "#251B17E8", scrim: "#0E0A086B", glass: "#0E0A0866", glassRaised: "#3628228F", edge: "#FFFFFF1F", edgeRaised: "#FFFFFF33", ink: "#F6ECE3", ink2: "#F6ECE3B4", ink3: "#F6ECE36B", focus: "#F6ECE3FA", focusInk: "#1B1411", trackOff: "#FFFFFF29", accent: "#E9A47B", accentInk: "#1B1411", positive: "#A9CF8F", warning: "#F0C46E", danger: "#F29A8E", info: "#9CC3D0", live: "#F29A8E" },
      light: { canvas: "#F3E9DE", sheet: "#FFF8F0EB", scrim: "#F3E9DE4D", glass: "#FFFFFF85", glassRaised: "#FFFFFFB8", edge: "#2B1A121F", edgeRaised: "#2B1A1233", ink: "#2B1A12", ink2: "#2B1A12CC", ink3: "#2B1A1282", focus: "#FFFFFFF2", focusInk: "#2B1A12", trackOff: "#2B1A121F", accent: "#8E4220", accentInk: "#FFFFFF", positive: "#3F6A2A", warning: "#7E5410", danger: "#A3382E", info: "#2F6577", live: "#A3382E" },
    },
  };

  const kebab = (k) => k.replace(/[A-Z]/g, (c) => "-" + c.toLowerCase()).replace(/(\D)(\d)/g, "$1-$2");

  function roles(familyId, mode) {
    const fam = families[familyId] || families["rose-pine"];
    const p = { ...fam[mode] };
    p.primary = p.primary || p.accent;
    // The stage carries the family's dark variant whole, so on-stage content resolves every role there.
    const d = { ...fam.dark, primary: fam.dark.primary || fam.dark.accent };
    for (const [k, v] of Object.entries(d)) p["stage" + k[0].toUpperCase() + k.slice(1)] = v;
    p.stage = d.canvas;
    return p;
  }

  function apply(opts = {}) {
    const q = new URLSearchParams(location.search);
    const family = opts.family || q.get("family") || document.documentElement.dataset.family || "rose-pine";
    const mode = opts.theme || q.get("theme") || document.documentElement.dataset.theme || "dark";
    const density = opts.density || q.get("density") || document.documentElement.dataset.density || "standard";
    const text = Number(opts.text || q.get("text") || 100) / 100;
    const motion = opts.motion || q.get("motion") || "full";
    const p = roles(family, mode);
    const css = Object.entries(p).map(([k, v]) => `--pal-${kebab(k)}:${v}`).join(";") + `;--lg-text-scale:${text}`;
    const root = document.documentElement;
    root.setAttribute("style", css);
    Object.assign(root.dataset, { family, theme: mode, density, motion, luProfile: "tv" });
    root.dataset.text = String(Math.round(text * 100));
  }

  window.LumaThemes = { families, roles, apply };
  apply();
})();
