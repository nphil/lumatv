import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "."

// The list behind a choice row (settings/Choice.js): the TV's own values for the key, each in LG's name, the one in force marked. OK
// writes it and goes back to the page that opened this one. Opened with `host.openPage("choice", { row, back, backId })`, `row`
// being the descriptor of the row that was pressed: it knows its options, the one in force, and how to write one.
PageList {
    id: page
    readonly property var src: arg && arg.row ? arg.row : null
    title: src ? String(ev(src.label)) : ""
    back: arg && arg.back ? arg.back : ""
    backId: arg && arg.backId ? arg.backId : ""

    function pick(i) {
        if (!src) return
        src.pick(i)
        if (host) host.up()
    }
    // The focus starts on the value in force.
    onOpened: {
        for (var i = 0; i < rows.length; i++) if (ev(rows[i].selected) === true && focusable(i)) { enter(i); return }
        enter(first())
    }

    function build() {
        var list = src ? src.options() : []
        var out = []
        for (var i = 0; i < list.length; i++) out.push(optionRow(i, list[i]))
        return out
    }
    function optionRow(i, o) {
        return { kind: "value", id: "o:" + o.value, label: o.label, chevron: false, enabled: o.enabled,
                 selected: function() { return !!page.src && page.src.current() === o.value },
                 act: function() { page.pick(i) } }
    }
    rows: build()
}
