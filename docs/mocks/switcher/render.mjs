// Renders the app switcher mocks to docs/mocks/switcher/*.png at 1920x1080 with the system Chromium (the shared renderer is ../render.mjs).
// Usage: node render.mjs [name ...]
// Checks at 100 % and 200 % text: no text overflows its box, none overlaps another, none sits outside the 96/54 safe area, none is under 21 px at 100 %.
import { chromium } from '/data/home/lucent-ha/node_modules/playwright-core/index.mjs';
import { fileURLToPath } from 'url';
import path from 'path';
import fs from 'fs';
import os from 'os';
const dir = path.dirname(fileURLToPath(import.meta.url));
const SHOTS = [
  ['open', 'index.html'],
  ['open-light', 'index.html?theme=light'],
  ['open-text200', 'index.html?text=200'],
  ['open-light-text200', 'index.html?theme=light&text=200'],
  ['plane', 'index.html?v=plane'],
  ['plane-light', 'index.html?v=plane&theme=light'],
  ['quit', 'index.html?v=quit'],
  ['quit-light', 'index.html?v=quit&theme=light'],
  ['quit-text200', 'index.html?v=quit&text=200'],
  ['confirm', 'index.html?v=confirm'],
  ['confirm-light', 'index.html?v=confirm&theme=light'],
  ['confirm-text200', 'index.html?v=confirm&text=200'],
  ['many', 'index.html?v=many'],
  ['many-light', 'index.html?v=many&theme=light'],
  ['empty', 'index.html?v=empty'],
  ['empty-light', 'index.html?v=empty&theme=light'],
  ['shrink', 'index.html?v=shrink'],
  ['shrink-light', 'index.html?v=shrink&theme=light'],
];
const only = process.argv.slice(2);
const browser = await chromium.launch({ executablePath: '/usr/bin/chromium', args: ['--no-sandbox', '--force-device-scale-factor=1', '--allow-file-access-from-files'] });
const page = await browser.newPage({ viewport: { width: 1920, height: 1080 }, deviceScaleFactor: 1 });
const problems = [];
for (const [name, file] of SHOTS) {
  if (only.length && !only.some(o => name.startsWith(o))) continue;
  await page.goto('file://' + path.join(dir, file));
  await page.evaluate(() => document.fonts.ready);
  await page.waitForTimeout(80);
  const found = await page.evaluate((storyboard) => {
    if (storyboard) return [];
    const out = [];
    const sel = '.sw-head .title, .sw-head .count, .sw-btn, .card .n, .card .s, .sw-hints span, .sw-empty .big, .sw-empty .small, .sw-ask .q, .sw-ask .d';
    const boxes = [];
    const text200 = document.documentElement.dataset.text === '200';
    document.querySelectorAll(sel).forEach(e => {
      if (!e.offsetParent) return;
      const r = e.getBoundingClientRect();
      if (e.scrollWidth > e.clientWidth + 1 && getComputedStyle(e).overflow !== 'hidden') out.push(`overflow: "${e.textContent.trim().slice(0, 40)}" ${e.scrollWidth}>${e.clientWidth}`);
      const fs = parseFloat(getComputedStyle(e).fontSize);
      if (!text200 && e.textContent.trim() && fs < 21) out.push(`small ${fs}px: "${e.textContent.trim().slice(0, 30)}"`);
      const isHints = e.closest('.sw-hints');
      const inSafe = r.left >= 96 - 1 && r.right <= 1920 - 96 + 1 && r.top >= 54 - 1 && r.bottom <= 1080 - 54 + 1;
      if (!inSafe && !e.closest('.sw-more')) out.push(`outside the safe area: "${e.textContent.trim().slice(0, 30)}" ${Math.round(r.left)},${Math.round(r.top)} ${Math.round(r.right)},${Math.round(r.bottom)}`);
      boxes.push({ e, r, label: e.textContent.trim().slice(0, 24) });
    });
    for (let i = 0; i < boxes.length; i++) for (let j = i + 1; j < boxes.length; j++) {
      const a = boxes[i], b = boxes[j];
      if (a.e.contains(b.e) || b.e.contains(a.e)) continue;
      const w = Math.min(a.r.right, b.r.right) - Math.max(a.r.left, b.r.left);
      const h = Math.min(a.r.bottom, b.r.bottom) - Math.max(a.r.top, b.r.top);
      if (w > 1 && h > 1) out.push(`overlap: "${a.label}" and "${b.label}"`);
    }
    return out;
  }, name.startsWith('shrink'));
  if (found.length) problems.push(name + ': ' + [...new Set(found)].join(' | '));
  // the text boxes, for contrast.py (measured on the finished picture, not on the tokens)
  const boxes = await page.evaluate(() => [...document.querySelectorAll('.sw-head .title, .sw-head .count, .sw-btn, .card .n, .card .s, .sw-hints span, .sw-empty .big, .sw-empty .small, .sw-ask .q, .sw-ask .d')]
    .filter(e => e.offsetParent && e.textContent.trim()).map(e => { const r = e.getBoundingClientRect(); return { t: e.textContent.trim().slice(0, 24), x: r.left, y: r.top, w: r.width, h: r.height }; }));
  fs.mkdirSync(path.join(os.tmpdir(), 'switcher-boxes'), { recursive: true });
  fs.writeFileSync(path.join(os.tmpdir(), 'switcher-boxes', name + '.json'), JSON.stringify(boxes));
  await page.screenshot({ path: path.join(dir, name + '.png'), type: 'png' });
  console.log('rendered', name);
}
await browser.close();
if (problems.length) { console.log('\nCHECKS:\n' + problems.join('\n')); process.exitCode = 1; }
