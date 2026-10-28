// What the store theme costs a page, on this workstation's Chromium: the heavy stand-in page (ten shelves of fourteen tiles, about 2,300
// elements, gen_pages.py's `heavy`) with no theme and with each variant of the script, as the main thread sees it (CDP Performance metrics).
//
//   node bench.mjs [--v1 FILE] [--runs N] [--throttle R] [--dialect pseudo] [--page heavy-spa|heavy]
//
//   --v1        a script built by the first version (git worktree at 88327e2, tools/store_theme.py build ...), for the comparison
//   --variant   name=file, any other script to measure beside the built ones (repeatable)
//   --page      heavy-spa (default): the shelves are inserted by script 300 ms after the document, as the store's cloud page renders; heavy: all in the markup
//   --throttle  CPU slow-down factor (the set's CPU is several times slower than this one; ratios carry over, milliseconds do not)
//
// Per variant, the median of N runs (interleaved, one fresh browser context each):
//   bytes     the script as WAM would read it
//   eval      the script's own run, from its first line to its last (parse and the work it does at start-up)
//   style     style recalculation from navigation to the load event plus 400 ms (the page's own and the theme's)
//   load      the main thread's busy time over the same span
//   fcp       first contentful paint
//   recalc    one full style recalculation of the loaded page (a class toggled on the root, the computed style read back)
//   move      one focus move to a tile and the style and layout it forces, mean over 30 moves
// These are host numbers on a stand-in page, not LG's page and not the TV: they say what the sheet costs relative to the page, and the TV
// round is what says what it does on the set (docs/research/store-result.md).
import { chromium } from '/data/home/lucent-ha/node_modules/playwright-core/index.mjs';
import { execFileSync } from 'child_process';
import { fileURLToPath } from 'url';
import fs from 'fs';
import os from 'os';
import path from 'path';

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, '..', '..', '..');
const args = process.argv.slice(2);
const flag = n => (args.includes(n) ? args[args.indexOf(n) + 1] : null);
const RUNS = Number(flag('--runs') || 15);
const THROTTLE = Number(flag('--throttle') || 1);
const DIALECT = flag('--dialect') || 'pseudo';
const PAGE = flag('--page') || 'heavy-spa';

const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'store-bench-'));
execFileSync('python3', [path.join(here, 'gen_pages.py'), path.join(tmp, 'pages'), '--dialects', DIALECT], { stdio: ['ignore', 'ignore', 'inherit'] });
const build = (parts) => {
  const out = path.join(tmp, parts.replace(/,/g, '-') + '.js');
  execFileSync('python3', [path.join(root, 'tools', 'store_theme.py'), 'build', path.join(root, 'payload', 'store'),
    path.join(root, 'payload', 'theme', 'theme.json'), '/dev/null', out, '--parts', parts], { stdio: ['ignore', 'ignore', 'inherit'] });
  return fs.readFileSync(out, 'utf8');
};
const variants = [['stock', null]];
if (flag('--v1')) variants.push(['v1 (look,motion)', fs.readFileSync(flag('--v1'), 'utf8')]);
for (let i = 0; i < args.length; i++) if (args[i] === '--variant') { const [n, f] = args[i + 1].split('='); variants.push([n, fs.readFileSync(f, 'utf8')]); }
variants.push(['look,motion', build('look,motion')], ['look', build('look')], ['motion', build('motion')], ['look,motion,canvas', build('look,motion,canvas')]);

const browser = await chromium.launch({ executablePath: '/usr/bin/chromium', args: ['--no-sandbox', '--force-color-profile=srgb'] });

async function once(script) {
  const ctx = await browser.newContext({ viewport: { width: 1920, height: 1080 } });
  if (script) await ctx.addInitScript('var __t0 = performance.now();\n' + script + '\n;window.__evalMs = performance.now() - __t0;');
  const p = await ctx.newPage();
  const cdp = await ctx.newCDPSession(p);
  await cdp.send('Performance.enable');
  if (THROTTLE > 1) await cdp.send('Emulation.setCPUThrottlingRate', { rate: THROTTLE });
  await p.goto('file://' + path.join(tmp, 'pages', `${PAGE}-${DIALECT}.html`), { waitUntil: 'load' });
  await p.waitForTimeout(PAGE === 'heavy-spa' ? 1000 : 400);
  const get = (m, k) => m.metrics.find(x => x.name === k).value * 1000;
  const m0 = await cdp.send('Performance.getMetrics');
  const info = await p.evaluate(async () => {
    await document.fonts.ready;
    const fcp = (performance.getEntriesByName('first-contentful-paint')[0] || {}).startTime || 0;
    const t = performance.now();
    document.documentElement.classList.toggle('x');
    getComputedStyle(document.body).color;
    const recalc = performance.now() - t;
    const tiles = [...document.querySelectorAll('.capf')];
    let moves = 0;
    const t1 = performance.now();
    for (let i = 0; i < 30; i++) { tiles[(i * 7) % tiles.length].focus(); getComputedStyle(document.body).color; document.body.offsetHeight; moves++; }
    const move = (performance.now() - t1) / moves;
    return { n: document.querySelectorAll('*').length, eval: window.__evalMs || 0, fcp, recalc, move };
  });
  await ctx.close();
  return { n: info.n, eval: info.eval, fcp: info.fcp, recalc: info.recalc, move: info.move, style: get(m0, 'RecalcStyleDuration'), load: get(m0, 'TaskDuration') };
}

const med = a => { const s = [...a].sort((x, y) => x - y); return s[Math.floor(s.length / 2)]; };
const rows = new Map(variants.map(([name]) => [name, []]));
for (let i = 0; i < RUNS + 2; i++) {
  for (const [name, script] of variants) {
    const r = await once(script);
    if (i >= 2) rows.get(name).push(r);                       // the first two rounds warm the browser and are dropped
  }
}
await browser.close();
const f = (x, d = 1) => x.toFixed(d);
const lines = ['| variant | bytes | eval ms | style ms | load ms | fcp ms | recalc ms | move ms |', '|---|---|---|---|---|---|---|---|'];
const result = {};
for (const [name, script] of variants) {
  const r = rows.get(name);
  const m = k => med(r.map(x => x[k]));
  result[name] = { bytes: script ? Buffer.byteLength(script) : 0, eval: m('eval'), style: m('style'), load: m('load'), fcp: m('fcp'), recalc: m('recalc'), move: m('move'), elements: r[0].n };
  lines.push(`| ${name} | ${result[name].bytes} | ${f(m('eval'))} | ${f(m('style'))} | ${f(m('load'))} | ${f(m('fcp'))} | ${f(m('recalc'))} | ${f(m('move'), 2)} |`);
}
console.log(`${PAGE} page, ${DIALECT} dialect, ${result.stock.elements} elements, ${RUNS} runs each, cpu x${THROTTLE}`);
console.log(lines.join('\n'));
console.log(JSON.stringify({ throttle: THROTTLE, runs: RUNS, result }));
