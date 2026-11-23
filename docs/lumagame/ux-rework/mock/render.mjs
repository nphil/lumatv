// Renders the UX rework option mocks at 1920x1080, device scale 1.
// Usage: OUT=<dir> node render.mjs [--check] [name ...]   (default OUT: ./shots, ignored by git)
// --check measures each text run on the picture, as ../../mock/render.mjs does: contrast against
// the pixels behind it (7:1), text overlapping text, and text cut by the screen.
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
  ["home-a-spotlight", "home-a.html"],
  ["home-a-spotlight-down", "home-a.html?view=hub"],
  ["home-a-spotlight-light", "home-a.html?theme=light"],
  ["home-b-board", "home-b.html"],
  ["home-c-quiet-row", "home-c.html"],
  ["home-quick-menu", "home-a.html?menu=1"],
  ["library-a-bar", "library-a.html"],
  ["library-a-bar-search", "library-a.html?search=1"],
  ["library-b-panel", "library-b.html"],
  ["library-c-list", "library-c.html"],
  ["game-a-feed", "game-a.html"],
  ["game-a-feed-text200", "game-a.html?text=200"],
  ["game-a-feed-community", "game-a.html?view=community"],
  ["game-b-split", "game-b.html"],
  ["game-b-split-text200", "game-b.html?text=200"],
  ["game-b-split-light", "game-b.html?theme=light"],
  ["game-b-split-community", "game-b.html?view=community"],
  ["game-b-split-media", "game-b.html?view=media"],
  ["game-b-split-reader", "game-b.html?view=reader"],
  ["game-b-split-where", "game-b.html?view=where"],
  ["game-b-split-marks", "game-b.html?view=community&marks=1"],
  ["game-c-glance", "game-c.html"],
  ["game-c-glance-text200", "game-c.html?text=200"],
  ["game-c-glance-community", "game-c.html?view=community"],
  ["game-viewer", "viewer.html"],
  ["game-viewer-chrome-hidden", "viewer.html?chrome=0"],
  ["steam-a-accounts-page", "steam-a.html"],
  ["steam-a-accounts-page-qr", "steam-a.html?state=qr"],
  ["steam-b-in-place", "steam-b.html"],
  ["steam-b-in-place-qr", "steam-b.html?state=qr"],
  ["steam-b-in-place-stale", "steam-b.html?state=stale"],
  ["steam-c-menu", "steam-c.html"],
  ["steam-c-menu-qr", "steam-c.html?state=qr"],
  ["settings-a-rail", "settings-a.html"],
  ["settings-b-tiles", "settings-b.html"],
  ["settings-c-quick", "settings-c.html"],
  ["launch-a-stage", "launch-a.html"],
  ["launch-a-stage-menu", "launch-a.html?view=menu"],
  ["launch-b-drawer", "launch-b.html"],
  ["launch-b-drawer-menu", "launch-b.html?view=menu"],
  ["launch-b-drawer-end", "launch-b.html?view=end"],
  ["launch-c-strip", "launch-c.html"],
  ["launch-c-strip-menu", "launch-c.html?view=menu"],
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
      for (let p = el; p && shown; p = p.parentElement) if (getComputedStyle(p).opacity === "0") shown = false;
      if (!shown) continue;
      const r = document.createRange(); r.selectNodeContents(n);
      for (const box of r.getClientRects()) {
        if (box.width < 2 || box.height < 2) continue;
        const x0 = Math.max(0, Math.floor(box.left)), y0 = Math.max(0, Math.floor(box.top));
        const x1 = Math.min(1920, Math.ceil(box.right)), y1 = Math.min(1080, Math.ceil(box.bottom));
        if (x1 <= x0 || y1 <= y0) continue;
        // Content that runs on below or beside the screen on purpose (a row that scrolls, a list
        // that pages) is marked data-runs; its cut lines are not failures.
        const runsOn = el.closest("[data-runs]");
        const cut = !runsOn && (box.left < -1 || box.top < -1 || box.right > 1921 || box.bottom > 1081);
        if (runsOn) {
          const rr = runsOn.getBoundingClientRect();
          if (box.bottom > Math.min(rr.bottom, 1080) - 1 || box.right > Math.min(rr.right, 1920) - 1) continue;
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
        runs.push({ node, text: text.slice(0, 40), size: parseFloat(cs.fontSize), ratio: +p10.toFixed(2), box: [x0, y0, x1, y1], cut });
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
  if (!fs.existsSync(path.join(dir, file.split("?")[0]))) { console.log(name.padEnd(34), "no file yet"); continue; }
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
