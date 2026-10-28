.pragma library

// Quick Access: the remote's number keys (docs/mocks/31-quick-access-toast, 31-quick-access-assign). The choice lives in the user's
// theme.json as `remote.quickAccess`:
//   { on: true, slots: { "4": "<app id>" } }
// `on` is the switch: off (the default until the TV check) leaves the number keys as they are, the first ten dock tiles and nothing
// said about it; on, a digit says what it opens in a toast, a digit with nothing behind it asks what to assign, and `slots` holds the
// owner's own choices, which come before the dock. A slot never set is absent: it is the dock tile of that number.
var digits = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"]

function config(theme) {
    var q = theme && theme.remote ? theme.remote.quickAccess : null
    return q && typeof q === "object" ? q : ({})
}
function on(theme) { return config(theme).on === true }
function slots(theme) {
    var s = config(theme).slots
    return s && typeof s === "object" ? s : ({})
}

// The dock tile a digit stands for: 1 is the first, 9 the ninth, 0 the tenth.
function tileOf(digit) { return digit === "0" ? 9 : Number(digit) - 1 }

// The whole `quickAccess` object after `digit` was given `choice` ("default" puts the dock tile back, an app id sets it).
function chosen(theme, digit, choice) {
    var out = { on: on(theme), slots: {} }
    var s = slots(theme)
    for (var k in s) if (k !== digit) out.slots[k] = s[k]
    if (choice !== "default") out.slots[digit] = choice
    return out
}
// The whole object with the switch turned.
function switched(theme, value) {
    var out = { on: value === true, slots: {} }
    var s = slots(theme)
    for (var k in s) out.slots[k] = s[k]
    return out
}

// What the digit opens now, for a list: { kind: "slot" | "dock" | "none", id, title } with `dockIds` (the dock's ids in order) and
// `byId` (an app's record by id, or null).
function target(theme, digit, dockIds, byId) {
    var s = slots(theme)
    if (s[digit] !== undefined && s[digit] !== "") {
        var a = byId(s[digit])
        return { kind: "slot", id: String(s[digit]), title: a ? (a.title || a.id) : String(s[digit]) }
    }
    var id = dockIds[tileOf(digit)]
    if (id !== undefined) {
        var d = byId(id)
        return { kind: "dock", id: id, title: d ? (d.title || d.id) : id }
    }
    return { kind: "none", id: "", title: "" }
}

// The picks of the question for an empty slot: apps that no digit opens yet, in the dock's order, then the rest by name; at most `n`.
function suggestions(theme, dockIds, apps, n) {
    var used = {}
    var s = slots(theme)
    for (var k in s) used[s[k]] = true
    for (var i = 0; i < digits.length && i < dockIds.length; i++) used[dockIds[tileOf(digits[i])]] = true
    var out = []
    for (var j = 0; j < apps.length && out.length < n; j++) if (!used[apps[j].id] && !apps[j].hidden) out.push(apps[j])
    return out
}
