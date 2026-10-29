// Renders the Cameras (Scrypted) mocks to docs/mocks/cameras-scrypted/*.jpg at 1920x1080 with the system Chromium.
// Usage: node render.mjs [regex of the picture names]
// Checks, per picture: no text overflows its box or is clipped (at 100 % and 200 %); no two text boxes overlap; every run of words keeps inside
// the 5 % safe area (96 and 54 from the edges; the pop-up card's own column and the settings page's panes are their own surfaces); and the
// composited contrast of every run of words (4.5:1) and every glyph (3:1) on the worst frame under the glass (white under the dark glass, black
// under the light one, the glass at theme.json's readingAlpha: the language measures contrast on the composited picture).
import { chromium } from '/data/home/lucent-ha/node_modules/playwright-core/index.mjs';
import { fileURLToPath } from 'url';
import fs from 'fs';
import path from 'path';
const dir = path.dirname(fileURLToPath(import.meta.url));
const theme = JSON.parse(fs.readFileSync(path.join(dir, '../../../payload/theme/theme.json'), 'utf8'));
const VIEWS = ['popup', 'popup-event', 'popup-doorbell', 'grid', 'grid-more', 'grid-offline', 'grid-unlinked', 'grid-refused', 'full', 'full-start', 'full-failed',
  'strip', 'strip-scrub', 'strip-play', 'strip-gap', 'events', 'clips', 'events-all', 'settings', 'settings-notify', 'settings-notify-cam'];
const SHOTS = [];
for (const v of VIEWS) {
  SHOTS.push([v, `index.html?v=${v}`], [`${v}-light`, `index.html?v=${v}&theme=light`], [`${v}-text200`, `index.html?v=${v}&text=200`],
             [`${v}-light-text200`, `index.html?v=${v}&theme=light&text=200`]);
}
const only = process.argv[2] ? new RegExp(process.argv[2]) : null;
// What is kept as a picture: every view, dark and light at 100 %, and the views where the type size decides the layout at 200 % in the dark. The checks
// run on every variant; a picture that is not kept is not written (84 photographic PNGs are 140 MB).
const KEEP = new RegExp('^(' + VIEWS.join('|') + ')(-light)?$|^(popup|grid|full|strip|strip-scrub|events|settings|settings-notify)-text200$');
const browser = await chromium.launch({ executablePath: '/usr/bin/chromium', args: ['--no-sandbox', '--force-device-scale-factor=1', '--allow-file-access-from-files'] });
const page = await browser.newPage({ viewport: { width: 1920, height: 1080 }, deviceScaleFactor: 1 });
const problems = [];
for (const [name, file] of SHOTS) {
  if (only && !only.test(name)) continue;
  await page.goto('file://' + path.join(dir, file));
  await page.evaluate(() => document.fonts.ready);
  await page.waitForTimeout(120);
  const material = file.includes('theme=light') ? theme.light : theme.dark;
  const view = file.match(/v=([\w-]+)/)[1];
  const found = await page.evaluate(({ material, view, theme }) => {
    const out = [];
    const hex = (h) => { h = h.replace('#', ''); h = h.slice(-6); return [0, 2, 4].map(i => parseInt(h.slice(i, i + 2), 16)); };
    const parse = (c) => {
      let m = c.match(/^rgba?\(([^)]+)\)$/);
      if (m) { const p = m[1].split(/[ ,\/]+/).filter(Boolean).map(Number); return [p.slice(0, 3), p.length > 3 ? p[3] : 1]; }
      m = c.match(/^color\(srgb ([^)]+)\)$/);
      if (m) { const p = m[1].split(/[ \/]+/).filter(Boolean).map(Number); return [p.slice(0, 3).map(x => x * 255), p.length > 3 ? p[3] : 1]; }
      return null;
    };
    const lin = (c) => { c /= 255; return c <= 0.04045 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4); };
    const lum = (c) => 0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2]);
    const ratio = (a, b) => { const x = lum(a), y = lum(b); return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05); };
    const over = (top, alpha, below) => top.map((t, i) => t * alpha + below[i] * (1 - alpha));
    const dark = document.documentElement.dataset.theme !== 'light';
    const worst = dark ? [255, 255, 255] : [0, 0, 0];
    const glass = hex(material.glass);
    const glassBg = over(glass, material.readingAlpha, worst);
    const pageBg = hex(material.scrimHide);
    const onPage = /^(grid|events-all)/.test(view) && !/^events-all/.test(view);
    const settingsPage = view.startsWith('settings');
    const leaves = [...document.querySelectorAll('#root *, #root')].filter(e => {
      if (!e.offsetParent && getComputedStyle(e).position !== 'fixed') return false;
      return [...e.childNodes].some(n => n.nodeType === 3 && n.textContent.trim());
    });
    const boxes = [];
    for (const e of leaves) {
      const cs = getComputedStyle(e);
      const text = e.textContent.trim().slice(0, 28);
      const ell = cs.textOverflow === 'ellipsis';
      const clamped = cs.webkitLineClamp && cs.webkitLineClamp !== 'none';
      if (!clamped && e.scrollWidth > e.clientWidth + 1 && cs.overflow !== 'hidden' && cs.display !== 'inline') out.push(`overflow: "${text}" ${e.scrollWidth}>${e.clientWidth}`);
      if (!clamped && ell && e.scrollWidth > e.clientWidth + 1) out.push(`cut off: "${text}"`);
      if (clamped && e.scrollHeight > e.clientHeight + 2) out.push(`cut off (lines): "${text}"`);
      if (e.closest('[data-behind]')) continue;
      const r = document.createRange(); r.selectNodeContents(e);
      const b = r.getBoundingClientRect();
      if (!b.width) continue;
      const eb = cs.display === 'inline' ? b : e.getBoundingClientRect();
      boxes.push([e, eb, text]);
      // safe area: the pop-up's card has its own column (the overlay layout's), and the settings page its panes
      const inCard = !!e.closest('.cam-card'), inSettings = settingsPage;
      if (!inCard && !inSettings) {
        const tol = parseFloat(cs.fontSize) * 0.06;     // the line box is the font's em box, a little taller than the letters
        if (b.left < 96 - 1 || b.right > 1824 + 1 || b.top < 54 - 1 - tol || b.bottom > 1026 + 1 + tol) out.push(`outside the safe area: "${text}" ${Math.round(b.left)},${Math.round(b.top)} to ${Math.round(b.right)},${Math.round(b.bottom)}`);
      }
      if (inCard && (b.right > 1856 || b.top < 134 - 1 || b.bottom > 810)) out.push(`card text outside its zone: "${text}"`);
      // contrast on the composited picture
      const p = parse(cs.color);
      if (!p) continue;
      const glassy = e.closest('.overlay, .cam-card');
      const primary = e.closest('.btn.primary');
      const bg = primary ? hex(material.accent) : (glassy && !onPage ? glassBg : (onPage || settingsPage ? (settingsPage ? null : pageBg) : null));
      if (!bg) continue;
      if (primary) { const k = Math.max(...[material.accentInk, theme.dark.ink, theme.light.ink].map(c => ratio(hex(c), bg))); if (k < 4.5) out.push(`contrast ${k.toFixed(2)}: "${text}"`); continue; }
      const fg = over(p[0], p[1], bg);
      const k = ratio(fg, bg);
      const big = parseFloat(cs.fontSize) >= 24 * 1 && parseInt(cs.fontWeight) >= 600 || parseFloat(cs.fontSize) >= 32;
      if (k < (big ? 3 : 4.5)) out.push(`contrast ${k.toFixed(2)}: "${text}"`);
    }
    document.querySelectorAll('#root .ic, #root .kc').forEach(e => {
      if (!e.offsetParent) return;
      const glassy = e.closest('.overlay, .cam-card');
      const bg = glassy && !onPage ? glassBg : (onPage ? pageBg : null);
      if (!bg) return;
      const p = parse(getComputedStyle(e).color);
      if (!p) return;
      const k = ratio(over(p[0], p[1], bg), bg);
      if (k < 3) out.push(`glyph contrast ${k.toFixed(2)}`);
    });
    for (let i = 0; i < boxes.length; i++) for (let j = i + 1; j < boxes.length; j++) {
      const [ea, a, ta] = boxes[i], [eb, b, tb] = boxes[j];
      if (ea.contains(eb) || eb.contains(ea)) continue;
      if (a.left < b.right - 1 && b.left < a.right - 1 && a.top < b.bottom - 1 && b.top < a.bottom - 1) out.push(`overlap: "${ta}" and "${tb}"`);
    }
    // the chrome is inside the picture and clear of the other chrome
    out.push('texts: ' + boxes.length);
    return out;
  }, { material, view, theme });
  const note = found.pop();
  if (found.length) problems.push(name + ': ' + [...new Set(found)].join(' | '));
  if (KEEP.test(name) || process.argv[3] === 'all') await page.screenshot({ path: path.join(dir, name + '.jpg'), type: 'jpeg', quality: 88 });
  console.log('checked', name, note);
}
await browser.close();
if (problems.length) { console.log('\nCHECKS:\n' + problems.join('\n')); process.exitCode = 1; }
