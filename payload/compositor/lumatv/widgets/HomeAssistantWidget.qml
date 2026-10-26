import QtQuick 2.4
import "../kit"

// The At-home card: what plugins put on Home, as the chips of the mock, and nothing invented.
//
// - Plugin cards (PluginHub.homeCards), each a chip with its icon disc, a title and what it says first. A `size: "full"` card
//   takes a whole row, a half card one of its two places; more than fit are reached by moving down (the card scrolls a row at
//   a time). OK on the card enters it (the widget's inner layer: `key()` exists only while there are cards), the D-pad moves
//   between chips and OK runs the focused card's first action, if it has one; Back leaves. The cards the helper's own weather,
//   news and clock plugins publish are not here: Home draws those in full in their own cards.
// - With no card, one chip that says why: Home Assistant is not linked yet, and where to link it. OK opens Settings at that page
//   (`openSettingsPage`). Never a made-up house: a chip is a plugin's, or this one.
//
// Chips follow the mock's grid (2 columns, 12px gap, 16px under the title) with the icon slot the mock left empty filled in: a
// 36px tinted disc, 22px glyph, text 14px to its right and centred as a block.
Item {
    id: root
    property var theme: ({})
    property var entry: ({})
    property var mat: ({ ink: "#F2EFF6", ink2: "#A8F2EFF6" })
    property bool isLight: false
    property var settings: ({})
    property bool focused: false
    property bool inner: false
    signal openSettingsPage(string id)

    // OK on the card while it has no chips (with chips it has an inner layer instead, see `key`).
    function activate() { if (!pluginMode) openSettingsPage("homeassistant") }

    // ---- what the chip says when no plugin has a card
    readonly property string noneTitle: "Home Assistant isn't linked yet"
    readonly property string noneSub: "Link it in Settings \u203a Home Assistant"

    // ---- the plugins' cards, laid out in rows of two places
    readonly property var cards: PluginHub.homeCards
    readonly property bool pluginMode: cards.length > 0
    // [{ card, row, col, span }]
    readonly property var places: {
        var out = [], row = 0, col = 0
        for (var i = 0; i < cards.length; i++) {
            var span = cards[i].size === "full" ? 2 : 1
            if (col + span > 2) { row++; col = 0 }
            out.push({ card: cards[i], row: row, col: col, span: span })
            col += span
            if (col >= 2) { row++; col = 0 }
        }
        return out
    }
    readonly property int rowCount: places.length ? places[places.length - 1].row + 1 : 0
    property int index: 0                         // the focused place (inner layer)
    property int scroll: 0                        // the first row shown
    readonly property int shownRows: 2

    onPlacesChanged: { if (index >= places.length) index = Math.max(0, places.length - 1); reveal() }
    function reveal() {
        if (!places.length) { scroll = 0; return }
        var r = places[Math.min(index, places.length - 1)].row
        if (r < scroll) scroll = r
        else if (r >= scroll + shownRows) scroll = r - shownRows + 1
    }
    onInnerChanged: if (inner) { index = Math.min(index, Math.max(0, places.length - 1)); reveal() }

    function toneOf(role) {
        return role === "accent" ? Lu.accent : role === "info" ? Lu.info : role === "positive" ? Lu.positive : role === "warning" ? Lu.warning
             : role === "danger" ? Lu.danger : role === "live" ? Lu.live : Lu.ink2
    }

    // Moves to the nearest place in a direction: along the row, or to the row above or below (the place under the same column).
    function go(dir) {
        if (!places.length) return false
        var cur = places[index]
        var best = -1
        if (dir === "left" || dir === "right") {
            var step = dir === "left" ? -1 : 1
            for (var i = index + step; i >= 0 && i < places.length; i += step) {
                if (places[i].row === cur.row) { best = i; break }
                break
            }
        } else {
            var wantRow = cur.row + (dir === "up" ? -1 : 1)
            var bestDist = 99
            for (var j = 0; j < places.length; j++) {
                if (places[j].row !== wantRow) continue
                var d = Math.abs(places[j].col - cur.col)
                if (d < bestDist) { bestDist = d; best = j }
            }
        }
        if (best < 0) return false
        index = best
        reveal()
        return true
    }
    // The widget's inner layer: only while there are cards to move over.
    function laneKey(dir) {
        if (dir === "ok") {
            var c = places.length ? places[index].card : null
            if (c && c.actions && c.actions.length) PluginHub.act(c.pluginId, c.actions[0].action, c.actions[0].target)
            return true
        }
        go(dir)
        return true
    }
    property var key: pluginMode ? laneKey : undefined

    Text {
        id: title
        x: 0; y: 0
        text: root.entry.title || "At home"
        color: root.mat.ink2
        font.family: "Manrope"; font.weight: Font.DemiBold; font.pixelSize: 24
        renderType: Text.NativeRendering
    }
    Text {
        visible: root.pluginMode && root.rowCount > root.shownRows
        x: parent.width - width; y: 0
        text: String(root.cards.length)
        color: root.mat.ink3 || root.mat.ink2
        font.family: "Manrope"; font.weight: Font.Medium; font.pixelSize: 20
        renderType: Text.NativeRendering
    }

    property int chipTop: 49
    property int chipGap: 12
    property int chipW: Math.floor((width - chipGap) / 2)
    property int chipH: Math.floor((height - chipTop - chipGap) / 2)
    readonly property color chipFill: isLight ? "#8cffffff" : "#12ffffff"
    readonly property color chipEdge: isLight ? "#e6ffffff" : "#14ffffff"

    // no card: one chip, the width of the lane
    Item {
        id: none
        visible: !root.pluginMode
        x: 0; y: root.chipTop
        width: root.width; height: root.chipH
        Rectangle {
            anchors.fill: parent
            radius: 18
            antialiasing: true
            color: root.chipFill
            border.width: 1
            border.color: root.chipEdge
        }
        Rectangle {
            x: 16; y: Math.round((parent.height - 36) / 2)
            width: 36; height: 36; radius: 18
            antialiasing: true
            color: Qt.rgba(Lu.ink2.r, Lu.ink2.g, Lu.ink2.b, 0.2)
            LuIcon { anchors.centerIn: parent; size: 22; name: "house-wifi"; color: Lu.ink2 }
        }
        Item {
            x: 66
            y: Math.round((parent.height - 58) / 2)
            width: parent.width - 66 - 12 - 34
            height: 58
            Text {
                x: 0; y: 0
                width: parent.width
                text: root.noneTitle
                elide: Text.ElideRight
                color: root.mat.ink
                font.family: "Manrope"; font.weight: Font.DemiBold; font.pixelSize: 24
                renderType: Text.NativeRendering
            }
            Text {
                x: 0; y: 30
                width: parent.width
                text: root.noneSub
                elide: Text.ElideRight
                color: root.focused ? Lu.accent : root.mat.ink2
                font.family: "Manrope"; font.weight: Font.Medium; font.pixelSize: 20
                renderType: Text.NativeRendering
            }
        }
        LuIcon { x: parent.width - 16 - 22; y: Math.round((parent.height - 22) / 2); size: 22; name: "chevron-right"; color: root.mat.ink2 }
    }

    // the plugins' cards
    Item {
        id: lane
        visible: root.pluginMode
        x: 0; y: root.chipTop
        width: root.width; height: root.height - root.chipTop
        clip: true
        Repeater {
            model: root.pluginMode ? root.places : []
            delegate: Item {
                id: chip
                readonly property var c: modelData.card
                readonly property bool on: root.inner && root.focused && root.index === index
                readonly property var words: PluginHub.glance(c)
                readonly property color tone: root.toneOf((c.content || {}).role)
                readonly property string verb: c.actions && c.actions.length ? c.actions[0].label : ""
                visible: modelData.row >= root.scroll && modelData.row < root.scroll + root.shownRows
                x: modelData.col * (root.chipW + root.chipGap)
                y: (modelData.row - root.scroll) * (root.chipH + root.chipGap)
                width: modelData.span === 2 ? root.width : root.chipW
                height: root.chipH
                Rectangle {
                    anchors.fill: parent
                    radius: 18
                    antialiasing: true
                    color: chip.on ? Lu.wash("focus") : root.chipFill
                    border.width: 1
                    border.color: chip.on ? Lu.edgeRaised : root.chipEdge
                }
                Rectangle {
                    x: 16; y: Math.round((parent.height - 36) / 2)
                    width: 36; height: 36; radius: 18
                    antialiasing: true
                    color: Qt.rgba(chip.tone.r, chip.tone.g, chip.tone.b, 0.2)
                    LuIcon { anchors.centerIn: parent; size: 22; name: PluginHub.icon((chip.c.content || {}).icon); color: chip.tone }
                }
                Item {
                    x: 66
                    y: Math.round((parent.height - 58) / 2)
                    width: parent.width - 66 - 12
                    height: 58
                    Text {
                        x: 0; y: 0
                        width: parent.width
                        text: chip.words.title
                        elide: Text.ElideRight
                        color: root.mat.ink
                        font.family: "Manrope"; font.weight: Font.DemiBold; font.pixelSize: 24
                        renderType: Text.NativeRendering
                    }
                    Text {
                        x: 0; y: 30
                        width: parent.width
                        text: chip.on && chip.verb !== "" ? chip.verb : chip.words.sub
                        elide: Text.ElideRight
                        color: chip.on && chip.verb !== "" ? Lu.accent : root.mat.ink2
                        font.family: "Manrope"; font.weight: Font.Medium; font.pixelSize: 20
                        renderType: Text.NativeRendering
                    }
                }
                MouseArea {
                    anchors.fill: parent
                    hoverEnabled: true
                    onEntered: if (root.inner) { root.index = index; root.reveal() }
                    onClicked: { root.index = index; root.laneKey("ok") }
                }
            }
        }
    }
}
