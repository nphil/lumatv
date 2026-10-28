// Renders the stand-in Content Store pages (gen_pages.py) with and without the store theme, and audits what the theme drew from the pixels.
//
//   node render.mjs [--audit-only] [--pages home,detail] [--dialects pseudo,child] [--scales 1,2]
//
// The theme is the real thing: tools/store_theme.py builds the script WAM would run (payload/store), and it is handed to the page as an
// init script, which runs before the page's own scripts in every new document, as WAM's user script does. The pages are LG's page as the
// TV showed it, in several ways of building the focus plate (gen_pages.py), because the markup of the store cannot be read from the set.
// Each render focuses one item, waits for LG's transitions and for the theme's plate-finder, takes a screenshot and measures it:
//   contrast   every word, from the rendered pixels (the most common colour of its box against the colour farthest from it; pixels, not CSS: a
//              plate drawn by ::before has no background to read): on the plate the theme drew 4.5:1 (3:1 from 40 px up), anywhere else no
//              worse than LG drew it (its own secondary buttons are 3.4:1)
//   overlaps   text boxes that cross each other
//   plate      the focused item's plate was found and is Lucent's glass, with the ring 3:1 against the page around it and the plate itself
//   font       the platform font the title and the tabs are drawn in is Manrope (the page declares Sandstone from a system face)
//   fade       60 ms after the item loses the focus, while LG fades its plate out (its transitions slowed to a tenth so that a screenshot sees
//              them in progress), the plate's place is not light (LG's own plate is, there where its fade is one of opacity)
//   icons      the white tile behind an app icon is not taken for a plate
// Writes shots/<page>-pseudo-<100|200>.jpg and <page>-pseudo-stock.jpg (whole pages, the primary dialect), focus-<dialect>.jpg (LG's focused item beside
// the themed one, each dialect) and audit.json, and prints one JSON object. Exit 0 when every render passes.
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
const auditOnly = args.includes('--audit-only');
const PAGES = (flag('--pages') || 'home,category,detail,search,popup').split(',');
const DIALECTS = (flag('--dialects') || 'pseudo,child,self,tile,opaque,late').split(',');
const SCALES = (flag('--scales') || '1,2').split(',').map(Number);
const PRIMARY = 'pseudo';

const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'store-render-'));
execFileSync('python3', [path.join(here, 'gen_pages.py'), path.join(tmp, 'pages')], { stdio: ['ignore', 'ignore', 'inherit'] });
const scriptFile = path.join(tmp, 'userScript.js');
execFileSync('python3', [path.join(root, 'tools', 'store_theme.py'), 'build', path.join(root, 'payload', 'store'),
  path.join(root, 'payload', 'theme', 'theme.json'), '/dev/null', scriptFile], { stdio: ['ignore', 'ignore', 'inherit'] });
const script = fs.readFileSync(scriptFile, 'utf8');

// Runs in the page: every word with its line box (the face's content area overlaps its neighbours' at any ordinary line height, and nobody
// sees that), skipping what is under something else; the focused item's box; what the theme marked.
function collect() {
  const out = { words: [], focus: null, marks: [], wrappers: [] };
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  for (let n; (n = walker.nextNode());) {
    const text = n.nodeValue.replace(/\s+/g, ' ').trim();
    if (!text) continue;
    const el = n.parentElement, cs = getComputedStyle(el);
    if (cs.visibility === 'hidden' || cs.display === 'none' || el.closest('svg, script, style')) continue;
    const range = document.createRange(); range.selectNodeContents(n);
    const box = range.getBoundingClientRect();
    if (box.width < 2 || box.height < 2) continue;
    const probe = el.getBoundingClientRect();
    const top = document.elementFromPoint(Math.min(Math.max(probe.left + probe.width / 2, 0), innerWidth - 1), Math.min(Math.max(probe.top + probe.height / 2, 0), innerHeight - 1));
    if (top && !el.contains(top) && !top.contains(el)) continue;
    // a box cut by an ancestor that clips (a row that scrolls) shows only what is inside it
    let l = 0, t = 0, r = innerWidth, b = innerHeight;
    for (let e = el.parentElement; e; e = e.parentElement) {
      const c = getComputedStyle(e);
      if (c.overflowX !== 'visible' || c.overflowY !== 'visible') { const q = e.getBoundingClientRect(); l = Math.max(l, q.left); t = Math.max(t, q.top); r = Math.min(r, q.right); b = Math.min(b, q.bottom); }
    }
    const fs = parseFloat(cs.fontSize), lh = cs.lineHeight === 'normal' ? fs * 1.2 : parseFloat(cs.lineHeight);
    const h = Math.min(box.height, lh), y0 = box.top + (box.height - h) / 2;
    const x = Math.max(box.left, l), x2 = Math.min(box.right, r), y = Math.max(y0, t), y2 = Math.min(y0 + h, b);
    if (x2 - x < 2 || y2 - y < 2) continue;
    out.words.push({ len: text.length, x, y, w: x2 - x, h: y2 - y, size: fs, tag: el.tagName.toLowerCase(), cls: String(el.className).slice(-12) });
  }
  const f = document.activeElement;
  if (f && f !== document.body) { const q = f.getBoundingClientRect(); out.focus = { x: q.left, y: q.top, w: q.width, h: q.height, tag: f.tagName.toLowerCase() }; }
  for (const e of document.querySelectorAll('[data-lu-ps],[data-lu-pb],[data-lu-pa],[data-lu-fr]')) {
    const q = e.getBoundingClientRect();
    out.marks.push({ kind: ['ps', 'pb', 'pa', 'fr'].filter(k => e.hasAttribute('data-lu-' + k)).join(','), x: q.left, y: q.top, w: q.width, h: q.height });
  }
  for (const e of document.querySelectorAll('[class*="imageIcon"]')) {
    out.wrappers.push({ white: getComputedStyle(e).backgroundColor === 'rgb(255, 255, 255)', marked: [...e.attributes].some(a => a.name.startsWith('data-lu-')) });
  }
  out.found = window.__lumatvFocus || null;
  return out;
}

// Runs in a blank page: decodes the screenshot and measures each box.
async function analyse([pngB64, W, H, boxes, points]) {
  const bytes = Uint8Array.from(atob(pngB64), c => c.charCodeAt(0));
  const bmp = await createImageBitmap(new Blob([bytes], { type: 'image/png' }));
  const cv = new OffscreenCanvas(W, H), cx = cv.getContext('2d');
  cx.drawImage(bmp, 0, 0);
  const d = cx.getImageData(0, 0, W, H).data;
  const L = (r, g, b) => { const f = v => { v /= 255; return v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4); }; return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b); };
  const ratio = (a, b) => { const x = L(...a), y = L(...b); return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05); };
  const at = (x, y) => { x = Math.max(0, Math.min(W - 1, Math.round(x))); y = Math.max(0, Math.min(H - 1, Math.round(y))); const i = (y * W + x) * 4; return [d[i], d[i + 1], d[i + 2]]; };
  const words = boxes.map(b => {
    const counts = new Map(); let n = 0;
    const x0 = Math.max(0, Math.floor(b.x)), y0 = Math.max(0, Math.floor(b.y)), x1 = Math.min(W, Math.ceil(b.x + b.w)), y1 = Math.min(H, Math.ceil(b.y + b.h));
    for (let y = y0; y < y1; y++) for (let x = x0; x < x1; x++) { const i = (y * W + x) * 4, k = ((d[i] >> 3) << 10) | ((d[i + 1] >> 3) << 5) | (d[i + 2] >> 3); counts.set(k, (counts.get(k) || 0) + 1); n++; }
    let bk = 0, bc = -1; for (const [k, c] of counts) if (c > bc) { bc = c; bk = k; }
    const bg = [((bk >> 10) & 31) * 8 + 4, ((bk >> 5) & 31) * 8 + 4, (bk & 31) * 8 + 4];
    let far = bg, fd = -1; const lb = L(...bg);
    for (let y = y0; y < y1; y++) for (let x = x0; x < x1; x++) { const i = (y * W + x) * 4, p = [d[i], d[i + 1], d[i + 2]], dd = Math.abs(L(...p) - lb); if (dd > fd) { fd = dd; far = p; } }
    return { ratio: ratio(bg, far), bg, fg: far };
  });
  const pts = (points || []).map(p => at(p[0], p[1]));
  return { words, pts, ratio: pts.length >= 3 ? [ratio(pts[0], pts[1]), ratio(pts[0], pts[2])] : null };
}

const browser = await chromium.launch({ executablePath: '/usr/bin/chromium', args: ['--no-sandbox', '--force-color-profile=srgb'] });
const helper = await (await browser.newContext()).newPage();
const shots = path.join(here, 'shots');
if (!auditOnly) { fs.rmSync(shots, { recursive: true, force: true }); fs.mkdirSync(shots, { recursive: true }); }

async function render(page, dialect, scale, themed) {
  const ctx = await browser.newContext({ viewport: { width: 1920, height: 1080 }, deviceScaleFactor: 1 });
  if (themed) await ctx.addInitScript(script);
  await ctx.addInitScript(`(function () { var set = function () { document.documentElement.style.setProperty('--ts', '${scale}'); };
    if (document.documentElement) set(); else new MutationObserver(function (m, o) { if (document.documentElement) { o.disconnect(); set(); } }).observe(document, { childList: true }); })();`);
  const p = await ctx.newPage();
  const errors = [];
  p.on('pageerror', e => errors.push(e.message));
  await p.goto('file://' + path.join(tmp, 'pages', `${page}-${dialect}.html`));
  await p.evaluate(() => document.fonts.ready);
  await p.waitForFunction(() => [...document.images].every(i => i.complete && i.naturalWidth > 0));
  const sel = await p.evaluate(() => document.body.getAttribute('data-focus'));
  if (sel) await p.focus(sel, { timeout: 3000 });
  await p.waitForTimeout(700);                       // LG's transitions (.2 s) and the plate-finder's retries (up to .26 s) are over
  const info = await p.evaluate(collect);
  const png = await p.screenshot({ type: 'png' });
  const cdp = await ctx.newCDPSession(p);
  await cdp.send('DOM.enable'); await cdp.send('CSS.enable');
  const doc = await cdp.send('DOM.getDocument', { depth: 0 });
  const fonts = {};
  for (const [name, selector] of [['title', 'header h1'], ['tab', '[role="tab"]'], ['name', '.txt > span:first-child']]) {
    try {
      const q = await cdp.send('DOM.querySelector', { nodeId: doc.root.nodeId, selector });
      if (q.nodeId) fonts[name] = (await cdp.send('CSS.getPlatformFontsForNode', { nodeId: q.nodeId })).fonts.map(f => f.familyName);
    } catch (e) { /* no such element on this page */ }
  }
  // the item loses the focus while LG fades its plate out: what is left of the plate 60 ms into the fade (LG's light plate must not show)
  let fade = null;
  const spot = info.marks.find(m => /p[sba]/.test(m.kind)) || (!themed && info.focus ? info.focus : null);
  if (spot && page === 'home') {
    await cdp.send('Animation.enable');
    await cdp.send('Animation.setPlaybackRate', { playbackRate: 0.1 });         // a screenshot takes longer than LG's whole fade: slow it, so that it is in progress
    await p.evaluate(() => document.activeElement.blur());
    await p.waitForTimeout(60);
    fade = await p.screenshot({ type: 'png', clip: { x: spot.x, y: spot.y, width: spot.w, height: spot.h } });
  }
  await ctx.close();
  return { info, png, fonts, errors, fade };
}

// Runs in a blank page: the share of a picture that is light (relative luminance 0.35 and up: LG's plate is 0.79, its words are thin strokes).
async function lightShare(b64) {
  const bmp = await createImageBitmap(new Blob([Uint8Array.from(atob(b64), c => c.charCodeAt(0))], { type: 'image/png' }));
  const cv = new OffscreenCanvas(bmp.width, bmp.height), g = cv.getContext('2d');
  g.drawImage(bmp, 0, 0);
  const d = g.getImageData(0, 0, bmp.width, bmp.height).data, f = v => { v /= 255; return v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4); };
  let k = 0;
  for (let i = 0; i < d.length; i += 4) if (0.2126 * f(d[i]) + 0.7152 * f(d[i + 1]) + 0.0722 * f(d[i + 2]) >= 0.35) k++;
  return k / (d.length / 4);
}

async function measure(r, stock) {
  const { info } = r;
  // the focused plate's ring: the pixel at the plate's top edge (the ring), just outside it (the page) and just inside (the plate)
  let points = null;
  const plates = info.marks.filter(m => /p[sba]/.test(m.kind));
  const plate = plates[0];
  if (plate) points = [[plate.x + plate.w / 2, plate.y + 1], [plate.x + plate.w / 2, plate.y - 6], [plate.x + 8, plate.y + plate.h / 2]];
  const a = await helper.evaluate(analyse, [r.png.toString('base64'), 1920, 1080, info.words, points]);
  const s = stock ? await helper.evaluate(analyse, [stock.png.toString('base64'), 1920, 1080, stock.info.words, null]) : null;
  const same = s && stock.info.words.length === info.words.length;
  const res = { words: info.words.length, lowContrast: [], worseThanLG: [], overlaps: [], minContrast: 99, plate: null, ring: null, fonts: r.fonts, errors: r.errors, iconTilesKept: info.wrappers.every(w => !w.marked) };
  info.words.forEach((w, i) => {
    const need = w.size >= 40 ? 3 : 4.5, c = a.words[i].ratio;
    if (c < res.minContrast) { res.minContrast = c; res.minWord = { tag: w.tag, size: w.size, bg: a.words[i].bg, fg: a.words[i].fg }; }
    const cx = w.x + w.w / 2, cy = w.y + w.h / 2;
    const onPlate = plates.some(m => cx >= m.x && cx <= m.x + m.w && cy >= m.y && cy <= m.y + m.h);
    // a word on the plate the theme drew must pass; any other word must be no worse than LG drew it (LG's own secondary buttons are 3.4:1)
    if (onPlate && c < need) res.lowContrast.push({ len: w.len, tag: w.tag, ratio: Math.round(c * 100) / 100, need });
    if (!onPlate && same && c < Math.min(need, s.words[i].ratio) - 0.25) res.worseThanLG.push({ len: w.len, tag: w.tag, ratio: Math.round(c * 100) / 100, lg: Math.round(s.words[i].ratio * 100) / 100 });
    for (let j = i + 1; j < info.words.length; j++) {
      const b = info.words[j];
      const ow = Math.min(w.x + w.w, b.x + b.w) - Math.max(w.x, b.x), oh = Math.min(w.y + w.h, b.y + b.h) - Math.max(w.y, b.y);
      if (ow > 3 && oh > 3) res.overlaps.push({ a: w.len, b: b.len, w: Math.round(ow), h: Math.round(oh) });
    }
  });
  if (r.fade) res.fade = { themed: Math.round((await helper.evaluate(lightShare, r.fade.toString('base64'))) * 100) / 100 };
  if (stock && stock.fade) res.fade = { ...(res.fade || {}), lg: Math.round((await helper.evaluate(lightShare, stock.fade.toString('base64'))) * 100) / 100 };
  res.plate = info.found ? { count: info.found.plates.length, words: info.found.words, kinds: info.found.plates.map(p => p.split(':')[0]), ms: info.found.ms } : null;
  if (a.ratio) res.ring = { vsPage: Math.round(a.ratio[0] * 100) / 100, vsPlate: Math.round(a.ratio[1] * 100) / 100, plateColour: a.pts[2] };
  return res;
}

const results = [];
const pairs = [];                                            // [page, dialect, scale] to render; the primary dialect gets every page
for (const d of DIALECTS) for (const p of PAGES) if (d === PRIMARY || p === 'home' || p === 'detail') for (const s of SCALES) pairs.push([p, d, s]);
for (const [page, dialect, scale] of pairs) {
  const themed = await render(page, dialect, scale, true);
  const stock = await render(page, dialect, scale, false);
  const r = await measure(themed, stock);
  results.push({ page, dialect, scale, ...r });
  if (!auditOnly) {
    // whole pages for the primary dialect; the others are shown by the focused item, LG's beside the themed one
    if (dialect === PRIMARY) fs.writeFileSync(path.join(shots, `${page}-${dialect}-${scale}00.jpg`), await jpeg(themed.png));
    if (scale === 1) {
      if (dialect === PRIMARY) fs.writeFileSync(path.join(shots, `${page}-${dialect}-stock.jpg`), await jpeg(stock.png));
      if (page === 'home') fs.writeFileSync(path.join(shots, `focus-${dialect}.jpg`), await sideBySide(stock.png, themed.png, themed.info.focus || stock.info.focus));
    }
  }
}
async function jpeg(png) {
  return helper.evaluate(async b64 => {
    const bytes = Uint8Array.from(atob(b64), c => c.charCodeAt(0));
    const bmp = await createImageBitmap(new Blob([bytes], { type: 'image/png' }));
    const cv = new OffscreenCanvas(bmp.width, bmp.height); cv.getContext('2d').drawImage(bmp, 0, 0);
    const blob = await cv.convertToBlob({ type: 'image/jpeg', quality: 0.82 });
    const buf = new Uint8Array(await blob.arrayBuffer()); let s = ''; for (let i = 0; i < buf.length; i += 8192) s += String.fromCharCode.apply(null, buf.subarray(i, i + 8192));
    return btoa(s);
  }, png.toString('base64')).then(b => Buffer.from(b, 'base64'));
}
async function sideBySide(stock, themed, f) {
  const box = f ? { x: Math.max(0, f.x - 40), y: Math.max(0, f.y - 30), w: Math.min(1920 - Math.max(0, f.x - 40), f.w + 80), h: f.h + 60 } : { x: 0, y: 700, w: 900, h: 300 };
  return helper.evaluate(async ([a, b, box]) => {
    const load = async s => createImageBitmap(new Blob([Uint8Array.from(atob(s), c => c.charCodeAt(0))], { type: 'image/png' }));
    const A = await load(a), B = await load(b), gap = 24, label = 34;
    const cv = new OffscreenCanvas(box.w * 2 + gap, box.h + label), g = cv.getContext('2d');
    g.fillStyle = '#000'; g.fillRect(0, 0, cv.width, cv.height);
    g.drawImage(A, box.x, box.y, box.w, box.h, 0, label, box.w, box.h); g.drawImage(B, box.x, box.y, box.w, box.h, box.w + gap, label, box.w, box.h);
    g.fillStyle = '#aaa'; g.font = '22px sans-serif'; g.fillText('LG', 6, 24); g.fillText('with the theme', box.w + gap + 6, 24);
    const blob = await cv.convertToBlob({ type: 'image/jpeg', quality: 0.86 });
    const buf = new Uint8Array(await blob.arrayBuffer()); let s = ''; for (let i = 0; i < buf.length; i += 8192) s += String.fromCharCode.apply(null, buf.subarray(i, i + 8192));
    return btoa(s);
  }, [stock.toString('base64'), themed.toString('base64'), box]).then(b => Buffer.from(b, 'base64'));
}
await browser.close();

const bad = r => r.overlaps.length || r.lowContrast.length || r.worseThanLG.length || (r.fade && r.fade.themed > 0.25) || r.errors.length || !r.iconTilesKept || !r.plate || r.plate.count < 1 || !r.ring ||
  r.ring.vsPage < 3 || r.ring.vsPlate < 3 || !(r.fonts.title || r.fonts.tab || r.fonts.name) ||
  Object.values(r.fonts).some(list => !list.some(n => /manrope/i.test(n)));
const summary = { ok: results.every(r => !bad(r)), failing: results.filter(bad).map(r => ({ page: r.page, dialect: r.dialect, scale: r.scale, overlaps: r.overlaps.slice(0, 2), lowContrast: r.lowContrast.slice(0, 3), worseThanLG: r.worseThanLG.slice(0, 3), plate: r.plate, ring: r.ring, fade: r.fade, fonts: r.fonts, errors: r.errors, iconTilesKept: r.iconTilesKept })), results };
if (!auditOnly) fs.writeFileSync(path.join(here, 'audit.json'), JSON.stringify(summary, null, 1) + '\n');
console.log(JSON.stringify(summary));
process.exitCode = summary.ok ? 0 : 1;                       // not process.exit(): a long line on a pipe is cut by it
