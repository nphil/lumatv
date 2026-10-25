import QtQuick 2.4
import "kit"

// The prompt that AI access opens on the TV (docs/mocks/05-ai-request-prompt): turning access on, a
// pairing code, a higher level, resuming an assistant, or an "Ask on TV" call. It draws the helper's
// `ai.prompt` topic and nothing else: who asks, what for, in the helper's words (never the assistant's;
// an assistant's own reason is shown apart and labelled as its own), and how long is left.
//
// It has no button and calls nothing. OK and Back are read by the helper itself from the physical
// remote's input device (Principle 0: a prompt an AI can reach is not a control), so this item only
// shows the question and keeps the keys from reaching whatever is behind it (LumaHome returns before
// any other handling while `up`).
Item {
    id: prompt
    anchors.fill: parent
    readonly property var doc: Helper.aiPrompt
    readonly property bool up: doc !== null && doc !== undefined
    visible: up
    property double nowMs: Date.now()
    readonly property int secondsLeft: up ? Math.max(0, Math.ceil((doc.expiresAt - nowMs) / 1000)) : 0
    Timer { interval: 500; running: prompt.up; repeat: true; onTriggered: prompt.nowMs = Date.now() }
    onUpChanged: nowMs = Date.now()

    Rectangle { anchors.fill: parent; color: Lu.scrim }
    Item {
        id: card
        width: 900
        height: body.height + 2 * Lu.space(7)
        x: Math.round((1920 - width) / 2)
        y: Math.round((1080 - height) / 2)
        Glass {
            mat: Lu.material; isLight: Lu.isLight; cardRadius: Lu.radius("sheet"); focusMix: 0
            screenX: card.x; screenY: card.y; screenW: card.width; screenH: card.height
            backdrop: Lu.backdrop
        }
        Column {
            id: body
            x: Lu.space(7); y: Lu.space(7)
            width: parent.width - 2 * Lu.space(7)
            spacing: Lu.space(4)
            LuText {
                width: parent.width
                role: "label"; color: Lu.ink2
                text: prompt.up && prompt.doc.assistant ? prompt.doc.assistant.name + " asks" : "AI access"
            }
            LuText { width: parent.width; role: "title"; wrapMode: Text.WordWrap; text: prompt.up ? prompt.doc.title : "" }
            LuText { width: parent.width; role: "body"; color: Lu.ink2; wrapMode: Text.WordWrap; text: prompt.up ? prompt.doc.detail : "" }
            Item {
                visible: prompt.up && !!prompt.doc.reason
                width: parent.width
                height: visible ? reasonText.height + reasonLabel.height + Lu.space(2) : 0
                LuText { id: reasonLabel; role: "caption"; color: Lu.ink3; text: "In the assistant's own words" }
                LuText {
                    id: reasonText
                    y: reasonLabel.height + Lu.space(2)
                    width: parent.width
                    role: "body"; wrapMode: Text.WordWrap; font.italic: true
                    text: prompt.up && prompt.doc.reason ? "\u201c" + prompt.doc.reason + "\u201d" : ""
                }
            }
            Row {
                spacing: Lu.space(5)
                LuChip { text: "OK  " + (prompt.up ? "Allow once" : ""); tone: "positive" }
                LuChip { text: "Back  Deny"; tone: "danger" }
                LuChip { text: prompt.secondsLeft + " s left" }
            }
            LuText {
                width: parent.width
                role: "caption"; color: Lu.ink3; wrapMode: Text.WordWrap
                text: "Only this TV's remote can answer. Nothing on the network can."
            }
        }
    }
}
