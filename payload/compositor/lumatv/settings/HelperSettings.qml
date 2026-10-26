import QtQuick 2.4
import "../kit"

// One owner of the helper's settings layer (`GET/POST /v1/settings/<owner>`, helper/app/src/settings.rs), read and written
// the way Sio reads and writes LG's settings, so the same row builders (Rows.js) draw both: a row names a field by its key,
// `get` gives the effective value (what the helper applies: the stored one, else the schema default), and `write` sends one
// change as a transaction that the helper checks against the owner's schema (type, range, step, listed choices). The value
// shown after a write is the one the helper answered with; a refused change leaves the row as it was.
//
// `known` is false until the helper answers with this owner (it is down, or an older one does not have it): the rows dim as
// "No answer from the TV", as a row of LG's settings does until its service answers. Nothing is ever `missing`: a field the
// schema lacks is one an older helper does not have yet, and the row stays dimmed, not hidden.
QtObject {
    id: hs

    property string owner: ""
    property bool known: false
    property int version: -1
    property var values: ({})          // key -> the effective value
    property var kinds: ({})           // key -> field kind ("bool", "integer", "choice", ...)
    property var schema: ({})          // the owner's page as the helper serves it: {title, sections: [{id, title, fields}]}
    property string problem: ""        // the helper's words for the last refused change

    function has(key) { return known && values[key] !== undefined }
    function usable(key) { return has(key) }
    function missing(key) { return false }
    function get(key) { return values[key] }
    function num(key, fallback) { var v = Number(values[key]); return isNaN(v) ? (fallback === undefined ? 0 : fallback) : v }

    function take(doc) {
        var eff = doc.effective || {}, v = {}
        for (var k in eff) v[k] = eff[k].value
        var kinds2 = {}
        var secs = (doc.schema && doc.schema.sections) || []
        for (var i = 0; i < secs.length; i++) for (var j = 0; j < secs[i].fields.length; j++) kinds2[secs[i].fields[j].key] = secs[i].fields[j].kind.type
        values = v
        kinds = kinds2
        schema = doc.schema || {}
        version = doc.version
        known = true
    }
    function refresh() {
        Helper.request("GET", "/v1/settings/" + owner, undefined, function(status, doc) {
            if (doc && doc.ok) take(doc)
            else known = false
        })
    }

    // `value` as the row builders hand it (a string from a switch or a segment, a number from a slider).
    function typed(key, value) {
        var kind = kinds[key]
        if (kind === "bool") return { type: "bool", value: value === true || String(value) === "true" }
        if (kind === "integer") return { type: "integer", value: Math.round(Number(value)) }
        if (kind === "ordered-set") return { type: "ordered", value: value }     // a list of ids, in the order they were picked
        return { type: kind, value: String(value) }
    }
    // Changes go one at a time, in the order they were made: two answers racing could leave the older value in force (a
    // slider held down makes several in a second). A change to a key still waiting is replaced by the newer one.
    property var queue: []
    property bool busy: false
    function write(key, value) {
        if (!known || kinds[key] === undefined) return
        var v = {}
        for (var k in values) v[k] = values[k]
        var typedValue = typed(key, value)
        v[key] = typedValue.value
        values = v
        problem = ""
        var q = queue.filter(function(c) { return c.key !== key })
        q.push({ key: key, value: typedValue })
        queue = q
        pump()
    }
    function pump() {
        if (busy || queue.length === 0) return
        var change = queue[0]
        queue = queue.slice(1)
        busy = true
        Helper.post("/v1/settings/" + owner, { changes: [change] }, function(status, doc) {
            busy = false
            if (doc && doc.ok && doc.settings) {
                if (queue.length === 0) take(doc.settings)      // a later change is still to go: its value is on screen already
            } else {
                problem = doc && doc.error ? String(doc.error.message || "") : "The helper refused the change or did not answer."
                queue = []
                refresh()                                       // a refused change: show what is really in force
            }
            pump()
        })
    }
}
