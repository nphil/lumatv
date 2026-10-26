// Run with: bun test tests/
// ArtDeck.js is the art saver's shuffle bag kept across restarts: what is saved (ids, not positions), what is dealt back after a restart (the
// rest of the round, less the prints whose file is gone, with the prints that have come since dealt in), and what a file that is not ours counts as.
import { test, expect } from "bun:test";
import { readFileSync } from "node:fs";

const read = (p) => readFileSync(new URL(p, import.meta.url), "utf8");
const Deck = new Function(read("../payload/compositor/lumatv/ArtDeck.js").replace(/^\.pragma library$/m, "")
  + "\nreturn { MAX_DECKS, MAX_IDS, parse, collectionKey, draw, restore, deckOf, put, stringify };")();

const pics = (...ids) => ids.map((id) => ({ id, file: id + ".jpg" }));
const SIX = pics("a", "b", "c", "d", "e", "f");
const always = () => true;
// a repeatable stand-in for Math.random
const lcg = (seed) => () => (seed = (seed * 1664525 + 1013904223) % 4294967296) / 4294967296;
const saved = (key, deck, at = 1) => ({ schema: 1, decks: { [key]: { at, known: SIX.map((p) => p.id), last: "", bag: [], ...deck } } });

test("anything that is not ours reads as an empty deck", () => {
  const none = { schema: 1, decks: {} };
  for (const text of ["", null, undefined, "not json", "[]", "7", "null", '{"schema":2,"decks":{}}', '{"schema":1}', '{"schema":1,"decks":[]}', '{"schema":1,"decks":"x"}'])
    expect(Deck.parse(text)).toEqual(none);
  expect(Deck.parse(" ".repeat(300000) + '{"schema":1,"decks":{"x":{"at":1,"bag":[]}}}')).toEqual(none);
});

test("a deck that is not a deck is dropped and the others kept", () => {
  const doc = Deck.parse(JSON.stringify({ schema: 1, decks: {
    good: { at: 5, bag: ["a"], known: ["a"], last: "a" }, nobag: { at: 1 }, noat: { bag: [] }, str: "x", nul: null, num: 4 } }));
  expect(Object.keys(doc.decks)).toEqual(["good"]);
});

test("a key named __proto__ does not reach the prototype", () => {
  const doc = Deck.parse('{"schema":1,"decks":{"__proto__":{"at":1,"bag":[],"polluted":true},"x":{"at":1,"bag":[]}}}');
  expect(Object.keys(doc.decks)).toEqual(["x"]);
  expect(({}).polluted).toBeUndefined();
  expect(Object.getPrototypeOf(doc.decks)).toBe(Object.prototype);
});

test("the collection is the folder of its index, below the module's own directory when it is inside it", () => {
  const base = "file:///var/lib/lumatv/qml/WebOSCompositor/lumatv/";
  expect(Deck.collectionKey(base + "art/index.json", base)).toBe("art/");
  expect(Deck.collectionKey(base + "index.json", base)).toBe("./");
  expect(Deck.collectionKey("file:///media/usb/prints/index.json", base)).toBe("file:///media/usb/prints/");
  expect(Deck.collectionKey("file:///media/usb/prints/index.json", "")).toBe("file:///media/usb/prints/");
  expect(Deck.collectionKey("", base)).toBe("./");
});

test("a round is every print once, in an order the random numbers decide", () => {
  for (const seed of [1, 2, 3, 99]) {
    const b = Deck.draw(6, [], lcg(seed));
    expect([...b].sort()).toEqual([0, 1, 2, 3, 4, 5]);
  }
  expect(Deck.draw(6, [], lcg(5))).toEqual(Deck.draw(6, [], lcg(5)));
  expect(Deck.draw(6, [], lcg(5))).not.toEqual(Deck.draw(6, [], lcg(6)));
  expect(Deck.draw(1, [0], lcg(1))).toEqual([0]);
  expect(Deck.draw(0, [], lcg(1))).toEqual([]);
});

test("a new round does not begin with a print that was just shown", () => {
  for (let seed = 1; seed < 400; seed++) {
    const b = Deck.draw(6, [2, 4], lcg(seed));
    expect([...b].sort()).toEqual([0, 1, 2, 3, 4, 5]);
    expect([2, 4]).not.toContain(b[0]);
  }
  // when every print is to be avoided there is nothing better to do than deal them
  expect(Deck.draw(2, [0, 1], lcg(1)).length).toBe(2);
  // the first print is every other print's chance too: none is favoured
  const first = [0, 0, 0, 0, 0, 0];
  const r = lcg(7);
  for (let i = 0; i < 6000; i++) first[Deck.draw(6, [3], r)[0]]++;
  expect(first[3]).toBe(0);
  for (const i of [0, 1, 2, 4, 5]) expect(Math.abs(first[i] - 1200)).toBeLessThan(150);
});

test("the rest of the round comes back after a restart, in its order", () => {
  const doc = saved("art/", { bag: ["d", "b", "f"], last: "c" });
  expect(Deck.restore(doc, "art/", SIX, always, lcg(1))).toEqual({ bag: [3, 1, 5], last: 2 });
});

test("another collection, or none, has no bag to give back", () => {
  const doc = saved("art/", { bag: ["d", "b"], last: "c" });
  expect(Deck.restore(doc, "file:///usb/", SIX, always, lcg(1))).toEqual({ bag: [], last: -1 });
  expect(Deck.restore(Deck.parse(""), "art/", SIX, always, lcg(1))).toEqual({ bag: [], last: -1 });
});

test("a print that is no longer in the index is dropped", () => {
  const doc = saved("art/", { bag: ["d", "gone", "b", "f"], last: "gone-too" });
  expect(Deck.restore(doc, "art/", SIX, always, lcg(1))).toEqual({ bag: [3, 1, 5], last: -1 });
});

test("a print whose file is gone is dropped, and one that is in the bag only once stays once", () => {
  const doc = saved("art/", { bag: ["d", "b", "d", "f", "b"] });
  const there = (p) => p.id !== "f";
  expect(Deck.restore(doc, "art/", SIX, there, lcg(1)).bag).toEqual([3, 1]);
});

test("a print that has come since is dealt in; one that was shown this round is not", () => {
  const later = pics("a", "b", "c", "d", "e", "f", "g", "h");
  const doc = saved("art/", { bag: ["d", "b"], last: "c" });     // a, c, e, f were shown; d and b are to come; g and h are new
  const got = Deck.restore(doc, "art/", later, always, lcg(3));
  expect([...got.bag].sort()).toEqual([1, 3, 6, 7]);
  expect(got.bag.filter((i) => i === 1 || i === 3)).toEqual([3, 1]);        // the saved order is kept among themselves
  expect(got.bag).not.toContain(0);
  expect(got.bag).not.toContain(2);
  expect(got.last).toBe(2);
});

test("a new print whose file is not there is not dealt in", () => {
  const later = pics("a", "b", "c", "d", "e", "f", "g");
  const doc = saved("art/", { bag: ["d"] });
  expect(Deck.restore(doc, "art/", later, (p) => p.id !== "g", lcg(1)).bag).toEqual([3]);
});

test("a deck with no list of what the collection had deals nothing in", () => {
  const doc = { schema: 1, decks: { "art/": { at: 1, bag: ["d"], last: "" } } };
  expect(Deck.restore(doc, "art/", pics("a", "b", "c", "d", "g"), always, lcg(1)).bag).toEqual([3]);
});

test("junk in the bag is skipped", () => {
  const doc = saved("art/", { bag: [3, null, { x: 1 }, ["a"], "d", "", "__proto__", "constructor", "b"], last: 7 });
  expect(Deck.restore(doc, "art/", SIX, always, lcg(1))).toEqual({ bag: [3, 1], last: -1 });
});

test("two prints with one id count as one", () => {
  const dup = [{ id: "a", file: "a.jpg" }, { id: "b", file: "b.jpg" }, { id: "a", file: "a2.jpg" }];
  const doc = { schema: 1, decks: { "art/": { at: 1, bag: ["a", "b", "a"], known: ["a", "b"], last: "a" } } };
  expect(Deck.restore(doc, "art/", dup, always, lcg(1))).toEqual({ bag: [0, 1], last: 0 });
});

test("what is saved is ids, and survives the file", () => {
  const deck = Deck.deckOf(SIX, [3, 1, 5], 2, 1234);
  expect(deck).toEqual({ at: 1234, known: ["a", "b", "c", "d", "e", "f"], bag: ["d", "b", "f"], last: "c" });
  expect(Deck.deckOf(SIX, [], -1, 5).last).toBe("");
  const text = Deck.stringify(Deck.put(Deck.parse(""), "art/", deck));
  expect(text.length).toBeLessThan(200);
  expect(Deck.restore(Deck.parse(text), "art/", SIX, always, lcg(1))).toEqual({ bag: [3, 1, 5], last: 2 });
});

test("saving one collection leaves the others, and the oldest is let go past eight", () => {
  let doc = Deck.parse("");
  for (let i = 0; i < Deck.MAX_DECKS + 3; i++) doc = Deck.put(doc, "folder" + i + "/", Deck.deckOf(SIX, [0], -1, 1000 + i));
  const keys = Object.keys(doc.decks).sort();
  expect(keys.length).toBe(Deck.MAX_DECKS);
  expect(keys).not.toContain("folder0/");
  expect(keys).not.toContain("folder2/");
  expect(keys).toContain("folder3/");
  expect(keys).toContain("folder10/");
  // saving an old one again makes it the newest
  doc = Deck.put(doc, "folder3/", Deck.deckOf(SIX, [1], -1, 5000));
  doc = Deck.put(doc, "newest/", Deck.deckOf(SIX, [2], -1, 5001));
  expect(Object.keys(doc.decks)).toContain("folder3/");
  expect(Object.keys(doc.decks)).not.toContain("folder4/");
});

test("a collection too big to be a small file is not kept", () => {
  const many = Array.from({ length: Deck.MAX_IDS + 1 }, (_, i) => ({ id: "p" + i, file: "p" + i + ".jpg" }));
  let doc = Deck.put(Deck.parse(""), "art/", Deck.deckOf(SIX, [0], -1, 1));
  doc = Deck.put(doc, "art/", Deck.deckOf(many, [0], -1, 2));
  expect(doc.decks["art/"]).toBeUndefined();
  const ok = Array.from({ length: Deck.MAX_IDS }, (_, i) => ({ id: "p" + i, file: "p" + i + ".jpg" }));
  expect(Deck.put(Deck.parse(""), "art/", Deck.deckOf(ok, [0], -1, 1)).decks["art/"]).toBeDefined();
});

test("a round across a restart shows every print once, and the next round does not begin with the last", () => {
  for (let seed = 1; seed < 200; seed++) {
    const r = lcg(seed);
    let bag = Deck.draw(6, [], r);
    const shown = [];
    let last = -1;
    for (let i = 0; i < 4; i++) { last = bag[0]; bag = bag.slice(1); shown.push(last); }
    // the restart: only the file survives
    const file = Deck.stringify(Deck.put(Deck.parse(""), "art/", Deck.deckOf(SIX, bag, last, 1)));
    const back = Deck.restore(Deck.parse(file), "art/", SIX, always, lcg(seed + 1000));
    expect(back.last).toBe(last);
    bag = back.bag;
    while (bag.length) { last = bag[0]; bag = bag.slice(1); shown.push(last); }
    expect([...shown].sort()).toEqual([0, 1, 2, 3, 4, 5]);
    expect(Deck.draw(6, [last, back.last], lcg(seed + 2000))[0]).not.toBe(last);
  }
});
