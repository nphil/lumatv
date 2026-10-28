// What the store theme costs a page, on this workstation's Chromium: the stand-in category page with and without the theme, the page
// repeated to a long one, and the main thread's time for a full style recalculation and for one focus move (CDP Performance metrics).
//
//   node bench.mjs [copies ...]      default: 0 15  (the page as it is, then 15 more copies of its grid: about 1800 elements)
//
// These are host numbers on a stand-in whose "stock" look is this directory's stock.css (Sandstone's focus: a light fill, a shadow, a 200 ms
// transform, will-change), not LG's page and not the TV: they say what the sheet costs and saves relative to such a page, and the TV round
// is what says what it does on the set (docs/research/store-result.md).
import { chromium } from '/data/home/lucent-ha/node_modules/playwright-core/index.mjs';
import { execFileSync } from 'child_process';
import { fileURLToPath } from 'url';
import fs from 'fs';
import os from 'os';
import path from 'path';

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, '..', '..', '..');
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'store-bench-'));
const out = path.join(tmp, 'dark.js');
execFileSync('python3', [path.join(root, 'tools', 'store_theme.py'), 'build', path.join(root, 'payload', 'store'), path.join(root, 'payload', 'theme', 'theme.json'), '/dev/null', out, '--material', 'dark'], { stdio: ['ignore', 'ignore', 'inherit'] });
const script = fs.readFileSync(out, 'utf8');
const copies = process.argv.slice(2).map(Number);
const browser = await chromium.launch({ executablePath: '/usr/bin/chromium', args: ['--no-sandbox'] });

async function run(themed, copies) {
  const ctx = await browser.newContext({ viewport: { width: 1920, height: 1080 } });
  if (themed) await ctx.addInitScript(script);
  const p = await ctx.newPage();
  await p.goto('file://' + path.join(here, 'category.html'));
  await p.evaluate(() => document.fonts.ready);
  await p.waitForFunction(() => [...document.images].every(i => i.complete && i.naturalWidth > 0));
  const cdp = await ctx.newCDPSession(p);
  await cdp.send('Performance.enable');
  const elements = await p.evaluate(c => {
    const g = document.querySelector('.grid');
    for (let i = 0; i < c; i++) { const k = g.cloneNode(true); k.classList.remove('grid'); k.classList.add('grid2'); g.parentNode.appendChild(k); }
    return document.querySelectorAll('*').length;
  }, copies);
  const m0 = await cdp.send('Performance.getMetrics');
  await p.evaluate(() => { document.documentElement.classList.toggle('x'); getComputedStyle(document.body).color; });      // one full recalculation
  const m1 = await cdp.send('Performance.getMetrics');
  const tiles = await p.$$('.grid button');
  const N = 80;
  for (let i = 0; i < N; i++) {
    await tiles[i % tiles.length].focus();
    await p.waitForTimeout(30);
    await p.evaluate(() => new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r))));
  }
  const m2 = await cdp.send('Performance.getMetrics');
  const get = (m, k) => m.metrics.find(x => x.name === k).value * 1000;
  await ctx.close();
  return { themed, elements, fullRecalcStyleMs: +(get(m1, 'RecalcStyleDuration') - get(m0, 'RecalcStyleDuration')).toFixed(1),
    perMoveTaskMs: +((get(m2, 'TaskDuration') - get(m1, 'TaskDuration')) / N).toFixed(2), perMoveStyleMs: +((get(m2, 'RecalcStyleDuration') - get(m1, 'RecalcStyleDuration')) / N).toFixed(2) };
}

for (const c of (copies.length ? copies : [0, 15])) for (let k = 0; k < 3; k++) for (const themed of [false, true]) console.log(JSON.stringify(await run(themed, c)));
await browser.close();
