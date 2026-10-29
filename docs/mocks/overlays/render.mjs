// Renders the overlay mocks to docs/mocks/overlays/*.png at 1920x1080 with the system Chromium (the shared renderer is ../render.mjs).
// Usage: node render.mjs
// Checks, per picture: no text overflows its box or is clipped (at 100 % and 200 %); no two text boxes of a card overlap; every card shown
// ends above 75 % of the height (810) and keeps to the column (x 1296 to 1856, Home's right edge, from y 134); and the composited contrast of every run of words
// (4.5:1) and every glyph (3:1) on the worst frame under the glass: white under the dark glass, black under the light one, the glass at theme.json's
// readingAlpha (the language measures contrast on the composited picture).
import { chromium } from '/data/home/lucent-ha/node_modules/playwright-core/index.mjs';
import { fileURLToPath } from 'url';
import fs from 'fs';
import path from 'path';
const dir = path.dirname(fileURLToPath(import.meta.url));
const theme = JSON.parse(fs.readFileSync(path.join(dir, '../../../payload/theme/theme.json'), 'utf8'));
const SHOTS = [];
for (const v of ['stack', 'alert', 'score', 'timer']) {
  SHOTS.push([v, `index.html?v=${v}`], [`${v}-light`, `index.html?v=${v}&theme=light`], [`${v}-text200`, `index.html?v=${v}&text=200`],
             [`${v}-light-text200`, `index.html?v=${v}&theme=light&text=200`]);
}
SHOTS.push(['doorbell', 'index.html?v=doorbell'], ['alarm', 'index.html?v=alarm'], ['alarm-light', 'index.html?v=alarm&theme=light'],
           ['zones', 'index.html?v=zones']);
const only = process.argv[2] ? new RegExp(process.argv[2]) : null;
const browser = await chromium.launch({ executablePath: '/usr/bin/chromium', args: ['--no-sandbox', '--force-device-scale-factor=1', '--allow-file-access-from-files'] });
const page = await browser.newPage({ viewport: { width: 1920, height: 1080 }, deviceScaleFactor: 1 });
const problems = [];
for (const [name, file] of SHOTS) {
  if (only && !only.test(name)) continue;
  await page.goto('file://' + path.join(dir, file));
  await page.evaluate(() => document.fonts.ready);
  await page.waitForTimeout(120);
  const material = file.includes('theme=light') ? theme.light : theme.dark;
  const found = await page.evaluate((material) => {
    const out = [];
    const hex = (h) => { h = h.replace('#', ''); const a = h.length === 8 ? parseInt(h.slice(0, 2), 16) / 255 : 1; h = h.slice(-6); return [[0, 2, 4].map(i => parseInt(h.slice(i, i + 2), 16)), a]; };
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
    const [glass] = hex(material.glass);
    const bg = over(glass, material.readingAlpha, worst);
    const cards = [...document.querySelectorAll('.st')].filter(s => getComputedStyle(s).display !== 'none').flatMap(s => [...s.querySelectorAll('[data-card]')]).filter(c => c.style.display !== 'none');
    const col = { l: 1296 - 1, r: 1856 + 1 };
    cards.forEach((c, n) => {
      const r = c.getBoundingClientRect();
      if (r.bottom > 810) out.push(`card ${n} ends at ${Math.round(r.bottom)}, below 75 % (810)`);
      if (r.top < 134 - 1) out.push(`card ${n} starts at ${Math.round(r.top)}, above 134`);
      if (r.left < col.l || r.right > col.r) out.push(`card ${n} leaves the column: ${Math.round(r.left)} to ${Math.round(r.right)}`);
      const boxes = [];
      c.querySelectorAll('.ov-t, .ov-d, .ov-src, .ov-hint, .ov-what, .ov-n, .ov-s, .name, .ov-live, .ov-foot > span').forEach(e => {
        if (!e.offsetParent || !e.textContent.trim()) return;
        const cs = getComputedStyle(e);
        const clamped = cs.webkitLineClamp && cs.webkitLineClamp !== 'none';
        if (!clamped && e.scrollWidth > e.clientWidth + 1 && cs.overflow !== 'hidden') out.push(`overflow: "${e.textContent.trim().slice(0, 30)}" ${e.scrollWidth}>${e.clientWidth}`);
        if (!clamped && cs.textOverflow === 'ellipsis' && e.scrollWidth > e.clientWidth + 1) out.push(`cut off: "${e.textContent.trim().slice(0, 30)}"`);
        if (clamped && e.scrollHeight > e.clientHeight + 6) out.push(`cut off (lines): "${e.textContent.trim().slice(0, 30)}"`);
        const b = e.getBoundingClientRect();
        boxes.push([e, b]);
        const rr = c.getBoundingClientRect();
        if (b.right > rr.right - 1 || b.bottom > rr.bottom - 1) out.push(`text past the card: "${e.textContent.trim().slice(0, 30)}"`);
        const p = parse(cs.color);
        if (p) {
          const fg = over(p[0], p[1], bg);
          const k = ratio(fg, bg);
          if (k < 4.5) out.push(`contrast ${k.toFixed(2)}: "${e.textContent.trim().slice(0, 30)}"`);
        }
      });
      c.querySelectorAll('.ic').forEach(e => {
        const p = parse(getComputedStyle(e).color);
        if (!p) return;
        const k = ratio(over(p[0], p[1], bg), bg);
        if (k < 3) out.push(`glyph contrast ${k.toFixed(2)}`);
      });
      for (let i = 0; i < boxes.length; i++) for (let j = i + 1; j < boxes.length; j++) {
        const [ea, a] = boxes[i], [eb, b] = boxes[j];
        if (ea.contains(eb) || eb.contains(ea)) continue;
        if (a.left < b.right - 1 && b.left < a.right - 1 && a.top < b.bottom - 1 && b.top < a.bottom - 1) out.push(`overlap: "${ea.textContent.trim().slice(0, 20)}" and "${eb.textContent.trim().slice(0, 20)}"`);
      }
    });
    for (let i = 1; i < cards.length; i++) {
      const a = cards[i - 1].getBoundingClientRect(), b = cards[i].getBoundingClientRect();
      if (b.top - a.bottom < 15.5) out.push(`cards ${i - 1} and ${i} are ${Math.round(b.top - a.bottom)} apart`);
    }
    out.push('cards: ' + cards.length);
    return out;
  }, material);
  const note = found.pop();
  if (found.length) problems.push(name + ': ' + [...new Set(found)].join(' | '));
  await page.screenshot({ path: path.join(dir, name + '.png'), type: 'png' });
  console.log('rendered', name, note);
}
await browser.close();
if (problems.length) { console.log('\nCHECKS:\n' + problems.join('\n')); process.exitCode = 1; }
