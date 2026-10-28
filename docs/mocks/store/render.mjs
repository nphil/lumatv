// Renders the stand-in Content Store pages (this directory) with and without the store theme, and checks what the theme drew.
//
//   node render.mjs [--audit-only] [--pages home,detail]
//
// The theme is the real thing: tools/store_theme.py builds the script that WAM would run (payload/store), and it is handed to the page
// as an init script, which runs before the page's own scripts in every new document, as WAM's user script does. Per page and material
// (dark, light) and text scale (1 and 2) it writes shots/<page>-<material>-<scale>00.jpg; shots/<page>-stock.jpg is the same page with
// no theme, as LG draws it. The audit runs on every themed render and prints one JSON object (and writes audit.json beside this file):
//   overlaps      text boxes that cross each other (not a box and what is inside it)
//   lowContrast   text under 4.5:1 against what is behind it (3:1 from 40 px up)
//   ring          the focused element's ring: colour against the canvas around it and the fill under it, 3:1 or better
// The pages are drawn at 1920x1080, the page size the store is laid out at (appinfo `resolution`).
import { chromium } from '/data/home/lucent-ha/node_modules/playwright-core/index.mjs';
import { execFileSync } from 'child_process';
import { fileURLToPath } from 'url';
import fs from 'fs';
import os from 'os';
import path from 'path';

const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, '..', '..', '..');
const args = process.argv.slice(2);
const auditOnly = args.includes('--audit-only');
const pagesArg = args.includes('--pages') ? args[args.indexOf('--pages') + 1].split(',') : null;
const PAGES = (pagesArg || ['home', 'category', 'detail', 'search', 'popup']);
const MATERIALS = ['dark', 'light'];
const SCALES = [1, 2];

function buildScript(material, tmp) {
  const out = path.join(tmp, `userScript-${material}.js`);
  execFileSync('python3', [path.join(root, 'tools', 'store_theme.py'), 'build', path.join(root, 'payload', 'store'),
    path.join(root, 'payload', 'theme', 'theme.json'), '/dev/null', out, '--material', material], { stdio: ['ignore', 'pipe', 'inherit'] });
  return fs.readFileSync(out, 'utf8');
}

// Runs in the page. Colours are composited from the root down, so glass over the canvas is judged as it is seen.
function audit() {
  const parse = s => { const m = s.match(/rgba?\(([^)]+)\)/); if (!m) return null; const p = m[1].split(/[ ,/]+/).filter(Boolean).map(Number); return { r: p[0], g: p[1], b: p[2], a: p.length > 3 ? p[3] : 1 }; };
  const over = (fg, bg) => ({ r: fg.r * fg.a + bg.r * (1 - fg.a), g: fg.g * fg.a + bg.g * (1 - fg.a), b: fg.b * fg.a + bg.b * (1 - fg.a), a: 1 });
  const lum = c => { const f = v => { v /= 255; return v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4); }; return 0.2126 * f(c.r) + 0.7152 * f(c.g) + 0.0722 * f(c.b); };
  const ratio = (a, b) => { const x = lum(a), y = lum(b); return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05); };
  const behind = (el, self) => {
    const chain = []; for (let e = el; e; e = e.parentElement) chain.unshift(e);
    let c = { r: 255, g: 255, b: 255, a: 1 };
    for (const e of chain) { if (e === el && !self) continue; const bg = parse(getComputedStyle(e).backgroundColor); if (bg && bg.a > 0) c = over(bg, c); }
    return c;
  };
  const clipOf = el => {
    let r = { l: 0, t: 0, r: innerWidth, b: innerHeight };
    for (let e = el.parentElement; e; e = e.parentElement) {
      const cs = getComputedStyle(e);
      if (cs.overflowX !== 'visible' || cs.overflowY !== 'visible') { const b = e.getBoundingClientRect(); r = { l: Math.max(r.l, b.left), t: Math.max(r.t, b.top), r: Math.min(r.r, b.right), b: Math.min(r.b, b.bottom) }; }
    }
    return r;
  };
  const out = { texts: 0, overlaps: [], lowContrast: [], ring: null, minContrast: 99 };
  const items = [];
  const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
  for (let n; (n = walker.nextNode());) {
    const text = n.nodeValue.replace(/\s+/g, ' ').trim();
    if (!text) continue;
    const el = n.parentElement, cs = getComputedStyle(el);
    if (cs.visibility === 'hidden' || cs.display === 'none' || Number(cs.opacity) < 0.05 || el.closest('svg, script, style')) continue;
    // Text under something opaque (the page beneath a popup) is not on screen.
    const probe = el.getBoundingClientRect(), top = document.elementFromPoint(Math.min(Math.max(probe.left + probe.width / 2, 0), innerWidth - 1), Math.min(Math.max(probe.top + probe.height / 2, 0), innerHeight - 1));
    if (top && !el.contains(top) && !top.contains(el)) continue;
    const range = document.createRange(); range.selectNodeContents(n);
    const box = range.getBoundingClientRect();
    if (box.width < 2 || box.height < 2) continue;
    // The text's box is its line box, not the font's content area: a face with tall ascenders and descenders has content areas that
    // overlap their neighbours' at any ordinary line height, and nobody sees that.
    const fs = parseFloat(cs.fontSize), lh = cs.lineHeight === 'normal' ? fs * 1.2 : parseFloat(cs.lineHeight);
    const h = Math.min(box.height, lh), rect = { left: box.left, right: box.right, top: box.top + (box.height - h) / 2, bottom: box.top + (box.height - h) / 2 + h };
    const clip = clipOf(el);
    const vis = { l: Math.max(rect.left, clip.l), t: Math.max(rect.top, clip.t), r: Math.min(rect.right, clip.r), b: Math.min(rect.bottom, clip.b) };
    if (vis.r - vis.l < 2 || vis.b - vis.t < 2) continue;
    items.push({ n, el, text, rect: vis });
  }
  out.texts = items.length;
  for (let i = 0; i < items.length; i++) {
    const a = items[i];
    const fg = parse(getComputedStyle(a.el).color), bg = behind(a.el, true);
    const c = ratio(over(fg, bg), bg), size = parseFloat(getComputedStyle(a.el).fontSize), need = size >= 40 ? 3 : 4.5;
    out.minContrast = Math.min(out.minContrast, c);
    if (c < need) out.lowContrast.push({ text: a.text.slice(0, 40), ratio: Math.round(c * 100) / 100, need });
    for (let j = i + 1; j < items.length; j++) {
      const b = items[j];
      if (a.el.contains(b.el) || b.el.contains(a.el)) continue;
      const w = Math.min(a.rect.r, b.rect.r) - Math.max(a.rect.l, b.rect.l), h = Math.min(a.rect.b, b.rect.b) - Math.max(a.rect.t, b.rect.t);
      if (w > 3 && h > 3) out.overlaps.push({ a: a.text.slice(0, 30), b: b.text.slice(0, 30), w: Math.round(w), h: Math.round(h) });
    }
  }
  const f = document.activeElement;
  if (f && f !== document.body) {
    const cs = getComputedStyle(f), oc = parse(cs.outlineColor), width = parseFloat(cs.outlineWidth) || 0;
    if (oc && width > 0 && cs.outlineStyle !== 'none') {
      const fill = behind(f, true), canvas = behind(f, false), ring = over(oc, fill);
      out.ring = { width, vsCanvas: Math.round(ratio(ring, canvas) * 100) / 100, vsFill: Math.round(ratio(ring, fill) * 100) / 100 };
    } else out.ring = { width: 0 };
  }
  return out;
}

const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'store-render-'));
const scripts = Object.fromEntries(MATERIALS.map(m => [m, buildScript(m, tmp)]));
const shots = path.join(here, 'shots');
if (!auditOnly) fs.mkdirSync(shots, { recursive: true });
const browser = await chromium.launch({ executablePath: '/usr/bin/chromium', args: ['--no-sandbox', '--force-color-profile=srgb'] });
const results = [];

async function render(page, material, scale) {
  const ctx = await browser.newContext({ viewport: { width: 1920, height: 1080 }, deviceScaleFactor: 1 });
  if (material) await ctx.addInitScript(scripts[material]);
  // The script runs as the document is created, before <html> exists: the scale waits for the root.
  await ctx.addInitScript(`(function () { var set = function () { document.documentElement.style.setProperty('--ts', '${scale}'); };
    if (document.documentElement) set(); else new MutationObserver(function (m, o) { if (document.documentElement) { o.disconnect(); set(); } }).observe(document, { childList: true }); })();`);
  const p = await ctx.newPage();
  await p.goto('file://' + path.join(here, page + '.html'));
  await p.evaluate(() => document.fonts.ready);
  await p.waitForFunction(() => [...document.images].every(i => i.complete && i.naturalWidth > 0));
  const sel = await p.evaluate(() => document.body.getAttribute('data-focus'));
  if (sel) await p.focus(sel, { timeout: 3000 });
  await p.waitForTimeout(500);
  const result = material ? await p.evaluate(audit) : null;
  if (!auditOnly) {
    const name = material ? `${page}-${material}-${scale}00` : `${page}-stock`;
    await p.screenshot({ path: path.join(shots, name + '.jpg'), type: 'jpeg', quality: 80 });
  }
  await ctx.close();
  return result;
}

for (const page of PAGES) {
  if (!auditOnly) await render(page, null, 1);
  for (const material of MATERIALS) for (const scale of SCALES) {
    const r = await render(page, material, scale);
    results.push({ page, material, scale, ...r });
  }
}
await browser.close();
const summary = { ok: results.every(r => r.overlaps.length === 0 && r.lowContrast.length === 0 && r.ring && r.ring.vsCanvas >= 3 && r.ring.vsFill >= 3), results };
if (!auditOnly) fs.writeFileSync(path.join(here, 'audit.json'), JSON.stringify(summary, null, 1) + '\n');
console.log(JSON.stringify(summary));
process.exit(summary.ok ? 0 : 1);
