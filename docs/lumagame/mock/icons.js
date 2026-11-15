// Icons for the LumaGame mocks: a Lucide v1.27.0 subset (ISC licence, lucide.dev) and the
// Lucent remote-key glyphs from assets/glyphs. Stroke 1.9 on a 24 grid, coloured by currentColor.
// Icons.fill() swaps every <i data-ic="name"> for its SVG.
(function () {
  const paths = {
    "house": "<path d=\"M15 21v-8a1 1 0 0 0-1-1h-4a1 1 0 0 0-1 1v8\"/><path d=\"M3 10a2 2 0 0 1 .709-1.528l7-6a2 2 0 0 1 2.582 0l7 6A2 2 0 0 1 21 10v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z\"/>",
    "library-big": "<rect width=\"8\" height=\"18\" x=\"3\" y=\"3\" rx=\"1\"/><path d=\"M7 3v18\"/><path d=\"M20.4 18.9c.2.5-.1 1.1-.6 1.3l-1.9.7c-.5.2-1.1-.1-1.3-.6L11.1 5.1c-.2-.5.1-1.1.6-1.3l1.9-.7c.5-.2 1.1.1 1.3.6Z\"/>",
    "search": "<path d=\"m21 21-4.34-4.34\"/><circle cx=\"11\" cy=\"11\" r=\"8\"/>",
    "user-round": "<circle cx=\"12\" cy=\"8\" r=\"5\"/><path d=\"M20 21a8 8 0 0 0-16 0\"/>",
    "settings": "<path d=\"M9.671 4.136a2.34 2.34 0 0 1 4.659 0 2.34 2.34 0 0 0 3.319 1.915 2.34 2.34 0 0 1 2.33 4.033 2.34 2.34 0 0 0 0 3.831 2.34 2.34 0 0 1-2.33 4.033 2.34 2.34 0 0 0-3.319 1.915 2.34 2.34 0 0 1-4.659 0 2.34 2.34 0 0 0-3.32-1.915 2.34 2.34 0 0 1-2.33-4.033 2.34 2.34 0 0 0 0-3.831A2.34 2.34 0 0 1 6.35 6.051a2.34 2.34 0 0 0 3.319-1.915\"/><circle cx=\"12\" cy=\"12\" r=\"3\"/>",
    "play": "<path d=\"M5 5a2 2 0 0 1 3.008-1.728l11.997 6.998a2 2 0 0 1 .003 3.458l-12 7A2 2 0 0 1 5 19z\"/>",
    "heart": "<path d=\"M2 9.5a5.5 5.5 0 0 1 9.591-3.676.56.56 0 0 0 .818 0A5.49 5.49 0 0 1 22 9.5c0 2.29-1.5 4-3 5.5l-5.492 5.313a2 2 0 0 1-3 .019L5 15c-1.5-1.5-3-3.2-3-5.5\"/>",
    "ellipsis": "<circle cx=\"12\" cy=\"12\" r=\"1\"/><circle cx=\"19\" cy=\"12\" r=\"1\"/><circle cx=\"5\" cy=\"12\" r=\"1\"/>",
    "arrow-left": "<path d=\"m12 19-7-7 7-7\"/><path d=\"M19 12H5\"/>",
    "chevron-left": "<path d=\"m15 18-6-6 6-6\"/>",
    "chevron-right": "<path d=\"m9 18 6-6-6-6\"/>",
    "chevron-down": "<path d=\"m6 9 6 6 6-6\"/>",
    "check": "<path d=\"M20 6 9 17l-5-5\"/>",
    "x": "<path d=\"M18 6 6 18\"/><path d=\"m6 6 12 12\"/>",
    "cloud": "<path d=\"M17.5 19H9a7 7 0 1 1 6.71-9h1.79a4.5 4.5 0 1 1 0 9Z\"/>",
    "monitor": "<rect width=\"20\" height=\"14\" x=\"2\" y=\"3\" rx=\"2\"/><line x1=\"8\" x2=\"16\" y1=\"21\" y2=\"21\"/><line x1=\"12\" x2=\"12\" y1=\"17\" y2=\"21\"/>",
    "radio": "<path d=\"M16.247 7.761a6 6 0 0 1 0 8.478\"/><path d=\"M19.075 4.933a10 10 0 0 1 0 14.134\"/><path d=\"M4.925 19.067a10 10 0 0 1 0-14.134\"/><path d=\"M7.753 16.239a6 6 0 0 1 0-8.478\"/><circle cx=\"12\" cy=\"12\" r=\"2\"/>",
    "tv": "<path d=\"m17 2-5 5-5-5\"/><rect width=\"20\" height=\"15\" x=\"2\" y=\"7\" rx=\"2\"/>",
    "joystick": "<path d=\"M21 17a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v2a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-2Z\"/><path d=\"M6 15v-2\"/><path d=\"M12 15V9\"/><circle cx=\"12\" cy=\"6\" r=\"3\"/>",
    "gamepad-2": "<line x1=\"6\" x2=\"10\" y1=\"11\" y2=\"11\"/><line x1=\"8\" x2=\"8\" y1=\"9\" y2=\"13\"/><line x1=\"15\" x2=\"15.01\" y1=\"12\" y2=\"12\"/><line x1=\"18\" x2=\"18.01\" y1=\"10\" y2=\"10\"/><path d=\"M17.32 5H6.68a4 4 0 0 0-3.978 3.59c-.006.052-.01.101-.017.152C2.604 9.416 2 14.456 2 16a3 3 0 0 0 3 3c1 0 1.5-.5 2-1l1.414-1.414A2 2 0 0 1 9.828 16h4.344a2 2 0 0 1 1.414.586L17 18c.5.5 1 1 2 1a3 3 0 0 0 3-3c0-1.545-.604-6.584-.685-7.258-.007-.05-.011-.1-.017-.151A4 4 0 0 0 17.32 5z\"/>",
    "wifi-off": "<path d=\"M12 20h.01\"/><path d=\"M8.5 16.429a5 5 0 0 1 7 0\"/><path d=\"M5 12.859a10 10 0 0 1 5.17-2.69\"/><path d=\"M19 12.859a10 10 0 0 0-2.007-1.523\"/><path d=\"M2 8.82a15 15 0 0 1 4.177-2.643\"/><path d=\"M22 8.82a15 15 0 0 0-11.288-3.764\"/><path d=\"m2 2 20 20\"/>",
    "moon": "<path d=\"M20.985 12.486a9 9 0 1 1-9.473-9.472c.405-.022.617.46.402.803a6 6 0 0 0 8.268 8.268c.344-.215.825-.004.803.401\"/>",
    "power": "<path d=\"M12 2v10\"/><path d=\"M18.4 6.6a9 9 0 1 1-12.77.04\"/>",
    "zap": "<path d=\"M15.914 4a1.5 1.5 0 00-2.474-1.561l-9 9A1.5 1.5 0 005.5 14h4.002a.5.5 0 01.471.666L8.086 20a1.5 1.5 0 002.475 1.56l9-9A1.5 1.5 0 0018.5 10h-3.997a.5.5 0 01-.472-.667z\"/>",
    "clock": "<circle cx=\"12\" cy=\"12\" r=\"10\"/><path d=\"M12 6v6l4 2\"/>",
    "users": "<path d=\"M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2\"/><path d=\"M16 3.128a4 4 0 0 1 0 7.744\"/><path d=\"M22 21v-2a4 4 0 0 0-3-3.87\"/><circle cx=\"9\" cy=\"7\" r=\"4\"/>",
    "download": "<path d=\"M12 15V3\"/><path d=\"M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4\"/><path d=\"m7 10 5 5 5-5\"/>",
    "qr-code": "<rect width=\"5\" height=\"5\" x=\"3\" y=\"3\" rx=\"1\"/><rect width=\"5\" height=\"5\" x=\"16\" y=\"3\" rx=\"1\"/><rect width=\"5\" height=\"5\" x=\"3\" y=\"16\" rx=\"1\"/><path d=\"M21 16h-3a2 2 0 0 0-2 2v3\"/><path d=\"M21 21v.01\"/><path d=\"M12 7v3a2 2 0 0 1-2 2H7\"/><path d=\"M3 12h.01\"/><path d=\"M12 3h.01\"/><path d=\"M12 16v.01\"/><path d=\"M16 12h1\"/><path d=\"M21 12v.01\"/><path d=\"M12 21v-1\"/>",
    "key-round": "<path d=\"M2.586 17.414A2 2 0 0 0 2 18.828V21a1 1 0 0 0 1 1h3a1 1 0 0 0 1-1v-1a1 1 0 0 1 1-1h1a1 1 0 0 0 1-1v-1a1 1 0 0 1 1-1h.172a2 2 0 0 0 1.414-.586l.814-.814a6.5 6.5 0 1 0-4-4z\"/><circle cx=\"16.5\" cy=\"7.5\" r=\".5\" fill=\"currentColor\"/>",
    "link": "<path d=\"M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71\"/><path d=\"M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71\"/>",
    "plug": "<path d=\"M12 22v-5\"/><path d=\"M15 8V2\"/><path d=\"M17 8a1 1 0 0 1 1 1v4a4 4 0 0 1-4 4h-4a4 4 0 0 1-4-4V9a1 1 0 0 1 1-1z\"/><path d=\"M9 8V2\"/>",
    "palette": "<path d=\"M12 22a1 1 0 0 1 0-20 10 9 0 0 1 10 9 5 5 0 0 1-5 5h-2.25a1.75 1.75 0 0 0-1.4 2.8l.3.4a1.75 1.75 0 0 1-1.4 2.8z\"/><circle cx=\"13.5\" cy=\"6.5\" r=\".5\" fill=\"currentColor\"/><circle cx=\"17.5\" cy=\"10.5\" r=\".5\" fill=\"currentColor\"/><circle cx=\"6.5\" cy=\"12.5\" r=\".5\" fill=\"currentColor\"/><circle cx=\"8.5\" cy=\"7.5\" r=\".5\" fill=\"currentColor\"/>",
    "sliders-horizontal": "<path d=\"M10 5H3\"/><path d=\"M12 19H3\"/><path d=\"M14 3v4\"/><path d=\"M16 17v4\"/><path d=\"M21 12h-9\"/><path d=\"M21 19h-5\"/><path d=\"M21 5h-7\"/><path d=\"M8 10v4\"/><path d=\"M8 12H3\"/>",
    "type": "<path d=\"M12 4v16\"/><path d=\"M4 7V5a1 1 0 0 1 1-1h14a1 1 0 0 1 1 1v2\"/><path d=\"M9 20h6\"/>",
    "layout-grid": "<rect width=\"7\" height=\"7\" x=\"3\" y=\"3\" rx=\"1\"/><rect width=\"7\" height=\"7\" x=\"14\" y=\"3\" rx=\"1\"/><rect width=\"7\" height=\"7\" x=\"14\" y=\"14\" rx=\"1\"/><rect width=\"7\" height=\"7\" x=\"3\" y=\"14\" rx=\"1\"/>",
    "circle-alert": "<circle cx=\"12\" cy=\"12\" r=\"10\"/><line x1=\"12\" x2=\"12\" y1=\"8\" y2=\"12\"/><line x1=\"12\" x2=\"12.01\" y1=\"16\" y2=\"16\"/>",
    "refresh-cw": "<path d=\"M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8\"/><path d=\"M21 3v5h-5\"/><path d=\"M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16\"/><path d=\"M8 16H3v5\"/>",
    "loader-circle": "<path d=\"M21 12a9 9 0 1 1-6.219-8.56\"/>",
    "pause": "<rect x=\"14\" y=\"3\" width=\"5\" height=\"18\" rx=\"1\"/><rect x=\"5\" y=\"3\" width=\"5\" height=\"18\" rx=\"1\"/>",
    "log-out": "<path d=\"m16 17 5-5-5-5\"/><path d=\"M21 12H9\"/><path d=\"M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4\"/>",
    "gauge": "<path d=\"m12 14 4-4\"/><path d=\"M3.34 19a10 10 0 1 1 17.32 0\"/>",
    "trophy": "<path d=\"M10 14.66V17a1 1 0 0 1-1 1 2 2 0 0 0-2 2v2\"/><path d=\"M14 14.66V17a1 1 0 0 0 1 1 2 2 0 0 1 2 2v2\"/><path d=\"M17.916 10H19.5A2.5 2.5 0 0 0 22 7.5V5a1 1 0 0 0-1-1h-3\"/><path d=\"M4 22h16\"/><path d=\"M6 9a6 6 0 0 0 12 0V3a1 1 0 0 0-1-1H7a1 1 0 0 0-1 1z\"/><path d=\"M6.084 10H4.5A2.5 2.5 0 0 1 2 7.5V5a1 1 0 0 1 1-1h3\"/>",
    "cloud-check": "<path d=\"m17 15-5.5 5.5L9 18\"/><path d=\"M5.516 16.07A7 7 0 1 1 15.71 8h1.79a4.5 4.5 0 0 1 3.501 7.327\"/>",
    "hourglass": "<path d=\"M5 22h14\"/><path d=\"M5 2h14\"/><path d=\"M17 22v-4.172a2 2 0 0 0-.586-1.414L12 12l-4.414 4.414A2 2 0 0 0 7 17.828V22\"/><path d=\"M7 2v4.172a2 2 0 0 0 .586 1.414L12 12l4.414-4.414A2 2 0 0 0 17 6.172V2\"/>",
    "monitor-play": "<path d=\"M15.033 9.44a.647.647 0 0 1 0 1.12l-4.065 2.352a.645.645 0 0 1-.968-.56V7.648a.645.645 0 0 1 .967-.56z\"/><path d=\"M12 17v4\"/><path d=\"M8 21h8\"/><rect x=\"2\" y=\"3\" width=\"20\" height=\"14\" rx=\"2\"/>",
    "circle-check": "<circle cx=\"12\" cy=\"12\" r=\"10\"/><path d=\"m9 12 2 2 4-4\"/>",
    "circle-question-mark": "<circle cx=\"12\" cy=\"12\" r=\"10\"/><path d=\"M9.09 9a3 3 0 0 1 5.83 1c0 2-3 3-3 3\"/><path d=\"M12 17h.01\"/>",
    "sun": "<circle cx=\"12\" cy=\"12\" r=\"4\"/><path d=\"M12 2v2\"/><path d=\"M12 20v2\"/><path d=\"m4.93 4.93 1.41 1.41\"/><path d=\"m17.66 17.66 1.41 1.41\"/><path d=\"M2 12h2\"/><path d=\"M20 12h2\"/><path d=\"m6.34 17.66-1.41 1.41\"/><path d=\"m19.07 4.93-1.41 1.41\"/>",
    "image": "<rect width=\"18\" height=\"18\" x=\"3\" y=\"3\" rx=\"2\" ry=\"2\"/><circle cx=\"9\" cy=\"9\" r=\"2\"/><path d=\"m21 15-3.086-3.086a2 2 0 0 0-2.828 0L6 21\"/>",
    "info": "<circle cx=\"12\" cy=\"12\" r=\"10\"/><path d=\"M12 16v-4\"/><path d=\"M12 8h.01\"/>",
    "plus": "<path d=\"M5 12h14\"/><path d=\"M12 5v14\"/>",
    "arrow-left-right": "<path d=\"M8 3 4 7l4 4\"/><path d=\"M4 7h16\"/><path d=\"m16 21 4-4-4-4\"/><path d=\"M20 17H4\"/>",
    "badge-check": "<path d=\"M3.85 8.62a4 4 0 0 1 4.78-4.77 4 4 0 0 1 6.74 0 4 4 0 0 1 4.78 4.78 4 4 0 0 1 0 6.74 4 4 0 0 1-4.77 4.78 4 4 0 0 1-6.75 0 4 4 0 0 1-4.78-4.77 4 4 0 0 1 0-6.76Z\"/><path d=\"m9 12 2 2 4-4\"/>",
    "save": "<path d=\"M15.2 3a2 2 0 0 1 1.4.6l3.8 3.8a2 2 0 0 1 .6 1.4V19a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2z\"/><path d=\"M17 21v-7a1 1 0 0 0-1-1H8a1 1 0 0 0-1 1v7\"/><path d=\"M7 3v4a1 1 0 0 0 1 1h7\"/>",
    "hard-drive": "<path d=\"M10 16h.01\"/><path d=\"M2.212 11.577a2 2 0 0 0-.212.896V18a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-5.527a2 2 0 0 0-.212-.896L18.55 5.11A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z\"/><path d=\"M21.946 12.013H2.054\"/><path d=\"M6 16h.01\"/>",
    "unlink": "<path d=\"m18.84 12.25 1.72-1.71h-.02a5.004 5.004 0 0 0-.12-7.07 5.006 5.006 0 0 0-6.95 0l-1.72 1.71\"/><path d=\"m5.17 11.75-1.71 1.71a5.004 5.004 0 0 0 .12 7.07 5.006 5.006 0 0 0 6.95 0l1.71-1.71\"/><line x1=\"8\" x2=\"8\" y1=\"2\" y2=\"5\"/><line x1=\"2\" x2=\"5\" y1=\"8\" y2=\"8\"/><line x1=\"16\" x2=\"16\" y1=\"19\" y2=\"22\"/><line x1=\"19\" x2=\"22\" y1=\"16\" y2=\"16\"/>",
    "cloud-off": "<path d=\"M10.94 5.274A7 7 0 0 1 15.71 10h1.79a4.5 4.5 0 0 1 4.222 6.057\"/><path d=\"M18.796 18.81A4.5 4.5 0 0 1 17.5 19H9A7 7 0 0 1 5.79 5.78\"/><path d=\"m2 2 20 20\"/>",
    "smartphone": "<rect width=\"14\" height=\"20\" x=\"5\" y=\"2\" rx=\"2\" ry=\"2\"/><path d=\"M12 18h.01\"/>",
    "timer": "<line x1=\"10\" x2=\"14\" y1=\"2\" y2=\"2\"/><line x1=\"12\" x2=\"15\" y1=\"14\" y2=\"11\"/><circle cx=\"12\" cy=\"14\" r=\"8\"/>",
    "calendar": "<path d=\"M8 2v4\"/><path d=\"M16 2v4\"/><rect width=\"18\" height=\"18\" x=\"3\" y=\"4\" rx=\"2\"/><path d=\"M3 10h18\"/>",
    "film": "<rect width=\"18\" height=\"18\" x=\"3\" y=\"3\" rx=\"2\"/><path d=\"M7 3v18\"/><path d=\"M3 7.5h4\"/><path d=\"M3 12h18\"/><path d=\"M3 16.5h4\"/><path d=\"M17 3v18\"/><path d=\"M17 7.5h4\"/><path d=\"M17 16.5h4\"/>",
    "keyboard": "<path d=\"M10 8h.01\"/><path d=\"M12 12h.01\"/><path d=\"M14 8h.01\"/><path d=\"M16 12h.01\"/><path d=\"M18 8h.01\"/><path d=\"M6 8h.01\"/><path d=\"M7 16h10\"/><path d=\"M8 12h.01\"/><rect width=\"20\" height=\"16\" x=\"2\" y=\"4\" rx=\"2\"/>",
    "ban": "<circle cx=\"12\" cy=\"12\" r=\"10\"/><path d=\"M4.929 4.929 19.07 19.071\"/>",
    "circle-x": "<circle cx=\"12\" cy=\"12\" r=\"10\"/><path d=\"m15 9-6 6\"/><path d=\"m9 9 6 6\"/>",
    "list-filter": "<path d=\"M2 5h20\"/><path d=\"M6 12h12\"/><path d=\"M9 19h6\"/>",
    "arrow-up-down": "<path d=\"m21 16-4 4-4-4\"/><path d=\"M17 20V4\"/><path d=\"m3 8 4-4 4 4\"/><path d=\"M7 4v16\"/>",
    "key-ok": "<circle cx=\"12\" cy=\"12\" r=\"9.5\" opacity=\".5\"/><circle cx=\"12\" cy=\"12\" r=\"4.2\" fill=\"currentColor\" stroke=\"none\"/>",
    "key-back": "<path d=\"M9 14 4 9l5-5\"/><path d=\"M4 9h10.5a5.5 5.5 0 0 1 5.5 5.5a5.5 5.5 0 0 1-5.5 5.5H11\"/>",
    "key-left": "<circle cx=\"12\" cy=\"12\" r=\"9.5\" opacity=\".5\"/><g transform=\"rotate(180 12 12)\"><path d=\"m10 8 4 4-4 4\"/></g>",
    "key-right": "<circle cx=\"12\" cy=\"12\" r=\"9.5\" opacity=\".5\"/><path d=\"m10 8 4 4-4 4\"/>",
    "key-up": "<circle cx=\"12\" cy=\"12\" r=\"9.5\" opacity=\".5\"/><g transform=\"rotate(270 12 12)\"><path d=\"m10 8 4 4-4 4\"/></g>",
    "key-down": "<circle cx=\"12\" cy=\"12\" r=\"9.5\" opacity=\".5\"/><g transform=\"rotate(90 12 12)\"><path d=\"m10 8 4 4-4 4\"/></g>",
    "key-menu": "<path d=\"M4 7h16\"/><path d=\"M4 12h16\"/><path d=\"M4 17h16\"/>",
    "key-ok-hold": "<circle cx=\"12\" cy=\"12\" r=\"6.2\" opacity=\".5\"/><circle cx=\"12\" cy=\"12\" r=\"2.8\" fill=\"currentColor\" stroke=\"none\"/><path d=\"M12 2.5a9.5 9.5 0 1 1-9.5 9.5\"/>",
  };
  function svg(name) {
    const inner = paths[name];
    if (!inner) throw new Error("icon " + name);
    return `<svg class="ic" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${inner}</svg>`;
  }
  function fill(root = document) {
    root.querySelectorAll("i[data-ic]").forEach((el) => {
      const t = document.createElement("template");
      t.innerHTML = svg(el.dataset.ic);
      const s = t.content.firstElementChild;
      if (el.className) s.setAttribute("class", "ic " + el.className);
      el.replaceWith(s);
    });
  }
  window.Icons = { svg, fill, names: Object.keys(paths) };
})();
