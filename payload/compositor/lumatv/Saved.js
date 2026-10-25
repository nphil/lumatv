.pragma library

// Saved-first cards. Weather and news draw from the last good reading on disk at a cold
// start, then refresh. The documents are plain JSON in {{dataDir}}/cache/weather.json and
// news.json, in the schema the native helper writes (helper/feeds), so whichever of the two
// fetched last is read by the other. Policy and shapes here are the helper's, number for number:
//
//   fresh    younger than 15 min: drawn, not refetched
//   stale    15 min to 6 h: drawn while a refetch runs
//   expired  6 h or more, or stamped more than 5 min in the future (the clock was wrong when it
//            was written): never drawn
//   retries  a failed refresh is tried again after 2, 10, then every 30 minutes

var SCHEMA = 1
var REFRESH_MS = 900000
var EXPIRE_MS = 21600000
var RETRY_MS = [120000, 600000, 1800000]
var FUTURE_SKEW_MS = 300000
var REFETCH_EARLY_MS = 5000
var MAX_BYTES = 1048576

// "fresh" | "stale" | "expired"
function freshness(fetchedAt, now) {
    var age = now - fetchedAt
    if (age < 0) {
        if (-age > FUTURE_SKEW_MS) return "expired"
        age = 0
    }
    if (age < REFRESH_MS) return "fresh"
    if (age < EXPIRE_MS) return "stale"
    return "expired"
}

function retryDelay(failures) {
    if (failures <= 0) return 0
    return RETRY_MS[Math.min(failures, RETRY_MS.length) - 1]
}

// When the next fetch is due, epoch ms. After failures: the last attempt plus the retry delay.
// Otherwise the reading's age reaching the refresh interval (15 min unless the card's settings
// say otherwise) minus 5 s; no reading at all is due at 0, which is now.
function nextAttempt(fetchedAt, failures, lastAttemptMs, refreshMs) {
    if (failures > 0) return lastAttemptMs + retryDelay(failures)
    if (!fetchedAt) return 0
    return fetchedAt + (refreshMs || REFRESH_MS) - REFETCH_EARLY_MS
}

// The parsed document, or null for anything that cannot be shown: no text, a file bigger than
// the helper reads, invalid JSON, not an object, or another schema.
function parse(text) {
    if (!text || text.length > MAX_BYTES) return null
    var doc
    try { doc = JSON.parse(text) } catch (e) { return null }
    if (!doc || typeof doc !== "object" || doc.schema !== SCHEMA) return null
    if (typeof doc.fetchedAt !== "number") return null
    return doc
}

function usable(doc, now) {
    return !!doc && freshness(doc.fetchedAt, now) !== "expired"
}

// A saved reading is for one place and unit; another zip or unit is a different reading.
function weatherUsable(doc, zip, units, now) {
    if (!usable(doc, now)) return false
    if (doc.zip !== zip || doc.units !== units) return false
    var d = doc.display
    return !!d && typeof d.temp === "string" && Array.isArray(d.days)
}

// Made for the same primary feed addresses in the same order.
function newsUsable(doc, feedUrls, now) {
    if (!usable(doc, now) || !Array.isArray(doc.feeds) || !Array.isArray(doc.items)) return false
    if (doc.feeds.length !== feedUrls.length) return false
    for (var i = 0; i < feedUrls.length; i++) if (doc.feeds[i] !== feedUrls[i]) return false
    return doc.items.length > 0
}

// The icon and text rules of the weather card, from a WMO weather code.
function kindFor(code, isDay) {
    if (code === 0) return isDay ? "sun" : "moon"
    if (code <= 2) return isDay ? "partly" : "partly-night"
    if (code === 3) return "cloud"
    if (code === 45 || code === 48) return "fog"
    if (code >= 95) return "storm"
    if ((code >= 71 && code <= 77) || code === 85 || code === 86) return "snow"
    if (code >= 61 || code >= 80) return "rain"
    if (code >= 51) return "drizzle"
    return "cloud"
}
function textFor(code) {
    if (code === 0) return "Clear"
    if (code === 1) return "Mostly clear"
    if (code === 2) return "Partly cloudy"
    if (code === 3) return "Overcast"
    if (code === 45 || code === 48) return "Fog"
    if (code >= 95) return "Thunderstorm"
    if ((code >= 71 && code <= 77) || code === 85 || code === 86) return "Snow"
    if (code >= 80) return "Showers"
    if (code >= 61) return "Rain"
    if (code >= 51) return "Drizzle"
    return "Cloudy"
}
function dayName(iso) { return ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"][new Date(iso + "T12:00:00").getDay()] }

// Open-Meteo's reply and the place it was asked for, as the document the helper stores, with
// the strings the card draws in `display`. A forecast day with a missing value is left out.
function weatherDoc(zip, units, geo, raw, now) {
    var c = raw.current, daily = [], rows = []
    for (var i = 0; i < raw.daily.time.length; i++) {
        var code = raw.daily.weather_code[i], hi = raw.daily.temperature_2m_max[i], lo = raw.daily.temperature_2m_min[i]
        if (code === null || hi === null || lo === null) continue
        daily.push({ date: raw.daily.time[i], code: code, hi: hi, lo: lo })
        rows.push({ day: rows.length === 0 && i === 0 ? "Today" : dayName(raw.daily.time[i]), kind: kindFor(code, true),
                    hi: Math.round(hi) + "\u00b0", lo: Math.round(lo) + "\u00b0" })
    }
    return {
        schema: SCHEMA, fetchedAt: now, staleAt: now + REFRESH_MS, expiresAt: now + EXPIRE_MS,
        zip: zip, units: units, place: geo.place, lat: geo.lat, lon: geo.lon,
        current: { time: c.time, temp: c.temperature_2m, feelsLike: c.apparent_temperature,
                   code: c.weather_code, isDay: c.is_day === 1, humidity: c.relative_humidity_2m,
                   windMph: c.wind_speed_10m },
        daily: daily,
        display: {
            place: geo.place,
            temp: Math.round(c.temperature_2m) + "\u00b0",
            cond: textFor(c.weather_code),
            stats: "Feels like " + Math.round(c.apparent_temperature) + "\u00b0 \u00b7 " + c.relative_humidity_2m + "% humidity \u00b7 " + Math.round(c.wind_speed_10m) + " mph",
            nowKind: kindFor(c.weather_code, c.is_day === 1),
            days: rows
        }
    }
}

// Stories as the card draws them, for the primary feed addresses in order. `pubMs` is the
// publication time as epoch ms, or null when the feed's text cannot be read.
function newsDoc(feedUrls, items, now) {
    var out = []
    for (var i = 0; i < items.length; i++) {
        var n = items[i], t = Date.parse(n.pub)
        out.push({ tag: n.tag, src: n.src, title: n.title, summary: n.summary, link: n.link,
                   img: n.img, pub: n.pub, pubMs: isNaN(t) ? null : t })
    }
    return { schema: SCHEMA, fetchedAt: now, staleAt: now + REFRESH_MS, expiresAt: now + EXPIRE_MS,
             feeds: feedUrls, items: out }
}
