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

    // Resident says when the picker is wanted (`presented`) and keeps it drawn while `leaving`; the sheet rises from the bottom edge and
    // back (PanelMotion: a transform and a fade, no layout while it moves; reduced motion is a fade)
    property bool presented: false
    readonly property bool leaving: motion.leaving
    PanelMotion { id: motion; presented: picker.presented }
    readonly property real progress: motion.progress     // 0 away, 1 shown
    readonly property real sheetY: sheet.y               // where the sheet is (612 at rest)

    // Live TV, then each HDMI port the owner has not hidden (kit/Inputs.tiles, the same list Home's Inputs row draws).
    readonly property var tiles: Inputs.tiles(Lu.theme.inputs)
    // "Sound: Denon on HDMI 2" once the receiver is found among the inputs, else only that the sound goes out over eARC
    readonly property string soundLine: {
        if (String(Sio.get("sound.soundOutput")) !== "external_arc") return Sio.has("sound.soundOutput") ? "Sound: TV speakers" : ""
        for (var i = 0; i < tiles.length; i++)
            if (tiles[i].sound) return "Sound: " + (/^\w+/.exec(tiles[i].name) || ["Receiver"])[0] + " on " + tiles[i].port.split(" \u00b7 ")[0]
        return "Sound: receiver on eARC"
    }

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

    Rectangle { anchors.fill: parent; color: Lu.scrim; opacity: motion.progress }
    LuPane {
        id: sheet
        x: 64; width: 1792
        height: 404
        // the one thing that moves: the sheet rises from the bottom edge and fades in (a transform and an opacity; nothing is laid out)
        readonly property real rise: Lu.reduced ? 0 : (height + 64) * (1 - motion.progress)
        y: 1080 - 64 - height + rise
        opacity: Lu.reduced ? motion.progress : Math.min(1, motion.progress * 2.5)
        onYChanged: if (motion.moving) locate()
        Row {
            id: head
            x: Lu.space(7); y: Lu.space(5)
            spacing: Lu.space(4)
            LuText { id: headTitle; role: "title"; text: "Inputs" }
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
                    height: 264
                    FocusLift { id: lift; focused: tile.focused; rested: picker.rested; reduced: Lu.reduced; duration: Lu.cardMs; easing: Lu.cardEasing }
                    z: focused ? 1 : 0
                    LuHalo { anchors.centerIn: parent; width: parent.width * 1.5; height: parent.height * 1.7; z: -1; on: tile.focused }
                    Rectangle {
                        anchors.fill: parent; radius: Lu.radius("card")
                        color: tile.focused ? Lu.wash("focus") : tile.t.front ? Lu.wash("selected") : Lu.wash("hover")
                        border.width: 1; border.color: tile.focused ? Lu.edgeRaised : Lu.edge
                        antialiasing: true
                        scale: 1 + (Lu.rowScale - 1) * 2 * lift.value      // the plate lifts; the words and the icon stay put
                        transformOrigin: Item.Center
                    }
                    LuText { id: portText; x: Lu.space(5); y: Lu.space(4); width: parent.width - 2 * Lu.space(5) - (soundMark.visible ? soundMark.width + Lu.space(2) : 0); role: "caption"; color: Lu.ink2; text: tile.t.port; wrapMode: Text.WordWrap; maximumLineCount: 2 }
                    // the input the sound goes out of (the head names it): a mark, not a second sentence
                    LuIcon { id: soundMark; visible: !!tile.t.sound; x: parent.width - width - Lu.space(4); y: Lu.space(4); name: "volume-2"; size: 26; color: Lu.positive }
                    LuIcon { x: Lu.space(5); y: Lu.space(4) + Math.max(48, portText.height + Lu.space(1)); name: tile.t.icon; size: portText.lineCount > 1 ? 44 : 56; color: tile.t.on ? Lu.ink : Lu.ink2 }
                    Column {
                        x: Lu.space(5); width: parent.width - 2 * Lu.space(5)
                        y: parent.height - height - Lu.space(4)
                        LuText { width: parent.width; role: "label"; text: tile.t.name; wrapMode: Text.WordWrap }
                        Row {
                            spacing: Lu.space(2)
                            Rectangle { width: Lu.space(2); height: width; radius: width / 2; anchors.verticalCenter: parent.verticalCenter; color: tile.t.connected ? Lu.positive : Lu.ink3 }
                            LuText { role: "caption"; color: Lu.ink2; text: tile.t.state; wrapMode: Text.WordWrap; width: tile.width - 2 * Lu.space(5) - Lu.space(4) }
                        }
                    }
                    MouseArea { anchors.fill: parent; hoverEnabled: true; onEntered: picker.index = index; onClicked: { picker.index = index; picker.key("ok") } }
                }
            }
        }
    }
}
