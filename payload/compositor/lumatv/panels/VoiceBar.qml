import QtQuick 2.4
import "../kit"

// Voice search (docs/mocks/28-voice-listening, 28-mic-indicator): what the TV shows while the mic key is held. It draws, it does
// not listen: Home drives it from the helper's `voice` topic (the helper turns the speech into text through Home Assistant's
// Assist pipeline; LG's voice service is not used, PLAN: voice is for search only). Home calls `listen()` when the key goes down,
// `working()` when it goes up, `failed(words)` when no words came back and `idle()` at the end. The heard words do not stop here:
// Home types them into Search, and the bar has nothing to pick, so what was said can never open an app or a page.
//   listening  a bar at the bottom left: the mic in an accent circle with one ring, "Listening", "Speak now...", and what to do
//   working    the key is up and the words are on their way: "One moment..."
//   failed     why there are no words ("Nothing was heard", "Home Assistant is not reachable"), shown for a moment
//   mic        the round mic at the bottom right, with a green dot, for as long as the microphone is open: the one thing on
//              screen that says the TV can hear (the host puts this surface above every other)
// Back leaves in every state, and `closeRequested()` follows it, so the host closes the layer in one place.
Item {
    id: voice
    anchors.fill: parent
    property string state: ""                 // "", "listening", "working", "failed"
    property string text: ""
    property bool micOpen: false
    signal closeRequested()
    readonly property bool built: true

    function listen() { state = "listening"; micOpen = true; text = "" }
    function working() { if (state === "listening") state = "working" }
    function failed(words) { state = "failed"; micOpen = false; text = words }
    function idle() { state = ""; micOpen = false; text = "" }
    function key(dir) { if (dir === "back") { idle(); closeRequested() } }

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
                     text: voice.state === "listening" ? "Listening" : voice.state === "working" ? "Working it out"
                         : "Voice search" }
            LuText { width: parent.width; role: "title"; font.pixelSize: 30; lineHeight: 1.0; elide: Text.ElideRight
                     text: voice.state === "listening" ? "Speak now\u2026" : voice.state === "working" ? "One moment\u2026"
                         : voice.text }
        }
        LuText {
            id: hint
            visible: voice.state === "listening"
            x: parent.width - width - 26; anchors.verticalCenter: parent.verticalCenter; role: "caption"; color: Lu.ink3
            text: "Release the key when done"
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
