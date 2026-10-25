// Run with: bun test tests/
// Loads payload/keyfilter/lumatv.js the way the compositor does (an include into a scope that
// holds the key-filter globals) with stand-ins for those globals, and checks what the filter
// answers to a stream of key events.
import { test, expect, beforeEach } from "bun:test";
import { readFileSync } from "node:fs";

const SRC = readFileSync(new URL("../payload/keyfilter/lumatv.js", import.meta.url), "utf8")
  .replace(/{{dataDir}}/g, "/var/lib/lumatv")
  .replace(/{{qmlModule}}/g, "lumatv");

const KeyPolicy = { Accepted: 1, NextPolicy: 2, NotAccepted: 3 };
const OK = 0x01000005, RIGHT = 0x01000014, HOME = 0x01000022;
const GEAR = 0x01000070, OPTION = 0x01000100;
const NETFLIX = 0x01000071, AMAZON = 0x01000072, DISNEY = 0x01000073, LGCH = 0x01000074;
const WEBOS = { Key_webOS_Settings: GEAR, Key_webOS_Netflix: NETFLIX, Key_webOS_Amazon: AMAZON, Key_webOS_Disneyplus: DISNEY, Key_webOS_LGChannels: LGCH };

let calls, front, bridgeAnswers, warnings, now, answering;
// The gear and the overlay: what the bridge says, the timers the filter asked LG's `genericTimer` for (advanced by hand) and what it did to LG's long press.
let gearReady, gearOpens, overlayWanted, overlayTakes, timers, nextTimer, lgCalls, timerBroken;
// The remote's app keys: whether the TV is on and what LumaHome says to a key
let power, appKeyTakes;

function advance(ms) {
  const end = now + ms;
  for (;;) {
    const due = timers.filter((t) => t.at <= end).sort((a, b) => a.at - b.at)[0];
    if (!due) break;
    now = Math.max(now, due.at);
    timers = timers.filter((t) => t !== due);
    due.fn();
  }
  now = end;
}

function makeFilter({ bridge = undefined, webos = WEBOS } = {}) {
  const Qt = {
    Key_Super_L: HOME,
    Key_Option: OPTION,
    createComponent: () => ({ status: 1, errorString: () => "", createObject: () => theBridge }),
  };
  const theBridge = bridge === undefined ? {
    dispatch(key, pressed, autoRepeat) { calls.push(["dispatch", key, pressed, autoRepeat]); return bridgeAnswers(key, pressed, autoRepeat); },
    homeKey(pressed, autoRepeat) { calls.push(["home", pressed, autoRepeat]); return false; },
    answering() { return answering; },
    overlayWants() { return overlayWanted; },
    dispatchOverlay(key, pressed, autoRepeat) { calls.push(["overlay", key, pressed, autoRepeat]); return overlayTakes(key, pressed, autoRepeat); },
    gearReady() { return gearReady; },
    gearHold() { calls.push(["gearHold"]); return gearOpens; },
    appKey(id, frontApp) { calls.push(["appKey", id, frontApp]); return appKeyTakes(id, frontApp); },
  } : bridge;
  const genericTimer = {
    set(ms, fn) { if (timerBroken) return -1; const id = ++nextTimer; timers.push({ id, at: now + ms, fn }); return id; },
    cancel(id) { timers = timers.filter((t) => t.id !== id); },
    isRunning(id) { return timers.some((t) => t.id === id); },
  };
  const env = {
    Qt, KeyPolicy, Component: { Ready: 1 },
    global: { get activeSurfaceAppId() { return front.app; }, get powerState() { return power; } },
    fullscreenView: { get activeFocus() { return front.focus; } },
    longPressTimer: { setBlockCancel(v) { lgCalls.push(["block", v]); }, cancel() { lgCalls.push(["cancel"]); }, setDeviceId(d) { lgCalls.push(["device", d]); } },
    console: { info() {}, warn: (m) => warnings.push(m) },
    Date: { now: () => now },
    genericTimer,
  };
  if (webos) env.WebOS = webos;
  const f = new Function(...Object.keys(env), SRC + "\nreturn { init, handleLumaHome };");
  const filter = f(...Object.values(env));
  filter.init();
  return filter;
}

beforeEach(() => {
  calls = [];
  warnings = [];
  now = 1_000_000;
  front = { app: "com.webos.app.home", focus: true };
  bridgeAnswers = () => true;
  answering = true;
  gearReady = true;
  gearOpens = true;
  overlayWanted = false;
  overlayTakes = () => true;
  timers = [];
  nextTimer = 0;
  lgCalls = [];
  timerBroken = false;
  power = "active";
  appKeyTakes = () => true;
});

test("a press LumaHome takes ends there, and so does its release", () => {
  const f = makeFilter();
  expect(f.handleLumaHome(RIGHT, true, false, 1)).toBe(KeyPolicy.Accepted);
  expect(f.handleLumaHome(RIGHT, false, false, 1)).toBe(KeyPolicy.Accepted);
  expect(calls).toEqual([["dispatch", RIGHT, true, false], ["dispatch", RIGHT, false, false]]);
});

test("a key LumaHome declines goes to LG, press, repeat and release alike", () => {
  const f = makeFilter();
  bridgeAnswers = () => false;
  expect(f.handleLumaHome(OK, true, false, 1)).toBe(KeyPolicy.NextPolicy);
  expect(f.handleLumaHome(OK, true, true, 1)).toBe(KeyPolicy.NextPolicy);
  expect(f.handleLumaHome(OK, false, false, 1)).toBe(KeyPolicy.NextPolicy);
  expect(calls).toEqual([["dispatch", OK, true, false]]);
});

test("a layer that is not answering is not given keys: they all go to LG", () => {
  const f = makeFilter();
  answering = false;
  expect(f.handleLumaHome(RIGHT, true, false, 1)).toBe(KeyPolicy.NextPolicy);
  expect(f.handleLumaHome(RIGHT, false, false, 1)).toBe(KeyPolicy.NextPolicy);
  expect(calls).toEqual([]);
  // it answers again: the next press is taken
  answering = true;
  expect(f.handleLumaHome(RIGHT, true, false, 1)).toBe(KeyPolicy.Accepted);
});

test("OK pressed on Home and released after an app opened still ends at LumaHome", () => {
  const f = makeFilter();
  expect(f.handleLumaHome(OK, true, false, 1)).toBe(KeyPolicy.Accepted);
  front = { app: "netflix", focus: false };
  expect(f.handleLumaHome(OK, false, false, 1)).toBe(KeyPolicy.Accepted);
  // the release was handed to LumaHome (which acts on it), not to the app
  expect(calls.at(-1)).toEqual(["dispatch", OK, false, false]);
});

test("a release whose press went to the app is not taken, even with Home in front by then", () => {
  const f = makeFilter();
  front = { app: "netflix", focus: false };
  expect(f.handleLumaHome(OK, true, false, 1)).toBe(KeyPolicy.NextPolicy);
  front = { app: "com.webos.app.home", focus: true };
  expect(f.handleLumaHome(OK, false, false, 1)).toBe(KeyPolicy.NextPolicy);
  expect(calls).toEqual([]);
});

test("repeats of a taken key are LumaHome's; repeats of a key LG has are LG's", () => {
  const f = makeFilter();
  expect(f.handleLumaHome(RIGHT, true, false, 1)).toBe(KeyPolicy.Accepted);
  expect(f.handleLumaHome(RIGHT, true, true, 1)).toBe(KeyPolicy.Accepted);
  expect(f.handleLumaHome(RIGHT, true, true, 1)).toBe(KeyPolicy.Accepted);
  expect(f.handleLumaHome(RIGHT, false, false, 1)).toBe(KeyPolicy.Accepted);
  expect(calls.filter((c) => c[3] === true).length).toBe(2);
  calls = [];
  expect(f.handleLumaHome(OK, true, true, 1)).toBe(KeyPolicy.NextPolicy);
  expect(calls).toEqual([]);
});

test("a lost release never turns the next release of an app's key into ours", () => {
  const f = makeFilter();
  expect(f.handleLumaHome(OK, true, false, 1)).toBe(KeyPolicy.Accepted);
  // the release never arrives; Home is left, the key is pressed again over an app
  front = { app: "netflix", focus: false };
  expect(f.handleLumaHome(OK, true, false, 1)).toBe(KeyPolicy.NextPolicy);
  expect(f.handleLumaHome(OK, false, false, 1)).toBe(KeyPolicy.NextPolicy);
});

test("an error in the filter or the handler passes the key to LG untouched", () => {
  const f = makeFilter();
  bridgeAnswers = () => { throw new Error("handler broke"); };
  expect(f.handleLumaHome(RIGHT, true, false, 1)).toBe(KeyPolicy.NextPolicy);
  // the press was never taken, so its release is LG's too
  bridgeAnswers = () => true;
  expect(f.handleLumaHome(RIGHT, false, false, 1)).toBe(KeyPolicy.NextPolicy);
  expect(warnings.length).toBe(1);
});

test("an error on the release of a taken key still ends the pair at LumaHome's side", () => {
  const f = makeFilter();
  expect(f.handleLumaHome(OK, true, false, 1)).toBe(KeyPolicy.Accepted);
  bridgeAnswers = () => { throw new Error("handler broke on release"); };
  // nothing can be said to LG about a press it never saw: the error path answers Next
  expect(f.handleLumaHome(OK, false, false, 1)).toBe(KeyPolicy.NextPolicy);
  // and the record is gone, so the next release is judged on its own press
  bridgeAnswers = () => true;
  expect(f.handleLumaHome(OK, false, false, 1)).toBe(KeyPolicy.NextPolicy);
});

test("losing the surface globals mid-stream fails open", () => {
  const f = makeFilter();
  const env = front;
  Object.defineProperty(env, "app", { get() { throw new TypeError("global is gone"); } });
  expect(f.handleLumaHome(RIGHT, true, false, 1)).toBe(KeyPolicy.NextPolicy);
});

test("no bridge: every key is LG's", () => {
  const f = makeFilter({ bridge: null });
  expect(f.handleLumaHome(RIGHT, true, false, 1)).toBe(KeyPolicy.NextPolicy);
  expect(f.handleLumaHome(HOME, true, false, 1)).toBe(KeyPolicy.NextPolicy);
});

test("Home is only watched: a short press reaches LG, the filter swallows only a held one", () => {
  let swallow = false;
  const f = makeFilter({ bridge: { homeKey() { return swallow; }, dispatch() { return true; } } });
  expect(f.handleLumaHome(HOME, true, false, 1)).toBe(KeyPolicy.NextPolicy);
  swallow = true;
  expect(f.handleLumaHome(HOME, false, false, 1)).toBe(KeyPolicy.Accepted);
});

test("an exception logs once per five seconds", () => {
  const f = makeFilter();
  bridgeAnswers = () => { throw new Error("again"); };
  for (let i = 0; i < 20; i++) { now += 100; f.handleLumaHome(RIGHT, true, false, 1); }
  expect(warnings.length).toBe(1);
  now += 5000;
  f.handleLumaHome(RIGHT, true, false, 1);
  expect(warnings.length).toBe(2);
});

// ---- the gear key: a tap is LG's, a hold of 650 ms is the notification drawer, LG's own long press (Settings at 1 s) never runs over it

const N = KeyPolicy.NextPolicy, A = KeyPolicy.Accepted;
const down = (f, key, repeat = false) => f.handleLumaHome(key, true, repeat, 1);
const up = (f, key) => f.handleLumaHome(key, false, false, 1);
const held = () => calls.filter((c) => c[0] === "gearHold").length;
const stockHoldStopped = () => JSON.stringify(lgCalls) === JSON.stringify([["block", false], ["cancel"], ["device", -1]]);

test("a tap of the gear is LG's from press to release: the filter only watches it", () => {
  bridgeAnswers = () => false;
  const f = makeFilter();
  expect(down(f, GEAR)).toBe(N);
  advance(400);
  expect(up(f, GEAR)).toBe(N);
  advance(10_000);
  expect(held()).toBe(0);
  expect(lgCalls).toEqual([]);
});

test("three taps stay three events for LG's AV reset, the hold's timer never survives a release", () => {
  bridgeAnswers = () => false;
  const f = makeFilter();
  for (let i = 0; i < 3; i++) {
    expect(down(f, GEAR)).toBe(N);
    advance(200);
    expect(up(f, GEAR)).toBe(N);
    advance(300);
  }
  advance(10_000);
  expect(held()).toBe(0);
  expect(lgCalls).toEqual([]);
  expect(timers).toEqual([]);
});

test("held 650 ms the gear opens the drawer, LG's long press is stopped and the rest of the hold ends at the filter", () => {
  bridgeAnswers = () => false;
  const f = makeFilter();
  expect(down(f, GEAR)).toBe(N);                 // LG's chain sees the press, as for a tap
  advance(649);
  expect(held()).toBe(0);
  advance(1);
  expect(held()).toBe(1);
  expect(stockHoldStopped()).toBe(true);
  // the repeats of the hold and its release are the filter's: LG would otherwise replay the key and open Quick Settings
  expect(down(f, GEAR, true)).toBe(A);
  expect(down(f, GEAR, true)).toBe(A);
  expect(up(f, GEAR)).toBe(A);
  // nothing is left running, and the next tap is an ordinary one
  expect(timers).toEqual([]);
  expect(down(f, GEAR)).toBe(N);
  expect(up(f, GEAR)).toBe(N);
  expect(held()).toBe(1);
});

test("a hold is taken once however long it lasts", () => {
  bridgeAnswers = () => false;
  const f = makeFilter();
  down(f, GEAR);
  advance(650);
  for (let i = 0; i < 30; i++) { advance(100); expect(down(f, GEAR, true)).toBe(A); }
  expect(held()).toBe(1);
  expect(up(f, GEAR)).toBe(A);
});

test("when nothing of ours can show the drawer the hold is LG's: its timer is left to run", () => {
  bridgeAnswers = () => false;
  gearReady = false;                             // the setting is off, or the layer cannot show it
  const f = makeFilter();
  expect(down(f, GEAR)).toBe(N);
  advance(2000);
  expect(held()).toBe(0);
  expect(lgCalls).toEqual([]);
  expect(up(f, GEAR)).toBe(N);
});

test("a drawer that does not open at the hold leaves the hold to LG", () => {
  bridgeAnswers = () => false;
  gearOpens = false;
  const f = makeFilter();
  down(f, GEAR);
  advance(650);
  expect(held()).toBe(1);                        // asked once
  expect(lgCalls).toEqual([]);
  expect(down(f, GEAR, true)).toBe(N);
  expect(up(f, GEAR)).toBe(N);
});

test("a layer that is not answering is not asked: the hold is LG's", () => {
  bridgeAnswers = () => false;
  answering = false;
  const f = makeFilter();
  expect(down(f, GEAR)).toBe(N);
  advance(2000);
  expect(held()).toBe(0);
  expect(lgCalls).toEqual([]);
  expect(up(f, GEAR)).toBe(N);
});

test("an LG overlay, popup or alert on top of the app has the focus and its gear is LG's", () => {
  bridgeAnswers = () => false;
  front = { app: "netflix", focus: false };
  const f = makeFilter();
  down(f, GEAR);
  advance(2000);
  expect(held()).toBe(0);
  expect(lgCalls).toEqual([]);
});

test("over an app with the focus the hold is the overlay's drawer", () => {
  bridgeAnswers = () => false;
  front = { app: "com.webos.app.hdmi2", focus: true };
  const f = makeFilter();
  down(f, GEAR);
  advance(650);
  expect(held()).toBe(1);
  expect(stockHoldStopped()).toBe(true);
  expect(up(f, GEAR)).toBe(A);
});

test("the saver takes the gear like any key: the press and its release are Home's and no hold starts", () => {
  bridgeAnswers = (key) => key === GEAR;         // LumaHome's saver eats it
  const f = makeFilter();
  expect(down(f, GEAR)).toBe(A);
  advance(2000);
  expect(up(f, GEAR)).toBe(A);
  expect(held()).toBe(0);
  expect(timers).toEqual([]);
});

test("a release that never comes does not swallow the gear for good", () => {
  bridgeAnswers = () => false;
  const f = makeFilter();
  down(f, GEAR);
  advance(650);
  expect(held()).toBe(1);
  advance(4001);                                 // the events stopped: the watch clears the hold
  lgCalls = [];
  expect(down(f, GEAR)).toBe(N);
  advance(100);
  expect(up(f, GEAR)).toBe(N);
  expect(lgCalls).toEqual([]);
});

test("a new press supersedes a hold whose release was lost", () => {
  bridgeAnswers = () => false;
  const f = makeFilter();
  down(f, GEAR);
  advance(650);
  expect(down(f, GEAR)).toBe(N);                 // a new press, not a repeat
  advance(100);
  expect(up(f, GEAR)).toBe(N);
  expect(held()).toBe(1);
});

test("a timer service that fails leaves the gear to LG, and the 1 s Key_Option is then the hold", () => {
  bridgeAnswers = () => false;
  timerBroken = true;
  const f = makeFilter();
  expect(down(f, GEAR)).toBe(N);
  advance(650);
  expect(held()).toBe(0);
  // LG's own long press runs: at 1 s it injects Key_Option
  expect(down(f, OPTION)).toBe(A);
  expect(held()).toBe(1);
  // LG must see the gear's release to end its own hold, and its Key_Option release is the filter's
  expect(up(f, GEAR)).toBe(N);
  expect(up(f, OPTION)).toBe(A);
  expect(up(f, OPTION)).toBe(N);
});

test("a hold that LG's timer outran (its cancel failed) still never opens LG's Settings over the drawer", () => {
  bridgeAnswers = () => false;
  const f = makeFilter();
  down(f, GEAR);
  advance(650);
  expect(held()).toBe(1);
  expect(down(f, OPTION)).toBe(A);               // LG's timer was not stopped: the conversion arrives anyway
  expect(held()).toBe(1);                        // the drawer is not asked twice
  expect(up(f, GEAR)).toBe(N);                   // LG finishes its own hold on it
  expect(up(f, OPTION)).toBe(A);
});

test("an Option key that is not LG's conversion of a held gear is not ours", () => {
  bridgeAnswers = () => false;
  const f = makeFilter();
  expect(down(f, OPTION)).toBe(N);
  expect(up(f, OPTION)).toBe(N);
  expect(held()).toBe(0);
});

test("without the gear key's code in scope the gear is LG's", () => {
  bridgeAnswers = () => false;
  const f = makeFilter({ webos: null });
  expect(warnings.length).toBe(1);
  expect(down(f, GEAR)).toBe(N);
  advance(2000);
  expect(held()).toBe(0);
});

test("an error while the hold is taken is logged once and the hold stays LG's", () => {
  const broken = { answering() { return true; }, gearReady() { return true; }, gearHold() { throw new Error("no drawer"); }, dispatch() { return false; }, homeKey() { return false; }, overlayWants() { return false; } };
  const f = makeFilter({ bridge: broken });
  expect(down(f, GEAR)).toBe(N);
  advance(650);
  expect(warnings.length).toBe(1);
  expect(lgCalls).toEqual([]);                   // LG's timer was not touched: its hold runs as it always did
  expect(up(f, GEAR)).toBe(N);
});

// ---- the overlay over an app takes keys only while it has a surface that wants them

test("over an app with a banner that has buttons the overlay takes the D-pad: press, repeat and release", () => {
  front = { app: "com.webos.app.hdmi2", focus: true };
  overlayWanted = true;
  const f = makeFilter();
  expect(down(f, RIGHT)).toBe(A);
  expect(down(f, RIGHT, true)).toBe(A);
  expect(up(f, RIGHT)).toBe(A);
  expect(calls).toEqual([["overlay", RIGHT, true, false], ["overlay", RIGHT, true, true], ["overlay", RIGHT, false, false]]);
});

test("an overlay with nothing that takes keys leaves every key to the app", () => {
  front = { app: "com.webos.app.hdmi2", focus: true };
  overlayWanted = false;
  const f = makeFilter();
  expect(down(f, RIGHT)).toBe(N);
  expect(up(f, RIGHT)).toBe(N);
  expect(calls).toEqual([]);
});

test("a key the overlay declines goes to the app, and so do its repeats and release", () => {
  front = { app: "com.webos.app.hdmi2", focus: true };
  overlayWanted = true;
  overlayTakes = () => false;
  const f = makeFilter();
  expect(down(f, OK)).toBe(N);
  expect(down(f, OK, true)).toBe(N);
  expect(up(f, OK)).toBe(N);
  expect(calls).toEqual([["overlay", OK, true, false]]);
});

test("an LG overlay or alert over the app has the focus: the overlay is not given its keys", () => {
  front = { app: "com.webos.app.hdmi2", focus: false };
  overlayWanted = true;
  const f = makeFilter();
  expect(down(f, RIGHT)).toBe(N);
  expect(calls).toEqual([]);
});

test("a release goes to the surface that took its press, whichever is in front by then", () => {
  front = { app: "com.webos.app.hdmi2", focus: true };
  overlayWanted = true;
  const f = makeFilter();
  expect(down(f, OK)).toBe(A);
  front = { app: "com.webos.app.home", focus: true };    // an input picked from the banner: Home arrives before the key is up
  expect(up(f, OK)).toBe(A);
  expect(calls).toEqual([["overlay", OK, true, false], ["overlay", OK, false, false]]);
  // and the other way round: a press Home took is Home's to the end
  calls = [];
  expect(down(f, RIGHT)).toBe(A);
  front = { app: "com.webos.app.hdmi2", focus: true };
  expect(up(f, RIGHT)).toBe(A);
  expect(calls).toEqual([["dispatch", RIGHT, true, false], ["dispatch", RIGHT, false, false]]);
});

test("Home in front is LumaHome's even if the overlay still says it wants keys", () => {
  front = { app: "com.webos.app.home", focus: true };
  overlayWanted = true;
  const f = makeFilter();
  expect(down(f, RIGHT)).toBe(A);
  expect(calls).toEqual([["dispatch", RIGHT, true, false]]);
});

test("an overlay that stops answering is not given keys", () => {
  front = { app: "com.webos.app.hdmi2", focus: true };
  overlayWanted = false;                          // the bus says so when the layer is not answering
  answering = false;
  const f = makeFilter();
  expect(down(f, RIGHT)).toBe(N);
  expect(calls).toEqual([]);
});

test("an exception in the overlay passes the key to the app", () => {
  front = { app: "com.webos.app.hdmi2", focus: true };
  overlayWanted = true;
  overlayTakes = () => { throw new Error("overlay broke"); };
  const f = makeFilter();
  expect(down(f, RIGHT)).toBe(N);
  expect(up(f, RIGHT)).toBe(N);
  expect(warnings.length).toBe(1);
});

// ---- the remote's app keys: watched in any foreground, taken when LumaHome says so, LG's when it does not

const asked = () => calls.filter((c) => c[0] === "appKey");

test("an app key LumaHome takes ends here, press, repeat and release, over an app", () => {
  front = { app: "com.webos.app.hdmi2", focus: true };
  const f = makeFilter();
  expect(down(f, NETFLIX)).toBe(A);
  expect(down(f, NETFLIX, true)).toBe(A);
  expect(up(f, NETFLIX)).toBe(A);
  expect(asked()).toEqual([["appKey", "netflix", "com.webos.app.hdmi2"]]);
  // the pair is over: the next press is decided afresh
  expect(down(f, NETFLIX)).toBe(A);
  expect(asked().length).toBe(2);
});

test("an app key LumaHome leaves to LG (its default, with the app installed) is LG's from press to release", () => {
  appKeyTakes = () => false;
  front = { app: "netflix", focus: true };
  const f = makeFilter();
  expect(down(f, NETFLIX)).toBe(N);
  expect(down(f, NETFLIX, true)).toBe(N);
  expect(up(f, NETFLIX)).toBe(N);
  expect(asked()).toEqual([["appKey", "netflix", "netflix"]]);
});

test("each of the four app keys is asked for by its own name", () => {
  bridgeAnswers = () => false;                   // Home is in front and its key handler has no use for these keys
  const f = makeFilter();
  for (const k of [NETFLIX, AMAZON, DISNEY, LGCH]) { down(f, k); up(f, k); }
  expect(asked().map((c) => c[1])).toEqual(["netflix", "amazon", "disneyplus", "lgchannels"]);
});

test("on Home the key goes to LumaHome's key handler first and is an app key only if it declines", () => {
  bridgeAnswers = () => false;
  const f = makeFilter();
  expect(down(f, AMAZON)).toBe(A);
  expect(calls).toEqual([["dispatch", AMAZON, true, false], ["appKey", "amazon", "com.webos.app.home"]]);
});

test("the saver wakes the TV on an app key like on any key, and the key is not an app key", () => {
  bridgeAnswers = () => true;                    // LumaHome's saver eats every key
  const f = makeFilter();
  expect(down(f, NETFLIX)).toBe(A);
  expect(up(f, NETFLIX)).toBe(A);
  expect(asked()).toEqual([]);
});

test("with the TV off or on its saver the key is LG's: it powers the TV on and opens the app itself", () => {
  bridgeAnswers = () => false;
  const f = makeFilter();
  for (const state of ["active standby", "screen saver", "screen down", "always on display", ""]) {
    power = state;
    expect(down(f, NETFLIX)).toBe(N);
    expect(up(f, NETFLIX)).toBe(N);
  }
  expect(asked()).toEqual([]);
});

test("an LG overlay, popup, keyboard or alert over the app keeps the key: it is not the fullscreen view's", () => {
  front = { app: "netflix", focus: false };
  const f = makeFilter();
  expect(down(f, NETFLIX)).toBe(N);
  expect(asked()).toEqual([]);
});

test("a layer that is not answering is not asked: LG's own target, auto-install and all, is the fallback", () => {
  answering = false;
  const f = makeFilter();
  expect(down(f, NETFLIX)).toBe(N);
  expect(up(f, NETFLIX)).toBe(N);
  expect(asked()).toEqual([]);
});

test("a release that never came does not leave the next press taken or left", () => {
  bridgeAnswers = () => false;
  const f = makeFilter();
  expect(down(f, NETFLIX)).toBe(A);
  appKeyTakes = () => false;                      // the owner changed it meanwhile: LG's default now
  expect(down(f, NETFLIX)).toBe(N);
  expect(up(f, NETFLIX)).toBe(N);
});

test("an overlay that wants keys does not take an app key, which is not the D-pad", () => {
  front = { app: "com.webos.app.hdmi2", focus: true };
  overlayWanted = true;
  overlayTakes = () => false;
  const f = makeFilter();
  expect(down(f, NETFLIX)).toBe(A);
  expect(calls).toEqual([["overlay", NETFLIX, true, false], ["appKey", "netflix", "com.webos.app.hdmi2"]]);
});

test("an app key whose code is not in scope stays LG's", () => {
  bridgeAnswers = () => false;
  const f = makeFilter({ webos: { Key_webOS_Settings: GEAR, Key_webOS_Netflix: NETFLIX } });
  expect(down(f, AMAZON)).toBe(N);
  expect(down(f, NETFLIX)).toBe(A);
  expect(asked().map((c) => c[1])).toEqual(["netflix"]);
});

test("an error in LumaHome's answer leaves the key to LG and logs once", () => {
  bridgeAnswers = () => false;
  appKeyTakes = () => { throw new Error("no answer"); };
  const f = makeFilter();
  expect(down(f, NETFLIX)).toBe(N);
  expect(up(f, NETFLIX)).toBe(N);
  expect(warnings.length).toBe(1);
});
