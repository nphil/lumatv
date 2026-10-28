// Run with: bun test tests/overlays.test.js
// Overlays.js, the queue behind the overlay cards (panels/Overlays.qml): what a record is, what is on the screen and in what order, for how
// long, and what waits. Pure state and numbers with the time passed in, so each rule of docs/research/overlays-result.md is one test here.
import { test, expect, beforeEach } from "bun:test";
import { readFileSync } from "node:fs";

const SRC = readFileSync(new URL("../payload/compositor/lumatv/Overlays.js", import.meta.url), "utf8").replace(/^\.pragma library$/m, "");
const NAMES = ["CLASSES", "DEFAULTS", "config", "classOf", "fromNote", "fromCard", "create", "offer", "withdraw", "dismiss", "cameraShown", "select", "armed", "okOf", "keysOf"];
const O = new Function(SRC + "\nreturn { " + NAMES.join(", ") + " };")();

const cfg = O.config({ enabled: true })
let q, t
const H = { doorbell: 150, alarm: 150, timer: 150, alert: 150, score: 150 }
const heightOf = (e) => H[e.cls]
const ctx = (extra) => Object.assign({ dnd: false, camera: false, top: 134, heightOf }, extra || {})
const note = (id, extra) => Object.assign({ pluginId: "ha", plugin: "Home Assistant", id, channel: "ha", channelTitle: "Home Assistant", channelIcon: "base.icons/home", revision: 1, expiresAt: null, title: "Low battery", body: "Door sensor is at 5 %", actions: [] }, extra || {})
const take = (n) => { const e = O.fromNote(n, cfg, t); const r = O.offer(q, e, cfg, t); return { e, r } }
const now = () => O.select(q, ctx(), cfg, t)
const keys = (sel) => sel.visible.map((e) => e.id)

beforeEach(() => { q = O.create(); t = 1_000_000 })

test("a theme that says nothing gets the defaults, and a number of the wrong kind is not taken", () => {
  const c = O.config({})
  expect(c.enabled).toBe(false)
  expect(c.maxVisible).toBe(2)
  expect(c.limit).toBe(810)
  expect(c.classes.doorbell.priority).toBe(0)
  expect(c.classes.score.priority).toBe(2)
  const c2 = O.config({ enabled: true, maxVisible: "many", limit: 9999, gap: 8, classes: { score: { dwellMs: 5000, tone: "pink" }, alert: { priority: 1.4 } } })
  expect(c2.enabled).toBe(true)
  expect(c2.maxVisible).toBe(2)
  expect(c2.limit).toBe(810)
  expect(c2.gap).toBe(8)
  expect(c2.classes.score.dwellMs).toBe(5000)
  expect(c2.classes.score.tone).toBe("accent")
  expect(c2.classes.alert.priority).toBe(1)
  expect(O.config({ enabled: "yes" }).enabled).toBe(false)
})

test("the defaults are the shipped theme's numbers, and the shipped theme has the switch off", () => {
  const theme = JSON.parse(readFileSync(new URL("../payload/theme/theme.json", import.meta.url), "utf8"))
  expect(theme.overlays.enabled).toBe(false)
  expect(O.config(theme.overlays)).toEqual(O.config({}))
})

test("a record's class: what a plugin calls its channel and its icon, an explicit kind over both, an alert otherwise", () => {
  expect(O.classOf({ channel: "doorbell", channelIcon: "base.icons/doorbell" })).toBe("doorbell")
  expect(O.classOf({ channel: "ha", channelIcon: "base.icons/home" })).toBe("alert")
  expect(O.classOf({ channel: "x", channelIcon: "base.icons/timer" })).toBe("timer")
  expect(O.classOf({ channel: "alarms" })).toBe("alarm")
  expect(O.classOf({ channel: "scores" })).toBe("score")
  expect(O.classOf({ channel: "nba", channelIcon: "base.icons/trophy" })).toBe("score")
  expect(O.classOf({ channel: "doorbell", kind: "alert" })).toBe("alert")
  expect(O.classOf({ channel: "ha", kind: "timer" })).toBe("timer")
  expect(O.classOf({ channel: "ha", kind: "nonsense" })).toBe("alert")
})

test("a notification becomes an entry; `low` importance and an empty one are the drawer's, a secret one shows no words of its own", () => {
  const e = O.fromNote(note("a"), cfg, t)
  expect(e.key).toBe("n:ha/a")
  expect(e.cls).toBe("alert")
  expect(e.priority).toBe(1)
  expect(e.title).toBe("Low battery")
  expect(e.source).toBe("Home Assistant")
  expect(O.fromNote(note("b", { importance: "low" }), cfg, t)).toBe(null)
  expect(O.fromNote(note("c", { title: "", body: "" }), cfg, t)).toBe(null)
  const s = O.fromNote(note("d", { privacy: "secret", title: "Code 1234", body: "do not read" }), cfg, t)
  expect(s.title).toBe("Home Assistant")
  expect(s.body).toBe("")
  // the raw record, `post` and all, is read too
  const raw = O.fromNote({ pluginId: "x", id: "r", channel: "doorbell", channelTitle: "Doorbell", channelIcon: "base.icons/doorbell", revision: 2, expiresAt: 0, post: { title: "Doorbell", body: "Front door", actions: [{ action: "clear", label: "Clear" }] } }, cfg, t)
  expect(raw.cls).toBe("doorbell")
  expect(raw.body).toBe("Front door")
  expect(raw.actions[0].label).toBe("Clear")
  expect(raw.expiresAt).toBe(0)
  // a level makes the tone, and a critical one stays twice as long
  const w = O.fromNote(note("w", { level: "warning" }), cfg, t)
  const k = O.fromNote(note("k", { level: "critical" }), cfg, t)
  expect([w.tone, k.tone]).toEqual(["warning", "danger"])
  expect(k.dwellMs).toBe(2 * e.dwellMs)
  // the end of the record is in seconds, as the broker states it, and an entry's is in milliseconds
  expect(O.fromNote(note("z", { expiresAt: 1_000_030 }), cfg, t).expiresAt).toBe(1_000_030_000)
})

test("a score reads its two sides, or is the title and the body of a card", () => {
  const sc = { league: "NBA", status: "Q4 2:31", live: true, home: { name: "Celtics", abbr: "BOS", score: 99 }, away: { name: "Lakers", abbr: "LAL", score: 102 } }
  const e = O.fromNote(note("g", { channel: "scores", channelIcon: "base.icons/trophy", title: "Lakers 102, Celtics 99", body: "Q4 2:31", score: sc }), cfg, t)
  expect(e.cls).toBe("score")
  expect(e.score.home.name).toBe("Celtics")
  expect(e.score.away.score).toBe("102")
  expect(e.score.live).toBe(true)
  expect(O.fromNote(note("h", { channel: "scores", score: { home: { name: "A" }, away: { name: "B", score: 1 } } }), cfg, t).score).toBe(null)
  expect(O.fromNote(note("i", { score: sc }), cfg, t).score).toBe(null)       // only a score card has one
})

test("only a card at the ringing priority is an entry; one that rings and is no timer or doorbell is an alarm", () => {
  const card = { pluginId: "t", plugin: "Timers", id: "pasta", revision: 1, priority: "ringing", expiresAt: null, actions: [{ action: "stop", label: "Stop" }], content: { type: "event", title: "Pasta", detail: "Time's up", icon: "base.icons/timer", role: "accent" } }
  const e = O.fromCard(card, cfg, t)
  expect(e.key).toBe("c:t/pasta")
  expect(e.cls).toBe("timer")
  expect(e.title).toBe("Pasta")
  expect(e.body).toBe("Time's up")
  expect(O.fromCard(Object.assign({}, card, { priority: "running" }), cfg, t)).toBe(null)
  expect(O.fromCard(Object.assign({}, card, { content: { type: "event", title: "Ring", icon: "base.icons/message" } }), cfg, t).cls).toBe("alarm")
})

test("an entry is taken once; the same record again is an update, and what the person dismissed is not shown again until it changes", () => {
  expect(take(note("a")).r).toBe("new")
  expect(take(note("a")).r).toBe("updated")
  expect(q.list.length).toBe(1)
  expect(take(note("a", { revision: 2, body: "Door sensor is at 4 %" })).r).toBe("updated")
  expect(q.list[0].body).toBe("Door sensor is at 4 %")
  now()
  expect(O.dismiss(q, "n:ha/a", t)).toBe(true)
  expect(q.list.length).toBe(0)
  expect(take(note("a", { revision: 2 })).r).toBe("dismissed")
  expect(take(note("a", { revision: 3 })).r).toBe("new")
  // a record the plugin ended and posted again is news whatever its revision
  O.dismiss(q, "n:ha/a", t)
  expect(O.withdraw(q, "n:ha/a")).toBe(false)
  expect(take(note("a", { revision: 1 })).r).toBe("new")
})

test("the most important class first, and a lower class waits while a higher one is up", () => {
  take(note("score", { channel: "scores", title: "Lakers 102, Celtics 99" }))
  take(note("alert"))
  take(note("door", { channel: "doorbell", channelIcon: "base.icons/doorbell", title: "Doorbell", body: "Front door" }))
  let sel = now()
  expect(keys(sel)).toEqual(["door"])
  // the doorbell is dealt with: the alert is next, the score still waits
  O.dismiss(q, "n:ha/door", t)
  sel = now()
  expect(keys(sel)).toEqual(["alert"])
  O.dismiss(q, "n:ha/alert", t)
  expect(keys(now())).toEqual(["score"])
})

test("two cards at most, of the same class; a doorbell and an alarm are one class; a third waits", () => {
  take(note("d", { channel: "doorbell", title: "Doorbell" }))
  take(note("t", { channel: "timers", title: "Pasta" }))
  take(note("a", { channel: "alarms", title: "Weekday" }))
  expect(keys(now())).toEqual(["d", "t"])
  O.dismiss(q, "n:ha/d", t)
  expect(keys(now())).toEqual(["t", "a"])
  take(note("x1")); take(note("x2"))
  expect(keys(now())).toEqual(["t", "a"])
})

test("two alerts show together, a second score does not show beside a score", () => {
  take(note("a1")); take(note("a2")); take(note("a3"))
  expect(keys(now())).toEqual(["a1", "a2"])
  O.dismiss(q, "n:ha/a1", t); O.dismiss(q, "n:ha/a2", t); O.dismiss(q, "n:ha/a3", t)
  take(note("s1", { channel: "scores" })); take(note("s2", { channel: "scores" }))
  expect(keys(now())).toEqual(["s1"])
})

test("a card that would end below the limit waits; the first card always shows", () => {
  H.alert = 400
  take(note("a1")); take(note("a2"))
  // 134 + 400 fits; the second would end at 134 + 400 + 16 + 400 = 950 > 810
  expect(keys(now())).toEqual(["a1"])
  O.dismiss(q, "n:ha/a1", t)
  expect(keys(now())).toEqual(["a2"])
  H.alert = 900
  expect(keys(now())).toEqual(["a2"])             // taller than the zone: it still shows, it is alone
  H.alert = 150
})

test("the camera's pop-up takes a slot and the top of the column: a second card goes under it if it fits, else waits", () => {
  take(note("t", { channel: "timers", title: "Pasta" }))
  let sel = O.select(q, ctx({ camera: true, top: 134 + 448 + 16 }), cfg, t)
  expect(keys(sel)).toEqual(["t"])                   // 598 + 150 = 748 <= 810
  sel = O.select(q, ctx({ camera: true, top: 134 + 626 + 16 }), cfg, t)
  expect(keys(sel)).toEqual([])                      // 776 + 150 = 926 > 810: it waits
  // an alert waits under a camera card: the camera is the most important class
  take(note("al"))
  O.dismiss(q, "n:ha/t", t)
  sel = O.select(q, ctx({ camera: true, top: 134 + 448 + 16 }), cfg, t)
  expect(keys(sel)).toEqual([])
  // with the camera's card gone the alert shows at the top
  expect(keys(O.select(q, ctx(), cfg, t + 1))).toEqual(["al"])
})

test("a doorbell the camera shows is not a second card: one queued is taken in, one that comes after is merged", () => {
  take(note("ring", { channel: "doorbell", channelIcon: "base.icons/doorbell", title: "Doorbell" }))
  expect(keys(now())).toEqual(["ring"])
  t += 400
  expect(O.cameraShown(q, "doorbell", cfg, t)).toBe(1)
  expect(keys(now())).toEqual([])
  // the record staying in the drawer does not come back as a card
  expect(take(note("ring", { channel: "doorbell", title: "Doorbell" })).r).toBe("merged")
  t += cfg.mergeMs + 1000
  expect(take(note("ring", { channel: "doorbell", title: "Doorbell" })).r).toBe("dismissed")
  // a ring that comes after the camera's card
  O.cameraShown(q, "doorbell", cfg, t)
  expect(take(note("ring2", { channel: "doorbell", title: "Doorbell" })).r).toBe("merged")
  // another kind of camera event is nothing to a doorbell record
  q = O.create()
  O.cameraShown(q, "person", cfg, t)
  expect(take(note("ring3", { channel: "doorbell", title: "Doorbell" })).r).toBe("new")
})

test("nothing shows while the host is busy; the cards wait, keep what time they had, and come back", () => {
  take(note("a"))
  expect(keys(now())).toEqual(["a"])
  t += 4000
  let sel = O.select(q, ctx({ dnd: true }), cfg, t)
  expect(keys(sel)).toEqual([])
  expect(q.list[0].visible).toBe(false)
  expect(q.list[0].shownMs).toBe(4000)
  t += 30000
  sel = O.select(q, ctx(), cfg, t)
  expect(keys(sel)).toEqual(["a"])
  // 4 s of its 10 were seen: first the keys become its (armMs), then 6 s from now it is done
  expect(sel.wakeAt).toBe(t + cfg.armMs)
  const back = t
  t += cfg.armMs
  expect(O.select(q, ctx(), cfg, t).wakeAt).toBe(back + 6000)
  t = back + 6000
  expect(keys(O.select(q, ctx(), cfg, t))).toEqual([])
  expect(q.list.length).toBe(0)
})

test("a card is up for its class's time, counted while it is shown, and is not news again for that revision", () => {
  take(note("s", { channel: "scores", title: "Lakers 102, Celtics 99" }))
  let sel = now()
  expect(keys(sel)).toEqual(["s"])
  expect(sel.wakeAt).toBe(t + cfg.armMs)
  t += 8000
  expect(keys(now())).toEqual([])
  expect(take(note("s", { channel: "scores" })).r).toBe("dismissed")
})

test("a waiting card is dropped when it has waited longer than its class allows, and a record's own end ends it", () => {
  take(note("s", { channel: "scores", title: "Final" }))
  O.select(q, ctx({ dnd: true }), cfg, t)
  t += cfg.classes.score.waitMs
  expect(keys(O.select(q, ctx(), cfg, t))).toEqual([])
  expect(q.list.length).toBe(0)
  take(note("e", { expiresAt: (t + 3000) / 1000 }))
  expect(keys(now())).toEqual(["e"])
  t += 3000
  expect(keys(now())).toEqual([])
})

test("a higher class that arrives takes the screen; the card it displaced comes back with the time it had left", () => {
  take(note("a"))
  now()
  t += 3000
  take(note("d", { channel: "doorbell", title: "Doorbell" }))
  expect(keys(now())).toEqual(["d"])
  expect(q.list.find((e) => e.id === "a").shownMs).toBe(3000)
  O.dismiss(q, "n:ha/d", t)
  t += 1000
  const back = t
  expect(keys(now())).toEqual(["a"])
  t += cfg.armMs
  expect(now().wakeAt).toBe(back + 7000)
})

test("a score that moves is news again while it is up, for no more than three times its time", () => {
  take(note("s", { channel: "scores", title: "A 1, B 0", revision: 1 }))
  now()
  t += 6000
  expect(take(note("s", { channel: "scores", title: "A 1, B 1", revision: 2 })).r).toBe("updated")
  const sel = now()
  expect(keys(sel)).toEqual(["s"])
  expect(sel.wakeAt).toBe(t + 8000)               // the keys were its long ago: only the end of its time is ahead
  expect(q.list[0].title).toBe("A 1, B 1")
  // far into its life it does not stretch for ever
  t += 8000 * 3
  take(note("s", { channel: "scores", title: "A 2, B 1", revision: 3 }))
  expect(q.list[0].shownAt).toBeLessThan(t)         // its time was not started afresh
})

test("a score again too soon after the last is not shown", () => {
  take(note("s", { channel: "scores", title: "A 1, B 0" }))
  now()
  t += 8000
  now()
  t += 1000
  expect(take(note("s", { channel: "scores", title: "A 2, B 0", revision: 2 })).r).toBe("cooldown")
  t += cfg.classes.score.cooldownMs
  expect(take(note("s", { channel: "scores", title: "A 3, B 0", revision: 3 })).r).toBe("new")
})

test("the keys are the card's only after it has been up for armMs", () => {
  take(note("a"))
  let sel = now()
  expect(O.armed(sel.visible, cfg, t)).toBe(false)
  expect(O.armed(sel.visible, cfg, t + cfg.armMs - 1)).toBe(false)
  expect(O.armed(sel.visible, cfg, t + cfg.armMs)).toBe(true)
  expect(O.armed([], cfg, t)).toBe(false)
})

test("what OK does: the record's first action, else the drawer, else it dismisses", () => {
  const withAction = O.fromNote(note("a", { actions: [{ action: "clear", label: "Clear" }, { action: "x", label: "More" }] }), cfg, t)
  expect(O.okOf(withAction, true).label).toBe("Clear")
  expect(O.keysOf(withAction, false)).toEqual([{ key: "OK", what: "Clear" }, { key: "Back", what: "Dismiss" }])
  const plain = O.fromNote(note("b"), cfg, t)
  expect(O.okOf(plain, true)).toEqual({ label: "Open", action: null })
  expect(O.keysOf(plain, true)).toEqual([{ key: "OK", what: "Open" }, { key: "Back", what: "Dismiss" }])
  expect(O.keysOf(plain, false)).toEqual([{ key: "OK", what: "Dismiss" }])
})

test("a withdrawn record's card goes at once", () => {
  take(note("a"))
  expect(keys(now())).toEqual(["a"])
  expect(O.withdraw(q, "n:ha/a")).toBe(true)
  expect(keys(now())).toEqual([])
})
