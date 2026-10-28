// Renders the OLED care group mocks to docs/mocks/oled-care-apps/*.png at 1920x1080 with the system Chromium (the shared renderer is ../render.mjs).
// Usage: node render.mjs
import { chromium } from '/data/home/lucent-ha/node_modules/playwright-core/index.mjs';
import { fileURLToPath } from 'url';
import path from 'path';
const dir = path.dirname(fileURLToPath(import.meta.url));
const SHOTS = [
  ['settings', 'index.html'],
  ['settings-light', 'index.html?theme=light'],
  ['settings-text200', 'index.html?text=200'],
  ['settings-light-text200', 'index.html?theme=light&text=200'],
  ['help', 'index.html?v=help'],
  ['help-light', 'index.html?v=help&theme=light'],
  ['help-text200', 'index.html?v=help&text=200'],
  ['help-light-text200', 'index.html?v=help&theme=light&text=200'],
];
const browser = await chromium.launch({ executablePath: '/usr/bin/chromium', args: ['--no-sandbox', '--force-device-scale-factor=1', '--allow-file-access-from-files'] });
const page = await browser.newPage({ viewport: { width: 1920, height: 1080 }, deviceScaleFactor: 1 });
const problems = [];
for (const [name, file] of SHOTS) {
  await page.goto('file://' + path.join(dir, file));
  await page.evaluate(() => document.fonts.ready);
  await page.waitForTimeout(80);
  // no text may overflow its box or leave the safe edge, at 100 % and at 200 %
  const found = await page.evaluate(() => {
    const out = [];
    document.querySelectorAll('.t, .v, .tab, .seg > span, h1, h2, .fig p, .ht, .help-sheet p').forEach(e => {
      if (!e.offsetParent) return;
      if (e.scrollWidth > e.clientWidth + 1 && getComputedStyle(e).overflow !== 'hidden') out.push(`overflow: "${e.textContent.trim().slice(0, 40)}" ${e.scrollWidth}>${e.clientWidth}`);
      const r = e.getBoundingClientRect();
      if (r.right > 1920 - 6 || r.bottom > 1080 - 6) out.push(`off the safe edge: "${e.textContent.trim().slice(0, 30)}"`);
    });
    const body = document.querySelector('.page-body');
    if (body && body.offsetParent && body.scrollHeight > body.clientHeight + 1) out.push(`rows continue below the page (scrolls on the TV) ${body.scrollHeight}>${body.clientHeight}`);
    return out;
  });
  if (found.length) problems.push(name + ': ' + [...new Set(found)].join(' | '));
  await page.screenshot({ path: path.join(dir, name + '.png'), type: 'png' });
  console.log('rendered', name);
}
await browser.close();
if (problems.length) console.log('\nCHECKS:\n' + problems.join('\n'));
