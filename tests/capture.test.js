// Run with: bun test tests/
// Capture.js is what the Settings page for screenshots and recording and the quick panel's two tiles make of the helper's /v1/capture
// answers (docs/research/capture-result.md): the state they keep and the words for what happened.
import { test, expect } from "bun:test";
import { readFileSync } from "node:fs";

const src = readFileSync(new URL("../payload/compositor/lumatv/settings/Capture.js", import.meta.url), "utf8").replace(/^\.pragma library$/m, "");
const C = new Function(src + "\nreturn { state, said, size, clock, files };")();

const ON = {
  capture: {
    enabled: true, available: true, recording: false,
    encoder: { path: "/var/lib/lumatv/bin/lumatv-venc", available: true },
    captures: { dir: "/var/lib/lumatv/captures", count: 3, bytes: 5_200_000, max: 268_435_456 },
    last: { kind: "shot", name: "shot-20261028-070000.png", ok: true },
  },
};

test("while the switch is off nothing is available and nothing else is said", () => {
  for (const doc of [null, undefined, {}, { capture: { enabled: false, available: false } }, { capture: { enabled: "yes" } }]) {
    expect(C.state(doc)).toMatchObject({ enabled: false, available: false, recording: false, count: 0, encoder: false });
  }
  expect(C.state(null).answered).toBe(false);
  expect(C.state({ capture: { enabled: false } }).answered).toBe(true);
  expect(C.files(C.state(null))).toBe("");
});

test("an answer is reduced to what a surface shows", () => {
  const s = C.state(ON);
  expect(s).toMatchObject({ enabled: true, available: true, recording: false, encoder: true, count: 3, bytes: 5_200_000 });
  expect(s.last.name).toBe("shot-20261028-070000.png");
  const rec = C.state({ capture: { ...ON.capture, recording: true, recorder: { stage: "recording", seconds: 62.9 } } });
  expect(rec).toMatchObject({ recording: true, seconds: 62 });
});

test("a compositor that did not answer is enabled but not available, and says why", () => {
  expect(C.state({ capture: { enabled: true, available: false, reason: "no such file" } })).toMatchObject({ enabled: true, available: false, reason: "no such file" });
});

test("the words for an answer are short and name what to do or what is missing", () => {
  const ok = { ok: true };
  expect(C.said("shot", 200, ok)).toBe("Screenshot saved");
  expect(C.said("start", 200, ok)).toBe("Recording");
  expect(C.said("stop", 200, ok)).toBe("Recording saved");
  const refused = (code) => C.said("start", 409, { ok: false, error: { code, message: "x" } });
  expect(refused("busy")).toBe("Already recording");
  expect(refused("no_indicator")).toBe("The recording dot could not be shown");
  expect(refused("no_encoder")).toBe("Recording is not set up on this TV");
  expect(refused("disk_full")).toBe("Not enough space on the TV");
  expect(refused("compositor_unavailable")).toBe("Needs LumaTV's own compositor");
  expect(refused("capture_disabled")).toBe("Capture is off");
  expect(refused("timeout")).toBe("That took too long");
  expect(refused("capture_failed")).toBe("The recording failed");
  expect(C.said("shot", 500, { ok: false, error: { code: "capture_failed" } })).toBe("The screenshot failed");
  expect(C.said("shot", 0, null)).toBe("The TV's helper did not answer");
  expect(C.said("shot", 404, null)).toBe("The screenshot failed");
});

test("a refusal whose body did not arrive is told from its status, or from the state read after it", () => {
  const up = C.state(ON);
  const down = C.state(null);
  expect(C.said("start", 403, null, up)).toBe("Capture is off");
  expect(C.said("start", 501, null, up)).toBe("Recording is not set up on this TV");
  expect(C.said("start", 507, null, up)).toBe("Not enough space on the TV");
  expect(C.said("shot", 503, null, up)).toBe("Needs LumaTV's own compositor");
  expect(C.said("shot", 504, null, up)).toBe("That took too long");
  // the workstation host hands a 409 over as status 0 with no body
  for (const status of [0, 409]) {
    expect(C.said("start", status, null, down)).toBe("The TV's helper did not answer");
    expect(C.said("start", status, null, up)).toBe("The recording dot could not be shown");
    expect(C.said("start", status, null, C.state({ capture: { ...ON.capture, recording: true } }))).toBe("Already recording");
    expect(C.said("stop", status, null, up)).toBe("Not recording");
    expect(C.said("shot", status, null, up)).toBe("The shell is not on screen");
    expect(C.said("start", status, null, C.state({ capture: { enabled: false } }))).toBe("Capture is off");
  }
});

test("sizes and times read the way a person says them", () => {
  expect(C.size(0)).toBe("0 KB");
  expect(C.size(812_345)).toBe("812 KB");
  expect(C.size(5_200_000)).toBe("5.2 MB");
  expect(C.size(1_234_000_000)).toBe("1.2 GB");
  expect(C.size("junk")).toBe("0 KB");
  expect(C.clock(0)).toBe("0:00");
  expect(C.clock(7)).toBe("0:07");
  expect(C.clock(754)).toBe("12:34");
  expect(C.clock(-3)).toBe("0:00");
  expect(C.files(C.state(ON))).toBe("3 files, 5.2 MB");
  expect(C.files(C.state({ capture: { ...ON.capture, captures: { count: 1, bytes: 1500 } } }))).toBe("1 file, 2 KB");
});
