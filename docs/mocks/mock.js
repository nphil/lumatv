/* Shared behaviour for the LumaTV mocks: the Settings rail, icons, theme and a D-pad focus model.
 * Query string: ?theme=light|dark  ?focus=<id> (initial focus)  ?hover=<id> (pointer hover state). */
(function () {
  const q = new URLSearchParams(location.search);
  const theme = q.get('theme') || document.documentElement.dataset.theme || 'dark';
  document.documentElement.dataset.theme = theme;
  if (q.get('v')) document.documentElement.dataset.v = q.get('v');

  /* The nine curated sections. A live line is one short fact (Nitin, 2026-10-25: minimal text), and none where there is nothing to
   * glance at; illustrative here. */
  const RAIL = [
    { id: 'picture', ic: 'image', name: 'Picture', line: 'Filmmaker' },
    { id: 'sound', ic: 'volume-2', name: 'Sound', line: 'Receiver' },
    { id: 'inputs', ic: 'cable', name: 'Inputs & devices', line: 'HDMI 2' },
    { id: 'network', ic: 'wifi', name: 'Network', line: 'Nate\u2019s Home · Strong' },
    { id: 'ambient', ic: 'lamp', name: 'Ambient light', line: 'On' },
    { id: 'home', ic: 'house', name: 'Home & look', line: '' },
    { id: 'remote', ic: 'mouse-pointer-2', name: 'Remote & pointer', line: 'Pointer on' },
    { id: 'ai', ic: 'bot', name: 'AI access', line: 'On' },
    { id: 'system', ic: 'settings', name: 'System', line: '' },
  ];

  function buildRail(root) {
    const open = root.dataset.open;
    root.innerHTML = RAIL.map(e =>
      `<div class="rail-entry focusable" id="rail-${e.id}" ${e.id === open ? 'aria-current="page"' : ''} tabindex="0">
         <div class="tile"><i data-ic="${e.ic}"></i></div>
         <div><div class="name">${e.name}</div>${e.line ? `<div class="line">${e.line}</div>` : ''}</div>
       </div>`).join('');
  }

  function focusables() {
    return [...document.querySelectorAll('.focusable')].filter(e => e.offsetParent !== null);
  }
  function setFocus(el) {
    document.querySelectorAll('.focusable.focus').forEach(e => e.classList.remove('focus'));
    if (el) el.classList.add('focus');
  }
  /* Nearest neighbour in the pressed direction. A candidate must start past the current item's middle in that
   * direction (its centre past the current edge); distance along the axis counts once, the sideways gap between
   * the two boxes (0 when they overlap) counts three times and may not exceed half the current item, so focus
   * never leaps to a far column that merely sits at a similar height. */
  function move(dir) {
    const cur = document.querySelector('.focusable.focus');
    const all = focusables();
    if (!cur) { setFocus(all[0]); return; }
    const c = cur.getBoundingClientRect();
    let best = null, bestScore = Infinity;
    for (const el of all) {
      if (el === cur || el.contains(cur) || cur.contains(el)) continue;
      const r = el.getBoundingClientRect();
      const gapX = Math.max(0, r.left - c.right, c.left - r.right);
      const gapY = Math.max(0, r.top - c.bottom, c.top - r.bottom);
      const cx = (r.left + r.right) / 2, cy = (r.top + r.bottom) / 2;
      let primary, secondary;
      if (dir === 'ArrowRight') { if (cx < c.right - 8) continue; primary = Math.max(0, r.left - c.right); secondary = gapY; }
      else if (dir === 'ArrowLeft') { if (cx > c.left + 8) continue; primary = Math.max(0, c.left - r.right); secondary = gapY; }
      else if (dir === 'ArrowDown') { if (cy < c.bottom - 8) continue; primary = Math.max(0, r.top - c.bottom); secondary = gapX; }
      else { if (cy > c.top + 8) continue; primary = Math.max(0, c.top - r.bottom); secondary = gapX; }
      if (secondary > (dir === 'ArrowLeft' || dir === 'ArrowRight' ? c.height : c.width) / 2) continue;
      const score = primary + secondary * 3;
      if (score < bestScore) { bestScore = score; best = el; }
    }
    if (best) setFocus(best);
  }
  function activate() {
    const cur = document.querySelector('.focusable.focus');
    if (!cur) return;
    const sw = cur.querySelector('.toggle');
    if (cur.classList.contains('toggle') || sw) {
      const t = sw || cur;
      t.setAttribute('aria-checked', t.getAttribute('aria-checked') === 'true' ? 'false' : 'true');
    } else if (cur.parentElement && cur.parentElement.classList.contains('seg')) {
      cur.parentElement.querySelectorAll('span').forEach(s => s.setAttribute('aria-selected', s === cur ? 'true' : 'false'));
    } else if (cur.classList.contains('tab')) {
      cur.parentElement.querySelectorAll('.tab').forEach(s => s.setAttribute('aria-selected', s === cur ? 'true' : 'false'));
    } else if (cur.classList.contains('mode')) {
      cur.parentElement.querySelectorAll('.mode').forEach(s => s.setAttribute('aria-checked', s === cur ? 'true' : 'false'));
    } else if (cur.classList.contains('rail-entry')) {
      const id = cur.id.replace('rail-', '');
      const target = { picture: '01-settings-picture.html', sound: '02-settings-sound.html', remote: '03-settings-remote.html', ai: '04-settings-ai-access.html' }[id];
      if (target) location.href = target + location.search;
    }
  }

  document.addEventListener('keydown', e => {
    if (['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(e.key)) { e.preventDefault(); move(e.key); }
    else if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); activate(); }
  });
  document.addEventListener('mouseover', e => {
    const f = e.target.closest && e.target.closest('.focusable');
    document.querySelectorAll('.focusable.hover').forEach(x => x.classList.remove('hover'));
    if (f && !f.classList.contains('focus')) f.classList.add('hover');
  });
  document.addEventListener('click', e => {
    const f = e.target.closest && e.target.closest('.focusable');
    if (f) { setFocus(f); activate(); }
  });

  window.addEventListener('DOMContentLoaded', () => {
    document.querySelectorAll('.rail').forEach(buildRail);
    window.Icons && Icons.fill(document);
    const focusId = q.get('focus');
    const el = (focusId && document.getElementById(focusId)) || document.querySelector('[data-autofocus]');
    if (el) setFocus(el);
    const hoverId = q.get('hover');
    if (hoverId && document.getElementById(hoverId)) document.getElementById(hoverId).classList.add('hover');
  });
})();
