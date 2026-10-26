.pragma library

// The art saver's shuffle bag, kept across restarts. One small file in the data directory holds a deck for each collection of prints, keyed by
// the folder of its index. A deck is
//   bag    the ids still to deal in this round, in the order they come,
//   known  the ids the collection had when it was saved: a print that has come since is dealt into the bag, one that was shown is not dealt again,
//   last   the id dealt last: a new round does not begin with it,
//   at     when it was saved: past MAX_DECKS collections the oldest is let go.
// Ids, not positions, because the index can change between runs. A print whose file is gone is dropped. A file that cannot be read, is of
// another schema or is not an object counts as none: the bag starts over, as it did before it was kept.

var SCHEMA = 1
var MAX_DECKS = 8
var MAX_IDS = 2000          // a collection too big to be a small file is not kept
var MAX_BYTES = 262144

function empty() { return { schema: SCHEMA, decks: {} } }

// The document in the file, or an empty one for anything that is not ours.
function parse(text) {
    var doc = empty()
    if (!text || text.length > MAX_BYTES) return doc
    var got
    try { got = JSON.parse(text) } catch (e) { return doc }
    if (!got || typeof got !== "object" || got.schema !== SCHEMA || !got.decks || typeof got.decks !== "object" || Array.isArray(got.decks)) return doc
    for (var key in got.decks) {
        var d = got.decks[key]
        if (key !== "__proto__" && d && typeof d === "object" && Array.isArray(d.bag) && typeof d.at === "number") doc.decks[key] = d
    }
    return doc
}

// The collection's name in the file: the folder of its index, below the module's own directory for the prints that ship with the set
// ("art/"), else the folder's own URL.
function collectionKey(indexUrl, baseUrl) {
    var dir = String(indexUrl || "").replace(/[^\/]*$/, "")
    var base = String(baseUrl || "")
    var rel = base !== "" && dir.indexOf(base) === 0 ? dir.slice(base.length) : dir
    return rel === "" ? "./" : rel
}

// A new round of `n` prints in a random order, not beginning with one of `avoid` (the positions last shown) when another can.
function draw(n, avoid, rand) {
    var b = []
    var i
    for (i = 0; i < n; i++) b.push(i)
    for (var j = n - 1; j > 0; j--) { var k = Math.floor(rand() * (j + 1)); var t = b[j]; b[j] = b[k]; b[k] = t }
    for (i = 0; i < b.length; i++) {
        if (avoid.indexOf(b[i]) < 0) { var s = b[0]; b[0] = b[i]; b[i] = s; break }
    }
    return b
}

// What the file says about `key`, as positions in `pictures`: { bag, last }. `exists(picture)` says whether the print's file is there.
function restore(doc, key, pictures, exists, rand) {
    var out = { bag: [], last: -1 }
    var d = doc.decks[key]
    if (!d) return out
    var at = {}
    var i
    for (i = 0; i < pictures.length; i++) if (at["k" + pictures[i].id] === undefined) at["k" + pictures[i].id] = i
    var taken = {}
    for (i = 0; i < d.bag.length; i++) {
        var p = typeof d.bag[i] === "string" ? at["k" + d.bag[i]] : undefined
        if (p !== undefined && !taken[p] && exists(pictures[p])) { taken[p] = true; out.bag.push(p) }
    }
    if (Array.isArray(d.known)) {
        var known = {}
        for (i = 0; i < d.known.length; i++) if (typeof d.known[i] === "string") known["k" + d.known[i]] = true
        for (i = 0; i < pictures.length; i++) {
            if (known["k" + pictures[i].id] || taken[i] || at["k" + pictures[i].id] !== i || !exists(pictures[i])) continue
            taken[i] = true
            out.bag.splice(Math.floor(rand() * (out.bag.length + 1)), 0, i)
        }
    }
    if (typeof d.last === "string" && at["k" + d.last] !== undefined) out.last = at["k" + d.last]
    return out
}

// The deck to save: the bag and the collection as ids, and the print dealt last.
function deckOf(pictures, bag, last, now) {
    return {
        at: now,
        known: pictures.map(function(p) { return p.id }),
        bag: bag.map(function(i) { return pictures[i].id }),
        last: last >= 0 && pictures[last] ? pictures[last].id : ""
    }
}

// `deck` as the one for `key`; the oldest collections go when there are more than MAX_DECKS.
function put(doc, key, deck) {
    if (deck.known.length > MAX_IDS) { delete doc.decks[key]; return doc }
    doc.decks[key] = deck
    var keys = Object.keys(doc.decks)
    if (keys.length > MAX_DECKS) {
        keys.sort(function(a, b) { return doc.decks[b].at - doc.decks[a].at })
        for (var i = MAX_DECKS; i < keys.length; i++) delete doc.decks[keys[i]]
    }
    return doc
}

function stringify(doc) { return JSON.stringify(doc) }
