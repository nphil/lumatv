// What the Content Store theme's script does in a real browser, run by tests/test_store_theme.py:
//
//   node store_behaviour.mjs <dark.js> <light.js> <look-only.js> <lazy.js>
//
// Each argument is a script built by tools/store_theme.py. It is handed to the page as an init script, which runs as the document is
// created, before <html> exists and before the page's own scripts, as WAM runs a user script. Prints one JSON object of results.
import { chromium } from '/data/home/lucent-ha/node_modules/playwright-core/index.mjs';
import fs from 'fs';
import os from 'os';
import path from 'path';

const [darkFile, lightFile, lookFile, lazyFile] = process.argv.slice(2);
const read = f => fs.readFileSync(f, 'utf8');
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'store-behaviour-'));
const page = (name, html) => { const p = path.join(tmp, name); fs.writeFileSync(p, html); return 'file://' + p; };

const BASIC = `<!doctype html><html><head><style>
  html { background: #000; color: #fff; font-family: Arial; } body { margin: 0; background: #000; }
  .box { width: 100px; height: 40px; background: #333; transition: transform 600ms linear; } .box.go { transform: translateX(200px); }
  @keyframes spin { to { transform: rotate(1turn); } } .spin { animation: spin 1s linear infinite; }
</style></head><body><h1 id="h">Hello</h1><div class="box" id="box"></div><div class="spin" id="spin">x</div></body></html>`;
const CSP = BASIC.replace('<head>', `<head><meta http-equiv="Content-Security-Policy" content="style-src 'none'; font-src 'none'">`);
const INNER = `<!doctype html><html><body><p id="p">inner</p></body></html>`;
const innerUrl = page('inner.html', INNER);
const OUTER = `<!doctype html><html><body><iframe id="own" src="${innerUrl}"></iframe><iframe id="srcdoc" srcdoc="<p>srcdoc</p>"></iframe></body></html>`;

const browser = await chromium.launch({ executablePath: '/usr/bin/chromium', args: ['--no-sandbox', '--force-color-profile=srgb'] });
const out = {};

async function open(url, scripts) {
  const ctx = await browser.newContext({ viewport: { width: 800, height: 600 } });
  for (const s of scripts) await ctx.addInitScript(s);
  const p = await ctx.newPage();
  const errors = [];
  p.on('pageerror', e => errors.push(e.message));
  await p.goto(url);
  await p.waitForTimeout(150);
  return { ctx, p, errors };
}

const TRANSITION = () => new Promise(res => {
  const b = document.getElementById('box'), t0 = performance.now();
  b.addEventListener('transitionend', () => res(performance.now() - t0), { once: true });
  b.classList.add('go');
});

{ // the look, dark
  const { ctx, p, errors } = await open(page('basic.html', BASIC), [read(darkFile)]);
  out.dark = await p.evaluate(async () => {
    const cs = e => getComputedStyle(e);
    await document.fonts.ready;
    return { mark: window.__lumatvStore, attr: document.documentElement.getAttribute('data-lumatv-store'), sheets: document.adoptedStyleSheets.length,
      htmlBg: cs(document.documentElement).backgroundColor, bodyBg: cs(document.body).backgroundColor, h1: cs(document.getElementById('h')).color,
      family: cs(document.body).fontFamily, loaded: [...document.fonts].filter(f => f.family === 'Manrope' && f.status === 'loaded').length,
      ring: cs(document.documentElement).getPropertyValue('--st-ring').trim() };
  });
  out.dark.errors = errors;
  await ctx.close();
}
{ // the look, light: the material is the build's, not an attribute of the page
  const { ctx, p } = await open(page('basic.html', BASIC), [read(lightFile)]);
  out.light = await p.evaluate(() => ({ htmlBg: getComputedStyle(document.documentElement).backgroundColor, h1: getComputedStyle(document.getElementById('h')).color,
    scheme: getComputedStyle(document.documentElement).colorScheme, theme: document.documentElement.getAttribute('data-theme') }));
  await ctx.close();
}
{ // a page that forbids inline styles and fonts still takes the theme: a constructable sheet is not a <style> element
  const { ctx, p } = await open(page('csp.html', CSP), [read(darkFile)]);
  out.csp = await p.evaluate(() => ({ htmlBg: getComputedStyle(document.documentElement).backgroundColor, h1: getComputedStyle(document.getElementById('h')).color }));
  await ctx.close();
}
{ // the motion: the same transition, with and without the cap
  const stock = await open(page('basic.html', BASIC), []);
  const capped = await open(page('basic.html', BASIC), [read(darkFile)]);
  const look = await open(page('basic.html', BASIC), [read(lookFile)]);
  out.stockMs = await stock.p.evaluate(TRANSITION);
  out.cappedMs = await capped.p.evaluate(TRANSITION);
  out.lookOnlyMs = await look.p.evaluate(TRANSITION);
  out.animate = await capped.p.evaluate(() => {
    const el = document.getElementById('box');
    const d = (kf, o) => el.animate(kf, o).effect.getTiming().duration;
    return { long: d([{ opacity: 0 }, { opacity: 1 }], { duration: 800 }), number: d([{ opacity: 0 }, { opacity: 1 }], 500), short: d([{ opacity: 0 }, { opacity: 1 }], { duration: 100 }),
      forever: d([{ opacity: 0 }, { opacity: 1 }], { duration: 800, iterations: Infinity }), keeps: el.animate([{ opacity: 0 }, { opacity: 1 }], { duration: 800, delay: 30 }).effect.getTiming().delay };
  });
  out.spinRate = await capped.p.evaluate(() => document.getElementById('spin').getAnimations()[0].playbackRate);
  for (const x of [stock, capped, look]) await x.ctx.close();
}
{ // lazy pictures only when asked for
  const plain = await open(page('basic.html', BASIC), [read(darkFile)]);
  const lazy = await open(page('basic.html', BASIC), [read(lazyFile)]);
  out.lazyOff = await plain.p.evaluate(() => { const i = document.createElement('img'); return [i.loading, i.decoding]; });
  out.lazyOn = await lazy.p.evaluate(() => { const i = document.createElement('img'); const j = new Image(); return [i.loading, i.decoding, j.loading, document.createElement('div').localName]; });
  for (const x of [plain, lazy]) await x.ctx.close();
}
{ // once per document, even when the script is handed over twice
  const { ctx, p } = await open(page('basic.html', BASIC), [read(darkFile), read(darkFile)]);
  out.twice = await p.evaluate(() => document.adoptedStyleSheets.length);
  await ctx.close();
}
{ // frames: the app's own files are the store's, an anonymous frame is not
  const { ctx, p } = await open(page('outer.html', OUTER), [read(darkFile)]);
  const frames = {};
  for (const f of p.frames()) {
    if (f === p.mainFrame()) continue;
    const name = await f.evaluate(() => (window.frameElement && window.frameElement.id) || '');
    frames[name] = await f.evaluate(() => ({ mark: window.__lumatvStore || null, bg: getComputedStyle(document.documentElement).backgroundColor }));
  }
  out.frames = frames;
  await ctx.close();
}
await browser.close();
console.log(JSON.stringify(out));
