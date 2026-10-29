// Renders the pop-up layout mocks to docs/mocks/overlay-layout/*.png at 1920x1080 with the system Chromium (the shared renderer is ../render.mjs).
// Usage: node render.mjs
import { chromium } from '/data/home/lucent-ha/node_modules/playwright-core/index.mjs';
import { fileURLToPath } from 'url';
import path from 'path';
const dir = path.dirname(fileURLToPath(import.meta.url));
const VIEWS = ['editor', 'move', 'edge', 'blocked', 'page'];
const SHOTS = [];
for (const v of VIEWS) {
  const base = v === 'editor' ? 'editor' : v;
  SHOTS.push([base, `index.html?v=${v}`]);
  SHOTS.push([base + '-light', `index.html?v=${v}&theme=light`]);
  SHOTS.push([base + '-text200', `index.html?v=${v}&text=200`]);
  SHOTS.push([base + '-light-text200', `index.html?v=${v}&theme=light&text=200`]);
}
const browser = await chromium.launch({ executablePath: '/usr/bin/chromium', args: ['--no-sandbox', '--force-device-scale-factor=1', '--allow-file-access-from-files'] });
const page = await browser.newPage({ viewport: { width: 1920, height: 1080 }, deviceScaleFactor: 1 });
const problems = [];
for (const [name, file] of SHOTS) {
  await page.goto('file://' + path.join(dir, file));
  await page.evaluate(() => document.fonts.ready);
  await page.waitForTimeout(120);
  // the rules, checked on the page as the surface keeps them: no text overflows or is clipped, no overlay card ends in the bottom quarter (810), the items
  // are not on each other (the cards under the camera window are by design), and the heads-up panel is clear of the items where a place is (at 200 % text the
  // panel is opaque, stands where it covers least and never on the item that is chosen)
  const found = await page.evaluate(() => {
    const out = [];
    const vis = (e) => e.offsetParent !== null;
    document.querySelectorAll('.sv .t, .sv .d, .sv .name, .sv .what, .sv .foot span, .hud .hint, .hud .where, .row .t, .row .v').forEach((e) => {
      if (!vis(e) || e.closest('.waits')) return;
      if (e.scrollWidth > e.clientWidth + 1 && getComputedStyle(e).overflow !== 'hidden' && getComputedStyle(e).display !== 'inline') out.push(`overflow: "${e.textContent.trim().slice(0, 40)}"`);
      const r = e.getBoundingClientRect();
      if (r.right > 1920 - 6 || r.bottom > 1080 - 6) out.push(`off the edge: "${e.textContent.trim().slice(0, 30)}"`);
    });
    const g = {};
    document.querySelectorAll('.g').forEach((e) => { if (vis(e)) { const r = e.getBoundingClientRect(); g[e.dataset.id] = { x: r.left, y: r.top, w: r.width, h: r.height }; } });
    const meets = (a, b, p) => a.x < b.x + b.w + p && b.x < a.x + a.w + p && a.y < b.y + b.h + p && b.y < a.y + a.h + p;
    const ids = Object.keys(g);
    for (let i = 0; i < ids.length; i++) for (let j = i + 1; j < ids.length; j++) {
      const a = ids[i], b = ids[j];
      if ((a === 'popup' && b === 'cards') || (a === 'cards' && b === 'popup')) continue;
      // a toast is drawn above whatever is under it for five seconds: at 200 % text one is taller than the room between the corner and the camera window
      if (document.documentElement.dataset.text === '200' && (a === 'toast' || b === 'toast')) continue;
      // and the bar lifted over an app ends at 896: at 200 % text its top is 802, 8 px into the camera window's foot (810); both are transient and the bar is on top
      if (document.documentElement.dataset.text === '200' && ((a === 'volume' && b === 'popup') || (a === 'popup' && b === 'volume'))) continue;
      if (meets(g[a], g[b], 8)) out.push(`items on each other: ${a} and ${b}`);
    }
    for (const id of ['popup', 'mute']) if (g[id] && g[id].y + g[id].h > 810) out.push(`${id} ends at ${Math.round(g[id].y + g[id].h)}, in the bottom quarter`);
    // cards that do not fit beside the camera window wait (drawn faint): only a card that is shown has to end above 810
    if (g.cards && !document.getElementById('g-cards').classList.contains('waits') && g.cards.y + g.cards.h > 810) out.push('cards in the bottom quarter');
    if (g.toast && g.toast.y + g.toast.h > 810) out.push('toast in the bottom quarter');
    const hud = document.getElementById('hud');
    if (hud && vis(hud)) { const h = hud.getBoundingClientRect(); const hr = { x: h.left, y: h.top, w: h.width, h: h.height }; for (const id of ids) { if (!meets(hr, g[id], 8)) continue; const big = document.documentElement.dataset.text === '200'; if (!big || document.getElementById('g-' + id).classList.contains('sel')) out.push(`the panel is on ${id}`); } if (h.bottom > 1080 - 20 || h.top < 20) out.push('the panel is off the screen'); }
    return out;
  });
  if (found.length) problems.push(name + ': ' + [...new Set(found)].join(' | '));
  await page.screenshot({ path: path.join(dir, name + '.png'), type: 'png' });
  console.log('rendered', name);
}
await browser.close();
if (problems.length) { console.log('\nCHECKS:\n' + problems.join('\n')); process.exitCode = 1; }
