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

let calls, front, bridgeAnswers, warnings, now, answering;

function makeFilter({ bridge = undefined } = {}) {
  const Qt = {
    Key_Super_L: HOME,
    createComponent: () => ({ status: 1, errorString: () => "", createObject: () => theBridge }),
  };
  const theBridge = bridge === undefined ? {
    dispatch(key, pressed, autoRepeat) { calls.push(["dispatch", key, pressed, autoRepeat]); return bridgeAnswers(key, pressed, autoRepeat); },
    homeKey(pressed, autoRepeat) { calls.push(["home", pressed, autoRepeat]); return false; },
    answering() { return answering; },
  } : bridge;
  const env = {
    Qt, KeyPolicy, Component: { Ready: 1 },
    global: { get activeSurfaceAppId() { return front.app; } },
    fullscreenView: { get activeFocus() { return front.focus; } },
    longPressTimer: { setBlockCancel() {}, cancel() {} },
    console: { info() {}, warn: (m) => warnings.push(m) },
    Date: { now: () => now },
  };
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
