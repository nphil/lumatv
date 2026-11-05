// The launcher app (index.html, launch.js, app.js) run as the TV runs it, with the page's DOM and the Homebrew Channel's root service played
// by stand-ins, and printed as JSON for tests/test_launcher_app.py. Scenarios: the app opened after an escape (LumaTV is not the compositor), opened
// while LumaTV runs, a start that fails (and the way to LG's Home from it), a recovery that is still running, and the row that opens Home.
'use strict';
const fs = require('fs');
const path = require('path');
const vm = require('vm');

const root = path.resolve(__dirname, '..');
const brand = JSON.parse(fs.readFileSync(path.join(root, 'brand.json'), 'utf8'));

// tools/build-ipk.sh renders the placeholders the same way before the files are packed.
function source(name) {
  return fs.readFileSync(path.join(root, name), 'utf8')
    .replace(/{{displayName}}/g, brand.displayName).replace(/{{shortId}}/g, brand.shortId)
    .replace(/{{appId}}/g, brand.appId).replace(/{{dataDir}}/g, brand.dataDir).replace(/{{qmlModule}}/g, brand.qmlModule);
}

const TOOL = '/media/developer/apps/usr/palm/applications/' + brand.appId + '/tools/' + brand.shortId + ' ';

function page(tv) {
  const calls = [];
  const elements = {};
  const handlers = { document: {}, window: {} };
  const shownAtCall = [];

  function element(id) {
    if (elements[id]) return elements[id];
    const listeners = {};
    const el = {
      id, textContent: '', innerHTML: '', value: '', dataset: {}, tabIndex: 0, scrollTop: 0, hidden: id.startsWith('overlay-'), attrs: {},
      classList: { contains: () => true, add() {}, remove() {} },
      setAttribute(k, v) { this.attrs[k] = v; if (k === 'hidden') this.hidden = true; },
      removeAttribute(k) { delete this.attrs[k]; if (k === 'hidden') this.hidden = false; },
      addEventListener(type, fn) { (listeners[type] = listeners[type] || []).push(fn); },
      async click() { for (const fn of listeners.click || []) await fn(); },
      focus() {}, querySelector: () => element('inner'), querySelectorAll: () => [], appendChild() {},
    };
    return (elements[id] = el);
  }

  // The root service's exec: what the tool answers to each command the page runs.
  function bridge(command) {
    const cmd = command.startsWith(TOOL) ? command.slice(TOOL.length) : command;
    calls.push(cmd);
    const answer = (obj) => ({ returnValue: true, stdoutString: JSON.stringify(obj) });
    if (cmd === 'status') {
      return answer({ ok: true, version: '0.4.0', firmware: '10.2.1', model: 'tv', standby: 'system', ntfy: { enabled: false }, compositor: { live: tv.live, flag: 'persistent' } });
    }
    if (cmd === 'compositor status') return answer({ ok: true, compositor: { live: tv.live } });
    if (cmd === 'compositor on') {
      shownAtCall.push(!element('overlay-apply-busy').hidden);
      const result = tv.on.shift();
      if (result === 'ok') { tv.live = true; return answer({ ok: true, compositor: {} }); }
      return answer({ ok: false, error: result });
    }
    return answer({ returnValue: true });
  }

  const sandbox = {
    console, setTimeout, clearTimeout, JSON, Promise,
    URLSearchParams,
    document: {
      readyState: 'interactive', // a deferred script runs here, before DOMContentLoaded
      visibilityState: 'visible',
      getElementById: element,
      querySelectorAll: () => [],
      addEventListener: (type, fn) => { (handlers.document[type] = handlers.document[type] || []).push(fn); },
    },
    window: null,
  };
  sandbox.window = sandbox;
  sandbox.window.location = { search: '' };
  sandbox.window.addEventListener = (type, fn) => { (handlers.window[type] = handlers.window[type] || []).push(fn); };
  sandbox.window.webOS = {
    service: {
      request(uri, options) {
        setImmediate(() => options.onSuccess(bridge(options.parameters.command)));
      },
    },
  };
  vm.createContext(sandbox);
  vm.runInContext(source('launch.js'), sandbox, { filename: 'launch.js' });
  vm.runInContext(source('app.js'), sandbox, { filename: 'app.js' });

  return {
    calls, elements, shownAtCall, tv,
    // the document finishes loading: the page's own start, which has run once already from the deferred script
    async loaded() {
      await new Promise((resolve) => setTimeout(resolve, 30));
      for (const fn of handlers.document.DOMContentLoaded || []) await fn();
      await new Promise((resolve) => setTimeout(resolve, 30));
    },
  };
}

function count(calls, what) {
  return calls.filter((c) => c === what).length;
}

async function main() {
  const out = {};

  // after an escape: LG's Home is up, LumaTV's compositor is not the compositor, and the owner opens the app from LG's launcher
  let p = page({ live: false, on: ['ok'] });
  await p.loaded();
  out.afterEscape = {
    calls: p.calls.slice(), on: count(p.calls, 'compositor on'), progressShownWhenStarted: p.shownAtCall, home: p.calls.filter((c) => c.includes('launch')),
    failureShown: !p.elements['overlay-enter-failed'].hidden, progressShownAtEnd: !p.elements['overlay-apply-busy'].hidden,
  };

  // opened while LumaTV runs: nothing is started
  p = page({ live: true, on: [] });
  await p.loaded();
  out.whileLive = { calls: p.calls.slice(), on: count(p.calls, 'compositor on'), home: p.calls.filter((c) => c.includes('launch')) };

  // the row that says Open: a running LumaTV is shown by opening its Home, and nothing is started
  await p.elements['row-apply'].click();
  out.rowWhileLive = { on: count(p.calls, 'compositor on'), homeKeys: p.calls.filter((c) => c.includes('launchDefaultApp')).length };

  // a start that fails: the reason is on the screen, with a way to LG's Home and one to try again; a second start that works ends it
  p = page({ live: false, on: ['the supervisor would not start: firmware: not a verified build', 'ok'] });
  await p.loaded();
  const failed = p.elements['overlay-enter-failed'];
  out.failure = { shown: !failed.hidden, reason: p.elements['enter-failed-reason'].textContent, on: count(p.calls, 'compositor on'), progressShown: !p.elements['overlay-apply-busy'].hidden };
  await p.elements['enter-lg'].click();
  out.failure.lgHome = p.calls.filter((c) => c.includes('applicationManager/launch '));
  await p.elements['enter-retry'].click();
  out.failure.afterRetry = { on: count(p.calls, 'compositor on'), failureShown: !failed.hidden, live: p.tv.live };


  // the decision itself, on a clock that sleeps instantly: a recovery that is still running is waited for, a failure is not retried
  const LumaLaunch = require(path.join(root, 'launch.js'));
  function run(on, live) {
    let clock = 0;
    const log = { cli: [], sleeps: [], progress: [] };
    const results = on.slice();
    const deps = {
      async cli(verb, arg) {
        log.cli.push(verb + ' ' + arg);
        if (verb === 'compositor' && arg === 'status') return { ok: true, compositor: { live: live.now } };
        const result = results.length ? results.shift() : 'ok';
        if (result !== 'ok') {
          if (live.cutOff) live.now = true;
          throw new Error(result);
        }
        live.now = true;
        return { ok: true };
      },
      async exec() { return { stdoutString: JSON.stringify({ returnValue: true }) }; },
      async sleep(ms) { log.sleeps.push(ms); clock += ms; },
      now: () => clock,
    };
    return LumaLaunch.enter(deps, (phase, reason) => log.progress.push(phase + (reason ? ': ' + reason : ''))).then((outcome) => Object.assign(log, { outcome }));
  }
  const BUSY = "a recovery of LG's Home is running; try again when it is done";
  const recovering = await run([BUSY, BUSY], { now: false });
  const never = await run(Array(100).fill(BUSY), { now: false });
  const refused = await run(['the supervisor would not start: firmware: not a verified build'], { now: false });
  const mounted = await run(['the compositor is already mounted'], { now: false });
  const cutOff = await run(['no answer: the restart cut it off'], { now: false, cutOff: true });
  out.decision = {
    recovering: { outcome: recovering.outcome, sleeps: recovering.sleeps, on: recovering.cli.filter((c) => c === 'compositor on').length, progress: recovering.progress },
    never: { outcome: never.outcome, on: never.cli.filter((c) => c === 'compositor on').length, waited: never.sleeps.reduce((a, b) => a + b, 0), lastCall: never.cli[never.cli.length - 1] },
    refused: { outcome: refused.outcome, cli: refused.cli, sleeps: refused.sleeps },
    mounted: mounted.outcome,
    cutOff: cutOff.outcome,
    busy: [BUSY, 'another mutation is still running', 'the compositor is already mounted'].map((m) => LumaLaunch.BUSY.test(m)),
  };
  const lg = [];
  await LumaLaunch.openLgHome({ exec: async (c) => { lg.push(c); return { stdoutString: JSON.stringify({ returnValue: true }) }; } });
  let refusedLg = null;
  await LumaLaunch.openLgHome({ exec: async () => ({ stdoutString: JSON.stringify({ returnValue: false, errorText: 'no way' }) }) }).catch((e) => { refusedLg = e.message; });
  out.openLgHome = { commands: lg, refused: refusedLg };

  console.log(JSON.stringify(out));
}

main().then(() => process.exit(0), (err) => { console.error(err); process.exit(1); });
