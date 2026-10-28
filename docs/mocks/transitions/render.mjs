// Renders the transition mocks to docs/mocks/transitions/*.png at 1920x1080 with the system Chromium (the shared renderer is ../render.mjs).
// Usage: node render.mjs
import { chromium } from '/data/home/lucent-ha/node_modules/playwright-core/index.mjs';
import { fileURLToPath } from 'url';
import path from 'path';
const dir = path.dirname(fileURLToPath(import.meta.url));
const SHOTS = [];
for (const v of ['launch', 'leave', 'switch', 'video', 'interrupt', 'splash', 'reduced']) {
  SHOTS.push([v, `index.html?v=${v}`]);
  SHOTS.push([`${v}-light`, `index.html?v=${v}&theme=light`]);
}
for (const [name, extra] of [['settings', ''], ['settings-light', '&theme=light'], ['settings-text200', '&text=200'], ['settings-light-text200', '&theme=light&text=200']]) {
  SHOTS.push([name, `index.html?v=settings${extra}`]);
}
const browser = await chromium.launch({ executablePath: '/usr/bin/chromium', args: ['--no-sandbox', '--force-device-scale-factor=1', '--allow-file-access-from-files'] });
const page = await browser.newPage({ viewport: { width: 1920, height: 1080 }, deviceScaleFactor: 1 });
const problems = [];
for (const [name, file] of SHOTS) {
  await page.goto('file://' + path.join(dir, file));
  await page.evaluate(() => document.fonts.ready);
  await page.waitForTimeout(120);
  // no text may overflow its box, be clipped or leave the safe edge, at 100 % and at 200 %; the sheet's own captions stay on the sheet
  const found = await page.evaluate(() => {
    const out = [];
    const edge = 6;
    document.querySelectorAll('.head h1, .head .rule, .shot figcaption span, .legend span, .set .t, .set .d, .set .title, .set h1').forEach(e => {
      if (!e.offsetParent) return;
      const r = e.getBoundingClientRect();
      if (e.scrollWidth > e.clientWidth + 1 && getComputedStyle(e).overflow !== 'hidden' && getComputedStyle(e).display !== 'inline') out.push(`overflow: "${e.textContent.trim().slice(0, 40)}" ${e.scrollWidth}>${e.clientWidth}`);
      if (r.right > 1920 - edge || r.bottom > 1080 - edge) out.push(`off the edge: "${e.textContent.trim().slice(0, 30)}"`);
    });
    // the board and the legend must not run into each other
    const b = document.querySelector('.board'), l = document.querySelector('.legend');
    if (b && l && b.getBoundingClientRect().bottom > l.getBoundingClientRect().top - 4) out.push(`the board (${Math.round(b.getBoundingClientRect().bottom)}) runs into the legend (${Math.round(l.getBoundingClientRect().top)})`);
    // settings: every row's label and help stay inside the row, rows do not overlap
    const rows = [...document.querySelectorAll('.set .row')];
    rows.forEach((r, i) => {
      const rr = r.getBoundingClientRect();
      r.querySelectorAll('.t, .d').forEach(e => { const er = e.getBoundingClientRect(); if (er.bottom > rr.bottom + 1 || er.right > rr.right + 1) out.push(`row text outside its row: "${e.textContent.trim().slice(0, 30)}"`); });
      if (i && rows[i - 1].parentElement === r.parentElement && rows[i - 1].offsetTop + rows[i - 1].offsetHeight > r.offsetTop + 1) out.push(`rows overlap at ${i}`);
    });
    const pg = document.querySelector('.set .page');
    if (pg && pg.scrollHeight > pg.clientHeight + 1) out.push(`page overflows ${pg.scrollHeight}>${pg.clientHeight}`);
    return out;
  });
  if (found.length) problems.push(name + ': ' + [...new Set(found)].join(' | '));
  await page.screenshot({ path: path.join(dir, name + '.png'), type: 'png' });
  console.log('rendered', name);
}
await browser.close();
if (problems.length) { console.log('\nCHECKS:\n' + problems.join('\n')); process.exitCode = 1; }
