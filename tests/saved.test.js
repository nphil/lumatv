// Run with: bun test tests/
// Saved.js is the QML side of the saved-first cache; its numbers and its documents are the
// native helper's (helper/feeds). The weather cases use the helper's own fixtures and the
// values its tests expect.
import { test, expect } from "bun:test";
import { readFileSync } from "node:fs";

const read = (p) => readFileSync(new URL(p, import.meta.url), "utf8");
const Saved = new Function(read("../payload/compositor/lumatv/Saved.js").replace(/^\.pragma library$/m, "")
  .replace(/{{dataDir}}/g, "/var/lib/lumatv") + "\nreturn { SCHEMA, REFRESH_MS, EXPIRE_MS, RETRY_MS, freshness, retryDelay, nextAttempt, parse, weatherUsable, newsUsable, weatherDoc, newsDoc, kindFor, textFor };")();

const T0 = 1_792_900_000_000;
const forecast = JSON.parse(read("../helper/feeds/tests/fixtures/open-meteo.json"));
const geo = { place: "Atlanta", lat: 33.723, lon: -84.4702 };

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

test("retries come after 2, 10 and then every 30 minutes", () => {
  expect(Saved.RETRY_MS).toEqual([120_000, 600_000, 1_800_000]);
  expect([0, 1, 2, 3, 4, 99].map(Saved.retryDelay)).toEqual([0, 120_000, 600_000, 1_800_000, 1_800_000, 1_800_000]);
});

test("the next attempt: reading age minus 5 s, the last attempt plus the retry delay, or now", () => {
  expect(Saved.nextAttempt(T0, 0, T0 + 1)).toBe(T0 + 895_000);
  expect(Saved.nextAttempt(0, 0, T0)).toBe(0);
  expect(Saved.nextAttempt(T0, 1, T0 + 5_000)).toBe(T0 + 5_000 + 120_000);
  expect(Saved.nextAttempt(0, 3, T0)).toBe(T0 + 1_800_000);
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

test("the weather document from the helper's fixture matches its tests", () => {
  const doc = Saved.weatherDoc("30311", "fahrenheit", geo, forecast, T0);
  expect([doc.schema, doc.fetchedAt, doc.staleAt, doc.expiresAt]).toEqual([1, T0, T0 + 900_000, T0 + 21_600_000]);
  expect([doc.zip, doc.units, doc.place, doc.lat, doc.lon]).toEqual(["30311", "fahrenheit", "Atlanta", 33.723, -84.4702]);
  expect(doc.current).toEqual({ time: "2026-10-25T00:15", temp: 54.2, feelsLike: 49.7, code: 0, isDay: false, humidity: 72, windMph: 7.8 });
  expect(doc.display.place).toBe("Atlanta");
  expect(doc.display.temp).toBe("54\u00b0");
  expect(doc.display.cond).toBe("Clear");
  expect(doc.display.stats).toBe("Feels like 50\u00b0 \u00b7 72% humidity \u00b7 8 mph");
  expect(doc.display.nowKind).toBe("moon");
  const want = [
    ["2026-10-25", 0, 69.8, 49.4, "Today", "sun", "70", "49"],
    ["2026-10-26", 3, 72.1, 49.9, "Mon", "cloud", "72", "50"],
    ["2026-10-27", 3, 76.0, 55.8, "Tue", "cloud", "76", "56"],
    ["2026-10-28", 3, 72.6, 56.0, "Wed", "cloud", "73", "56"],
    ["2026-10-29", 3, 75.7, 57.9, "Thu", "cloud", "76", "58"],
    ["2026-10-30", 3, 77.5, 57.3, "Fri", "cloud", "78", "57"],
    ["2026-10-31", 3, 75.4, 57.5, "Sat", "cloud", "75", "58"],
  ];
  expect(doc.daily.length).toBe(7);
  expect(doc.display.days.length).toBe(7);
  want.forEach(([date, code, hi, lo, day, kind, rhi, rlo], i) => {
    expect(doc.daily[i]).toEqual({ date, code, hi, lo });
    expect(doc.display.days[i]).toEqual({ day, kind, hi: rhi + "\u00b0", lo: rlo + "\u00b0" });
  });
});

test("the stored keys are exactly the helper's", () => {
  const doc = Saved.weatherDoc("30311", "fahrenheit", geo, forecast, T0);
  expect(Object.keys(doc).sort()).toEqual(["current", "daily", "display", "expiresAt", "fetchedAt", "lat", "lon", "place", "schema", "staleAt", "units", "zip"]);
  expect(Object.keys(doc.current).sort()).toEqual(["code", "feelsLike", "humidity", "isDay", "temp", "time", "windMph"]);
  expect(Object.keys(doc.display).sort()).toEqual(["cond", "days", "nowKind", "place", "stats", "temp"]);
});

test("rounding and rows follow the card: halves round up, a day with a missing value is left out", () => {
  const raw = {
    current: { time: "2026-10-25T12:00", temperature_2m: -2.5, apparent_temperature: -7.5, weather_code: 71, is_day: 1, relative_humidity_2m: 72.5, wind_speed_10m: 0.4 },
    daily: { time: ["2026-10-25", "2026-10-26", "2026-10-27"], weather_code: [71, null, 3], temperature_2m_max: [1.5, 2.0, 3.0], temperature_2m_min: [-3.5, -2.0, null] },
  };
  const doc = Saved.weatherDoc("30311", "fahrenheit", geo, raw, T0);
  expect(doc.display.temp).toBe("-2\u00b0");
  expect(doc.display.stats).toBe("Feels like -7\u00b0 \u00b7 72.5% humidity \u00b7 0 mph");
  expect(doc.display.nowKind).toBe("snow");
  expect(doc.display.cond).toBe("Snow");
  expect(doc.daily.length).toBe(1);
  expect(doc.display.days[0]).toEqual({ day: "Today", kind: "snow", hi: "2\u00b0", lo: "-3\u00b0" });
});

test("icon and text rules", () => {
  const kind = (c, d) => Saved.kindFor(c, d);
  expect([kind(0, true), kind(0, false), kind(2, true), kind(2, false), kind(3, true), kind(45, true), kind(95, true), kind(73, true), kind(85, true), kind(65, true), kind(81, true), kind(53, true), kind(10, true)])
    .toEqual(["sun", "moon", "partly", "partly-night", "cloud", "fog", "storm", "snow", "snow", "rain", "rain", "drizzle", "cloud"]);
  expect([0, 1, 2, 3, 45, 96, 75, 82, 63, 55, 10].map(Saved.textFor))
    .toEqual(["Clear", "Mostly clear", "Partly cloudy", "Overcast", "Fog", "Thunderstorm", "Snow", "Showers", "Rain", "Drizzle", "Cloudy"]);
});

test("a saved weather reading is for one place and unit and must not be expired", () => {
  const doc = Saved.weatherDoc("30311", "fahrenheit", geo, forecast, T0);
  expect(Saved.weatherUsable(doc, "30311", "fahrenheit", T0 + 3_600_000)).toBe(true);
  expect(Saved.weatherUsable(doc, "10001", "fahrenheit", T0)).toBe(false);
  expect(Saved.weatherUsable(doc, "30311", "celsius", T0)).toBe(false);
  expect(Saved.weatherUsable(doc, "30311", "fahrenheit", T0 + 21_600_000)).toBe(false);
  expect(Saved.weatherUsable(null, "30311", "fahrenheit", T0)).toBe(false);
  expect(Saved.weatherUsable({ ...doc, display: undefined }, "30311", "fahrenheit", T0)).toBe(false);
});

test("the news document keeps the feed's own text and a parsed time", () => {
  const items = [
    { tag: "WORLD", src: "BBC News", title: "A", summary: "s", link: "https://x/a", img: "https://i/a.jpg", pub: "Sun, 25 Oct 2026 04:30:00 GMT" },
    { tag: "U.S.", src: "BBC News", title: "B", summary: "", link: "", img: "", pub: "someday" },
  ];
  const feeds = ["https://feeds.bbci.co.uk/news/world/rss.xml", "https://feeds.bbci.co.uk/news/world/us_and_canada/rss.xml"];
  const doc = Saved.newsDoc(feeds, items, T0);
  expect(Object.keys(doc).sort()).toEqual(["expiresAt", "feeds", "fetchedAt", "items", "schema", "staleAt"]);
  expect(Object.keys(doc.items[0]).sort()).toEqual(["img", "link", "pub", "pubMs", "src", "summary", "tag", "title"]);
  expect(doc.items[0].pubMs).toBe(Date.UTC(2026, 9, 25, 4, 30, 0));
  expect(doc.items[1].pubMs).toBeNull();
  expect(Saved.newsUsable(doc, feeds, T0 + 1000)).toBe(true);
  expect(Saved.newsUsable(doc, [feeds[1], feeds[0]], T0)).toBe(false);
  expect(Saved.newsUsable(doc, [feeds[0]], T0)).toBe(false);
  expect(Saved.newsUsable(doc, feeds, T0 + 21_600_000)).toBe(false);
  expect(Saved.newsUsable({ ...doc, items: [] }, feeds, T0)).toBe(false);
});
