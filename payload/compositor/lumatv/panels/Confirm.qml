import QtQuick 2.4
import "../kit"

// A question with two answers (docs/mocks/06-confirm-dialog): "Uninstall Crunchyroll?" with the app's icon, what
// happens, and Keep it / Uninstall. The safe answer is focused first; Back is that answer. Built once and given its
// text when it opens (`ask(spec)`), `answered(id)` carries the choice and the host does the work, so a dialog
// never does anything itself. A danger answer is drawn in the danger colour.
//
//   spec = { icon | iconUrl, title, body, answers: [{ id, label, danger, icon }], focus: 0 }
Item {
    id: dialog
    anchors.fill: parent
    property var spec: ({ answers: [] })
    property int index: 0
    property bool rested: true
    signal answered(string id)
    signal closeRequested()
    readonly property bool built: true
    readonly property var answers: spec.answers || []

    function ask(s) { spec = s; index = s.focus || 0 }
    function key(dir) {
        if (dir === "left") index = Math.max(0, index - 1)
        else if (dir === "right") index = Math.min(answers.length - 1, index + 1)
        else if (dir === "ok" && answers[index]) answered(answers[index].id)
        else if (dir === "back") { answered(answers.length ? answers[spec.focus || 0].id : ""); closeRequested() }
        rested = false
        restTimer.restart()
    }
    Timer { id: restTimer; interval: Lu.restMs; onTriggered: dialog.rested = true }

    Rectangle { anchors.fill: parent; color: Lu.scrim }
    LuPane {
        id: card
        width: 760
        height: body.height + 2 * Lu.space(6)
        x: Math.round((1920 - width) / 2); y: Math.round((1080 - height) / 2)
        Column {
            id: body
            x: Lu.space(6); y: Lu.space(6); width: parent.width - 2 * Lu.space(6); spacing: Lu.space(4)
            Row {
                spacing: Lu.space(4)
                Image { visible: !!dialog.spec.iconUrl; width: 56; height: 56; source: dialog.spec.iconUrl || ""; fillMode: Image.PreserveAspectFit; asynchronous: false; cache: true }
                LuTile { visible: !dialog.spec.iconUrl; size: 56; icon: dialog.spec.icon || "circle-help"; accent: true }
                LuText { anchors.verticalCenter: parent.verticalCenter; role: "title"; text: dialog.spec.title || "" }
            }
            LuText { width: parent.width; role: "body"; color: Lu.ink2; wrapMode: Text.WordWrap; text: dialog.spec.body || "" }
            Row {
                spacing: Lu.space(4)
                Repeater {
                    model: dialog.answers.length
                    delegate: LuButton {
                        readonly property var a: dialog.answers[index]
                        text: a.label
                        primary: index === dialog.index && !a.danger
                        danger: !!a.danger
                        focused: dialog.index === index
                        rested: dialog.rested
                        onHoverFocus: dialog.index = index
                        onActivated: { dialog.index = index; dialog.key("ok") }
                    }
                }
            }
        }
    }
}
