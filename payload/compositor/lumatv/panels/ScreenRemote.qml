import QtQuick 2.4
import "../kit"
import "../kit/Spatial.js" as Spatial

// The on-screen remote, for the keys a person without the remote's buttons in reach still needs: the playback
// keys, the number keys (for a channel, a PIN) and the four colour keys. A sheet along the bottom; the D-pad and
// the pointer both work (LG's own is pointer-first). Nothing here presses a key by itself: OK on a button says
// `pressed(name)` and the host turns that into the TV's key (the compositor's key injection, or the helper's
// playback keys for the five that it allows), so this item can be exercised anywhere.
//
//   names: rewind, play, pause, stop, forward, 0-9, red, green, yellow, blue, info, back
Item {
    id: remote
    anchors.fill: parent
    property int index: 0
    property bool rested: true
    signal pressed(string name)
    signal closeRequested()
    readonly property bool built: true

    readonly property var keys: [
        { n: "rewind", icon: "fast-forward", flip: true, g: 0 }, { n: "play", icon: "play", g: 0 }, { n: "pause", icon: "pause", g: 0 },
        { n: "stop", icon: "square", g: 0 }, { n: "forward", icon: "fast-forward", g: 0 },
        { n: "1", t: "1", g: 1 }, { n: "2", t: "2", g: 1 }, { n: "3", t: "3", g: 1 }, { n: "4", t: "4", g: 1 }, { n: "5", t: "5", g: 1 }, { n: "6", t: "6", g: 1 },
        { n: "7", t: "7", g: 1 }, { n: "8", t: "8", g: 1 }, { n: "9", t: "9", g: 1 }, { n: "info", t: "Info", g: 1 }, { n: "0", t: "0", g: 1 }, { n: "back", icon: "undo-2", g: 1 },
        { n: "red", color: "#EB6F92", g: 2 }, { n: "green", color: "#9CCFD8", g: 2 }, { n: "yellow", color: "#F6C177", g: 2 }, { n: "blue", color: "#7AA2F7", g: 2 }
    ]
    // Boxes in the sheet's own coordinates, for the nearest-neighbour move.
    function boxes() {
        var out = []
        for (var i = 0; i < repeater.count; i++) { var it = repeater.itemAt(i); out.push(it ? { x: it.x, y: it.y, w: it.width, h: it.height } : null) }
        return out
    }
    function key(dir) {
        if (dir === "ok") pressed(keys[index].n)
        else if (dir === "back") closeRequested()
        else { var n = Spatial.nearest(boxes(), index, dir); if (n >= 0) index = n }
        rested = false
        restTimer.restart()
    }
    Timer { id: restTimer; interval: Lu.restMs; onTriggered: remote.rested = true }

    LuPane {
        x: 64; width: 1792; height: 360; y: 1080 - 64 - height
        LuText { x: Lu.space(7); y: Lu.space(4); role: "title"; text: "Remote" }
        LuText { x: Lu.space(7) + 200; y: Lu.space(4) + 10; role: "caption"; color: Lu.ink2; text: "Pick a key with the D-pad, OK presses it, Back closes" }
        Item {
            id: sheetBody
            x: Lu.space(7); y: 96; width: parent.width - 2 * Lu.space(7); height: parent.height - 96 - Lu.space(4)
            Repeater {
                id: repeater
                model: remote.keys
                delegate: Item {
                    id: key
                    readonly property bool focused: remote.index === index
                    // three groups: playback along the top left, the number pad in the middle, the colour keys at the right
                    readonly property int slot: index - (modelData.g === 0 ? 0 : modelData.g === 1 ? 5 : 17)
                    x: modelData.g === 0 ? slot * 128 : modelData.g === 1 ? 720 + (slot % 3) * 128 : 1200 + slot * 130
                    y: modelData.g === 0 ? 40 : modelData.g === 1 ? Math.floor(slot / 3) * 62 : 40
                    width: modelData.g === 2 ? 120 : 112
                    height: modelData.g === 1 ? 54 : 88
                    FocusLift { id: lift; focused: key.focused; rested: remote.rested; reduced: Lu.reduced; duration: Lu.cardMs; easing: Lu.cardEasing }
                    scale: 1 + (Lu.buttonScale - 1) * lift.value
                    transformOrigin: Item.Center
                    z: focused ? 1 : 0
                    Rectangle {
                        anchors.fill: parent; radius: modelData.g === 1 ? Lu.radius("control") : height / 2
                        color: key.focused ? Lu.glassRaised : Lu.glass
                        border.width: 1; border.color: key.focused ? Lu.edgeRaised : Lu.edge; antialiasing: true
                        Rectangle { visible: !!modelData.color; anchors.centerIn: parent; width: 56; height: 16; radius: 8; color: modelData.color || "transparent" }
                    }
                    LuIcon { visible: !!modelData.icon; anchors.centerIn: parent; name: modelData.icon || ""; size: 36; color: Lu.ink; rotation: modelData.flip ? 180 : 0 }
                    LuText { visible: !!modelData.t; anchors.centerIn: parent; role: "label"; text: modelData.t || "" }
                    MouseArea { anchors.fill: parent; hoverEnabled: true; onEntered: remote.index = index; onClicked: { remote.index = index; remote.pressed(modelData.n) } }
                }
            }
        }
    }
}
