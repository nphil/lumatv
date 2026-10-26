// Run with: bun test tests/
// LayoutGrid.js is the one model of Home's grid: what LumaHome draws, what Settings' Home widgets page changes and what the layout
// editor moves. It is the layout simulator's model (sim/sim.js on the workstation) with the simulator's rules, so the first tests run
// the simulator's own functions, copied below, against it on the same layouts; the rest are the operations the editor is built on.
import { test, expect } from "bun:test";
import { readFileSync, existsSync } from "node:fs";

const read = (p) => readFileSync(new URL(p, import.meta.url), "utf8");
const NAMES = ["area", "rowCount", "rowHeights", "cellSize", "rectOf", "tidyLayout", "toUniform", "overlaps", "blocker", "change", "move", "resize",
  "firstFree", "newId", "add", "remove", "hide", "show", "sizeOf", "setSpan", "order", "swap", "nudge", "minimize", "forFile", "placeWords",
  "snapMove", "snapResize", "find", "titleOf", "hasBeside", "dockId", "hiddenOf", "same", "TYPES", "SIZES", "ADDABLE", "UNIFORM_ROWS"];
const G = new Function(read("../payload/compositor/lumatv/LayoutGrid.js").replace(/^\.pragma library$/m, "") + "\nreturn { " + NAMES.join(", ") + " };")();

const SHIPPED = JSON.parse(read("../payload/theme/layout.json"));
const clone = (v) => JSON.parse(JSON.stringify(v));

// ---- the simulator's model, as sim/sim.js writes it (the pure functions of its "state" block, with `L` a variable of this closure).
// If the simulator is on this machine, the copy is checked against its file, so a change there is noticed here.
const SIM_SRC = `
  function area() {
    var y = L.safe.y + L.statusBar.height + L.statusBar.gap;
    return { x: L.safe.x, y: y, w: 1920 - 2 * L.safe.x, h: 1080 - L.safe.y - y };
  }
  function cell() {
    var a = area(), g = L.grid, n = rowCount();
    return { w: (a.w - (g.cols - 1) * g.gutter) / g.cols, h: (a.h - (n - 1) * g.gutter) / n };
  }
  function rowCount() { return Array.isArray(L.grid.rows) ? L.grid.rows.length : L.grid.rows; }
  function rectOf(e) {
    var a = area(), g = L.grid, c = cell();
    if (Array.isArray(g.rows)) {
      var y = a.y, h = 0;
      for (var r = 0; r < e.row; r++) y += g.rows[r] + g.gutter;
      for (var k = e.row; k < e.row + e.rows && k < g.rows.length; k++) h += g.rows[k] + (k > e.row ? g.gutter : 0);
      return { x: Math.round(a.x + e.col * (c.w + g.gutter)), y: Math.round(y), w: Math.round(e.span * c.w + (e.span - 1) * g.gutter), h: Math.round(h) };
    }
    return { x: Math.round(a.x + e.col * (c.w + g.gutter)), y: Math.round(a.y + e.row * (c.h + g.gutter)),
             w: Math.round(e.span * c.w + (e.span - 1) * g.gutter), h: Math.round(e.rows * c.h + (e.rows - 1) * g.gutter) };
  }
  function toUniform() {
    if (!Array.isArray(L.grid.rows)) return;
    var pix = L.grid.rows, n = 16, a = area(), c = (a.h - (n - 1) * L.grid.gutter) / n;
    L.widgets.forEach(function (e) {
      var y0 = 0; for (var r = 0; r < e.row; r++) y0 += pix[r] + L.grid.gutter;
      var h = 0; for (var k = e.row; k < e.row + e.rows; k++) h += pix[k] + (k > e.row ? L.grid.gutter : 0);
      e.row = Math.round(y0 / (c + L.grid.gutter));
      e.rows = Math.max(1, Math.round((h + L.grid.gutter) / (c + L.grid.gutter)));
    });
    L.grid.rows = n;
  }
  function overlaps(e, self) {
    self = self || e;
    return L.widgets.some(function (o) { return o !== self && e.col < o.col + o.span && o.col < e.col + e.span && e.row < o.row + o.rows && o.row < e.row + e.rows; });
  }`;
function sim(layout) {
  const box = new Function("L", SIM_SRC + "\nreturn { get L() { return L }, area, cell, rowCount, rectOf, toUniform, overlaps };")(clone(layout));
  return box;
}

const SIM_FILE = "/data/home/lgtv/sim/sim.js";
test("the copy of the simulator's functions is the simulator's (when the simulator is here)", () => {
  if (!existsSync(SIM_FILE)) return;
  const file = readFileSync(SIM_FILE, "utf8").replace(/\s+/g, "");
  for (const src of SIM_SRC.split(/\n(?=  function )/)) expect(file).toContain(src.replace(/\s+/g, ""));
});

// layouts to compare on: the shipped pixel rows, uniform counts, odd gutters and safe areas, the simulator's own presets
const LAYOUTS = [SHIPPED];
for (const rows of [8, 12, 16, 20]) LAYOUTS.push({ ...clone(SHIPPED), grid: { ...SHIPPED.grid, rows } });
LAYOUTS.push({ ...clone(SHIPPED), safe: { x: 48, y: 44 }, grid: { cols: 12, gutter: 24, rows: 16 } });
LAYOUTS.push({ ...clone(SHIPPED), grid: { cols: 10, gutter: 16, rows: [200, 300, 300] } });

test("a card's rectangle is the simulator's, for pixel rows and for a count of rows", () => {
  for (const L of LAYOUTS) {
    const s = sim(L), n = G.rowCount(L);
    for (let row = 0; row < n; row++) for (let rows = 1; row + rows <= n; rows++) for (let col = 0; col < L.grid.cols; col += 3) {
      const e = { col, row, span: Math.min(4, L.grid.cols - col), rows };
      expect(G.rectOf(L, e)).toEqual(s.rectOf(e));
    }
  }
});

test("Home's own cards sit where LumaHome has always drawn them (the pixel rows 222/238/392)", () => {
  const r = Object.fromEntries(SHIPPED.widgets.map((e) => [e.id, G.rectOf(SHIPPED, e)]));
  expect(r.clock).toEqual({ x: 64, y: 134, w: 584, h: 222 });
  expect(r.home).toEqual({ x: 64, y: 376, w: 584, h: 238 });
  expect(r.news).toEqual({ x: 64 + 4 * 151, y: 134, w: 735, h: 480 });
  expect(r.dock).toEqual({ x: 64, y: 634, w: 1792, h: 392 });
});

test("the TV's pixel rows become sixteen equal rows the way the simulator does it", () => {
  const s = sim(SHIPPED);
  s.toUniform();
  const u = G.toUniform(SHIPPED);
  expect(u.grid.rows).toBe(16);
  expect(u.widgets).toEqual(s.L.widgets);
  // the numbers the simulator's own saved layout has for the same cards
  expect(u.widgets.map((e) => [e.id, e.row, e.rows])).toEqual([["clock", 0, 4], ["home", 4, 5], ["news", 0, 9], ["weather", 0, 9], ["dock", 9, 7]]);
  expect(G.toUniform(u)).toBe(u);                       // a layout that is uniform already comes back as it was
});

test("overlap is the simulator's rule, with hidden cards taking no room", () => {
  const u = G.toUniform(SHIPPED), s = sim(u);
  const probe = [{ id: "p", col: 0, row: 0, span: 4, rows: 4 }, { id: "p", col: 4, row: 0, span: 1, rows: 1 }, { id: "p", col: 9, row: 8, span: 3, rows: 2 },
    { id: "p", col: 0, row: 9, span: 12, rows: 7 }, { id: "p", col: 6, row: 10, span: 2, rows: 2 }, { id: "p", col: 3, row: 3, span: 2, rows: 2 }];
  for (const e of probe) expect(G.overlaps(u, e)).toBe(s.overlaps(e));
  const hidden = G.hide(u, "news").layout;
  expect(G.overlaps(hidden, { id: "p", col: 4, row: 0, span: 2, rows: 2 })).toBe(false);
});

// ---- changing a card
const U = G.toUniform(SHIPPED);

test("a card moves a cell at a time and is refused where it would overlap or leave the grid", () => {
  const free = G.hide(U, "news").layout;               // the clock has room to its right now
  const m = G.move(free, "clock", 1, 0);
  expect(m.ok).toBe(true);
  expect(m.entry.col).toBe(1);
  expect(G.move(U, "clock", 1, 0)).toMatchObject({ ok: false, why: "overlap", blocker: { id: "news" } });
  expect(G.move(U, "clock", -1, 0)).toMatchObject({ ok: false, why: "edge" });
  expect(G.move(U, "clock", 0, -1)).toMatchObject({ ok: false, why: "edge" });
  expect(G.move(U, "nope", 1, 0)).toMatchObject({ ok: false, why: "missing" });
});

test("the corner takes the card's size a cell at a time, never past the grid, never below the smallest", () => {
  const free = G.hide(U, "news").layout;
  const r = G.resize(free, "clock", 2, 0);
  expect(r.ok && r.entry.span).toBe(6);
  expect(G.resize(U, "clock", 1, 0)).toMatchObject({ ok: false, why: "overlap", blocker: { id: "news" } });
  const tiny = G.change(U, "clock", { span: 2, rows: 3 }).layout;
  expect(G.resize(tiny, "clock", -1, 0)).toMatchObject({ ok: false, why: "small" });
  expect(G.resize(tiny, "clock", 0, -1)).toMatchObject({ ok: false, why: "small" });
  const empty = G.hide(free, "weather").layout;
  const edge = G.change(empty, "clock", { col: 10, span: 2 }).layout;
  expect(G.resize(edge, "clock", 1, 0)).toMatchObject({ ok: false, why: "edge" });
});

test("a card that is refused leaves the layout as it was (nothing is changed in place)", () => {
  const before = JSON.stringify(U);
  G.move(U, "clock", 1, 0); G.resize(U, "clock", 1, 0); G.add(U, "clock"); G.hide(U, "news"); G.swap(U, "clock", "news");
  expect(JSON.stringify(U)).toBe(before);
});

// ---- adding, hiding, removing
test("a new card takes the first free place at the first size of its ladder that fits", () => {
  const room = G.hide(G.hide(U, "news").layout, "weather").layout;
  const a = G.add(G.remove(room, "news").layout, "news");
  expect(a.ok).toBe(true);
  const e = G.find(a.layout, a.id);
  expect([e.col, e.row, e.span, e.rows]).toEqual([4, 0, 5, 7]);                 // top to bottom, left to right: beside the clock, at the ladder's first size
  const c = G.add(room, "clock");                                              // a clock already there: the second gets its own id
  expect(c.id).toBe("clock-2");
  expect(G.find(c.layout, "clock-2").type).toBe("clock");
  expect(G.find(c.layout, "clock-2").hidden).toBeUndefined();
});

test("a ladder falls to a smaller size when the first does not fit, and says full when none does", () => {
  const tight = G.toUniform(SHIPPED);
  tight.widgets = [{ id: "all", type: "news", col: 0, row: 0, span: 12, rows: 8 }, { id: "dock", type: "dock", col: 0, row: 8, span: 12, rows: 8 }];
  expect(G.add(tight, "clock")).toMatchObject({ ok: false, why: "full" });
  const gap = G.change(tight, "all", { rows: 4 }).layout;                       // four free rows: the clock's 4x4 fits at the left
  const a = G.add(gap, "clock");
  expect(a.ok && G.find(a.layout, a.id)).toMatchObject({ col: 0, row: 4, span: 4, rows: 4 });
  const narrow = G.change(gap, "all", { rows: 6 }).layout;                      // two free rows: nothing of the clock's ladder is that short
  expect(G.add(narrow, "clock")).toMatchObject({ ok: false, why: "full" });
  expect(G.add(U, "dock")).toMatchObject({ ok: false, why: "fixed" });
  expect(G.add(U, "nothing")).toMatchObject({ ok: false, why: "unknown" });
});

test("the dock can be neither removed nor hidden", () => {
  expect(G.remove(U, "dock")).toMatchObject({ ok: false, why: "fixed" });
  expect(G.hide(U, "dock")).toMatchObject({ ok: false, why: "fixed" });
  expect(G.remove(U, "news").layout.widgets.map((e) => e.id)).toEqual(["clock", "home", "weather", "dock"]);
});

test("a hidden card comes back where it was, or to the first free place when that is taken", () => {
  const h = G.hide(U, "weather").layout;
  expect(G.find(h, "weather").hidden).toBe(true);
  const back = G.show(h, "weather");
  expect(back.ok).toBe(true);
  expect(G.find(back.layout, "weather")).toMatchObject({ col: 9, row: 0, span: 3, rows: 9 });
  expect(G.find(back.layout, "weather").hidden).toBeUndefined();
  // something else moved into its place meanwhile: it comes back in what is left, smaller
  const taken = G.change(h, "news", { span: 6 }).layout;
  const shown = G.show(taken, "weather");
  expect(shown.ok).toBe(true);
  const e = G.find(shown.layout, "weather");
  expect(G.overlaps(shown.layout, e)).toBe(false);
  expect([e.col, e.span]).toEqual([10, 2]);
  // and with no room at all it says so
  expect(G.show(G.change(h, "news", { span: 8 }).layout, "weather")).toMatchObject({ ok: false, why: "full" });
  expect(G.add(h, "weather").layout.widgets.filter((x) => x.type === "weather").length).toBe(1);   // add brings the hidden one back, it does not make a second
});

// ---- sizes and order
test("small, half and wide are four, six and eight of the twelve columns, and a card on the right grows to the left", () => {
  expect(G.SIZES.map((s) => [s.id, s.span])).toEqual([["small", 4], ["half", 6], ["wide", 8]]);
  expect(G.sizeOf({ span: 4 })).toBe("small");
  expect(G.sizeOf({ span: 5 })).toBe("custom");
  const free = G.hide(U, "news").layout;
  const half = G.setSpan(free, "weather", 6);
  expect(half.ok).toBe(true);
  expect(G.find(half.layout, "weather")).toMatchObject({ col: 6, span: 6 });          // the weather stays on the right edge
  const clock = G.setSpan(half.layout, "clock", 6);
  expect(G.find(clock.layout, "clock")).toMatchObject({ col: 0, span: 6 });           // the clock keeps its left edge
  expect(G.setSpan(U, "clock", 6)).toMatchObject({ ok: false, why: "overlap", blocker: { id: "news" } });
  expect(G.setSpan(free, "clock", 12)).toMatchObject({ ok: false, why: "overlap" });
});

test("reading order is top to bottom then left to right, and a nudge trades places with the next card", () => {
  expect(G.order(U)).toEqual(["clock", "news", "weather", "home"]);
  const n = G.nudge(U, "home", -1);                                                   // home trades with the weather card, which is before it
  expect(n.ok).toBe(true);
  expect(G.find(n.layout, "home")).toMatchObject({ col: 9, row: 0, span: 3, rows: 9 });
  expect(G.find(n.layout, "weather")).toMatchObject({ col: 0, row: 4, span: 4, rows: 5 });
  expect(G.overlaps(n.layout, G.find(n.layout, "home"))).toBe(false);
  expect(G.nudge(U, "clock", -1)).toMatchObject({ ok: false, why: "edge" });
  expect(G.nudge(U, "home", 1)).toMatchObject({ ok: false, why: "edge" });
  // the dock and hidden cards are not in the order
  expect(G.order(G.hide(U, "news").layout)).toEqual(["clock", "weather", "home"]);
});

test("trading places is refused when a card would end up smaller than its type's smallest", () => {
  const s = G.change(U, "clock", { span: 2, rows: 3 }).layout;                        // 2x3 is the clock's smallest
  const small = G.change(s, "home", { span: 2, rows: 3, row: 5 }).layout;
  const sw = G.swap(small, "news", "clock");                                         // the news card (smallest 3x4) into the clock's 2x3
  expect(sw).toMatchObject({ ok: false, why: "small" });
});

// ---- the owner's file
test("the file holds only what differs from the shipped layout, and nothing at all for the shipped one", () => {
  expect(G.forFile(SHIPPED, SHIPPED)).toEqual({});
  expect(G.forFile(clone(SHIPPED), SHIPPED)).toEqual({});
  const u = G.toUniform(SHIPPED);
  const f = G.forFile(u, SHIPPED);
  expect(Object.keys(f).sort()).toEqual(["grid", "widgets"]);
  expect(f.grid).toEqual({ rows: 16 });
  const moved = G.hide(u, "news").layout;
  expect(G.forFile(moved, SHIPPED).widgets.find((e) => e.id === "news").hidden).toBe(true);
  const reordered = clone(SHIPPED);
  reordered.widgets = reordered.widgets.map((e) => Object.fromEntries(Object.entries(e).reverse()));
  expect(G.forFile(reordered, SHIPPED)).toEqual({});                                  // the order of keys inside a card is nothing
});

test("the merged file reads back as the same layout (LumaHome merges the owner's file over the shipped one)", () => {
  function merge(base, over) {
    if (over === undefined) return base;
    if (over === null || typeof over !== "object" || Array.isArray(over)) return over;
    if (base === null || typeof base !== "object" || Array.isArray(base)) return over;
    const out = {};
    for (const k in base) out[k] = base[k];
    for (const k in over) out[k] = merge(base[k], over[k]);
    return out;
  }
  const edited = G.setSpan(G.hide(G.toUniform(SHIPPED), "news").layout, "weather", 6).layout;
  const file = G.forFile(edited, SHIPPED);
  expect(merge(SHIPPED, JSON.parse(JSON.stringify(file)))).toEqual(JSON.parse(JSON.stringify(edited)));
});

test("a hand-edited layout is cut to the grid, not refused", () => {
  const messy = clone(U);
  messy.widgets = [{ id: "a", type: "clock", col: 11, row: 14, span: 5, rows: 6 }, null, { type: "weather" }, { id: "a", type: "news", col: -3, row: 0, span: "x", rows: 2 },
    { id: "dock", type: "dock", col: 0, row: 9, span: 12, rows: 7 }];
  const t = G.tidyLayout(messy);
  expect(t.widgets.map((e) => e.id)).toEqual(["a", "weather", "a-2", "dock"]);
  for (const e of t.widgets) expect(e.col >= 0 && e.col + e.span <= 12 && e.row >= 0 && e.row + e.rows <= 16).toBe(true);
});

// ---- the pointer
test("a drag lands on whole cells and the corner sizes in whole cells, as in the simulator", () => {
  const c = G.cellSize(U);
  const pitchX = c.w + U.grid.gutter, pitchY = c.h + U.grid.gutter;
  const free = G.hide(U, "news").layout;
  expect(G.snapMove(free, "clock", pitchX * 1.4, 0)).toEqual({ col: 1, row: 0 });
  expect(G.snapMove(free, "clock", pitchX * 1.6, pitchY * 2.2)).toEqual({ col: 2, row: 2 });
  expect(G.snapMove(free, "clock", -9999, 9999)).toEqual({ col: 0, row: 12 });
  const r = G.rectOf(free, G.find(free, "clock"));
  expect(G.snapResize(free, "clock", pitchX * 2, 0)).toEqual({ span: 6, rows: 4 });
  expect(G.snapResize(free, "clock", -9999, -9999)).toEqual({ span: 2, rows: 3 });      // never below the clock's smallest
  expect(r.w).toBe(584);
});

test("Up leaves the grid from a card with nothing over it, by the columns it shares", () => {
  expect(G.hasBeside(U, "clock", -1)).toBe(false);
  expect(G.hasBeside(U, "home", -1)).toBe(true);                          // the clock is over the home card
  expect(G.hasBeside(U, "news", -1)).toBe(false);                         // the news card is tall: the clock is beside it, not over it
  expect(G.hasBeside(U, "dock", -1)).toBe(true);
  expect(G.hasBeside(U, "dock", 1)).toBe(false);
  expect(G.hasBeside(U, "home", 1)).toBe(true);                           // the dock is under it
  expect(G.hasBeside(G.hide(U, "clock").layout, "home", -1)).toBe(false); // a hidden card is not over anything
});

test("the dock is found by its type, and a hidden card of a type is told apart from no card of it", () => {
  expect(G.dockId(U)).toBe("dock");
  expect(G.dockId({ widgets: [] })).toBe("");
  expect(G.hiddenOf(U, "news")).toBe(false);
  expect(G.hiddenOf(G.hide(U, "news").layout, "news")).toBe(true);
  expect(G.hiddenOf(G.remove(U, "news").layout, "news")).toBe(false);
});

test("every type the add list offers has an id, a title, an icon and a ladder that starts no smaller than its smallest", () => {
  for (const t of G.ADDABLE) {
    const d = G.TYPES[t];
    expect(d.title && d.icon).toBeTruthy();
    expect(d.want.every((w) => w[0] >= d.min[0] && w[1] >= d.min[1])).toBe(true);
  }
  expect(G.ADDABLE).toEqual(["clock", "homeassistant", "weather", "news"]);
});
