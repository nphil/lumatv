// Loads one page with the store theme's script (built with the probe part) as WAM would run it, focuses the element named, presses an arrow
// and a letter, waits for the probe's posts to go out, and prints what the page's console said about them as one JSON object. Run by tests/test_store_probe.py:
//
//   node store_probe_browser.mjs <script.js> <page.html> <selector-to-focus> [<second page.html>]
//
// A second page is opened as a navigation of the same document (location.assign), which is how the store moves between its pages.
import { chromium } from '/data/home/lucent-ha/node_modules/playwright-core/index.mjs';
import fs from 'fs';

const [scriptFile, pageFile, selector, second] = process.argv.slice(2);
const browser = await chromium.launch({ executablePath: '/usr/bin/chromium', args: ['--no-sandbox'] });
const ctx = await browser.newContext({ viewport: { width: 1920, height: 1080 } });
await ctx.addInitScript(fs.readFileSync(scriptFile, 'utf8'));
const p = await ctx.newPage();
const said = [], errors = [];
p.on('console', m => { if (/lumatv-probe/.test(m.text())) said.push(m.text().slice(0, 200)); });
p.on('pageerror', e => errors.push(e.message));
await p.goto('file://' + pageFile);
await p.waitForTimeout(2200);                       // the probe looks at the page 1.5 s after the load
if (selector) { await p.focus(selector); await p.waitForTimeout(900); await p.keyboard.press('ArrowDown'); await p.keyboard.press('q'); await p.waitForTimeout(1300); }
if (second) { await p.goto('file://' + second); await p.waitForTimeout(2200); }
await browser.close();
console.log(JSON.stringify({ said, errors }));
