import QtQuick 2.4
import "../kit"
import "../LayoutGrid.js" as Grid
import "Rows.js" as R
import "."

// Home widgets (Home & look): which cards Home shows, how wide each is, and in what order. A row per card, Off, Small, Half or Wide
// (four, six or eight of Home's twelve columns); a card that does not fit where it is says what is in the way, and stays as it was.
// Order trades a card's place with the one before or after it in reading order. Every change is a whole layout built with
// LayoutGrid.js and asked for through HomeLayout: Home writes the owner's layout.json (never the shipped one) and applies it at once.
// "Edit on Home" opens the layout editor, where a card is moved and sized by the cell and new ones are added.
PageList {
    id: page
    title: "Home widgets"
    back: "Home & look"
    backId: "look"
    tabs: ["Show", "Order"]

    readonly property var layout: HomeLayout.layout
    readonly property var cards: (layout.widgets || []).filter(function(e) { return e.type !== "dock" })
    // the cards and the types Home could add that it has no card of: what rows there are; a change of size or order is not a change of rows
    readonly property string shape: cards.map(function(e) { return e.id + ":" + e.type }).join("|") + "/" + missing().join("|")
    summary: cards.filter(function(e) { return !e.hidden }).length + " shown"

    function missing() {
        var have = {}, out = []
        var w = (layout.widgets || [])
        for (var i = 0; i < w.length; i++) have[w[i].type] = true
        for (var j = 0; j < Grid.ADDABLE.length; j++) if (!have[Grid.ADDABLE[j]]) out.push(Grid.ADDABLE[j])
        return out
    }
    function entryOf(id) { return Grid.find(HomeLayout.layout, id) }
    function iconOf(type) { return Grid.TYPES[type] ? Grid.TYPES[type].icon : "layout-grid" }
    function titleOf(id, type) { var e = entryOf(id); return e ? Grid.titleOf(e) : (Grid.TYPES[type] ? Grid.TYPES[type].title : type) }

    // what a refused change says, for a few seconds, in the row it was asked in
    property var notice: ({})
    function say(id, text) { var n = {}; n[id] = text; notice = n; noticeTimer.restart() }
    Timer { id: noticeTimer; interval: 3500; onTriggered: page.notice = ({}) }
    function reason(r) {
        return r.why === "overlap" && r.blocker ? Grid.titleOf(r.blocker) + " is in the way" : r.why === "full" ? "No room" : r.why === "small" ? "Too small" : "Not here"
    }

    // 0 Off, 1 Small, 2 Half, 3 Wide; a width that is none of the three has no mark
    function sizeIndex(e) {
        if (!e || e.hidden) return 0
        var s = Grid.sizeOf(e)
        return s === "small" ? 1 : s === "half" ? 2 : s === "wide" ? 3 : undefined
    }
    function choose(id, type, i) {
        if (!HomeLayout.layout.grid) return                      // Home has not handed its layout over (the page on its own)
        var start = HomeLayout.layout, L = start, e = Grid.find(L, id), r, at = id
        if (i === 0) {
            if (!e || e.hidden) return
            r = Grid.hide(L, id)
            if (r.ok) HomeLayout.change(r.layout); else say(id, reason(r))
            return
        }
        if (!e) {
            r = Grid.add(L, type)
            if (!r.ok) { say(id, reason(r)); return }
            L = r.layout; at = r.id
        } else if (e.hidden) {
            r = Grid.show(L, id)
            if (!r.ok) { say(id, reason(r)); return }
            L = r.layout
        }
        var s = Grid.setSpan(L, at, Grid.SIZES[i - 1].span)
        if (s.ok) L = s.layout
        else if (s.why !== "edge") say(id, reason(s))            // the card is on Home at the width it has; the one asked for does not fit
        if (L !== start) HomeLayout.change(L)
    }
    function nudge(id, d) {
        if (!HomeLayout.layout.grid) return
        var r = Grid.nudge(HomeLayout.layout, id, d)
        if (r.ok) HomeLayout.change(r.layout)
        else if (r.why !== "edge") say(id, reason(r))
    }

    function build(shape) {
        var out = []
        var items = cards.map(function(e) { return { id: e.id, type: e.type } })
        missing().forEach(function(t) { items.push({ id: Grid.TYPES[t].id, type: t }) })
        items.forEach(function(c) {
            out.push({ kind: "seg", tab: 0, id: "w:" + c.id, icon: iconOf(c.type), label: function() { return titleOf(c.id, c.type) },
                       detail: function() { return page.notice[c.id] || "" }, options: ["Off", "Small", "Half", "Wide"],
                       index: function() { return sizeIndex(entryOf(c.id)) }, act: function(i) { choose(c.id, c.type, i) } })
        })
        out.push({ kind: "value", tab: 0, id: "edit", icon: "layout-grid", label: "Edit on Home", act: function() { HomeLayout.edit("") } })
        out.push({ kind: "seg", tab: 0, id: "reset", icon: "rotate-ccw", label: "Reset to default", options: ["Keep", "Reset"],
                   index: function() { return 0 }, enabled: function() { return HomeLayout.customised },
                   act: function(i) { if (i === 1) HomeLayout.reset() } })
        cards.forEach(function(e) {
            var id = e.id
            out.push({ kind: "nudge", tab: 1, id: "n:" + id, icon: iconOf(e.type), label: function() { return titleOf(id, e.type) },
                       value: function() { var o = Grid.order(HomeLayout.layout); return (o.indexOf(id) + 1) + " of " + o.length },
                       visible: function() { var x = entryOf(id); return !!x && !x.hidden },
                       detail: function() { return page.notice[id] || "" }, act: function(d) { nudge(id, d) } })
        })
        return out
    }
    rows: build(shape)
}
