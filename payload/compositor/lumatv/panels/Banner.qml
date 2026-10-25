import QtQuick 2.4
import "../kit"

// What the TV says about what is on it (docs/mocks/20-channel-banner, 25-input-banner, 25-input-pill, 18-no-signal),
// for the three moments LG's `inputcommon` draws:
//   channel   the channel banner: number and call sign, the programme, its time and how much is left, what is next, the
//             signal facts as chips. Live TV is rarely used (Nitin, 2026-10-23), so this is the banner only, no guide.
//   input     the pill that says which input and what it carries (4K 120 Hz HDR10, Game Optimizer); as a bar with
//             Details, Inputs and Picture buttons when the INFO key asks for more.
//   nosignal  the black screen of an input with nothing on it: a dim icon, which port, what to do, and two buttons.
//             It is black on purpose: a still frame on an OLED is what the screen must never hold.
//
// One built item that is given a spec (`show(spec)`) and hides itself after `spec.holdMs`; the buttons take the D-pad when
// there are some (`key(dir)`), and `picked(id)` says which. Nothing is created per show.
//
//   spec = { kind: "channel", number, call, title, sub, progress, next, chips: [..] }
//        | { kind: "input", icon, name, facts, buttons: bool }
//        | { kind: "nosignal", icon, port, title, body }
Item {
    id: banner
    anchors.fill: parent
    property var spec: ({ kind: "" })
    property int index: 0
    property bool rested: true
    signal picked(string id)
    signal closeRequested()
    readonly property bool built: true
    readonly property string kind: spec.kind || ""
    readonly property var buttons: kind === "nosignal" ? [{ id: "inputs", icon: "cable", label: "Inputs" }, { id: "home", icon: "house", label: "Home" }]
                                  : kind === "input" && spec.buttons ? [{ id: "details", icon: "info", label: "Details" }, { id: "inputs", icon: "cable", label: "Inputs" }, { id: "picture", icon: "image", label: "Picture" }] : []
    visible: kind !== ""
    function show(s) {
        spec = s
        index = 0
        hold.interval = s.holdMs || (s.kind === "nosignal" ? 0 : s.buttons ? 8000 : 4000)
        if (hold.interval > 0) hold.restart(); else hold.stop()
    }
    function hide() { spec = { kind: "" }; hold.stop() }
    function key(dir) {
        if (buttons.length === 0) { if (dir === "back" || dir === "ok") closeRequested(); return }
        if (dir === "left") index = Math.max(0, index - 1)
        else if (dir === "right") index = Math.min(buttons.length - 1, index + 1)
        else if (dir === "ok") picked(buttons[index].id)
        else if (dir === "back") closeRequested()
        rested = false; restTimer.restart(); hold.restart()
    }
    Timer { id: hold; onTriggered: banner.hide() }
    Timer { id: restTimer; interval: Lu.restMs; onTriggered: banner.rested = true }

    // ---- no signal: black, the icon dim, the words, the two buttons
    Rectangle { anchors.fill: parent; visible: banner.kind === "nosignal"; color: "#000000" }
    Column {
        visible: banner.kind === "nosignal"
        anchors.horizontalCenter: parent.horizontalCenter
        y: 420
        spacing: Lu.space(3)
        LuIcon { anchors.horizontalCenter: parent.horizontalCenter; name: banner.spec.icon || "unplug"; size: 56; color: Lu.ink3 }
        LuText { anchors.horizontalCenter: parent.horizontalCenter; role: "caption"; color: Lu.ink3; text: banner.spec.port || "" }
        LuText { anchors.horizontalCenter: parent.horizontalCenter; role: "title"; color: Lu.ink2; text: banner.spec.title || "" }
        LuText { anchors.horizontalCenter: parent.horizontalCenter; width: 760; horizontalAlignment: Text.AlignHCenter; role: "label"; color: Lu.ink3; wrapMode: Text.WordWrap; text: banner.spec.body || "" }
        Row {
            anchors.horizontalCenter: parent.horizontalCenter
            spacing: Lu.space(4)
            Repeater {
                model: banner.kind === "nosignal" ? banner.buttons.length : 0
                delegate: LuButton { text: banner.buttons[index].label; focused: banner.index === index; rested: banner.rested; onHoverFocus: banner.index = index; onActivated: banner.picked(banner.buttons[index].id) }
            }
        }
    }

    // ---- channel banner and input pill/bar: a glass pill at the bottom left
    LuPane {
        id: pill
        visible: banner.kind === "channel" || banner.kind === "input"
        x: 64; y: 1080 - 64 - height
        width: banner.kind === "channel" ? 1280 : Math.max(420, bar.width + 2 * Lu.space(5))
        height: banner.kind === "channel" ? 132 : 96
        cardRadius: Lu.radius("card")

        // channel
        Item {
            visible: banner.kind === "channel"
            anchors.fill: parent
            Rectangle {
                id: chTile
                x: Lu.space(5); anchors.verticalCenter: parent.verticalCenter; width: 104; height: 88; radius: Lu.radius("tile"); color: Lu.wash("selected"); border.width: 1; border.color: Lu.edge; antialiasing: true
                Column { anchors.centerIn: parent; LuText { anchors.horizontalCenter: parent.horizontalCenter; role: "title"; lineHeight: 1.0; text: banner.spec.number || "" }
                         LuText { anchors.horizontalCenter: parent.horizontalCenter; role: "caption"; lineHeight: 1.0; color: Lu.ink2; text: banner.spec.call || "" } }
            }
            Column {
                x: chTile.x + chTile.width + Lu.space(5); width: parent.width - x - 320; anchors.verticalCenter: parent.verticalCenter; spacing: Lu.space(1)
                LuText { width: parent.width; role: "label"; lineHeight: 1.0; text: banner.spec.title || ""; elide: Text.ElideRight }
                Row {
                    spacing: Lu.space(4)
                    LuText { role: "caption"; lineHeight: 1.0; color: Lu.ink2; text: banner.spec.sub || "" }
                    LuText { role: "caption"; lineHeight: 1.0; color: Lu.ink3; text: banner.spec.next ? "Next \u00b7 " + banner.spec.next : "" }
                }
                Rectangle { width: parent.width * 0.5; height: 4; radius: 2; color: Lu.trackOff
                            Rectangle { width: parent.width * (banner.spec.progress || 0); height: parent.height; radius: 2; color: Lu.ink } }
            }
            Row {
                x: parent.width - width - Lu.space(5); anchors.verticalCenter: parent.verticalCenter; spacing: Lu.space(2)
                Repeater { model: banner.spec.chips || []; delegate: LuChip { text: modelData } }
            }
        }
        // input
        Row {
            id: bar
            visible: banner.kind === "input"
            x: Lu.space(5); anchors.verticalCenter: parent.verticalCenter; spacing: Lu.space(4)
            LuTile { anchors.verticalCenter: parent.verticalCenter; size: 52; icon: banner.spec.icon || "cable" }
            Column {
                anchors.verticalCenter: parent.verticalCenter
                LuText { role: "label"; lineHeight: 1.0; text: banner.spec.name || "" }
                LuText { role: "caption"; lineHeight: 1.0; color: Lu.ink2; text: banner.spec.facts || "" }
            }
            Repeater {
                model: banner.kind === "input" ? banner.buttons.length : 0
                delegate: LuButton { anchors.verticalCenter: parent.verticalCenter; text: banner.buttons[index].label; focused: banner.index === index; rested: banner.rested
                                     onHoverFocus: banner.index = index; onActivated: banner.picked(banner.buttons[index].id) }
            }
        }
    }
}
