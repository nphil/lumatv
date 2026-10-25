import QtQuick 2.4
import "kit"

// The question AI access asks on the TV (docs/mocks/05-ai-request-prompt): an assistant wants to turn the
// TV off, switch an input, open an app, or the owner has asked for a wider level, a pairing code or the
// listener. It draws the helper's `ai.prompt` topic and nothing else: who asks, the action in the helper's own
// words (never the assistant's), the assistant's reason apart and labelled as its own, and how long is left.
//
// It has no button and calls nothing. OK and Back are read by the helper itself from the physical remote's
// input device (LGE RCU, LGE M-RCU): a press on "Smart Remote RCU Input" or "LGE Network Input", where Home
// Assistant's and every other network key arrives, is not an answer. Principle 0: a prompt an AI can reach is
// not a control. So this item only shows the question and keeps keys from reaching whatever is behind it
// (LumaHome returns before any other handling while `up`).
Item {
    id: prompt
    anchors.fill: parent
    readonly property var doc: Helper.aiPrompt
    readonly property bool up: doc !== null && doc !== undefined
    visible: up
    property double nowMs: Date.now()
    readonly property int total: up && doc.seconds ? doc.seconds : 30
    readonly property real remaining: up ? Math.max(0, (doc.expiresAt - nowMs) / 1000) : 0
    readonly property int secondsLeft: Math.ceil(remaining)
    readonly property string who: up && doc.assistant && doc.assistant.name ? doc.assistant.name : ""
    readonly property var icons: ({ tv_standby: "power", tv_launch_app: "layout-grid", tv_switch_input: "cable", enable_listener: "bot", pair: "key-round",
                                    resume: "play", raise_level: "sliders-horizontal", raise_volume: "volume-2", raise_limit: "volume-2" })
    Timer { interval: 500; running: prompt.up; repeat: true; onTriggered: prompt.nowMs = Date.now() }
    onUpChanged: nowMs = Date.now()

    Rectangle { anchors.fill: parent; color: Lu.scrim }
    LuPane {
        id: card
        width: 880
        height: body.height + 2 * Lu.space(7)
        x: Math.round((1920 - width) / 2)
        y: Math.round((1080 - height) / 2)
        Column {
            id: body
            x: Lu.space(7); y: Lu.space(7)
            width: parent.width - 2 * Lu.space(7)
            spacing: Lu.space(4)
            Row {
                spacing: Lu.space(3)
                LuIcon { anchors.verticalCenter: parent.verticalCenter; name: "sparkles"; size: 26; color: Lu.accent }
                LuText { anchors.verticalCenter: parent.verticalCenter; role: "caption"; color: Lu.ink2; text: (prompt.who !== "" ? prompt.who + " is asking" : "AI access") + " \u00b7 through AI access" }
            }
            LuText { width: parent.width; role: "title"; wrapMode: Text.WordWrap; text: prompt.up ? prompt.doc.title : "" }
            Item {
                width: parent.width
                height: Math.max(happen.height, 64) + Lu.space(6)
                Rectangle { anchors.fill: parent; radius: Lu.radius("control"); color: Lu.wash("selected"); border.width: 1; border.color: Lu.edge; antialiasing: true }
                LuIcon { x: Lu.space(5); anchors.verticalCenter: parent.verticalCenter; name: prompt.up && prompt.icons[prompt.doc.kind] ? prompt.icons[prompt.doc.kind] : "circle-help"; size: 32; color: Lu.ink }
                Column {
                    id: happen
                    x: Lu.space(5) + 32 + Lu.space(4)
                    width: parent.width - x - Lu.space(5)
                    anchors.verticalCenter: parent.verticalCenter
                    LuText { role: "caption"; color: Lu.ink2; text: "What will happen" }
                    LuText { width: parent.width; role: "body"; wrapMode: Text.WordWrap; text: prompt.up ? prompt.doc.detail : "" }
                }
            }
            Column {
                visible: prompt.up && !!prompt.doc.reason
                width: parent.width
                LuText { role: "caption"; color: Lu.ink2; text: (prompt.who !== "" ? prompt.who + "\u2019s" : "The assistant\u2019s") + " reason, in its own words" }
                LuText { width: parent.width; role: "body"; wrapMode: Text.WordWrap; font.italic: true; text: prompt.up && prompt.doc.reason ? "\u201c" + prompt.doc.reason + "\u201d" : "" }
            }
            // What the remote's two keys do. They are not buttons: nothing on screen answers.
            Row {
                spacing: Lu.space(4)
                Rectangle {
                    width: Math.round(Lu.target * 3); height: Lu.target; radius: height / 2
                    color: Lu.glass; border.width: 1; border.color: Lu.edgeRaised; antialiasing: true
                    LuText { anchors.centerIn: parent; role: "label"; text: "Allow once" }
                }
                Rectangle {
                    width: Math.round(Lu.target * 2.4); height: Lu.target; radius: height / 2
                    color: Lu.glassRaised; border.width: 1; border.color: Lu.edgeRaised; antialiasing: true
                    LuText { anchors.centerIn: parent; role: "label"; text: "Deny" }
                }
            }
            Item {
                width: parent.width
                height: Lu.space(7)
                LuText { id: closes; anchors.verticalCenter: parent.verticalCenter; role: "caption"; color: Lu.ink2; text: "Closes in " + prompt.secondsLeft + " s" }
                Rectangle {
                    x: closes.width + Lu.space(4)
                    width: Math.round(parent.width * 0.3)
                    height: Lu.space(1)
                    anchors.verticalCenter: parent.verticalCenter
                    radius: height / 2
                    color: Lu.trackOff
                    Rectangle {
                        width: parent.width * Math.min(1, prompt.remaining / prompt.total)
                        height: parent.height
                        radius: height / 2
                        color: Lu.accent
                        Behavior on width { NumberAnimation { duration: 500 } }
                    }
                }
                LuText { anchors.right: parent.right; anchors.verticalCenter: parent.verticalCenter; role: "caption"; color: Lu.ink2; text: "Answer with the remote: OK allows, Back denies" }
            }
        }
    }
}
