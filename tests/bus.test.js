// Run with: bun test tests/
// LumaBus.js as the compositor's engine shares it between LumaHome, the overlay and the key filter (a `.pragma library`: one instance),
// wired to the real key filter the way LumaKeyBridge.qml wires them: who is asked for a key, who opens the drawer when the gear is held,
// and that a layer that stops answering is asked for nothing.
import { test, expect, beforeEach } from "bun:test";
import { readFileSync } from "node:fs";

const read = (rel) => readFileSync(new URL(rel, import.meta.url), "utf8");
const BUS_SRC = read("../payload/compositor/lumatv/LumaBus.js").replace(".pragma library", "");
const FILTER_SRC = read("../payload/keyfilter/lumatv.js").replace(/{{dataDir}}/g, "/var/lib/lumatv").replace(/{{qmlModule}}/g, "lumatv");

const KeyPolicy = { Accepted: 1, NextPolicy: 2, NotAccepted: 3 };
const RIGHT = 0x01000014, OK = 0x01000005, GEAR = 0x01000070, INPUT = 0x01000075, HOME_APP = "com.webos.app.home", APP = "com.webos.app.hdmi2";

let now, warnings, front, timers, nextTimer, lgCalls, log, Bus;

function makeBus() {
  const env = { console: { warn: (m) => warnings.push(m), info() {} }, Date: { now: () => now } };
  const names = ["setHandler", "setEscapeHandler", "setGear", "setOverlay", "setAppKey", "appKey", "setSystemLauncher", "systemLaunch", "beat", "answering", "dispatch", "overlayWants", "dispatchOverlay", "gearReady", "gearHold", "setPanels", "panelReady", "panel", "wantSettings", "takeSettings"];
  return new Function(...Object.keys(env), BUS_SRC + "\nreturn { " + names.join(", ") + " };")(...Object.values(env));
}

// The key filter in front of the real bus, its bridge a thin wrapper as LumaKeyBridge.qml is.
function makeFilter(bus) {
  const bridge = {
    dispatch: (...a) => bus.dispatch(...a), answering: () => bus.answering(), homeKey: () => false,
    dispatchOverlay: (...a) => bus.dispatchOverlay(...a), overlayWants: () => bus.overlayWants(),
    gearReady: () => bus.gearReady(), gearHold: () => bus.gearHold(),
    panelReady: (k) => bus.panelReady(k), panel: (k) => bus.panel(k),
  };
  const genericTimer = {
    set(ms, fn) { const id = ++nextTimer; timers.push({ id, at: now + ms, fn }); return id; },
    cancel(id) { timers = timers.filter((t) => t.id !== id); },
  };
  const env = {
    Qt: { Key_Super_L: 0x01000022, Key_Option: 0x01000100, createComponent: () => ({ status: 1, errorString: () => "", createObject: () => bridge }) },
    KeyPolicy, Component: { Ready: 1 }, WebOS: { Key_webOS_Settings: GEAR, Key_webOS_InputSource: INPUT },
    access: { keyAction: { launchApp: true } }, bootManager: { bootFirstUse: false }, systemPropertyService: { powerOnlyMode: false },
    global: { get activeSurfaceAppId() { return front.app; }, powerState: "active", isForegroundApp: () => false },
    fullscreenView: { get activeFocus() { return front.focus; } },
    longPressTimer: { setBlockCancel() {}, cancel() { lgCalls.push("cancel"); }, setDeviceId() {} },
    console: { info() {}, warn: (m) => warnings.push(m) },
    Date: { now: () => now },
    genericTimer,
  };
  const f = new Function(...Object.keys(env), FILTER_SRC + "\nreturn { init, handleLumaHome, handleLumaQuick };")(...Object.values(env));
  f.init();
  return f;
}

const advance = (ms) => {
  const end = now + ms;
  for (;;) {
    const due = timers.filter((t) => t.at <= end).sort((a, b) => a.at - b.at)[0];
    if (!due) break;
    now = Math.max(now, due.at);
    timers = timers.filter((t) => t !== due);
    due.fn();
  }
  now = end;
};

// LumaHome and the overlay as they register: a key handler (the saver and the AI prompt are not modelled), the gear, and the overlay's surface.
function registerHome(bus, { active = true } = {}) {
  bus.setHandler((key, pressed) => { log.push(["home", key, pressed]); return key === RIGHT; });
  bus.setGear({ ready: () => active, hold: () => { log.push(["home gear"]); return true; } });
  bus.beat();
}
function registerOverlay(bus, state) {
  bus.setOverlay({
    wants: () => state.wants, ready: () => state.ready,
    key: (key, pressed) => { log.push(["overlay", key, pressed]); return state.takes; },
    hold: () => { log.push(["overlay gear"]); return true; },
  });
}

beforeEach(() => {
  now = 5_000_000; warnings = []; front = { app: HOME_APP, focus: true }; timers = []; nextTimer = 0; lgCalls = []; log = [];
  Bus = makeBus();
});

test("nothing registered: nothing answers, and the filter leaves every key to LG", () => {
  const f = makeFilter(Bus);
  expect(Bus.answering()).toBe(false);
  expect(Bus.gearReady()).toBe(false);
  expect(Bus.gearHold()).toBe(false);
  expect(f.handleLumaHome(RIGHT, true, false, 1)).toBe(KeyPolicy.NextPolicy);
});

test("Home answers while its beat is under three seconds old, and not after", () => {
  registerHome(Bus);
  expect(Bus.answering()).toBe(true);
  now += 2999;
  expect(Bus.answering()).toBe(true);
  now += 2;
  expect(Bus.answering()).toBe(false);
  expect(Bus.gearReady()).toBe(false);
  const f = makeFilter(Bus);
  expect(f.handleLumaHome(RIGHT, true, false, 1)).toBe(KeyPolicy.NextPolicy);
  expect(log).toEqual([]);
  Bus.beat();
  expect(f.handleLumaHome(RIGHT, true, false, 1)).toBe(KeyPolicy.Accepted);
});

test("the gear's hold goes to Home while Home can show the drawer, else to the overlay", () => {
  const state = { wants: false, ready: true, takes: true };
  registerHome(Bus, { active: true });
  registerOverlay(Bus, state);
  expect(Bus.gearHold()).toBe(true);
  expect(log).toEqual([["home gear"]]);
  // an app is in front: Home's layer is hidden and says it cannot, the overlay opens its own
  log = [];
  registerHome(Bus, { active: false });
  expect(Bus.gearReady()).toBe(true);
  expect(Bus.gearHold()).toBe(true);
  expect(log).toEqual([["overlay gear"]]);
  // neither can
  state.ready = false;
  expect(Bus.gearReady()).toBe(false);
  expect(Bus.gearHold()).toBe(false);
});

test("a held gear over an app opens the overlay's drawer through the filter and the bus", () => {
  const state = { wants: false, ready: true, takes: true };
  registerHome(Bus, { active: false });
  registerOverlay(Bus, state);
  front = { app: APP, focus: true };
  const f = makeFilter(Bus);
  expect(f.handleLumaHome(GEAR, true, false, 1)).toBe(KeyPolicy.NextPolicy);
  advance(650);
  expect(log).toEqual([["overlay gear"]]);
  expect(lgCalls).toEqual(["cancel"]);
  expect(f.handleLumaHome(GEAR, false, false, 1)).toBe(KeyPolicy.Accepted);
});

test("the overlay is given keys only while it wants them and the app has them", () => {
  const state = { wants: false, ready: true, takes: true };
  registerHome(Bus, { active: false });
  registerOverlay(Bus, state);
  front = { app: APP, focus: true };
  const f = makeFilter(Bus);
  expect(f.handleLumaHome(RIGHT, true, false, 1)).toBe(KeyPolicy.NextPolicy);
  expect(f.handleLumaHome(RIGHT, false, false, 1)).toBe(KeyPolicy.NextPolicy);
  state.wants = true;
  expect(f.handleLumaHome(RIGHT, true, false, 1)).toBe(KeyPolicy.Accepted);
  expect(f.handleLumaHome(RIGHT, false, false, 1)).toBe(KeyPolicy.Accepted);
  expect(log).toEqual([["overlay", RIGHT, true], ["overlay", RIGHT, false]]);
  // the surface closes before the release: the release still ends at the overlay (the press went there)
  log = [];
  expect(f.handleLumaHome(OK, true, false, 1)).toBe(KeyPolicy.Accepted);
  state.wants = false;
  expect(f.handleLumaHome(OK, false, false, 1)).toBe(KeyPolicy.Accepted);
  expect(log).toEqual([["overlay", OK, true], ["overlay", OK, false]]);
});

test("an overlay that wants keys but whose layer has stopped answering is given none", () => {
  const state = { wants: true, ready: true, takes: true };
  registerHome(Bus, { active: false });
  registerOverlay(Bus, state);
  front = { app: APP, focus: true };
  const f = makeFilter(Bus);
  expect(Bus.overlayWants()).toBe(true);
  now += 3500;
  expect(Bus.overlayWants()).toBe(false);
  expect(f.handleLumaHome(RIGHT, true, false, 1)).toBe(KeyPolicy.NextPolicy);
  expect(log).toEqual([]);
});

test("an overlay that throws answers no and logs once", () => {
  registerHome(Bus, { active: false });
  Bus.setOverlay({ wants: () => true, ready: () => true, key: () => { throw new Error("overlay broke"); }, hold: () => { throw new Error("no drawer"); } });
  front = { app: APP, focus: true };
  const f = makeFilter(Bus);
  expect(f.handleLumaHome(RIGHT, true, false, 1)).toBe(KeyPolicy.NextPolicy);
  expect(f.handleLumaHome(RIGHT, true, false, 1)).toBe(KeyPolicy.NextPolicy);
  expect(Bus.gearHold()).toBe(false);
  expect(warnings.length).toBe(1);
});

test("a handler that is taken away (the layer was destroyed) is not asked", () => {
  const state = { wants: true, ready: true, takes: true };
  registerHome(Bus, { active: false });
  registerOverlay(Bus, state);
  Bus.setOverlay(null);
  Bus.setGear(null);
  expect(Bus.overlayWants()).toBe(false);
  expect(Bus.gearReady()).toBe(false);
  expect(Bus.gearHold()).toBe(false);
  expect(Bus.dispatchOverlay(RIGHT, true, false, 1)).toBe(false);
});

test("the remote's app keys are LumaHome's to decide while the layer answers, and LG's when it does not, has no handler or throws", () => {
  expect(Bus.appKey("netflix", "x")).toBe(false);                       // nothing registered
  Bus.setAppKey((id, front) => { log.push(["appKey", id, front]); return id === "netflix"; });
  expect(Bus.appKey("netflix", "x")).toBe(false);                       // registered, but Home has not beaten: not answering
  registerHome(Bus);
  expect(Bus.appKey("netflix", APP)).toBe(true);
  expect(Bus.appKey("amazon", APP)).toBe(false);                        // LumaHome left it to LG
  expect(log).toEqual([["appKey", "netflix", APP], ["appKey", "amazon", APP]]);
  now += 3500;
  expect(Bus.appKey("netflix", APP)).toBe(false);                       // stopped answering
  Bus.beat();
  Bus.setAppKey(() => { throw new Error("no answer"); });
  expect(Bus.appKey("netflix", APP)).toBe(false);
  expect(warnings.length).toBe(1);
  Bus.setAppKey(null);
  expect(Bus.appKey("netflix", APP)).toBe(false);
});

test("a system window is asked for through the launcher the key filter registered, and false says there is none", () => {
  expect(Bus.systemLaunch("com.webos.app.quicksettings")).toBe(false);          // nothing registered: the caller falls back
  Bus.setSystemLauncher((appId, params) => { log.push(["launch", appId, params]); return true; });
  expect(Bus.systemLaunch("com.webos.app.quicksettings")).toBe(true);
  expect(Bus.systemLaunch("com.webos.app.quickinputpicker", { a: 1 })).toBe(true);
  expect(log).toEqual([["launch", "com.webos.app.quicksettings", undefined], ["launch", "com.webos.app.quickinputpicker", { a: 1 }]]);
  // it does not depend on Home answering: the launcher is the filter's, which is up for as long as the compositor is
  now += 60_000;
  expect(Bus.systemLaunch("com.webos.app.quicksettings")).toBe(true);
  Bus.setSystemLauncher(() => false);
  expect(Bus.systemLaunch("x")).toBe(false);
  Bus.setSystemLauncher(() => { throw new Error("no client"); });
  expect(Bus.systemLaunch("x")).toBe(false);
  expect(warnings.length).toBe(1);
  Bus.setSystemLauncher(null);
  expect(Bus.systemLaunch("x")).toBe(false);
});

// ---- the quick panel and the input picker (the filter's second entry)

function registerPanels(bus, state) {
  bus.setPanels({
    ready: (k) => state.home, open: (k) => { log.push(["home panel", k]); return state.opens; },
  });
}
function registerOverlayPanels(bus, state) {
  bus.setOverlay({
    wants: () => false, ready: () => true, key: () => false, hold: () => true,
    panelReady: (k) => state.ready, panel: (k) => { log.push(["overlay panel", k]); return state.opens; },
  });
}

test("a panel is shown by Home while Home can, else by the overlay, else by nobody", () => {
  const home = { home: true, opens: true }, over = { ready: true, opens: true };
  registerHome(Bus);
  registerPanels(Bus, home);
  registerOverlayPanels(Bus, over);
  expect(Bus.panelReady("quick")).toBe(true);
  expect(Bus.panel("quick")).toBe(true);
  expect(log).toEqual([["home panel", "quick"]]);
  // an app is in front: Home's layer says it cannot, the overlay shows the same panel
  log = [];
  home.home = false;
  expect(Bus.panelReady("picker")).toBe(true);
  expect(Bus.panel("picker")).toBe(true);
  expect(log).toEqual([["overlay panel", "picker"]]);
  // neither can: the key is LG's
  over.ready = false;
  expect(Bus.panelReady("quick")).toBe(false);
  expect(Bus.panel("quick")).toBe(false);
  // a panel that is asked and does nothing is not an answer
  home.home = true; home.opens = false;
  expect(Bus.panel("quick")).toBe(false);
});

test("a panel is not asked for while the layer is not answering, and a registry that throws answers no and logs once", () => {
  const home = { home: true, opens: true };
  registerHome(Bus);
  registerPanels(Bus, home);
  now += 3001;
  expect(Bus.panelReady("quick")).toBe(false);
  Bus.beat();
  Bus.setPanels({ ready: () => { throw new Error("boom"); }, open: () => true });
  expect(Bus.panelReady("quick")).toBe(false);
  expect(Bus.panel("quick")).toBe(false);
  expect(warnings.length).toBe(1);
  Bus.setPanels(null);
  expect(Bus.panelReady("quick")).toBe(false);
});

test("an overlay registered before panels existed is not asked for them", () => {
  registerHome(Bus);
  Bus.setOverlay({ wants: () => false, ready: () => true, key: () => false, hold: () => true });
  expect(Bus.panelReady("quick")).toBe(false);
  expect(Bus.panel("quick")).toBe(false);
  expect(warnings).toEqual([]);
});

test("a gear tap and the Input key reach the panels through the real filter and the bus", () => {
  const home = { home: true, opens: true }, over = { ready: true, opens: true };
  registerHome(Bus);
  registerPanels(Bus, home);
  registerOverlayPanels(Bus, over);
  const f = makeFilter(Bus);
  // LG's replay of a tap: the first entry lets it through, the late one takes it
  const tap = () => [true, false].map((p) => { const a = f.handleLumaHome(GEAR, p, false, 1); return a === KeyPolicy.NextPolicy ? f.handleLumaQuick(GEAR, p, false, 1) : a; });
  expect(tap()).toEqual([KeyPolicy.Accepted, KeyPolicy.Accepted]);
  expect(log.filter((l) => l[0].endsWith("panel"))).toEqual([["home panel", "quick"]]);
  log = [];
  front = { app: APP, focus: true };
  home.home = false;
  expect(tap()).toEqual([KeyPolicy.Accepted, KeyPolicy.Accepted]);
  expect(f.handleLumaQuick(INPUT, true, false, 1)).toBe(KeyPolicy.Accepted);
  expect(log.filter((l) => l[0].endsWith("panel"))).toEqual([["overlay panel", "quick"], ["overlay panel", "picker"]]);
  // LG's own window is on top of the app: the late entry has no key to take
  log = [];
  front = { app: APP, focus: false };
  expect(tap()).toEqual([KeyPolicy.NextPolicy, KeyPolicy.NextPolicy]);
  expect(log).toEqual([]);
});

test("a Settings page asked for over an app waits for Home, once, and is dropped after fifteen seconds", () => {
  expect(Bus.takeSettings()).toBe(null);
  Bus.wantSettings("network");
  expect(Bus.takeSettings()).toBe("network");
  expect(Bus.takeSettings()).toBe(null);
  Bus.wantSettings("");
  expect(Bus.takeSettings()).toBe("");
  Bus.wantSettings("receiver");
  now += 15_001;
  expect(Bus.takeSettings()).toBe(null);
});
