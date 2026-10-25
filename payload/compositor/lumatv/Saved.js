.pragma library

// Saved-first cards. Weather and news fetch in the native helper (helper/feeds), which keeps its last
// good reading in {{dataDir}}/cache/weather.json and news.json and serves it on the loopback API
// (kit/Helper.qml). The cards draw from that file at a cold start, before the helper has answered,
// then from the API. Policy is the helper's, number for number:
//
//   fresh    younger than 15 min
//   stale    15 min to 6 h: drawn while the helper refetches
//   expired  6 h or more, or stamped more than 5 min in the future (the clock was wrong when it
//            was written): never drawn

var SCHEMA = 1
var REFRESH_MS = 900000
var EXPIRE_MS = 21600000
var FUTURE_SKEW_MS = 300000
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

// The helper drops a reading for another place or unit when the setting changes, so the document
// itself says what it is for.
function weatherUsable(doc, now) {
    if (!usable(doc, now)) return false
    var d = doc.display
    return !!d && typeof d.temp === "string" && Array.isArray(d.days)
}

function newsUsable(doc, now) {
    return usable(doc, now) && Array.isArray(doc.items) && doc.items.length > 0
}
