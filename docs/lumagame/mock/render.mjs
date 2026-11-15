// Renders the LumaGame mocks at 1920x1080, device scale 1, and measures them.
// Usage: OUT=<dir> node render.mjs [--check] [name ...]   (default OUT: ./shots, ignored by git)
//
// --check measures every text run on the picture: the page is shot twice, once as drawn and
// once with text made transparent, and each glyph pixel is compared with the background pixel
// under it. Reports runs under 7:1, text that overlaps other text, and text cut off by the
// screen or a clipping box.
import { chromium } from "/data/home/lucent-ha/node_modules/playwright-core/index.mjs";
import { fileURLToPath } from "node:url";
import fs from "node:fs";
import path from "node:path";

const dir = path.dirname(fileURLToPath(import.meta.url));
const out = process.env.OUT || path.join(dir, "shots");
const args = process.argv.slice(2);
const check = args.includes("--check");
const only = args.filter((a) => !a.startsWith("--"));

const SHOTS = [
  ["01-home", "home.html"],
  ["01-home-light", "home.html?theme=light"],
  ["01-home-play-focus", "home.html?focus=-1:0"],
  ["02-home-most-played", "home.html?focus=1:1"],
  ["03-home-genres", "home.html?focus=3:1"],
  ["04-home-native-plate", "home.html?focus=4:3"],
  ["05-home-cinematic", "home-cinematic.html"],
  ["06-home-compact", "home-compact.html"],
  ["07-library", "library.html"],
  ["08-hub", "hub.html"],
  ["08-hub-light", "hub.html?theme=light"],
  ["09-chooser", "chooser.html"],
  ["10-integrations", "integrations.html"],
  ["11-pairing", "pairing.html"],
  ["12-signin", "signin.html"],
  ["13-settings-appearance", "settings.html"],
  ["13-settings-appearance-light", "settings.html?theme=light"],
  ["14-settings-streaming", "settings.html?section=streaming"],
  ["15-overlay", "overlay.html"],
  ["16-overlay-end-confirm", "overlay.html?confirm=end"],
  ["17-state-checking", "states.html?s=checking"],
  ["18-state-queue", "states.html?s=queue"],
  ["19-state-starting", "states.html?s=starting"],
  ["20-state-failed", "states.html?s=failed"],
  ["21-state-offline-pc", "states.html?s=offline"],
  ["22-state-first-run", "states.html?s=empty"],
  ["23-components", "components.html"],
  ["24-components-content", "components.html?board=content"],
  ["30-home-text200", "home.html?text=200"],
  ["31-hub-text200", "hub.html?text=200"],
  ["32-chooser-text200", "chooser.html?text=200"],
  ["33-settings-text200", "settings.html?section=streaming&text=200"],
  ["34-home-graphite-light", "home.html?family=graphite-chalk&theme=light"],
  ["35-hub-fjord", "hub.html?family=nordic-fjord"],
  ["36-settings-copper-light", "settings.html?family=copper-kiln&theme=light"],
];

const HIDE_TEXT = `*, *::before, *::after { color: transparent !important; -webkit-text-fill-color: transparent !important; text-shadow: none !important; caret-color: transparent !important; }`;

async function measure(page, a, b) {
  return page.evaluate(async ([a, b]) => {
    const load = async (src) => { const im = new Image(); im.src = src; await im.decode(); const c = new OffscreenCanvas(1920, 1080); const x = c.getContext("2d"); x.drawImage(im, 0, 0); return x.getImageData(0, 0, 1920, 1080).data; };
    const A = await load(a), B = await load(b);
    const lin = (v) => { v /= 255; return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; };
    const L = (d, i) => 0.2126 * lin(d[i]) + 0.7152 * lin(d[i + 1]) + 0.0722 * lin(d[i + 2]);
    const runs = [];
    const hits = document.createElement("style");
    hits.textContent = "* { pointer-events: auto !important; }";
    document.head.append(hits);
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    let node = 0;
    for (let n; (n = walker.nextNode()); node++) {
      const text = n.textContent.trim();
      const el = n.parentElement;
      if (!text || el.closest("[aria-hidden=true], .plate-mono, script, style")) continue;
      const cs = getComputedStyle(el);
      if (cs.visibility === "hidden" || cs.display === "none") continue;
      let shown = true;
      for (let a = el; a && shown; a = a.parentElement) if (getComputedStyle(a).opacity === "0") shown = false;
      if (!shown) continue;
      const r = document.createRange(); r.selectNodeContents(n);
      for (const box of r.getClientRects()) {
        if (box.width < 2 || box.height < 2) continue;
        const x0 = Math.max(0, Math.floor(box.left)), y0 = Math.max(0, Math.floor(box.top));
        const x1 = Math.min(1920, Math.ceil(box.right)), y1 = Math.min(1080, Math.ceil(box.bottom));
        const cut = box.left < -1 || box.top < -1 || box.right > 1921 || box.bottom > 1081;
        if (x1 <= x0 || y1 <= y0) { continue; }
        // A list that scrolls inside a sheet clips its last row on purpose, under a feathered
        // edge; data-scroll holds the feather height when the feather is on.
        const sc = el.closest("[data-scroll]");
        if (sc) {
          const sr = sc.getBoundingClientRect();
          const feather = getComputedStyle(sc).maskImage !== "none" ? +sc.dataset.scroll || 0 : 0;
          if (box.bottom > sr.bottom - feather - 1 || box.top < sr.top + 1) continue;
        }
        const hit = document.elementFromPoint((x0 + x1) / 2, (y0 + y1) / 2);
        if (hit && !el.contains(hit) && !hit.contains(el)) continue; // covered by another layer
        const px = [];
        for (let y = y0; y < y1; y++) for (let x = x0; x < x1; x++) {
          const i = (y * 1920 + x) * 4;
          const d = Math.abs(A[i] - B[i]) + Math.abs(A[i + 1] - B[i + 1]) + Math.abs(A[i + 2] - B[i + 2]);
          if (d > 30) px.push([d, i]);
        }
        if (px.length < 8) continue;
        px.sort((p, q) => q[0] - p[0]);
        const core = px.slice(0, Math.max(6, Math.floor(px.length * 0.25)));
        const ratios = core.map(([, i]) => { const la = L(A, i), lb = L(B, i); return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05); }).sort((p, q) => p - q);
        const p10 = ratios[Math.floor(ratios.length * 0.1)];
        runs.push({ node, text: text.slice(0, 40), size: parseFloat(cs.fontSize), ratio: +p10.toFixed(2), box: [x0, y0, x1, y1], cut, cls: el.className && String(el.className).slice(0, 40) });
      }
    }
    hits.remove();
    const overlaps = [];
    for (let i = 0; i < runs.length; i++) for (let j = i + 1; j < runs.length; j++) {
      if (runs[i].node === runs[j].node) continue;
      const p = runs[i].box, q = runs[j].box;
      const ix = Math.min(p[2], q[2]) - Math.max(p[0], q[0]), iy = Math.min(p[3], q[3]) - Math.max(p[1], q[1]);
      if (ix > 4 && iy > 6) overlaps.push([runs[i].text, runs[j].text]);
    }
    return { low: runs.filter((r) => r.ratio < 7), small: runs.filter((r) => r.size < 24), cut: runs.filter((r) => r.cut), overlaps, count: runs.length };
  }, [a, b]);
}

const browser = await chromium.launch({ executablePath: "/usr/bin/chromium", args: ["--no-sandbox", "--force-device-scale-factor=1", "--allow-file-access-from-files", "--font-render-hinting=none"] });
const page = await browser.newPage({ viewport: { width: 1920, height: 1080 }, deviceScaleFactor: 1 });
page.on("pageerror", (e) => console.log("  page error:", e.message));
fs.mkdirSync(out, { recursive: true });
const report = {};
for (const [name, file] of SHOTS) {
  if (only.length && !only.some((o) => name.includes(o))) continue;
  await page.goto("file://" + path.join(dir, file));
  try {
    await page.waitForSelector("html[data-ready='1']", { state: "attached", timeout: 8000 });
  } catch {
    console.log(name.padEnd(34), "never became ready");
    continue;
  }
  await page.waitForTimeout(120);
  const shot = path.join(out, name + ".png");
  const a = await page.screenshot({ path: shot });
  if (check) {
    const style = await page.addStyleTag({ content: HIDE_TEXT });
    const b = await page.screenshot();
    await style.evaluate((s) => s.remove());
    const r = await measure(page, "data:image/png;base64," + a.toString("base64"), "data:image/png;base64," + b.toString("base64"));
    report[name] = r;
    const flags = [r.low.length && `${r.low.length} under 7:1`, r.small.length && `${r.small.length} under 24px`, r.cut.length && `${r.cut.length} cut`, r.overlaps.length && `${r.overlaps.length} overlaps`].filter(Boolean);
    console.log(name.padEnd(34), `${r.count} runs`, flags.join(", ") || "clean");
  } else console.log(shot);
}
if (check) fs.writeFileSync(path.join(out, "report.json"), JSON.stringify(report, null, 1));
await browser.close();
