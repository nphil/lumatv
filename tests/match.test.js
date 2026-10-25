// Run with: bun test tests/
// kit/Match.js is what the search sheet and the voice bar answer with: what was typed or said against the apps and the
// Settings pages the TV has. Local only; a web search is a row that opens the browser.
import { test, expect } from "bun:test";
import { readFileSync } from "node:fs";

const read = (p) => readFileSync(new URL(p, import.meta.url), "utf8");
const M = new Function(read("../payload/compositor/lumatv/kit/Match.js").replace(/^\.pragma library$/m, "")
  + "\nreturn { at, highlight, answers, webUrl, voice };")();

const APPS = [
  { id: "netflix", title: "Netflix", iconUrl: "n.png" }, { id: "plx", title: "PlxNative", iconUrl: "p.png" },
  { id: "lgm", title: "LG Movies & TV", iconUrl: "m.png" }, { id: "netx", title: "Net Tools", iconUrl: "t.png" },
  { id: "yt", title: "YouTube", iconUrl: "y.png" },
];
const PAGES = [
  { id: "network", title: "Network", sub: "Settings \u00b7 Nate's Home", icon: "wifi" },
  { id: "ai", title: "AI access", sub: "Settings \u00b7 AI access", icon: "bot" },
];

test("text starts at the title's start or at a word's, never inside a word", () => {
  expect(M.at("LG Movies & TV", "mov")).toBe(3);
  expect(M.at("Netflix", "net")).toBe(0);
  expect(M.at("Netflix", "flix")).toBe(-1);
  expect(M.at("Netflix", "")).toBe(-1);
  expect(M.at("YouTube", "YOU")).toBe(0);
});

test("answers: apps first, then pages, the web always last, four at most", () => {
  const a = M.answers("Net", APPS, PAGES);
  expect(a.map((x) => x.kind + ":" + x.id)).toEqual(["app:netflix", "app:netx", "page:network", "web:web"]);
  expect(a[3].title).toBe("Web search \u201cNet\u201d");
  expect(M.answers("  ", APPS, PAGES)).toEqual([]);
  expect(M.answers("zzz", APPS, PAGES).map((x) => x.kind)).toEqual(["web"]);
});

test("answers carry what the sheet draws: icon file for an app, glyph for a page and the web", () => {
  const a = M.answers("ai", APPS, PAGES);
  expect(a.map((x) => x.kind)).toEqual(["page", "web"]);
  expect(a[0].icon).toBe("bot");
  expect(M.answers("netf", APPS, PAGES)[0].iconUrl).toBe("n.png");
});

test("highlight marks the typed span of the title as it is written and escapes the rest", () => {
  expect(M.highlight("Netflix", "net", "#a79bf5")).toBe('<font color="#a79bf5">Net</font>flix');
  expect(M.highlight("LG Movies & TV", "mov", "#a79bf5")).toBe('LG <font color="#a79bf5">Mov</font>ies &amp; TV');
  expect(M.highlight("Netflix", "zzz", "#a79bf5")).toBe("Netflix");
  expect(M.highlight("<b>", "", "#a79bf5")).toBe("&lt;b&gt;");
});

test("the web search address carries the text, encoded", () => {
  expect(M.webUrl("https://duckduckgo.com/?q=", " series about chess ")).toBe("https://duckduckgo.com/?q=series%20about%20chess");
});

test("a command that names one app acts; anything else is a list that ends with the web", () => {
  expect(M.voice("open YouTube", APPS, PAGES).act.id).toBe("yt");
  expect(M.voice("launch lg movies", APPS, PAGES).act.id).toBe("lgm");
  const two = M.voice("open net", APPS, PAGES);         // two apps start with "net": a choice, not an act
  expect(two.act).toBeNull();
  expect(two.options.map((o) => o.id)).toEqual(["app:netflix", "app:netx", "web"]);
  const said = M.voice("netflix series about chess", APPS, PAGES);   // the app is named first: open it, or search the web
  expect(said.act).toBeNull();
  expect(said.options.map((o) => o.id)).toEqual(["app:netflix", "web"]);
  const page = M.voice("network", APPS, PAGES);
  expect(page.options.map((o) => o.id)).toEqual(["page:network", "web"]);
  expect(M.voice("what is the weather", APPS, PAGES).options.map((o) => o.id)).toEqual(["web"]);
});
