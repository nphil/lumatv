// Renders every mock to docs/mocks/shots/*.png at 1920×1080 with the system Chromium.
// Usage: node render.mjs [name ...]   (names from SHOTS below; none = all)
import { chromium } from '/data/home/lucent-ha/node_modules/playwright-core/index.mjs';
import { fileURLToPath } from 'url';
import path from 'path';

const dir = path.dirname(fileURLToPath(import.meta.url));
const SHOTS = [
  ['01-settings-picture', '01-settings-picture.html'],
  ['01-settings-picture-light', '01-settings-picture.html?theme=light'],
  ['02-settings-sound', '02-settings-sound.html'],
  ['03-settings-remote', '03-settings-remote.html'],
  ['04-settings-ai-access', '04-settings-ai-access.html'],
  ['04-settings-ai-access-light', '04-settings-ai-access.html?theme=light'],
  ['05-ai-request-prompt', '05-ai-request-prompt.html'],
  ['06-confirm-dialog', '06-confirm-dialog.html'],
  ['07-toast', '07-toast.html'],
  ['08-input-picker', '08-input-picker.html'],
  ['09-search-keyboard', '09-search-keyboard.html'],
  ['10-quick-panel', '10-quick-panel.html'],
  ['10-quick-panel-light', '10-quick-panel.html?theme=light'],
  ['10-quick-panel-video', '10-quick-panel.html?v=video'],
  ['11-volume-osd', '11-volume-osd.html'],
  ['11-volume-osd-light', '11-volume-osd.html?theme=light'],
  ['11-volume-osd-db', '11-volume-osd.html?v=db'],
  ['11-volume-osd-muted', '11-volume-osd.html?v=muted'],
  ['11-volume-osd-muted-light', '11-volume-osd.html?v=muted&theme=light'],
  ['11-volume-osd-mark', '11-volume-osd.html?v=mark'],
  ['11-volume-osd-mark-light', '11-volume-osd.html?v=mark&theme=light'],
  ['11-volume-osd-tv', '11-volume-osd.html?v=tv'],
  ['11-volume-osd-arc', '11-volume-osd.html?v=arc'],
  ['11-volume-osd-offline', '11-volume-osd.html?v=offline'],
  ['12-all-apps', '12-all-apps.html'],
  ['12-all-apps-light', '12-all-apps.html?theme=light'],
  ['12-all-apps-menu', '12-all-apps.html?v=menu'],
  ['13-notifications', '13-notifications.html'],
  ['14-recents', '14-recents.html'],
  ['15-launch-splash', '15-launch-splash.html'],
  ['15-long-press-ring', '15-launch-splash.html?v=ring'],
  ['15-spinner', '15-launch-splash.html?v=spinner'],
  ['16-context-menu', '16-context-menu.html'],
  ['17-power-menu', '17-power-menu.html'],
  ['17-power-countdown', '17-power-menu.html?v=countdown'],
  ['18-no-signal', '18-no-signal.html'],
  ['18-device-connected', '18-no-signal.html?v=connected'],
  ['19-pin-prompt', '19-pin-prompt.html'],
  ['20-channel-banner', '20-channel-banner.html'],
  ['21-screensaver-clock', '21-screensaver.html'],
  ['21-screensaver-art', '21-screensaver.html?v=art'],
  ['22-bluetooth-pair', '22-bluetooth-update.html'],
  ['22-update-prompt', '22-bluetooth-update.html?v=update'],
  ['23-boot', '23-boot.html'],
  ['24-settings-receiver', '24-settings-receiver.html'],
  ['24-settings-receiver-levels', '24-settings-receiver.html?tab=levels'],
  ['24-settings-receiver-audyssey', '24-settings-receiver.html?tab=audyssey'],
  ['24-settings-receiver-zones', '24-settings-receiver.html?tab=zones'],
  ['25-input-pill', '25-input-banner.html'],
  ['25-input-banner', '25-input-banner.html?v=ok'],
  ['25-input-info', '25-input-banner.html?v=info'],
  ['25-input-update', '25-input-banner.html?v=update'],
  ['26-power-off-logo', '26-power-off.html'],
  ['26-startup-cover', '26-power-off.html?v=cover'],
  ['27-standby-clock', '27-standby.html'],
  ['27-standby-art', '27-standby.html?v=art'],
  ['27-standby-off', '27-standby.html?v=off'],
  ['28-voice-listening', '28-voice.html'],
  ['28-voice-heard', '28-voice.html?v=heard'],
  ['28-voice-results', '28-voice.html?v=results'],
  ['28-mic-indicator', '28-voice.html?v=mic'],
  ['29-qp-slider', '29-quick-panel-popups.html'],
  ['29-qp-picker', '29-quick-panel-popups.html?v=picker'],
  ['29-qp-edit', '29-quick-panel-popups.html?v=edit'],
  ['29-qp-add', '29-quick-panel-popups.html?v=add'],
  ['30-settings-inputs', '30-settings-inputs.html'],
  ['30-settings-network', '30-settings-network.html'],
  ['30-settings-ambient', '30-settings-ambient.html'],
  ['30-settings-home', '30-settings-home.html'],
  ['30-settings-system', '30-settings-system.html'],
  ['31-quick-access-toast', '31-quick-access.html'],
  ['31-quick-access-assign', '31-quick-access.html?v=assign'],
  ['32-pairing-prompt', '32-pairing-prompt.html'],
  ['33-pointer-menu', '33-pointer-menu.html'],
  ['34-game-dashboard', '34-game-dashboard.html'],
  ['35-app-update-required', '35-app-prompts.html'],
  ['35-app-not-available', '35-app-prompts.html?v=unavailable'],
  ['35-toast-advanced', '35-app-prompts.html?v=toast'],
];
const only = process.argv.slice(2);
const browser = await chromium.launch({ executablePath: '/usr/bin/chromium', args: ['--no-sandbox', '--force-device-scale-factor=1'] });
const page = await browser.newPage({ viewport: { width: 1920, height: 1080 }, deviceScaleFactor: 1 });
const problems = [];
for (const [name, file] of SHOTS) {
  if (only.length && !only.some(o => name.startsWith(o))) continue;
  await page.goto('file://' + path.join(dir, file));
  await page.evaluate(() => document.fonts.ready);
  await page.waitForTimeout(80);
  const overflow = await page.evaluate(() => {
    const out = [];
    document.querySelectorAll('.t,.d,.v,.name,.line,.mt,.ms,.tab,.seg>span,.btn,.chip,.sub,h1,p,.k,.a,.q,.e .what,.n,.s,.mi,.sv,.meta,.prog,.lbl,.state,.port,.hint,.keyhints span,.fact .v,.stat .v,.q,.reply,.g').forEach(e => {
      if (e.scrollWidth > e.clientWidth + 1 && getComputedStyle(e).overflow !== 'hidden') out.push(`overflow: "${e.textContent.trim().slice(0, 40)}" ${e.scrollWidth}>${e.clientWidth}`);
      const fs = parseFloat(getComputedStyle(e).fontSize);
      if (e.textContent.trim() && fs < 21) out.push(`small ${fs}px: "${e.textContent.trim().slice(0, 30)}"`);
    });
    const page = document.querySelector('.page-body, .dialog, .sheet, .qpanel, .drawer, .lib, .menu, .qpop, .ibanner, .gdash, .vbar');
    if (page && page.scrollHeight > page.clientHeight + 1) out.push(`page overflows vertically ${page.scrollHeight}>${page.clientHeight}`);
    return out;
  });
  if (overflow.length) problems.push(name + ': ' + [...new Set(overflow)].join(' | '));
  await page.screenshot({ path: path.join(dir, 'shots', name + '.png'), type: 'png' });
  console.log('rendered', name);
}
await browser.close();
if (problems.length) { console.log('\nCHECKS:\n' + problems.join('\n')); }
