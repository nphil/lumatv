import QtQuick 2.4
import "../kit"

// Voice search (docs/mocks/28-voice-listening, 28-voice-heard, 28-voice-results, 28-mic-indicator): what the TV shows
// while the mic key is held. It draws, it does not listen: whoever has the recogniser (a plugin, the AI assistant; LG's
// voice service is not used, PLAN: voice is for search only and nothing leaves the TV) calls `listen()`, `heard(words,
// options, doing)` and `idle()`, and `picked(id)` says which option was taken.
//   listening  a bar at the bottom left: the mic in an accent circle with one ring, "Listening", "Speak now...", and what
//              to do
//   heard      the words that were heard in the bar; one command acts at once (`doing`, "Opening...", at the right); any
//              other words list up to three things they can do above the bar, the first focused, Up and Down move, OK takes
//   mic        the round mic at the bottom right, with a green dot, for as long as the microphone is open: the one thing on
//              screen that says the TV can hear (the host puts this surface above every other)
//
//   options = [{ id, title, sub, iconUrl | icon }]
// Back leaves in every state, `closeRequested()` follows a pick too, so the host closes the layer in one place.
Item {
    id: voice
    anchors.fill: parent
    property string state: ""                 // "", "listening", "heard"
    property string text: ""
    property string status: ""
    property var options: []
    property int index: 0
    property bool micOpen: false
    property bool rested: true
    signal picked(string id)
    signal closeRequested()
    readonly property bool built: true

    function listen() { state = "listening"; micOpen = true; text = ""; options = []; status = "" }
    function heard(words, opts, doing) { text = words; options = opts || []; index = 0; state = "heard"; micOpen = false; status = doing || "" }
    function idle() { state = ""; micOpen = false; status = ""; options = [] }
    function key(dir) {
        if (dir === "back") { idle(); closeRequested(); return }
        if (state !== "heard" || options.length === 0) return
        if (dir === "up") index = Math.max(0, index - 1)
        else if (dir === "down") index = Math.min(options.length - 1, index + 1)
        else if (dir === "ok" && options[index]) { picked(options[index].id); closeRequested() }
        rested = false; restTimer.restart()
    }
    Timer { id: restTimer; interval: Lu.restMs; onTriggered: voice.rested = true }

    // ---- the bar at the bottom left (both states)
    LuPane {
        id: bar
        visible: voice.state !== ""
        x: 64; y: 1080 - 56 - height; width: 760; height: 110
        cardRadius: Lu.radius("card")
        Rectangle {
            id: ring
            x: 18 - 8; anchors.verticalCenter: parent.verticalCenter; width: 88; height: 88; radius: 44
            color: "transparent"; border.width: 3; antialiasing: true
            border.color: Qt.rgba(Lu.accent.r, Lu.accent.g, Lu.accent.b, 0.45)
        }
        Rectangle {
            id: micTile
            x: 18; anchors.verticalCenter: parent.verticalCenter; width: 72; height: 72; radius: 36; antialiasing: true
            color: voice.state === "listening" ? Lu.accent : Lu.wash("selected")
            border.width: voice.state === "listening" ? 0 : 1; border.color: Lu.edge
            LuIcon { anchors.centerIn: parent; name: "mic"; size: 34; color: voice.state === "listening" ? Lu.accentInk : Lu.ink }
        }
        Column {
            x: 18 + 72 + 20; anchors.verticalCenter: parent.verticalCenter; spacing: Lu.space(1)
            width: parent.width - x - 26 - hint.width - (hint.visible ? Lu.space(4) : 0)
            LuText { width: parent.width; role: "caption"; font.weight: Font.DemiBold; lineHeight: 1.0; color: Lu.ink2
                     text: voice.state === "listening" ? "Listening" : voice.options.length > 0 ? "Heard \u00b7 pick one" : "Heard" }
            LuText { width: parent.width; role: "title"; font.pixelSize: 30; lineHeight: 1.0; elide: Text.ElideRight
                     text: voice.state === "listening" ? "Speak now\u2026" : "\u201c" + voice.text + "\u201d" }
        }
        LuText {
            id: hint
            visible: voice.state === "listening" || voice.status !== ""
            x: parent.width - width - 26; anchors.verticalCenter: parent.verticalCenter; role: "caption"; color: Lu.ink3
            text: voice.state === "listening" ? "Release the key when done" : voice.status
        }
    }

    // ---- the options above the bar
    LuPane {
        visible: voice.state === "heard" && voice.options.length > 0
        x: 64; width: 760; height: voice.options.length * 76 + (voice.options.length - 1) * 4 + 32; y: 1080 - 190 - height
        cardRadius: Lu.radius("card")
        Column {
            x: 16; y: 16; width: parent.width - 32; spacing: 4
            Repeater {
                model: voice.options.length
                delegate: Item {
                    id: row
                    readonly property var o: voice.options[index]
                    readonly property bool focused: voice.index === index
                    width: parent.width; height: 76
                    z: focused ? 1 : 0
                    FocusLift { id: lift; focused: row.focused; rested: voice.rested; reduced: Lu.reduced; duration: Lu.cardMs; easing: Lu.cardEasing }
                    scale: 1 + (Lu.rowScale - 1) * lift.value
                    transformOrigin: Item.Center
                    Rectangle {
                        anchors.fill: parent; radius: 18; antialiasing: true
                        color: row.focused ? Lu.wash("focus") : "transparent"
                        border.width: row.focused ? 1 : 0; border.color: Lu.edgeRaised
                    }
                    Rectangle {
                        id: lead
                        x: 16; anchors.verticalCenter: parent.verticalCenter; width: 52; height: 52; radius: 14; antialiasing: true
                        color: row.o.iconUrl ? "transparent" : Lu.wash("selected")
                        Image { visible: !!row.o.iconUrl; anchors.fill: parent; source: row.o.iconUrl || ""; fillMode: Image.PreserveAspectFit; asynchronous: true; cache: true; smooth: true }
                        LuIcon { visible: !row.o.iconUrl; anchors.centerIn: parent; name: row.o.icon || "search"; size: 28; color: Lu.ink }
                    }
                    Column {
                        x: lead.x + lead.width + 16; anchors.verticalCenter: parent.verticalCenter; width: parent.width - x - 16; spacing: Lu.space(1)
                        LuText { width: parent.width; role: "label"; font.weight: Font.DemiBold; lineHeight: 1.0; elide: Text.ElideRight; text: row.o.title || "" }
                        LuText { width: parent.width; role: "caption"; lineHeight: 1.0; color: Lu.ink2; elide: Text.ElideRight; text: row.o.sub || ""; visible: !!row.o.sub }
                    }
                    MouseArea { anchors.fill: parent; hoverEnabled: true; onEntered: voice.index = index; onClicked: { voice.index = index; voice.key("ok") } }
                }
            }
        }
    }

    // ---- the mic indicator
    Rectangle {
        visible: voice.micOpen
        x: 1920 - 64 - width; y: 1080 - 56 - height; width: 72; height: 72; radius: 36; antialiasing: true
        color: Lu.glass; border.width: 1; border.color: Lu.edge
        LuIcon { anchors.centerIn: parent; name: "mic"; size: 32; color: Lu.positive }
        Rectangle { x: parent.width - 22; y: 8; width: 12; height: 12; radius: 6; color: Lu.positive }
    }
}
