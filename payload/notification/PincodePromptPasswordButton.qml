import QtQuick 2.12

// One box of the PIN, in LumaTV's glass, in place of LG's `PincodePromptPasswordButton.qml` (tools/lumatv,
// `notifications on`). PincodePrompt reads and writes exactly what LG's file had: `style`, `text` (the digit
// kept in the box), `current`, `inprogress` and `pressed`. A digit is never drawn, only a dot.
//
// A palette name between two pairs of at-signs is a token (tools/tokens.py), filled in when the tree is staged.

Item {
    id: root

    property variant style
    property string text: ""
    property bool current
    property bool inprogress
    property bool pressed

    readonly property bool dotVisible: root.text !== "" || (root.current && root.inprogress && !root.pressed)

    width: root.style.getRValue(80)
    height: root.style.getRValue(92)
    focus: false

    Rectangle {
        anchors.fill: parent
        radius: root.style.getRValue(20)
        color: "@@row@@"
        border.width: root.current ? root.style.getRValue(2) : 1
        border.color: root.current ? "@@focusEdge@@" : "@@hairline@@"
    }

    Rectangle {
        anchors.centerIn: parent
        width: root.style.getRValue(24)
        height: width
        radius: width / 2
        color: "@@ink@@"
        visible: root.dotVisible
    }
}
