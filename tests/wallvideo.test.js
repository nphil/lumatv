// Run with: bun test tests/wallvideo.test.js
import { test, expect } from "bun:test";
import { readFileSync } from "node:fs";

const src = readFileSync(new URL("../payload/compositor/lumatv/WallVideo.js", import.meta.url), "utf8").replace(/^\.pragma library$/m, "");
const Wall = new Function(src + "\nreturn { look, differs, sameScrim, pick, clipName, KEYS, DEFAULTS };")();
const shipped = JSON.parse(readFileSync(new URL("../payload/vplane/clips/wall.json", import.meta.url), "utf8"));
const theme = JSON.parse(readFileSync(new URL("../payload/theme/theme.json", import.meta.url), "utf8"));
const scrim = theme.dark.wallScrim;

test("the shipped theme's look and wallpaper pick the shipped clip", () => {
  const got = Wall.pick(shipped, theme.wallpaper, theme.wallpaperMotion, scrim);
  expect(got.why).toBe("");
  expect(got.entry.id).toBe("silk");
  expect(Wall.clipName(got.entry)).toBe("wall-silk");
});

// TV round 13: the owner's look (set through `lumatv theme set`) is not the shipped one; it has a clip of its own, and a look between the two has none.
const tuned = { amplitude: 12, depth: 0.45, scale: 1.2, detail: 0.5, vignette: 0.9, bloom: 0.25, saturation: 1.25, contrast: 1.25, enabled: true };

test("the owner's tuned look picks the clip made for it, and a look between the two picks none", () => {
  const got = Wall.pick(shipped, theme.wallpaper, tuned, scrim);
  expect(got.why).toBe("");
  expect(got.entry.id).toBe("silk-tuned");
  expect(Wall.clipName(got.entry)).toBe("wall-silk-tuned");
  expect(Wall.pick(shipped, theme.wallpaper, Object.assign({}, tuned, { amplitude: 11 }), scrim).entry).toBeNull();
  expect(shipped.walls.map((w) => w.id)).toEqual(["silk", "silk-tuned"]);
});

test("a setting the theme leaves out is the QML's own default, as the generator takes it", () => {
  expect(Wall.look({})).toEqual(Wall.DEFAULTS);
  expect(Wall.pick(shipped, theme.wallpaper, {}, scrim).entry).not.toBeNull();
  expect(Wall.pick(shipped, theme.wallpaper, null, scrim).entry).not.toBeNull();
});

test("a look other than the clip's has no clip, and names what differs", () => {
  const got = Wall.pick(shipped, theme.wallpaper, { ...theme.wallpaperMotion, amplitude: 9, depth: 0.5 }, scrim);
  expect(got.entry).toBeNull();
  expect(got.why).toBe("the look is not the clip's (amplitude, depth)");
  for (const k of ["speed", "scale", "detail", "vignette", "specular", "bloom", "saturation", "contrast"]) {
    const bent = { ...theme.wallpaperMotion, [k]: Wall.DEFAULTS[k] + 0.5 };
    expect(Wall.pick(shipped, theme.wallpaper, bent, scrim).entry).toBeNull();
  }
});

test("what the clip does not bake does not matter: the switch, the fps, enabled, the glass", () => {
  const motion = { ...theme.wallpaperMotion, source: "video", glass: "tint", fps: 30, enabled: true };
  expect(Wall.pick(shipped, theme.wallpaper, motion, scrim).entry).not.toBeNull();
});

test("another wallpaper, a path to one, or a different scrim has no clip", () => {
  expect(Wall.pick(shipped, "other.png", theme.wallpaperMotion, scrim).why).toBe("no clip for this wallpaper");
  expect(Wall.pick(shipped, "/media/x/" + theme.wallpaper, theme.wallpaperMotion, scrim).entry).toBeNull();
  expect(Wall.pick(shipped, theme.wallpaper, theme.wallpaperMotion, [0.2, 0.3, 0.6]).why).toBe("the scrim is not the clip's");
  expect(Wall.pick(shipped, theme.wallpaper, theme.wallpaperMotion, []).entry).toBeNull();
});

test("no manifest, an empty one, or one with nothing for the picture is a reason, not an error", () => {
  expect(Wall.pick(null, "x", {}, scrim)).toEqual({ entry: null, why: "no clips" });
  expect(Wall.pick({ walls: [] }, "x", {}, scrim)).toEqual({ entry: null, why: "no clips" });
  expect(Wall.pick({}, "x", {}, scrim).entry).toBeNull();
  expect(Wall.clipName(null)).toBe("");
});

test("the manifest says what the shell needs of the clip", () => {
  const w = shipped.walls[0];
  expect(w.file).toBe("wall-silk.vpc");
  expect([w.width, w.height, w.fps]).toEqual([1920, 1080, 60]);
  expect(w.source.file).toBe(theme.wallpaper);
  expect(w.glass.alpha).toBeGreaterThanOrEqual(theme.dark.readingAlpha);
  expect(w.glass.ratio).toBeGreaterThanOrEqual(7);
  expect(w.tint.tint).toMatch(/^#[0-9a-f]{6}$/);
  expect(w.tint.luma).toBeLessThan(theme.materialThreshold);
});
