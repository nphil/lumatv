// Run with: bun test tests/
import { test, expect } from "bun:test";
import { readFileSync } from "node:fs";

const src = readFileSync(new URL("../payload/compositor/lumatv/Tint.js", import.meta.url), "utf8").replace(/^\.pragma library$/m, "");
const Tint = new Function(src + "\nreturn { dominant, luma, accentFrom, hex, parseHex, parseXpm, mixToward, rgbToHsl, hslToRgb, luminance, contrast, bestInk };")();

function bitmap(pixels) {
  const data = new Uint8Array(pixels.length * 4);
  pixels.forEach(([r, g, b, a = 255], i) => { data.set([r, g, b, a], i * 4); });
  return data;
}
const fill = (n, c) => Array.from({ length: n }, () => c);

test("the most common bin wins and its centre is the tint", () => {
  const px = [...fill(300, [200, 40, 40]), ...fill(200, [40, 40, 200]), ...fill(76, [128, 128, 128])];
  const t = Tint.dominant(bitmap(px), px.length);
  // 200 >> 5 = 6 -> 6 * 32 + 16 = 208; 40 >> 5 = 1 -> 48
  expect([t.r, t.g, t.b]).toEqual([208, 48, 48]);
  expect(t.share).toBeCloseTo(300 / 576, 5);
});

test("near-black is skipped even when it is most of the picture", () => {
  const px = [...fill(500, [10, 12, 20]), ...fill(76, [70, 60, 160])];
  const t = Tint.dominant(bitmap(px), px.length);
  expect([t.r, t.g, t.b]).toEqual([80, 48, 176]);
});

test("a bin just above black counts: one channel at 32 is not near-black", () => {
  const t = Tint.dominant(bitmap([[31, 31, 31], [32, 0, 0]]), 2);
  expect([t.r, t.g, t.b]).toEqual([48, 16, 16]);
});

test("transparent pixels are skipped; a bitmap with nothing left has no tint", () => {
  expect(Tint.dominant(bitmap(fill(10, [200, 100, 50, 0])), 10)).toBeNull();
  expect(Tint.dominant(bitmap(fill(10, [0, 0, 0])), 10)).toBeNull();
  const t = Tint.dominant(bitmap([[200, 100, 50, 127], [100, 200, 50, 128]]), 2);
  expect([t.r, t.g, t.b]).toEqual([112, 208, 48]);
});

test("a tie goes to the lower bin, whatever the pixel order", () => {
  const a = Tint.dominant(bitmap([[200, 40, 40], [40, 40, 200]]), 2);
  const b = Tint.dominant(bitmap([[40, 40, 200], [200, 40, 40]]), 2);
  expect(a).toEqual(b);
  expect([a.r, a.g, a.b]).toEqual([48, 48, 208]);
});

test("mean luma of the opaque pixels", () => {
  expect(Tint.luma(bitmap(fill(4, [255, 255, 255])), 4)).toBeCloseTo(1, 5);
  expect(Tint.luma(bitmap(fill(4, [0, 0, 0])), 4)).toBeCloseTo(0, 5);
  expect(Tint.luma(bitmap([[255, 255, 255], [0, 0, 0], [255, 0, 0, 0]]), 3)).toBeCloseTo(0.5, 5);
  expect(Tint.luma(bitmap(fill(3, [9, 9, 9, 0])), 3)).toBe(-1);
});

test("the accent keeps the tint's hue and is set to a legible lightness", () => {
  const dark = Tint.accentFrom({ r: 48, g: 48, b: 208 }, false);
  const light = Tint.accentFrom({ r: 48, g: 48, b: 208 }, true);
  const hd = Tint.rgbToHsl(dark.r, dark.g, dark.b), hl = Tint.rgbToHsl(light.r, light.g, light.b), h0 = Tint.rgbToHsl(48, 48, 208);
  expect(hd.h).toBeCloseTo(h0.h, 1);
  expect(hl.h).toBeCloseTo(h0.h, 1);
  expect(hd.l).toBeCloseTo(0.74, 1);
  expect(hl.l).toBeCloseTo(0.42, 1);
  // a grey wallpaper still gets a coloured accent: saturation has a floor
  const grey = Tint.accentFrom({ r: 80, g: 80, b: 80 }, false);
  expect(Tint.rgbToHsl(grey.r, grey.g, grey.b).s).toBeGreaterThan(0.3);
});

test("a saved tint reads back as written", () => {
  const c = { r: 208, g: 48, b: 16 };
  expect(Tint.parseHex(Tint.hex(c))).toEqual(c);
  expect(Tint.parseHex("#D03010")).toEqual(c);
  expect(["", "d03010", "#d0301", "#d03010ff", "#gg0000", undefined].map(Tint.parseHex)).toEqual([null, null, null, null, null, null]);
});

test("hex and mixing", () => {
  expect(Tint.hex({ r: 208, g: 48, b: 16 })).toBe("#d03010");
  expect(Tint.hex({ r: 300, g: -4, b: 0.4 })).toBe("#ff0000");
  const m = Tint.mixToward({ r: 0.2, g: 0.2, b: 0.2, a: 0.56 }, { r: 255, g: 0, b: 0 }, 0.5);
  expect(m.r).toBeCloseTo(0.6, 5);
  expect(m.g).toBeCloseTo(0.1, 5);
  expect(m.a).toBe(0.56);
  expect(Tint.mixToward({ r: 0.2, g: 0.2, b: 0.2, a: 1 }, { r: 255, g: 0, b: 0 }, 0)).toEqual({ r: 0.2, g: 0.2, b: 0.2, a: 1 });
});

// What QImage::save(".xpm") writes: one palette entry per distinct colour, then a string per row.
function xpm(width, height, pixels, cpp = 2) {
  const codes = [];
  const palette = new Map();
  const chars = " .XoO+@#$%&*=-;>,')!~{]^/(_:<[}|1234567890abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ`";
  for (const p of pixels) {
    const k = p.join(",");
    if (!palette.has(k)) {
      const n = palette.size;
      palette.set(k, chars[Math.floor(n / chars.length) % chars.length] + chars[n % chars.length]);
    }
  }
  const rows = [];
  for (let y = 0; y < height; y++) rows.push('"' + pixels.slice(y * width, (y + 1) * width).map((p) => palette.get(p.join(","))).join("") + '"');
  const colours = [...palette].map(([k, code]) => {
    const [r, g, b] = k.split(",").map(Number);
    return `"${code} c #${[r, g, b].map((v) => v.toString(16).padStart(2, "0")).join("")}"`;
  });
  return `/* XPM */\nstatic const char * grab[] = {\n"${width} ${height} ${palette.size} ${cpp}",\n${[...colours, ...rows].join(",\n")}};\n`;
}

test("a grab saved as XPM reads back pixel for pixel, spaces in the palette codes included", () => {
  const px = [[208, 48, 16], [10, 12, 20], [10, 12, 20], [80, 80, 112], [208, 48, 16], [10, 12, 20]];
  const parsed = Tint.parseXpm(xpm(3, 2, px));
  expect([parsed.width, parsed.height, parsed.count]).toEqual([3, 2, 6]);
  const flat = px.flatMap(([r, g, b]) => [r, g, b, 255]);
  expect(parsed.data).toEqual(flat);
  // the first palette code is two spaces, so the line starts with three; the rows depend on it
  expect(xpm(3, 2, px)).toContain('"   c #d03010"');
  const t = Tint.dominant(parsed.data, parsed.count);
  expect([t.r, t.g, t.b]).toEqual([208, 48, 16]);
});

test("a transparent entry reads as transparent and XPM junk reads as null", () => {
  const text = '/* XPM */\nstatic const char * g[] = {\n"2 1 2 1",\n"a c None",\n"b c #ffffff",\n"ab"};\n';
  expect(Tint.parseXpm(text).data).toEqual([0, 0, 0, 0, 255, 255, 255, 255]);
  expect(Tint.parseXpm("")).toBeNull();
  expect(Tint.parseXpm("not an xpm")).toBeNull();
  expect(Tint.parseXpm('"2 2 1 1",\n"a c #000000",\n"aa"')).toBeNull();
});

test("contrast follows WCAG: black on white is 21, a colour on itself is 1", () => {
  expect(Tint.contrast({ r: 0, g: 0, b: 0 }, { r: 1, g: 1, b: 1 })).toBeCloseTo(21, 5);
  expect(Tint.contrast({ r: 0.3, g: 0.5, b: 0.7 }, { r: 0.3, g: 0.5, b: 0.7 })).toBeCloseTo(1, 5);
  // the dark ink on the shipped dark accent: about 7, as the palette's 4.5:1 target needs
  const c = (hex) => { const t = Tint.parseHex(hex); return { r: t.r / 255, g: t.g / 255, b: t.b / 255 }; };
  expect(Tint.contrast(c("#A79BF5"), c("#17131F"))).toBeGreaterThan(6.5);
  expect(Tint.contrast(c("#A79BF5"), c("#F2EFF6"))).toBeLessThan(3);
});

test("the ink with the most contrast wins, the first on a tie", () => {
  const c = (hex) => { const t = Tint.parseHex(hex); return { r: t.r / 255, g: t.g / 255, b: t.b / 255 }; };
  const dark = c("#17131F"), light = c("#F2EFF6");
  expect(Tint.bestInk(c("#A79BF5"), [light, dark])).toBe(dark);
  expect(Tint.bestInk(c("#2A2540"), [dark, light])).toBe(light);
  expect(Tint.bestInk(c("#808080"), [dark])).toBe(dark);
});
