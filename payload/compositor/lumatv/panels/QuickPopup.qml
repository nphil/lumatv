import QtQuick 2.4
import "../kit"

// What opens beside the quick panel (docs/mocks/29-qp-slider, 29-qp-picker, 29-qp-add): a glass card to the left of the sheet, 48 px from it,
// while the tile it came from stays raised. One card, three contents, chosen by `spec.kind`:
//   slider  a value with a track: `title`, `text`, `from`, `to`, `step`, `value()` (a function, so the card follows what the TV says),
//           `set(v)`, `unit`, `marks` (the words under the track), `icons` (the two ends). Left and Right change it, OK and Back close.
//   pick    a list of choices: `options` (words), `index()`, `detail(i)` (the grey words at a row's right, e.g. when the TV would turn
//           off), `icon`, `set(i)`. Up and Down move, OK chooses and closes, Back closes and chooses nothing. The current choice has a tick
//           where the detail would be.
//   add     the same list for "Add a tile": `items` [{ id, icon, label }], `pick(id)`.
// The host (QuickPanel) opens it with `show(spec)`, hands it every key while it is up (`key(dir)`, true when it used the key) and is told
// when it has closed. It never writes a setting itself: `set` and `pick` are the tile's own descriptors. Built once and hidden.
Item {
    id: pop
    property var spec: null
    property bool open: false
    property bool rested: true
    property int index: 0
    property real shownValue: 0                       // the slider's value as the card draws it, kept while a key is held
    readonly property string kind: spec ? spec.kind : ""
    readonly property int cardW: 560
    readonly property int pad: Lu.space(6)
    signal closed()
    width: cardW
    visible: open

    function show(s) {
        spec = s
        if (s.kind === "slider") shownValue = Number(s.value())
        else if (s.kind === "pick") { var i = s.index(); index = i === undefined || i < 0 ? 0 : i }
        else index = 0
        rested = true
        open = true
    }
    function close() { if (!open) return; open = false; closed() }
    function count() { return kind === "pick" ? (spec.options || []).length : kind === "add" ? (spec.items || []).length : 0 }
    function choose() {
        if (kind === "pick") spec.set(index)
        else if (kind === "add" && count() > 0) spec.pick(spec.items[index].id)
        close()
    }
    // true: the card used the key (it uses every one while it is up, so none reaches the list under it)
    function key(dir) {
        if (!open) return false
        if (kind === "slider") {
            if (dir === "left" || dir === "right") {
                var v = Math.max(spec.from, Math.min(spec.to, shownValue + (dir === "left" ? -1 : 1) * spec.step))
                if (v !== shownValue) { shownValue = v; spec.set(v) }
            } else if (dir === "ok" || dir === "back") close()
        } else {
            if (dir === "up") index = Math.max(0, index - 1)
            else if (dir === "down") index = Math.min(count() - 1, index + 1)
            else if (dir === "ok") choose()
            else if (dir === "back") close()
            rested = false
            restTimer.restart()
        }
        return true
    }
    Timer { id: restTimer; interval: Lu.restMs; onTriggered: pop.rested = true }
    // the slider follows what the TV says when it says something else (a remote's own key, a plugin), but not while a key is moving it
    Timer { interval: 400; running: pop.open && pop.kind === "slider"; repeat: true; onTriggered: { var v = Number(pop.spec.value()); if (!isNaN(v) && !keyDown.running) pop.shownValue = v } }
    Timer { id: keyDown; interval: 800 }
    onShownValueChanged: keyDown.restart()

    height: card.height
    LuPane {
        id: card
        width: pop.cardW
        height: body.height + pop.pad + Lu.space(5)
        cardRadius: Lu.radius("card")
        Column {
            id: body
            x: pop.pad; y: pop.pad
            width: pop.cardW - 2 * pop.pad
            spacing: Lu.space(3)
            Item {
                width: parent.width
                height: Math.max(titleText.height, valueText.height)
                LuText { id: titleText; width: parent.width - valueText.width - Lu.space(3); role: "title"; elide: Text.ElideRight; text: pop.spec ? pop.spec.title : "" }
                LuText { id: valueText; x: parent.width - width; y: Math.round((titleText.height - height) / 2); role: "title"; visible: pop.kind === "slider"
                         text: pop.kind === "slider" ? Math.round(pop.shownValue) + (pop.spec.unit || "") : "" }
            }
            LuText { width: parent.width; role: "caption"; color: Lu.ink2; wrapMode: Text.WordWrap; visible: !!pop.spec && !!pop.spec.text; text: pop.spec ? (pop.spec.text || "") : "" }

            // ---- slider
            Rectangle {
                width: parent.width
                height: sliderRow.height + marks.height + Lu.space(5)
                visible: pop.kind === "slider"
                radius: Lu.radius("row")
                color: Lu.wash("focus")
                border.width: 1
                border.color: Lu.edgeRaised
                Row {
                    id: sliderRow
                    x: Lu.space(4); y: Lu.space(3)
                    width: parent.width - 2 * Lu.space(4)
                    spacing: Lu.space(3)
                    LuIcon { anchors.verticalCenter: parent.verticalCenter; name: pop.kind === "slider" && pop.spec.icons ? pop.spec.icons[0] : "sun-dim"; size: 26; color: Lu.ink3 }
                    LuSlider {
                        anchors.verticalCenter: parent.verticalCenter
                        width: parent.width - 2 * 26 - 2 * Lu.space(3)
                        active: pop.open
                        from: pop.kind === "slider" ? pop.spec.from : 0
                        to: pop.kind === "slider" ? pop.spec.to : 1
                        step: pop.kind === "slider" ? pop.spec.step : 1
                        value: pop.shownValue
                    }
                    LuIcon { anchors.verticalCenter: parent.verticalCenter; name: pop.kind === "slider" && pop.spec.icons ? pop.spec.icons[1] : "sun"; size: 26; color: Lu.ink3 }
                }
                Row {
                    id: marks
                    x: Lu.space(4) + 26 + Lu.space(3); y: sliderRow.y + sliderRow.height
                    width: parent.width - 2 * (Lu.space(4) + 26 + Lu.space(3))
                    Repeater {
                        model: pop.kind === "slider" && pop.spec.marks ? pop.spec.marks.length : 0
                        delegate: LuText {
                            width: marks.width / (pop.spec.marks.length)
                            horizontalAlignment: index === 0 ? Text.AlignLeft : index === pop.spec.marks.length - 1 ? Text.AlignRight : Text.AlignHCenter
                            role: "caption"; color: Lu.ink3
                            text: pop.spec.marks[index]
                        }
                    }
                }
            }

            // ---- pick and add
            Column {
                width: parent.width
                visible: pop.kind === "pick" || pop.kind === "add"
                spacing: Lu.space(1)
                Repeater {
                    model: pop.count()
                    delegate: Item {
                        id: opt
                        readonly property bool isFocus: pop.open && pop.index === index
                        readonly property bool current: pop.kind === "pick" && pop.spec.index() === index
                        readonly property string label: pop.kind === "pick" ? pop.spec.options[index] : pop.spec.items[index].label
                        readonly property string glyph: pop.kind === "pick" ? (pop.spec.icon || "") : (pop.spec.items[index].icon || "")
                        readonly property string detail: pop.kind === "pick" && pop.spec.detail ? pop.spec.detail(index) : ""
                        width: parent.width
                        height: Math.max(72, optLabel.implicitHeight + Lu.space(4))
                        FocusLift { id: lift; focused: opt.isFocus; rested: pop.rested; reduced: Lu.reduced; duration: Lu.cardMs; easing: Lu.cardEasing }
                        Rectangle {
                            anchors.fill: parent
                            radius: Lu.radius("row")
                            color: opt.isFocus ? Lu.wash("focus") : "transparent"
                            border.width: opt.isFocus ? 1 : 0
                            border.color: Lu.edgeRaised
                            antialiasing: true
                            scale: 1 + (Lu.rowScale - 1) * lift.value
                        }
                        LuIcon { x: Lu.space(4); anchors.verticalCenter: parent.verticalCenter; visible: opt.glyph !== ""; name: opt.glyph; size: 28; color: Lu.ink2 }
                        LuText {
                            id: optLabel
                            x: Lu.space(4) + (opt.glyph !== "" ? 28 + Lu.space(4) : 0); anchors.verticalCenter: parent.verticalCenter
                            width: parent.width - x - 120
                            role: "label"; font.weight: Font.DemiBold; wrapMode: Text.WordWrap; text: opt.label
                        }
                        LuText { x: parent.width - width - Lu.space(4); anchors.verticalCenter: parent.verticalCenter; visible: !opt.current && opt.detail !== ""; role: "caption"; color: Lu.ink2; text: opt.detail }
                        LuIcon { x: parent.width - width - Lu.space(4); anchors.verticalCenter: parent.verticalCenter; visible: opt.current; name: "check"; size: 28; color: Lu.accent }
                        MouseArea { anchors.fill: parent; hoverEnabled: true; onEntered: pop.index = index; onClicked: { pop.index = index; pop.choose() } }
                    }
                }
            }
        }
    }
}
