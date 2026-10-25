import QtQuick 2.4
import "../kit"

// The input picker (docs/mocks/08-input-picker): a sheet along the bottom with one tile for Live TV and one for
// each HDMI port, built once and kept (LG's picker is 51 files rebuilt per open, 110-190 ms frozen). Each tile
// shows the port, a device icon chosen from the name the device gave itself over HDMI-CEC (a console, a streamer,
// a receiver, a disc player), that name, and a state line: connected, in front, the sound output. The owner's
// choice per input (Auto: only while something is connected, Always, Hidden) is in theme.json `inputs.<port>`
// (Settings > Inputs & devices) and decides which tiles are here; the data is `Inputs` (kit), refreshed when
// the picker opens.
//
// OK on a tile launches that input's app (`com.webos.app.hdmiN`, `com.webos.app.livetv`) through the host;
// pressing the input key again, or Right, goes on to the next tile; Back closes. The tile of the input in front
// is marked, and the sound output is named on the right of the head, from the settings service.
Item {
    id: picker
    anchors.fill: parent

    property int index: 0
    property bool rested: true
    signal activated(string appId)
    signal closeRequested()
    readonly property bool built: true

    readonly property var choices: Lu.theme.inputs || ({})
    function visibleFor(key, connected) {
        var c = choices[key] || "auto"
        return c === "always" || (c === "auto" && connected)
    }
    // Live TV, then each HDMI port the owner has not hidden; a port with nothing connected shows only when it is Always.
    readonly property var tiles: {
        var out = []
        if (visibleFor("livetv", true)) out.push({ appId: "com.webos.app.livetv", port: "LIVE TV", icon: "antenna", name: "Antenna", state: "Live TV", on: true, connected: true })
        var l = Inputs.list
        for (var i = 0; i < l.length; i++) {
            var e = l[i]
            var key = "hdmi" + (/hdmi(\d)/.exec(e.appId) || [0, i + 1])[1]
            if (!visibleFor(key, e.connected)) continue
            var front = e.appId === Inputs.foreground
            out.push({ appId: e.appId, port: e.port.toUpperCase() + (/denon|marantz|yamaha|receiver|avr/i.test(e.label) && e.connected ? " \u00b7 eARC" : ""),
                       icon: e.icon, name: e.connected ? (e.label || e.port) : "Nothing connected",
                       state: !e.connected ? "Plug in a device to use it" : front ? "In front" : "Connected", on: e.connected, connected: e.connected, front: front,
                       sound: /denon|marantz|yamaha|receiver|avr/i.test(e.label) && e.connected })
        }
        return out
    }
    readonly property string soundLine: String(Sio.get("sound.soundOutput")) === "external_arc" ? "Sound: receiver on eARC" : Sio.has("sound.soundOutput") ? "Sound: TV speakers" : ""

    function open(params) {
        Inputs.refresh()
        Sio.read("sound", ["soundOutput"])
        // the tile of the input in front first, so OK on open keeps what is on
        index = 0
        for (var i = 0; i < tiles.length; i++) if (tiles[i].front) { index = i; break }
    }
    function next() { index = tiles.length ? (index + 1) % tiles.length : 0 }
    function key(dir) {
        if (dir === "left") index = Math.max(0, index - 1)
        else if (dir === "right") index = Math.min(tiles.length - 1, index + 1)
        else if (dir === "ok") { if (tiles.length) activated(tiles[index].appId) }
        else if (dir === "back") closeRequested()
        rested = false
        restTimer.restart()
    }
    Timer { id: restTimer; interval: Lu.restMs; onTriggered: picker.rested = true }

    Rectangle { anchors.fill: parent; color: Lu.scrim }
    LuPane {
        id: sheet
        x: 64; width: 1792
        height: 380
        y: 1080 - 64 - height
        Row {
            id: head
            x: Lu.space(7); y: Lu.space(5)
            spacing: Lu.space(4)
            LuText { id: headTitle; role: "title"; text: "Inputs" }
            LuText { anchors.baseline: headTitle.baseline; role: "label"; color: Lu.ink2; text: "Names come from the devices themselves (HDMI-CEC)" }
        }
        Row {
            anchors.verticalCenter: head.verticalCenter
            x: parent.width - width - Lu.space(7)
            spacing: Lu.space(3)
            visible: picker.soundLine !== ""
            LuIcon { anchors.verticalCenter: parent.verticalCenter; name: "volume-2"; size: 26; color: Lu.ink2 }
            LuText { anchors.verticalCenter: parent.verticalCenter; role: "label"; color: Lu.ink2; text: picker.soundLine }
        }
        Row {
            id: tileRow
            x: Lu.space(7); y: head.y + head.height + Lu.space(5)
            spacing: Lu.space(4)
            Repeater {
                model: picker.tiles.length
                delegate: Item {
                    id: tile
                    readonly property var t: picker.tiles[index]
                    readonly property bool focused: picker.index === index
                    width: Math.floor((sheet.width - 2 * Lu.space(7) - (Math.max(picker.tiles.length, 5) - 1) * Lu.space(4)) / Math.max(picker.tiles.length, 5))
                    height: 240
                    opacity: t.on ? 1 : Lu.disabledOpacity
                    FocusLift { id: lift; focused: tile.focused; rested: picker.rested; reduced: Lu.reduced; duration: Lu.cardMs; easing: Lu.cardEasing }
                    scale: 1 + (Lu.rowScale - 1) * 2 * lift.value
                    transformOrigin: Item.Center
                    z: focused ? 1 : 0
                    LuHalo { anchors.centerIn: parent; width: parent.width * 1.5; height: parent.height * 1.7; z: -1; on: tile.focused }
                    Rectangle {
                        anchors.fill: parent; radius: Lu.radius("card")
                        color: tile.focused ? Lu.wash("focus") : tile.t.front ? Lu.wash("selected") : Lu.wash("hover")
                        border.width: 1; border.color: tile.focused ? Lu.edgeRaised : Lu.edge
                        antialiasing: true
                    }
                    LuText { x: Lu.space(5); y: Lu.space(4); width: parent.width - 2 * Lu.space(5) - (soundChip.visible ? soundChip.width : 0); role: "caption"; color: Lu.ink2; text: tile.t.port; elide: Text.ElideRight }
                    LuChip { id: soundChip; visible: !!tile.t.sound; x: parent.width - width - Lu.space(3); y: Lu.space(3); text: "Sound out"; tone: "positive" }
                    LuIcon { x: Lu.space(5); y: Lu.space(4) + 48; name: tile.t.icon; size: 56; color: Lu.ink }
                    Column {
                        x: Lu.space(5); width: parent.width - 2 * Lu.space(5)
                        y: parent.height - height - Lu.space(4)
                        LuText { width: parent.width; role: "label"; text: tile.t.name; elide: Text.ElideRight }
                        Row {
                            spacing: Lu.space(2)
                            Rectangle { width: Lu.space(2); height: width; radius: width / 2; anchors.verticalCenter: parent.verticalCenter; color: tile.t.connected ? Lu.positive : Lu.ink3 }
                            LuText { role: "caption"; color: Lu.ink2; text: tile.t.state; elide: Text.ElideRight; width: tile.width - 2 * Lu.space(5) - Lu.space(4) }
                        }
                    }
                    MouseArea { anchors.fill: parent; hoverEnabled: true; onEntered: picker.index = index; onClicked: { picker.index = index; picker.key("ok") } }
                }
            }
        }
    }
}
