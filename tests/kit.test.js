// Run with: bun test tests/
// The kit reads its tokens through the singleton Lu (payload/compositor/lumatv/kit/Lu.qml). QML
// reports a misspelt singleton member only at run time, as an undefined, so this checks that every
// Lu.<name> any file uses is a property or function Lu declares, and that every kit type is
// listed in the kit's qmldir (a file missing from it is invisible to `import "kit"`).
import { test, expect } from "bun:test";
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

const root = new URL("../payload/compositor/lumatv/", import.meta.url).pathname;
const read = (p) => readFileSync(join(root, p), "utf8");
const lu = read("kit/Lu.qml");
const declared = new Set([
  ...[...lu.matchAll(/(?:readonly )?property \w+ (\w+)/g)].map((m) => m[1]),
  ...[...lu.matchAll(/function (\w+)\(/g)].map((m) => m[1]),
]);

function qmlFiles(dir) {
  return readdirSync(join(root, dir), { withFileTypes: true }).flatMap((e) =>
    e.isDirectory() ? qmlFiles(join(dir, e.name)) : e.name.endsWith(".qml") ? [join(dir, e.name)] : []);
}

test("every Lu member used anywhere is declared by Lu", () => {
  const files = qmlFiles(".").filter((f) => f !== "kit/Lu.qml");
  const used = new Map();
  for (const f of files) for (const m of read(f).matchAll(/\bLu\.(?!qml\b)(\w+)/g)) used.set(m[1], f);
  const missing = [...used].filter(([name]) => !declared.has(name)).map(([name, f]) => `${name} (${f})`);
  expect(missing).toEqual([]);
  expect(used.size).toBeGreaterThan(20);
});

test("every kit type is in the qmldir, and the qmldir lists only files that exist", () => {
  const qmldir = read("kit/qmldir").trim().split("\n");
  const listed = qmldir.map((l) => l.split(/\s+/).pop()).sort();
  const files = readdirSync(join(root, "kit")).filter((f) => f.endsWith(".qml")).sort();
  expect(listed).toEqual(files);
  expect(qmldir[0]).toBe("singleton Lu 1.0 Lu.qml");
});

test("kit files that use Lu import the directory so the singleton is visible", () => {
  for (const f of readdirSync(join(root, "kit")).filter((n) => n.endsWith(".qml") && n !== "Lu.qml")) {
    const text = read(join("kit", f));
    if (/\bLu\./.test(text)) expect(text.includes('import "."')).toBe(true);
  }
});

test("Lu reads its tokens from theme.json: every key it asks the theme for exists in the shipped file", () => {
  const theme = JSON.parse(readFileSync(new URL("../payload/theme/theme.json", import.meta.url), "utf8"));
  const roles = [...lu.matchAll(/role\("(\w+)"\)/g)].map((m) => m[1]);
  for (const r of roles) {
    expect(typeof theme.dark[r]).toBe("string");
    expect(typeof theme.light[r]).toBe("string");
  }
  for (const key of ["font", "type", "radii", "space", "target", "row", "focus", "states", "motion"]) expect(theme[key]).toBeDefined();
  for (const t of ["display", "title", "body", "label", "caption"]) expect(theme.type[t].size).toBeGreaterThan(0);
  for (const r of ["sheet", "card", "tile", "row", "control", "pill"]) expect(theme.radii[r]).toBeGreaterThan(0);
  for (const k of ["focusMs", "cardMs", "layerMs", "pressMs", "restMs", "labelMs"]) expect(theme.motion[k]).toBeGreaterThan(0);
});
