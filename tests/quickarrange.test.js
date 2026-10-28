// Run with: bun test tests/
// QuickArrange.js is the quick panel's arrangement: which tiles show and in what order (docs/mocks/29-qp-edit, 29-qp-add).
import { test, expect } from "bun:test";
import { readFileSync } from "node:fs";

const src = readFileSync(new URL("../payload/compositor/lumatv/panels/QuickArrange.js", import.meta.url), "utf8").replace(/^\.pragma library$/m, "");
const Q = new Function(src + "\nreturn { shown, missing, without, added, moved, same, patch };")();

const ALL = [
  { id: "picture", dflt: true }, { id: "receiver", dflt: true }, { id: "sleep", dflt: true }, { id: "wifi", dflt: true },
  { id: "plugin.ambient.ambient", dflt: true }, { id: "ai", dflt: true },
  { id: "screenoff", dflt: false }, { id: "energy", dflt: false }, { id: "oled", dflt: false },
];

test("no arrangement shows the default tiles in the catalogue's order", () => {
  expect(Q.shown(undefined, ALL)).toEqual(["picture", "receiver", "sleep", "wifi", "plugin.ambient.ambient", "ai"]);
  expect(Q.shown(null, ALL)).toEqual(Q.shown(undefined, ALL));
});

test("a saved arrangement is shown as saved, without ids nothing answers to and without repeats", () => {
  expect(Q.shown(["sleep", "picture", "gone", "sleep", "oled"], ALL)).toEqual(["sleep", "picture", "oled"]);
  expect(Q.shown([], ALL)).toEqual([]);
});

test("Add a tile offers what is not shown, in the catalogue's order", () => {
  expect(Q.missing(["picture", "ai"], ALL)).toEqual(["receiver", "sleep", "wifi", "plugin.ambient.ambient", "screenoff", "energy", "oled"]);
  expect(Q.missing(Q.shown(undefined, ALL), ALL)).toEqual(["screenoff", "energy", "oled"]);
});

test("remove, add and move leave the input alone and return the new order", () => {
  const o = ["picture", "receiver", "sleep"];
  expect(Q.without(o, "receiver")).toEqual(["picture", "sleep"]);
  expect(Q.added(o, "oled")).toEqual(["picture", "receiver", "sleep", "oled"]);
  expect(Q.added(o, "sleep")).toEqual(o);
  expect(Q.moved(o, "sleep", -1)).toEqual(["picture", "sleep", "receiver"]);
  expect(Q.moved(o, "picture", 1)).toEqual(["receiver", "picture", "sleep"]);
  expect(o).toEqual(["picture", "receiver", "sleep"]);
});

test("a move past either end changes nothing", () => {
  const o = ["a", "b", "c"];
  expect(Q.moved(o, "a", -1)).toEqual(o);
  expect(Q.moved(o, "c", 1)).toEqual(o);
  expect(Q.moved(o, "zz", 1)).toEqual(o);
});

test("same compares order, and the patch carries the whole list", () => {
  expect(Q.same(["a", "b"], ["a", "b"])).toBe(true);
  expect(Q.same(["a", "b"], ["b", "a"])).toBe(false);
  expect(Q.same(["a"], ["a", "b"])).toBe(false);
  const p = Q.patch(["a", "b"]);
  expect(p).toEqual({ quickPanel: { tiles: ["a", "b"] } });
});
