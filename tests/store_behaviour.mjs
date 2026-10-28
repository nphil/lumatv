// What the Content Store theme's script does in a real browser, run by tests/test_store_theme.py:
//
//   node store_behaviour.mjs <default.js> <look.js> <motion.js> <lazy.js> <canvas.js>
//
// Each argument is a script built by tools/store_theme.py (default = look,motion; look; motion; look,motion,lazy; look,motion,canvas). It
// is handed to the page as an init script, which runs as the document is created, before <html> exists and before the page's own scripts,
// as WAM runs a user script. The pages are small and made for one question each, in the page's own words (LG's page is not in the
// firmware snapshot: docs/mocks/store/render.mjs holds the stand-ins that look like it). Prints one JSON object of measurements.
import { chromium } from '/data/home/lucent-ha/node_modules/playwright-core/index.mjs';
import fs from 'fs';
import os from 'os';
import path from 'path';

const [defaultFile, lookFile, motionFile, lazyFile, canvasFile] = process.argv.slice(2);
const read = f => fs.readFileSync(f, 'utf8');
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'store-behaviour-'));
const page = (name, html) => { const p = path.join(tmp, name); fs.writeFileSync(p, html); return 'file://' + p; };

const HEAD = `<!doctype html><html><head><meta charset="utf-8"><style>
  @font-face { font-family: Sandstone; font-weight: 300; src: local("Liberation Sans"); }
  @font-face { font-family: Sandstone; font-weight: 400; src: local("Liberation Sans"); }
  @font-face { font-family: Sandstone; font-weight: 600; src: local("Liberation Sans Bold"), local("Liberation Sans"); }
  @font-face { font-family: Sandstone; font-weight: 700; src: local("Liberation Sans Bold"), local("Liberation Sans"); }
  html { background: #000; color: #ececec; font-family: Arial; } body { margin: 0; background: #000; }
  .row { position: relative; display: block; width: 400px; height: 100px; padding: 10px; box-sizing: border-box; margin: 6px; }
  .n { position: relative; display: block; font-size: 24px; }
  `;
const BASIC = `${HEAD}
  .box { width: 100px; height: 40px; background: #333; transition: transform 600ms linear; } .box.go { transform: translateX(200px); }
  @keyframes spin { to { transform: rotate(1turn); } } .spin { animation: spin 1s linear infinite; }
</style></head><body><h1 id="h">Hello</h1><div class="box" id="box"></div><div class="spin" id="spin">x</div>
  <p id="p-sand" style="font-family: Sandstone; font-weight: 400">Sandstone words</p><p id="p-300" style="font-family: Sandstone; font-weight: 300">Light words</p>
  <p id="p-700" style="font-family: Sandstone; font-weight: 700">Bold words</p><p id="p-museo" style="font-family: MuseoSans, sans-serif; font-weight: 900">Museo words</p>
  <p id="p-lg" style="font-family: 'LG Smart UI', serif; font-weight: 400">LG words</p><p id="p-arial" style="font-family: Arial">Arial words</p>
  <p id="p-cjk" style="font-family: Sandstone">日本語</p></body></html>`;
const CSP = BASIC.replace('<head>', `<head><meta http-equiv="Content-Security-Policy" content="style-src 'none'; font-src 'none'">`);
const INNER = `<!doctype html><html><body><p id="p">inner</p></body></html>`;
const innerUrl = page('inner.html', INNER);
const OUTER = `<!doctype html><html><body><iframe id="own" src="${innerUrl}"></iframe><iframe id="srcdoc" srcdoc="<p>srcdoc</p>"></iframe></body></html>`;

// the focus: one item per way LG might draw a plate, and the things that must not be taken for one
const FOCUS = `${HEAD}
  #pa::before { content: ""; position: absolute; inset: 0; background: rgb(230, 230, 230); opacity: 0; transition: opacity .2s; } #pa:focus::before { opacity: 1; }
  #pa:focus .n { color: rgb(75, 76, 80); }
  #bt { width: 200px; height: 60px; background: rgba(255, 255, 255, .1); color: #eee; border: 0; font-size: 24px; }
  #ic-icon { width: 60px; height: 60px; background: #fff; } #ic:focus { outline: 1px dotted #888; }
  #bd::before { content: ""; position: absolute; inset: 0; background: rgb(230, 230, 230); opacity: 0; } #bd:focus::before { opacity: 1; }
  #bd:focus .n { color: rgb(75, 76, 80); } #badge { position: relative; background: #c00; color: #fff; padding: 2px 6px; display: inline-block; }
  #big { width: 980px; height: 560px; background: #eee; color: #111; }
  #lt .bg { position: absolute; inset: 0; background-color: transparent; transition: background-color .2s; } #lt.on .bg { background-color: rgb(230, 230, 230); }
  #lt.on .n { color: rgb(75, 76, 80); }
  #hd { height: 150px; background: #000; }
  #pn { background: rgb(95, 107, 122); padding: 20px; width: 520px; }
  #pq::before { content: ""; position: absolute; inset: 0; background: rgb(230, 230, 230); opacity: 0; } #pq:focus::before { opacity: 1; } #pq:focus .n { color: rgb(75, 76, 80); }
  #pr { background: rgb(95, 107, 122); width: 200px; height: 60px; border: 0; color: #eee; font-size: 24px; }
</style></head><body>
  <div class="row" id="pa" tabindex="0"><span class="n" id="pa1">Name</span><span class="n" id="pa2" style="opacity: .8">Meta</span><span class="n" id="pa3" style="opacity: .5">Price</span></div>
  <button id="bt" tabindex="0">Save</button>
  <div class="row" id="ic" tabindex="0"><div id="ic-icon"></div><span class="n" id="ic1">Words</span></div>
  <div class="row" id="bd" tabindex="0"><span class="n" id="bd1">Name</span><span id="badge">New</span></div>
  <div id="big" tabindex="0">Panel words</div>
  <div class="row" id="lt" tabindex="0"><span class="bg"></span><span class="n" id="lt1">Late</span></div>
  <header id="hd"><h1 id="hd1">Header</h1></header>
  <div id="pn"><div class="row" id="pq" tabindex="0"><span class="n" id="pq1">Install</span></div><button id="pr" tabindex="0" style="color: #eee">Cancel</button></div>
  <script>
    document.addEventListener('focusin', function (e) { if (e.target.id === 'lt') { var t = e.target; setTimeout(function () { t.classList.add('on'); }, 0); } });
    document.addEventListener('focusout', function (e) { if (e.target.id === 'lt') e.target.classList.remove('on'); });
  </script></body></html>`;

const browser = await chromium.launch({ executablePath: '/usr/bin/chromium', args: ['--no-sandbox', '--force-color-profile=srgb'] });
const out = {};

async function open(url, scripts, size = { width: 1000, height: 700 }) {
  const ctx = await browser.newContext({ viewport: size });
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

async function fonts(p, ids) {
  const cdp = await p.context().newCDPSession(p);
  await cdp.send('DOM.enable'); await cdp.send('CSS.enable');
  const doc = await cdp.send('DOM.getDocument', { depth: 0 });
  const res = {};
  for (const id of ids) {
    const q = await cdp.send('DOM.querySelector', { nodeId: doc.root.nodeId, selector: '#' + id });
    res[id] = (await cdp.send('CSS.getPlatformFontsForNode', { nodeId: q.nodeId })).fonts.map(f => f.familyName);
  }
  return res;
}

// what an item looks like with the focus on it (and after), from the page's computed styles
const SNAP = ids => {
  const cs = (e, ps) => getComputedStyle(e, ps || null);
  const out = { found: window.__lumatvFocus || null, marks: {}, color: {}, opacity: {} };
  for (const id of ids) {
    const e = document.getElementById(id);
    out.marks[id] = [...e.attributes].filter(a => a.name.startsWith('data-lu-')).map(a => a.name.slice(8) + (a.value ? '=' + a.value : ''));
    out.color[id] = cs(e).color; out.opacity[id] = cs(e).opacity;
  }
  const a = document.getElementById('pa');
  out.plate = { before: cs(a, '::before').backgroundColor, ring: cs(a, '::before').outlineStyle, radius: cs(a, '::before').borderRadius };
  out.active = document.activeElement && document.activeElement.id;
  // the ring as drawn on black (the sheet's own) and on a panel (the script's), and what it left on the elements' style
  const q = document.getElementById('pq'), a2 = document.getElementById('pa'), r = document.getElementById('pr'), b = document.getElementById('bt');
  out.ring = { onBlack: { outline: cs(a2, '::before').outlineColor, style: a2.getAttribute('style') }, onPanel: { outline: cs(q, '::before').outlineColor, style: q.getAttribute('style') },
    buttonOnPanel: { outline: cs(r).outlineColor, style: r.getAttribute('style') }, buttonOnBlack: { outline: cs(b).outlineColor, style: b.getAttribute('style') } };
  return out;
};

{ // the look: the face behind LG's family names, the page's own words in it, a character it lacks falls back
  const { ctx, p, errors } = await open(page('basic.html', BASIC), [read(defaultFile)]);
  out.dark = await p.evaluate(async () => {
    const cs = e => getComputedStyle(e);
    await document.fonts.ready;
    return { mark: window.__lumatvStore, attr: document.documentElement.getAttribute('data-lumatv-store'), sheets: document.adoptedStyleSheets.length,
      htmlBg: cs(document.documentElement).backgroundColor, bodyBg: cs(document.body).backgroundColor, h1: cs(document.getElementById('h')).color,
      manrope: [...document.fonts].filter(f => f.family === 'Manrope').length, sandstone: [...document.fonts].filter(f => f.family === 'Sandstone').length };
  });
  out.dark.errors = errors;
  out.fonts = await fonts(p, ['p-sand', 'p-300', 'p-700', 'p-museo', 'p-lg', 'p-arial', 'p-cjk']);
  await ctx.close();
}
{ // a page that forbids inline styles and fonts still takes the theme: a constructable sheet is not a <style> element
  const { ctx, p } = await open(page('csp.html', CSP), [read(defaultFile)]);
  out.csp = await p.evaluate(() => ({ sheets: document.adoptedStyleSheets.length, mark: window.__lumatvStore }));
  await ctx.close();
}
{ // the focus, item by item
  const { ctx, p, errors } = await open(page('focus.html', FOCUS), [read(defaultFile)]);
  const ids = ['pa', 'pa1', 'pa2', 'pa3', 'bt', 'ic', 'ic-icon', 'ic1', 'bd', 'bd1', 'badge', 'big', 'lt', 'lt1', 'hd', 'hd1', 'pq', 'pq1', 'pr'];
  const before = await p.evaluate(SNAP, ids);
  out.focus = { before, errors };
  for (const [name, selector, wait] of [['plate', '#pa', 80], ['button', '#bt', 400], ['icon', '#ic', 400], ['badge', '#bd', 80], ['panel', '#big', 400], ['late', '#lt', 80],
    ['ringPanel', '#pq', 80], ['ringButton', '#pr', 400]]) {
    await p.focus(selector);
    await p.waitForTimeout(wait);
    out.focus[name] = await p.evaluate(SNAP, ids);
  }
  // the focus leaves: everything the theme set comes off (a plate LG hides by fading it keeps its glass 300 ms, then goes)
  await p.evaluate(() => document.activeElement.blur());
  await p.waitForTimeout(500);
  out.focus.after = await p.evaluate(SNAP, ids);
  // the same item twice in a row, quickly: no mark is left behind by the one before
  await p.focus('#pa'); await p.focus('#bd'); await p.focus('#pa'); await p.waitForTimeout(80);
  out.focus.again = await p.evaluate(SNAP, ids);
  // time to the plate: the item's plate comes by a class the page adds after the focus event, and by a colour transition
  out.focus.lateMs = out.focus.late.found && out.focus.late.found.ms;
  await ctx.close();
}
{ // no look part, no focus layer: the page is LG's whatever is focused
  const { ctx, p } = await open(page('focus.html', FOCUS), [read(motionFile)]);
  await p.focus('#pa'); await p.waitForTimeout(100);
  out.focusOff = await p.evaluate(SNAP, ['pa', 'pa1']);
  await ctx.close();
}
{ // the canvas (opt-in): the palette's canvas in place of black, the header band with it; without it the page's black stays
  const on = await open(page('focus.html', FOCUS), [read(canvasFile)]);
  await on.p.waitForTimeout(800);
  out.canvasOn = await on.p.evaluate(() => ({ html: getComputedStyle(document.documentElement).backgroundColor, body: getComputedStyle(document.body).backgroundColor,
    header: getComputedStyle(document.getElementById('hd')).backgroundColor, marked: document.getElementById('hd').hasAttribute('data-lu-bare'),
    row: document.getElementById('pa').hasAttribute('data-lu-bare') }));
  await on.ctx.close();
  const off = await open(page('focus.html', FOCUS), [read(defaultFile)]);
  await off.p.waitForTimeout(800);
  out.canvasOff = await off.p.evaluate(() => ({ html: getComputedStyle(document.documentElement).backgroundColor, header: getComputedStyle(document.getElementById('hd')).backgroundColor,
    marked: document.getElementById('hd').hasAttribute('data-lu-bare') }));
  await off.ctx.close();
}
{ // the motion: the same transition, with and without the cap
  const stock = await open(page('basic.html', BASIC), []);
  const capped = await open(page('basic.html', BASIC), [read(defaultFile)]);
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
  const plain = await open(page('basic.html', BASIC), [read(defaultFile)]);
  const lazy = await open(page('basic.html', BASIC), [read(lazyFile)]);
  out.lazyOff = await plain.p.evaluate(() => { const i = document.createElement('img'); return [i.loading, i.decoding]; });
  out.lazyOn = await lazy.p.evaluate(() => { const i = document.createElement('img'); const j = new Image(); return [i.loading, i.decoding, j.loading, document.createElement('div').localName]; });
  for (const x of [plain, lazy]) await x.ctx.close();
}
{ // once per document, even when the script is handed over twice
  const { ctx, p } = await open(page('basic.html', BASIC), [read(defaultFile), read(defaultFile)]);
  out.twice = await p.evaluate(() => document.adoptedStyleSheets.length);
  await ctx.close();
}
{ // frames: the app's own files are the store's, an anonymous frame is not
  const { ctx, p } = await open(page('outer.html', OUTER), [read(defaultFile)]);
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
