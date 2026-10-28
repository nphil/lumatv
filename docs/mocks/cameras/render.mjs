// Renders the camera mocks to docs/mocks/cameras/*.png at 1920x1080 with the system Chromium (the shared renderer is ../render.mjs).
// Usage: node render.mjs
import { chromium } from '/data/home/lucent-ha/node_modules/playwright-core/index.mjs';
import { fileURLToPath } from 'url';
import path from 'path';
const dir = path.dirname(fileURLToPath(import.meta.url));
const SHOTS = [
  ['popup', 'index.html'],
  ['popup-light', 'index.html?theme=light'],
  ['popup-text200', 'index.html?text=200'],
  ['popup-light-text200', 'index.html?theme=light&text=200'],
  ['doorbell', 'index.html?v=doorbell'],
  ['full', 'index.html?v=full'],
  ['full-light', 'index.html?v=full&theme=light'],
  ['full-text200', 'index.html?v=full&text=200'],
  ['grid', 'index.html?v=grid'],
  ['grid-light', 'index.html?v=grid&theme=light'],
  ['grid-text200', 'index.html?v=grid&text=200'],
];
const browser = await chromium.launch({ executablePath: '/usr/bin/chromium', args: ['--no-sandbox', '--force-device-scale-factor=1', '--allow-file-access-from-files'] });
const page = await browser.newPage({ viewport: { width: 1920, height: 1080 }, deviceScaleFactor: 1 });
const problems = [];
for (const [name, file] of SHOTS) {
  await page.goto('file://' + path.join(dir, file));
  await page.evaluate(() => document.fonts.ready);
  await page.waitForTimeout(80);
  // no text may overflow its box or be clipped, at 100 % and at 200 %
  const found = await page.evaluate(() => {
    const out = [];
    document.querySelectorAll('.st:not([style*="display: none"]) .name, .what, .ago, .foot span, .pill span, .label, .wait, .counter, .hint').forEach(e => {
      if (!e.offsetParent) return;
      if (e.scrollWidth > e.clientWidth + 1 && getComputedStyle(e).overflow !== 'hidden') out.push(`overflow: "${e.textContent.trim().slice(0, 40)}" ${e.scrollWidth}>${e.clientWidth}`);
      const r = e.getBoundingClientRect();
      if (r.right > 1920 - 6 || r.bottom > 1080 - 6) out.push(`off the safe edge: "${e.textContent.trim().slice(0, 30)}"`);
    });
    return out;
  });
  if (found.length) problems.push(name + ': ' + [...new Set(found)].join(' | '));
  await page.screenshot({ path: path.join(dir, name + '.png'), type: 'png' });
  console.log('rendered', name);
}
await browser.close();
if (problems.length) { console.log('\nCHECKS:\n' + problems.join('\n')); process.exitCode = 1; }
