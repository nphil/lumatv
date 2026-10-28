// Run with: bun test tests/
// KeyboardLayouts.js is the model of LumaTV's own on-screen keyboard (panels/Keyboard.qml draws it): which layout a text field gets from its
// content type, the keys of each layout, the D-pad over them (wrap, the remembered column, holes), what a key does and the remote's repeats.
import { test, expect } from "bun:test";
import { readFileSync } from "node:fs";

const read = (p) => readFileSync(new URL(p, import.meta.url), "utf8");
const NAMES = ["PURPOSE", "HINT", "ENTER", "MAX_KEYS", "MAX_ICONS", "layoutFor", "isPad", "secret", "initialShift", "enterLook", "build", "rects", "home", "move", "hit", "press", "labelOf", "gate", "repeats", "at"];
const K = new Function(read("../payload/compositor/lumatv/panels/KeyboardLayouts.js").replace(/^\.pragma library$/m, "") + "\nreturn { " + NAMES.join(", ") + " };")();

const ids = (layout) => layout.keys.map((k) => k.id);
const find = (layout, id) => layout.keys.findIndex((k) => k.id === id);
const cell = (layout, row, col) => layout.keys.findIndex((k) => k.row === row && col >= k.col && col < k.col + k.span);
// the key the D-pad ends on after `dirs`, from key `from`, carrying the remembered column between the steps as the keyboard does
const walk = (layout, from, dirs) => {
  let at = from, want;
  for (const d of dirs) { const m = K.move(layout, at, d, want); at = m.index; want = m.want; }
  return layout.keys[at].id;
};

test("the field's purpose picks the layout: pads for digits, numbers, phones and dates, e-mail and url letters, letters for the rest", () => {
  const P = K.PURPOSE;
  expect(K.layoutFor(P.normal, 7)).toBe("letters");
  expect(K.layoutFor(P.alpha, 0)).toBe("letters");
  expect(K.layoutFor(P.name, 0)).toBe("letters");
  expect(K.layoutFor(P.terminal, 0)).toBe("letters");
  expect(K.layoutFor(P.password, 0xc0)).toBe("letters");
  expect(K.layoutFor(P.digits, 0)).toBe("pad-digits");
  expect(K.layoutFor(P.number, 0)).toBe("pad-number");
  expect(K.layoutFor(P.phone, 0)).toBe("pad-phone");
  expect(K.layoutFor(P.date, 0)).toBe("pad-date");
  expect(K.layoutFor(P.time, 0)).toBe("pad-date");
  expect(K.layoutFor(P.datetime, 0)).toBe("pad-date");
  expect(K.layoutFor(P.email, 0)).toBe("email");
  expect(K.layoutFor(P.url, 0)).toBe("url");
  expect(K.layoutFor(undefined, undefined)).toBe("letters");
  expect(K.layoutFor(99, 0)).toBe("letters");
  expect(K.isPad("pad-number")).toBe(true);
  expect(K.isPad("email")).toBe(false);
});

test("a PIN is a digit field whose characters are hidden: the pad, and the keyboard knows to keep what is typed to itself", () => {
  const H = K.HINT, P = K.PURPOSE;
  expect(K.layoutFor(P.digits, H.hiddenText | H.sensitiveData)).toBe("pad-digits");
  expect(K.secret(P.digits, H.hiddenText)).toBe(true);
  expect(K.secret(P.normal, H.sensitiveData)).toBe(true);
  expect(K.secret(P.password, 0)).toBe(true);
  expect(K.secret(P.normal, 0x7)).toBe(false);
  expect(K.secret(P.email, 0)).toBe(false);
});

test("shift starts as the field asks: caps lock for uppercase, one capital for a name or a title, off otherwise", () => {
  const H = K.HINT, P = K.PURPOSE;
  expect(K.initialShift(P.normal, 0x7)).toBe(0);
  expect(K.initialShift(P.normal, H.uppercase)).toBe(2);
  expect(K.initialShift(P.normal, H.titlecase)).toBe(1);
  expect(K.initialShift(P.name, 0x7)).toBe(1);
  expect(K.initialShift(P.name, H.lowercase)).toBe(0);
});

test("the enter key says what the field does with it, and the keyboard goes away after the ones that finish a field", () => {
  const E = K.ENTER;
  expect(K.enterLook(E.def)).toEqual({ icon: "corner-down-left", text: "", hides: false });
  expect(K.enterLook(E.ret).hides).toBe(false);
  for (const [t, w] of [[E.done, "Done"], [E.go, "Go"], [E.send, "Send"], [E.search, "Search"]]) expect(K.enterLook(t)).toEqual({ icon: "", text: w, hides: true });
  expect(K.enterLook(E.next)).toEqual({ icon: "", text: "Next", hides: false });
  expect(K.enterLook(E.previous)).toEqual({ icon: "", text: "Prev", hides: false });
  const l = K.build("letters", { enter: E.search, voice: true });
  const enter = l.keys[find(l, "enter")];
  expect([enter.label, enter.accent, enter.hides]).toEqual(["Search", true, true]);
  expect(K.press(l, find(l, "enter"), { shift: 0 })).toEqual({ action: "enter", hide: true });
  const d = K.build("letters", { enter: E.def });
  expect(K.press(d, find(d, "enter"), { shift: 0 })).toEqual({ action: "enter" });
});

test("the letters, e-mail and url layouts share one grid; e-mail and url swap a few keys; symbols is the same grid again", () => {
  const letters = K.build("letters", { voice: true });
  expect(letters.cols).toBe(12);
  expect(letters.rows).toBe(5);
  expect(letters.keys.length).toBe(12 * 4 + 3);
  // every cell of the four character rows is one key and the bottom row is arrows around the space bar
  for (let r = 0; r < 4; r++) for (let c = 0; c < 12; c++) expect(cell(letters, r, c)).toBeGreaterThanOrEqual(0);
  const bottom = letters.keys.filter((k) => k.row === 4).map((k) => [k.id, k.col, k.span]);
  expect(bottom).toEqual([["left", 0, 1], ["space", 1, 10], ["right", 11, 1]]);
  const typed = (l, row) => l.keys.filter((k) => k.row === row && k.col >= 1 && k.col <= 10).map((k) => k.id).join("");
  expect([typed(letters, 0), typed(letters, 1), typed(letters, 2), typed(letters, 3)]).toEqual(["1234567890", "qwertyuiop", "asdfghjkl?", "zxcvbnm,.-"]);
  const email = K.build("email", {});
  expect([typed(email, 2), typed(email, 3)]).toEqual(["asdfghjkl_", "zxcvbnm.-+"]);
  expect(ids(email)).toContain("@");
  expect(ids(email)).toContain("dotcom");
  const url = K.build("url", {});
  expect([typed(url, 2), typed(url, 3)]).toEqual(["asdfghjkl:", "zxcvbnm.-_"]);
  expect(ids(url)).toContain("/");
  expect(ids(url)).toContain("dotcom");
  expect(ids(letters)).toContain("@");
  const symbols = K.build("symbols", {});
  expect([typed(symbols, 1), typed(symbols, 2), typed(symbols, 3)]).toEqual(["!@#$%^&*()", "-_=+[]{}|\\", ";:'\",.<>/?"]);
  expect(symbols.shiftable).toBe(false);
  expect(symbols.keys[find(symbols, "mode")].label).toBe("ABC");
  expect(letters.keys[find(letters, "mode")].label).toBe("?123");
  // all 32 printable ASCII symbols are reachable from the symbols layout
  const all = new Set(symbols.keys.filter((k) => k.kind === "char").map((k) => k.text));
  for (const c of "!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~") expect(all.has(c)).toBe(true);
  // the layouts are built fresh: a key of one is not the key of another
  expect(K.build("letters", {}).keys[0]).not.toBe(letters.keys[0]);
});

test("the microphone key is there only where LG's voice can be asked; the grid has no hole without it: a key that puts the keyboard away takes its place", () => {
  expect(ids(K.build("letters", { voice: true }))).toContain("voice");
  expect(ids(K.build("letters", { voice: true }))).not.toContain("hide");
  const quiet = K.build("letters", { voice: false });
  expect(ids(quiet)).not.toContain("voice");
  expect(ids(quiet)).toContain("hide");
  expect(quiet.keys.length).toBe(12 * 4 + 3);
  expect(cell(quiet, 2, 11)).toBe(find(quiet, "hide"));
  expect(K.press(quiet, find(quiet, "hide"), { shift: 0 })).toEqual({ hide: true });
});

test("the pads: three columns of digits and a column of Backspace, Enter and Clear, with the corner keys of the purpose and holes where there are none", () => {
  const digits = K.build("pad-digits", {});
  expect([digits.cols, digits.rows, digits.pad]).toEqual([4, 4, true]);
  expect(ids(digits).sort()).toEqual(["0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "backspace", "clear", "enter"].sort());
  expect(cell(digits, 3, 0)).toBe(-1);
  expect(cell(digits, 3, 2)).toBe(-1);
  expect(cell(digits, 3, 3)).toBe(-1);
  const corner = (id, col) => { const l = K.build(id, {}); const i = cell(l, 3, col); return i < 0 ? null : l.keys[i].id; };
  expect([corner("pad-number", 0), corner("pad-number", 2)]).toEqual([".", "-"]);
  expect([corner("pad-phone", 0), corner("pad-phone", 2)]).toEqual(["+", "-"]);
  expect([corner("pad-date", 0), corner("pad-date", 2)]).toEqual(["/", ":"]);
  // the digits sit as on a phone
  expect(digits.keys[cell(digits, 0, 0)].id).toBe("1");
  expect(digits.keys[cell(digits, 2, 2)].id).toBe("9");
  expect(digits.keys[cell(digits, 3, 1)].id).toBe("0");
});

test("the focus starts in the middle of the letters and on the 5 of a pad", () => {
  const l = K.build("letters", {});
  expect(l.keys[K.home(l)].id).toBe("g");
  const e = K.build("email", {});
  expect(e.keys[K.home(e)].id).toBe("g");
  const p = K.build("pad-number", {});
  expect(p.keys[K.home(p)].id).toBe("5");
});

test("Left and Right walk a row and wrap to its other end", () => {
  const l = K.build("letters", { voice: true });
  const g = find(l, "g");
  expect(walk(l, g, ["right"])).toBe("h");
  expect(walk(l, g, ["left"])).toBe("f");
  // the middle row, from its apostrophe to the microphone: a, ..., l, ?, voice, and round to the apostrophe
  expect(walk(l, find(l, "l"), ["right"])).toBe("?");
  expect(walk(l, find(l, "l"), ["right", "right"])).toBe("voice");
  expect(walk(l, find(l, "l"), ["right", "right", "right"])).toBe("'");
  expect(walk(l, find(l, "'"), ["left"])).toBe("voice");
  // the bottom row: the arrows around the space bar
  expect(walk(l, find(l, "left"), ["right", "right"])).toBe("right");
  expect(walk(l, find(l, "right"), ["right"])).toBe("left");
  expect(walk(l, find(l, "space"), ["left"])).toBe("left");
});

test("Up and Down keep the column, wrap top to bottom and back, and the space bar hands the column back", () => {
  const l = K.build("letters", { voice: true });
  expect(walk(l, find(l, "g"), ["up"])).toBe("t");
  expect(walk(l, find(l, "g"), ["down"])).toBe("b");
  expect(walk(l, find(l, "g"), ["up", "up"])).toBe("5");
  expect(walk(l, find(l, "g"), ["up", "up", "up"])).toBe("space");               // the top row wraps to the bottom
  expect(walk(l, find(l, "g"), ["down", "down", "down"])).toBe("5");              // and the bottom to the top, at g's column
  expect(walk(l, find(l, "g"), ["down", "down", "up"])).toBe("b");
  expect(walk(l, find(l, "b"), ["down", "up"])).toBe("b");
  expect(walk(l, find(l, "t"), ["down", "down", "down", "up", "up", "up"])).toBe("t");
  // the left and right columns go straight down
  expect(walk(l, find(l, "mode"), ["down"])).toBe("shift");
  expect(walk(l, find(l, "backspace"), ["down", "down"])).toBe("voice");
  expect(walk(l, find(l, "backspace"), ["down", "down", "down"])).toBe("clear");
  expect(walk(l, find(l, "clear"), ["down"])).toBe("right");
  expect(walk(l, find(l, "mode"), ["up"])).toBe("left");
  // without the microphone its place is the key that puts the keyboard away
  const quiet = K.build("letters", {});
  expect(walk(quiet, find(quiet, "enter"), ["down"])).toBe("hide");
  expect(walk(quiet, find(quiet, "enter"), ["down", "down"])).toBe("clear");
  expect(walk(quiet, find(quiet, "clear"), ["up", "up"])).toBe("enter");
});

test("no layout has more keys, or more keys with a picture, than the QML makes once and never again", () => {
  let most = 0, icons = 0;
  for (const id of ["letters", "email", "url", "symbols", "pad-digits", "pad-number", "pad-phone", "pad-date"]) {
    for (const voice of [true, false]) for (const enter of [0, 5]) {
      const l = K.build(id, { voice, enter });
      most = Math.max(most, l.keys.length);
      icons = Math.max(icons, l.keys.filter((k) => k.icon).length);
      // every key is in the grid, and none shares a cell with another
      const cells = new Set();
      for (const k of l.keys) for (let c = k.col; c < k.col + k.span; c++) {
        expect(k.row).toBeLessThan(l.rows);
        expect(c).toBeLessThan(l.cols);
        expect(cells.has(k.row + "/" + c)).toBe(false);
        cells.add(k.row + "/" + c);
      }
    }
  }
  expect(most).toBeLessThanOrEqual(K.MAX_KEYS);
  expect(icons).toBeLessThanOrEqual(K.MAX_ICONS);
  expect(most).toBe(K.MAX_KEYS);                         // the QML makes no more than it needs
});

test("a pad's holes are walked over: the nearest key on the row, then back at the column that was meant", () => {
  const d = K.build("pad-digits", {});
  expect(walk(d, find(d, "7"), ["down"])).toBe("0");                    // row 3 has only the 0
  expect(walk(d, find(d, "0"), ["up"])).toBe("8");
  expect(walk(d, find(d, "9"), ["down"])).toBe("0");
  expect(walk(d, find(d, "9"), ["down", "up"])).toBe("9");
  expect(walk(d, find(d, "clear"), ["down"])).toBe("0");
  expect(walk(d, find(d, "clear"), ["down", "up"])).toBe("clear");
  expect(walk(d, find(d, "1"), ["left"])).toBe("backspace");
  expect(walk(d, find(d, "backspace"), ["right"])).toBe("1");
  expect(walk(d, find(d, "0"), ["right"])).toBe("0");
  const n = K.build("pad-number", {});
  expect(walk(n, find(n, "0"), ["left"])).toBe(".");
  expect(walk(n, find(n, "0"), ["right"])).toBe("-");
  expect(walk(n, find(n, "-"), ["right"])).toBe(".");
});

test("a key's rectangle: the grid in pixels, and the pointer finds the key under it", () => {
  const l = K.build("letters", {});
  const g = K.rects(l, 104, 68, 10);
  expect([g.width, g.height]).toEqual([12 * 104 + 11 * 10, 5 * 68 + 4 * 10]);
  expect(g.rects[find(l, "space")]).toEqual({ x: 114, y: 4 * 78, w: 10 * 104 + 9 * 10, h: 68 });
  expect(K.hit(g, 114 + 10, 4 * 78 + 10)).toBe(find(l, "space"));
  expect(K.hit(g, 108, 10)).toBe(-1);                    // the gap between two keys
  expect(K.hit(g, -5, 10)).toBe(-1);
  expect(K.hit(g, 10, 10)).toBe(find(l, "mode"));
});

test("a key types its text, shifted when Shift is on; one capital is used up by a letter and not by a digit; caps lock stays", () => {
  const l = K.build("letters", {});
  const a = find(l, "a");
  expect(K.press(l, a, { shift: 0 })).toEqual({ text: "a" });
  expect(K.press(l, a, { shift: 1 })).toEqual({ text: "A", shift: 0 });
  expect(K.press(l, a, { shift: 2 })).toEqual({ text: "A" });
  expect(K.press(l, find(l, "1"), { shift: 1 })).toEqual({ text: "1" });
  expect(K.press(l, find(l, "shift"), { shift: 0 })).toEqual({ shift: 1 });
  expect(K.press(l, find(l, "shift"), { shift: 1 })).toEqual({ shift: 2 });
  expect(K.press(l, find(l, "shift"), { shift: 2 })).toEqual({ shift: 0 });
  expect(K.labelOf(l, a, 0)).toBe("a");
  expect(K.labelOf(l, a, 1)).toBe("A");
  expect(K.labelOf(l, find(l, "clear"), 1)).toBe("Clear");
  const s = K.build("symbols", {});
  expect(K.press(s, find(s, "@"), { shift: 2 })).toEqual({ text: "@" });
  expect(K.labelOf(s, find(s, "!"), 2)).toBe("!");
});

test("the other keys: space, the way to the symbols and back, Backspace, Clear, the cursor, the microphone and .com", () => {
  const l = K.build("email", { voice: true });
  expect(K.press(l, find(l, "space"), { shift: 0 })).toEqual({ text: " " });
  expect(K.press(l, find(l, "mode"), { shift: 0 })).toEqual({ mode: "symbols" });
  expect(K.press(l, find(l, "backspace"), { shift: 0 })).toEqual({ action: "backspace" });
  expect(K.press(l, find(l, "clear"), { shift: 0 })).toEqual({ action: "clear" });
  expect(K.press(l, find(l, "left"), { shift: 0 })).toEqual({ action: "left" });
  expect(K.press(l, find(l, "right"), { shift: 0 })).toEqual({ action: "right" });
  expect(K.press(l, find(l, "voice"), { shift: 0 })).toEqual({ voice: true });
  expect(K.press(l, find(l, "dotcom"), { shift: 2 })).toEqual({ text: ".com" });
  const s = K.build("symbols", {});
  expect(K.press(s, find(s, "mode"), { shift: 0 })).toEqual({ mode: "letters" });
  expect(K.press(l, 999, { shift: 0 })).toEqual({});
});

test("the remote's repeats: the first press acts, repeats wait for the delay and then come at the interval, faster after a few steps", () => {
  const cfg = { delay: 300, interval: 100, fast: 60, fastAfter: 3 };
  const g = {};
  const at = (t, repeat = true, key = "right") => K.gate(g, key, repeat, t, cfg);
  expect(at(0, false)).toBe(true);
  expect(at(33)).toBe(false);                            // the remote repeats every 33 ms
  expect(at(250)).toBe(false);
  expect(at(301)).toBe(true);                            // step 1
  expect(at(333)).toBe(false);
  expect(at(401)).toBe(true);                            // step 2
  expect(at(466)).toBe(false);
  expect(at(501)).toBe(true);                            // step 3, then the fast pace
  expect(at(533)).toBe(false);
  expect(at(561)).toBe(true);
  expect(at(600)).toBe(false);
  expect(at(621)).toBe(true);
  // a new key is a new first press, and a repeat of a key whose press was not seen starts a count of its own
  expect(at(640, false, "left")).toBe(true);
  expect(at(700, true, "left")).toBe(false);
  expect(at(941, true, "left")).toBe(true);
  expect(at(1000, true, "up")).toBe(true);
});

test("which keys repeat: the arrows, and OK on Backspace and the cursor keys, never on a letter", () => {
  const l = K.build("letters", { voice: true });
  for (const d of ["up", "down", "left", "right"]) expect(K.repeats(d, l, find(l, "a"))).toBe(true);
  expect(K.repeats("ok", l, find(l, "backspace"))).toBe(true);
  expect(K.repeats("ok", l, find(l, "left"))).toBe(true);
  expect(K.repeats("ok", l, find(l, "a"))).toBe(false);
  expect(K.repeats("ok", l, find(l, "space"))).toBe(false);
  expect(K.repeats("ok", l, find(l, "enter"))).toBe(false);
  expect(K.repeats("back", l, 0)).toBe(false);
});
