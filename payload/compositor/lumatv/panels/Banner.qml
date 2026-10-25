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
// there are some (`key(dir)`), and `picked(id)` says which. Nothing is created per show. Any end, the timer, Back or a
// pick on a banner without buttons, is `hide()`, and `closeRequested()` follows it so the host drops the layer.
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
    readonly property bool interactive: buttons.length > 0
    readonly property var buttons: kind === "nosignal" ? [{ id: "inputs", icon: "cable", label: "Inputs" }, { id: "home", icon: "house", label: "Home" }]
                                  : kind === "input" && spec.buttons ? [{ id: "details", icon: "info", label: "Details" }, { id: "inputs", icon: "cable", label: "Inputs" }, { id: "picture", icon: "image", label: "Picture" }] : []
    visible: kind !== ""
    function show(s) {
        spec = s
        index = 0
        hold.interval = s.holdMs || (s.kind === "nosignal" ? 0 : s.buttons ? 8000 : 4000)
        if (hold.interval > 0) hold.restart(); else hold.stop()
    }
    function hide() {
        var was = kind !== ""
        spec = { kind: "" }
        hold.stop()
        if (was) closeRequested()
    }
    function key(dir) {
        if (!interactive) { if (dir === "back" || dir === "ok") hide(); return }
        if (dir === "left") index = Math.max(0, index - 1)
        else if (dir === "right") index = Math.min(buttons.length - 1, index + 1)
        else if (dir === "ok") { picked(buttons[index].id); hide() }
        else if (dir === "back") hide()
        rested = false; restTimer.restart(); if (hold.interval > 0) hold.restart()
    }
    Timer { id: hold; onTriggered: banner.hide() }
    Timer { id: restTimer; interval: Lu.restMs; onTriggered: banner.rested = true }

    // ---- no signal: black, the icon dim, the words, the two buttons
    Rectangle { anchors.fill: parent; visible: banner.kind === "nosignal"; color: "#000000" }
    Column {
        visible: banner.kind === "nosignal"
        anchors.horizontalCenter: parent.horizontalCenter
        y: 336
        width: 760
        spacing: Lu.space(4)
        opacity: 0.62
        LuIcon { anchors.horizontalCenter: parent.horizontalCenter; name: banner.spec.icon || "unplug"; size: 64; color: Lu.ink2 }
        LuText { anchors.horizontalCenter: parent.horizontalCenter; role: "caption"; font.weight: Font.DemiBold; color: Lu.ink2; text: banner.spec.port || "" }
        LuText { anchors.horizontalCenter: parent.horizontalCenter; role: "title"; text: banner.spec.title || "" }
        LuText { width: parent.width; horizontalAlignment: Text.AlignHCenter; role: "label"; color: Lu.ink2; wrapMode: Text.WordWrap; text: banner.spec.body || "" }
        Item { width: 1; height: Lu.space(1) }
        Row {
            anchors.horizontalCenter: parent.horizontalCenter
            spacing: Lu.space(3)
            Repeater {
                model: banner.kind === "nosignal" ? banner.buttons.length : 0
                delegate: BannerButton {
                    icon: banner.buttons[index].icon; text: banner.buttons[index].label
                    focused: banner.index === index; rested: banner.rested
                    onHoverFocus: banner.index = index
                    onActivated: { banner.index = index; banner.key("ok") }
                }
            }
        }
    }

    // ---- channel banner: a glass bar at the bottom left, 1240 wide
    LuPane {
        id: channel
        visible: banner.kind === "channel"
        x: 64; y: 1080 - 56 - height; width: 1240; height: 148
        cardRadius: Lu.radius("card")
        Rectangle {
            id: chTile
            x: 22; anchors.verticalCenter: parent.verticalCenter; width: 104; height: 104; radius: Lu.radius("tile"); antialiasing: true
            color: Lu.wash("selected"); border.width: 1; border.color: Lu.edge
            Column {
                anchors.centerIn: parent; spacing: Lu.space(1)
                LuText { anchors.horizontalCenter: parent.horizontalCenter; role: "title"; font.pixelSize: 32; font.weight: Font.DemiBold; lineHeight: 1.0; text: banner.spec.number || "" }
                LuText { anchors.horizontalCenter: parent.horizontalCenter; role: "caption"; font.pixelSize: 20; font.weight: Font.DemiBold; lineHeight: 1.0; color: Lu.ink2; text: banner.spec.call || "" }
            }
        }
        Row {
            id: tags
            x: parent.width - width - 28; anchors.verticalCenter: parent.verticalCenter; spacing: Lu.space(2)
            Repeater { model: banner.kind === "channel" ? (banner.spec.chips || []) : []; delegate: LuChip { text: modelData } }
        }
        Column {
            x: chTile.x + chTile.width + 24; width: tags.x - x - 24; anchors.verticalCenter: parent.verticalCenter; spacing: Lu.space(2)
            LuText { width: parent.width; role: "body"; font.weight: Font.DemiBold; lineHeight: 1.0; elide: Text.ElideRight; text: banner.spec.title || "" }
            Item {
                width: parent.width; height: subText.implicitHeight
                LuText { id: subText; role: "caption"; lineHeight: 1.0; color: Lu.ink2; text: banner.spec.sub || "" }
                LuText { x: parent.width - width; width: Math.min(implicitWidth, parent.width - subText.implicitWidth - Lu.space(5)); role: "caption"; lineHeight: 1.0; color: Lu.ink3; elide: Text.ElideRight
                         visible: !!banner.spec.next; text: banner.spec.next ? "Next \u00b7 " + banner.spec.next : "" }
            }
            Rectangle {
                width: parent.width; height: 5; radius: 3; color: Lu.trackOff; antialiasing: true
                Rectangle { width: parent.width * (banner.spec.progress || 0); height: parent.height; radius: 3; color: Lu.ink; antialiasing: true }
            }
        }
    }

    // ---- input pill, and the same pill grown into three actions
    LuPane {
        id: pill
        visible: banner.kind === "input"
        x: 64; y: 1080 - 56 - height; height: 96
        width: Math.max(420, bar.width + 16 + 30)
        cardRadius: Lu.radius("pill")
        Row {
            id: bar
            x: 16; anchors.verticalCenter: parent.verticalCenter; spacing: Lu.space(4)
            LuTile { anchors.verticalCenter: parent.verticalCenter; size: 64; radiusScale: 0.5; icon: banner.spec.icon || "cable" }
            Column {
                anchors.verticalCenter: parent.verticalCenter; spacing: Lu.space(1)
                LuText { role: "body"; font.weight: Font.DemiBold; lineHeight: 1.0; text: banner.spec.name || "" }
                LuText { role: "caption"; lineHeight: 1.0; color: Lu.ink2; text: banner.spec.facts || "" }
            }
            Rectangle { visible: banner.interactive && banner.kind === "input"; anchors.verticalCenter: parent.verticalCenter; width: 1; height: 40; color: Lu.edge }
            Repeater {
                model: banner.kind === "input" ? banner.buttons.length : 0
                delegate: BannerButton {
                    anchors.verticalCenter: parent.verticalCenter
                    icon: banner.buttons[index].icon; text: banner.buttons[index].label
                    focused: banner.index === index; rested: banner.rested
                    onHoverFocus: banner.index = index
                    onActivated: { banner.index = index; banner.key("ok") }
                }
            }
        }
    }
}
