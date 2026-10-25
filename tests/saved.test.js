// Run with: bun test tests/
// Saved.js is the QML side of the saved-first cache: the policy numbers are the native helper's
// (helper/feeds) and the documents are the helper's, which it saves in {{dataDir}}/cache and serves
// on its API. QML no longer fetches or builds either document.
import { test, expect } from "bun:test";
import { readFileSync } from "node:fs";

const read = (p) => readFileSync(new URL(p, import.meta.url), "utf8");
const Saved = new Function(read("../payload/compositor/lumatv/Saved.js").replace(/^\.pragma library$/m, "")
  .replace(/{{dataDir}}/g, "/var/lib/lumatv") + "\nreturn { SCHEMA, REFRESH_MS, EXPIRE_MS, freshness, parse, weatherUsable, newsUsable };")();

const T0 = 1_792_900_000_000;

test("freshness boundaries are the helper's", () => {
  expect(Saved.freshness(T0, T0)).toBe("fresh");
  expect(Saved.freshness(T0, T0 + 899_999)).toBe("fresh");
  expect(Saved.freshness(T0, T0 + 900_000)).toBe("stale");
  expect(Saved.freshness(T0, T0 + 21_599_999)).toBe("stale");
  expect(Saved.freshness(T0, T0 + 21_600_000)).toBe("expired");
});

test("a stamp up to 5 minutes ahead is age 0, further ahead is expired", () => {
  expect(Saved.freshness(T0 + 1, T0)).toBe("fresh");
  expect(Saved.freshness(T0 + 300_000, T0)).toBe("fresh");
  expect(Saved.freshness(T0 + 300_001, T0)).toBe("expired");
});

test("parse refuses what cannot be shown", () => {
  expect(Saved.parse("")).toBeNull();
  expect(Saved.parse("not json")).toBeNull();
  expect(Saved.parse("[]")).toBeNull();
  expect(Saved.parse(JSON.stringify({ schema: 2, fetchedAt: 1 }))).toBeNull();
  expect(Saved.parse(JSON.stringify({ schema: 1 }))).toBeNull();
  expect(Saved.parse("x".repeat(1_048_577))).toBeNull();
  expect(Saved.parse(JSON.stringify({ schema: 1, fetchedAt: 5 }))).toEqual({ schema: 1, fetchedAt: 5 });
});

test("a saved weather reading must have its strings and must not be expired", () => {
  const doc = { schema: 1, fetchedAt: T0, display: { place: "Atlanta", temp: "54\u00b0", days: [] } };
  expect(Saved.weatherUsable(doc, T0 + 3_600_000)).toBe(true);
  expect(Saved.weatherUsable(doc, T0 + 21_600_000)).toBe(false);
  expect(Saved.weatherUsable(null, T0)).toBe(false);
  expect(Saved.weatherUsable({ ...doc, display: undefined }, T0)).toBe(false);
  expect(Saved.weatherUsable({ ...doc, display: { temp: 54, days: [] } }, T0)).toBe(false);
  expect(Saved.weatherUsable({ ...doc, display: { temp: "54", days: "x" } }, T0)).toBe(false);
});

test("saved stories need at least one item and must not be expired", () => {
  const doc = { schema: 1, fetchedAt: T0, items: [{ title: "A" }] };
  expect(Saved.newsUsable(doc, T0 + 1000)).toBe(true);
  expect(Saved.newsUsable(doc, T0 + 21_600_000)).toBe(false);
  expect(Saved.newsUsable({ ...doc, items: [] }, T0)).toBe(false);
  expect(Saved.newsUsable({ ...doc, items: null }, T0)).toBe(false);
  expect(Saved.newsUsable(null, T0)).toBe(false);
});

test("the policy numbers are the helper's", () => {
  expect([Saved.REFRESH_MS, Saved.EXPIRE_MS]).toEqual([900_000, 21_600_000]);
});
