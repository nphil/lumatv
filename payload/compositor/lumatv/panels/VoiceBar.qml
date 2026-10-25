import QtQuick 2.4
import "../kit"

// Voice search (docs/mocks/28-voice-listening, 28-voice-results, 28-mic-indicator): what the TV shows while the mic key
// is held. It draws, it does not listen: whoever has the recogniser (LG's voice service, a plugin, the AI assistant)
// calls `listen()`, `heard(text, options)` and `idle()`, and `picked(id)` says which option was taken.
//   listening  a bar at the bottom left: the mic in an accent circle, "Listening", "Speak now...", and what to do
//   heard      the words that were heard in a bar, and above it up to three things the words can do (Search Netflix for
//              "series about chess", Search Plex, Search the web); the first is focused, Up and Down move, OK takes
//   mic        the small round mic at the bottom right, with a green dot, for as long as the microphone is open: the
//              one thing on screen that says the TV can hear (it is not hidden by any other surface, z above all)
//
//   options = [{ id, title, sub, iconUrl | icon }]
Item {
    id: voice
    anchors.fill: parent
    property string state: ""                 // "", "listening", "heard"
    property string text: ""
    property var options: []
    property int index: 0
    property bool micOpen: false
    property bool rested: true
    signal picked(string id)
    signal closeRequested()
    readonly property bool built: true

    function listen() { state = "listening"; micOpen = true; text = ""; options = [] }
    function heard(words, opts) { text = words; options = opts || []; index = 0; state = "heard"; micOpen = false }
    function idle() { state = ""; micOpen = false }
    function key(dir) {
        if (state !== "heard") { if (dir === "back") closeRequested(); return }
        if (dir === "up") index = Math.max(0, index - 1)
        else if (dir === "down") index = Math.min(options.length - 1, index + 1)
        else if (dir === "ok" && options[index]) picked(options[index].id)
        else if (dir === "back") closeRequested()
        rested = false; restTimer.restart()
    }
    Timer { id: restTimer; interval: Lu.restMs; onTriggered: voice.rested = true }

    // ---- the bar at the bottom left (both states)
    LuPane {
        id: bar
        visible: voice.state !== ""
        x: 64; y: 1080 - 64 - height; width: voice.state === "listening" ? 640 : 720; height: 96
        cardRadius: Lu.radius("card")
        Rectangle {
            id: micTile
            x: Lu.space(5); anchors.verticalCenter: parent.verticalCenter; width: 56; height: 56; radius: 28
            color: voice.state === "listening" ? Lu.accent : Lu.wash("selected"); border.width: 1; border.color: Lu.edge; antialiasing: true
            LuIcon { anchors.centerIn: parent; name: "mic"; size: 28; color: voice.state === "listening" ? Lu.accentInk : Lu.ink }
        }
        Column {
            x: micTile.x + micTile.width + Lu.space(4); anchors.verticalCenter: parent.verticalCenter; width: parent.width - x - Lu.space(5)
            LuText { role: "caption"; lineHeight: 1.0; color: Lu.ink2; text: voice.state === "listening" ? "Listening" : "Heard \u00b7 pick one" }
            LuText { width: parent.width; role: "label"; lineHeight: 1.0; elide: Text.ElideRight; text: voice.state === "listening" ? "Speak now\u2026" : "\u201c" + voice.text + "\u201d" }
        }
        LuText { visible: voice.state === "listening"; x: parent.width - width - Lu.space(5); anchors.verticalCenter: parent.verticalCenter; role: "caption"; color: Lu.ink3; text: "Release the key when done" }
    }

    // ---- the options above the bar
    LuPane {
        visible: voice.state === "heard" && voice.options.length > 0
        x: bar.x; width: bar.width; height: voice.options.length * Math.round(Lu.row * 0.9) + 2 * Lu.space(3); y: bar.y - Lu.space(3) - height
        cardRadius: Lu.radius("card")
        Column {
            x: Lu.space(3); y: Lu.space(3); width: parent.width - 2 * Lu.space(3)
            Repeater {
                model: voice.options.length
                delegate: LuRow {
                    readonly property var o: voice.options[index]
                    width: parent.width
                    height: Math.round(Lu.row * 0.9)
                    icon: o.icon || (o.iconUrl ? "" : "search")
                    label: o.title || ""
                    detail: o.sub || ""
                    focused: voice.index === index
                    rested: voice.rested
                    onHoverFocus: voice.index = index
                    onActivated: { voice.index = index; voice.key("ok") }
                }
            }
        }
    }

    // ---- the mic indicator
    Rectangle {
        visible: voice.micOpen
        x: 1920 - 64 - width; y: 1080 - 64 - height; width: 56; height: 56; radius: 28
        color: Lu.glass; border.width: 1; border.color: Lu.edge; antialiasing: true
        LuIcon { anchors.centerIn: parent; name: "mic"; size: 28; color: Lu.ink }
        Rectangle { x: parent.width - 18; y: 4; width: 12; height: 12; radius: 6; color: Lu.positive }
    }
}
