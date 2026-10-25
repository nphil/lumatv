import QtQuick 2.12

// One key of the PIN pad, in LumaTV's glass, in place of LG's `PincodePromptNumButton.qml` (tools/lumatv,
// `notifications on`). PincodePrompt and the mouse and key handling it inherits from LG's
// `PincodePromptBaseButton.qml` are LG's; the properties below are what LG's file had, so the prompt sets
// them as before. The focus is the same lift, brighter glass and edge as everywhere else, with no DropShadow.
// `accentFill` draws the key that sends the PIN in the accent.
//
// A palette name between two pairs of at-signs is a token (tools/tokens.py), filled in when the tree is staged.

PincodePromptBaseButton {
    id: root

    property color numPadNormalFontColor: "@@ink@@"
    property color numPadFontActiveColor: "@@ink@@"
    property string buttonTextFontFamilyName: "Manrope"
    property bool noColorButton: false
    property bool accentFill: false
    property int textSize: root.style.getRValue(34)

    width: root.style.getRValue(124)
    height: root.style.getRValue(68)

    Rectangle {
        id: background
        anchors.fill: parent
        radius: root.style.getRValue(@@rowRadius@@)
        antialiasing: true
        opacity: root.activated ? 1 : 0.4
        scale: root.highlight && !root.pressed ? 1.05 : 1
        Behavior on scale { NumberAnimation { duration: 100; easing.type: Easing.OutCubic } }
        color: root.accentFill ? (root.highlight ? Qt.lighter("@@accent@@", 1.18) : "@@accent@@")
                               : (root.highlight ? Qt.tint("@@row@@", "@@focus@@") : "@@row@@")
        border.width: root.highlight ? root.style.getRValue(2) : 0
        border.color: "@@focusEdge@@"

        Text {
            anchors.centerIn: parent
            width: parent.width
            text: root.text
            color: root.accentFill ? "@@accentInk@@" : root.numPadNormalFontColor
            font.family: root.buttonTextFontFamilyName
            font.pixelSize: root.textSize
            font.weight: Font.Medium
            horizontalAlignment: Text.AlignHCenter
            verticalAlignment: Text.AlignVCenter
            renderType: Text.NativeRendering
            elide: Text.ElideRight
        }
    }
}
