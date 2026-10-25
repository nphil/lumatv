.pragma library

// What the search sheet and the voice bar share: what was typed or said against what is on the set. Everything here is
// local (PLAN: search and voice answer from the apps and the Settings pages the TV has; a web search is only a row that
// opens the browser), so it is a handful of pure functions over lists the host gives.
//
//   apps  = [{ id, title, iconUrl }]          pages = [{ id, title, sub, icon }]

function norm(s) { return String(s === undefined || s === null ? "" : s).toLowerCase().replace(/\s+/g, " ").trim() }

// Where `text` starts inside `title`: at the title's start or at the start of one of its words ("mov" finds "LG Movies &
// TV", "net" finds "Network" and "Netflix", "flix" finds nothing). -1 when it does not.
function at(title, text) {
    var t = norm(title), q = norm(text)
    if (q === "") return -1
    var i = t.indexOf(q)
    while (i >= 0) {
        if (i === 0 || /[^a-z0-9]/.test(t.charAt(i - 1))) return i
        i = t.indexOf(q, i + 1)
    }
    return -1
}

function escapeHtml(s) { return String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;") }

// `title` as StyledText with the span `text` starts (at the title's start or a word's, as at() finds it) in `color`: the "Net"
// of "Netflix". Cut from the original by its own offsets, so the title is shown as it is written; unchanged when nothing
// matches.
function highlight(title, text, color) {
    var plain = String(title === undefined || title === null ? "" : title)
    var q = String(text === undefined || text === null ? "" : text).toLowerCase().trim()
    var lower = plain.toLowerCase()
    if (q === "" || lower.length !== plain.length) return escapeHtml(plain)
    var i = lower.indexOf(q)
    while (i > 0 && /[a-z0-9]/.test(lower.charAt(i - 1))) i = lower.indexOf(q, i + 1)
    if (i < 0) return escapeHtml(plain)
    return escapeHtml(plain.substring(0, i)) + "<font color=\"" + color + "\">" + escapeHtml(plain.substring(i, i + q.length)) + "</font>" + escapeHtml(plain.substring(i + q.length))
}

// The answers to a typed text: up to two apps, then Settings pages, then the web, four at most. Empty text has none.
function answers(text, apps, pages) {
    var q = norm(text), out = []
    if (q === "") return out
    var a = 0, p = 0
    for (var i = 0; i < apps.length && a < 2; i++)
        if (at(apps[i].title, q) >= 0) { out.push({ kind: "app", id: apps[i].id, title: apps[i].title, sub: "App \u00b7 open", iconUrl: apps[i].iconUrl, match: q }); a++ }
    for (var j = 0; j < pages.length && out.length < 3 && p < 2; j++)
        if (at(pages[j].title, q) >= 0) { out.push({ kind: "page", id: pages[j].id, title: pages[j].title, sub: pages[j].sub || "Settings", icon: pages[j].icon || "settings", match: q }); p++ }
    out.push({ kind: "web", id: "web", title: "Web search \u201c" + String(text).trim() + "\u201d", sub: "Opens the browser", icon: "globe", match: "" })
    return out.slice(0, 4)
}

// The browser's address for a web search: `base` is the engine's query address ("https://duckduckgo.com/?q=").
function webUrl(base, text) { return base + encodeURIComponent(String(text).trim()) }

// What was said. A command that names one app ("open Plex") acts at once: `act` is that app. Anything else is a list of at
// most three things the words can do, always ending with the web: an app whose name the words begin with or are the start
// of, a Settings page, "Search the web". Searching inside an app is not offered (an app's own search is its own).
function voice(words, apps, pages) {
    var said = norm(words)
    var cmd = /^(?:open|launch|start|go to|switch to)\s+(.+)$/.exec(said)
    var q = cmd ? cmd[1] : said
    var hits = [], i
    for (i = 0; i < apps.length; i++) {
        var t = norm(apps[i].title)
        if (t === q || at(apps[i].title, q) >= 0 || (t !== "" && (q === t || q.indexOf(t + " ") === 0))) hits.push(apps[i])
    }
    if (cmd && hits.length === 1) return { act: hits[0], options: [] }
    var out = []
    for (i = 0; i < hits.length && out.length < 2; i++) out.push({ id: "app:" + hits[i].id, title: "Open " + hits[i].title, sub: "App", iconUrl: hits[i].iconUrl })
    for (i = 0; i < pages.length && out.length < 2; i++)
        if (at(pages[i].title, q) >= 0) out.push({ id: "page:" + pages[i].id, title: pages[i].title + " settings", sub: pages[i].sub || "Settings", icon: pages[i].icon || "settings" })
    out.push({ id: "web", title: "Search the web", sub: "Opens the browser", icon: "globe" })
    return { act: null, options: out.slice(0, 3) }
}
