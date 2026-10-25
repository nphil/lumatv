import QtQuick 2.12
import WebOS.Global 1.0
import "Utils.js" as Utils

// LumaTV's toast, laid over LG's notification tree in place of LG's own `Toast.qml` (tools/lumatv,
// `notifications on`). ToastView creates one of these per toast and reads exactly the interface LG's
// file had: the properties below, the signals `activated`, `closed` and `entered`, the `state`
// that runs the open and close slide, and `rotateToastView()`. Data, timers, text-to-speech and the
// stack are LG's and untouched.
//
// What differs from LG's: a glass pill at the top right where the mock puts it (ToastView's `x` is
// patched to match) instead of a grey bar at the bottom; an icon tile and, when the message has more
// than one line, a title above a dimmer subtitle; the link of an advanced toast shown as an accent
// chevron. No `layer.enabled` and no DropShadow: a live blurred copy per toast is what the frame gate
// forbids, and the sheet's own edge is enough against Home or video.
//
// A palette name between two pairs of at-signs is a token (tools/tokens.py), filled in when the tree is staged.

FocusScope {
    id: root

    property string iconPath
    property string message
    property string readable
    property string type
    property bool containsMouse: mouseArea.containsMouse
    property bool generalToast: root.type === "standard"
    property bool advancedToast: root.type === "advanced"
    property bool lightToast: root.type === "light"
    property bool isLinkActive: advancedToast && (model.action && Object.keys(model.action).length > 0)
    property var style
    property var userdataId

    signal activated()
    signal closed()
    signal entered()

    // Layout, in the design's 1920-wide units.
    readonly property int pad: style.getRValue(20)
    readonly property int tile: style.getRValue(52)
    readonly property int gap: style.getRValue(18)
    readonly property bool hasIcon: root.iconPath !== "" && !lightToast
    readonly property var lines: String(root.message).split("\n")
    readonly property string title: lines[0]
    readonly property string subtitle: lines.length > 1 ? lines.slice(1).join(" ") : ""
    readonly property string linkLabel: (model.actionLabel !== undefined && model.actionLabel !== null) ? String(model.actionLabel) : ""
    readonly property int linkWidth: isLinkActive ? (linkLabel !== "" ? linkText.implicitWidth + chevron.width + style.getRValue(10) : chevron.width) : 0
    readonly property int textMax: style.toast.width - pad * 2 - (hasIcon ? tile + gap : 0) - (isLinkActive ? linkWidth + gap : 0)
    // Measured without wrapping, plus a pixel or two so that a line that fits exactly is not broken by rounding.
    readonly property int textWidth: Math.min(textMax, Math.ceil(Math.max(titleMeasure.implicitWidth, subMeasure.implicitWidth)) + 2)

    width: Math.max(style.getRValue(300), pad * 2 + (hasIcon ? tile + gap : 0) + textWidth + (isLinkActive ? linkWidth + gap : 0))
    height: Math.max(subtitle !== "" || hasIcon ? tile + pad * 2 : style.getRValue(72), body.implicitHeight + pad * 2)

    Text { id: titleMeasure; visible: false; text: root.title; font: titleText.font }
    Text { id: subMeasure; visible: false; text: root.subtitle; font: subText.font }

    FocusScope {
        // The slide is on this item so that ListView's own placement of the delegate is not fought.
        id: toast
        width: parent.width
        height: parent.height
        y: 0

        Rectangle {
            id: sheet
            anchors.fill: parent
            radius: root.style.getRValue(@@rowRadius@@)
            color: "@@sheetBottom@@"
            border.width: 1
            border.color: "@@hairline@@"
            scale: (mouseArea.containsMouse && root.isLinkActive) ? 1.02 : 1
            Behavior on scale { NumberAnimation { duration: 150 } }
        }

        Rectangle {
            id: iconTile
            visible: root.hasIcon
            width: root.tile
            height: root.tile
            radius: root.style.getRValue(14)
            color: "@@row@@"
            anchors.left: parent.left
            anchors.leftMargin: root.pad
            anchors.verticalCenter: parent.verticalCenter
            Image {
                anchors.fill: parent
                anchors.margins: root.style.getRValue(8)
                source: root.iconPath ? Utils.applyImagePath(encodeURIComponent(root.iconPath)) : ""
                fillMode: Image.PreserveAspectFit
                smooth: true
                asynchronous: true
            }
        }

        Column {
            id: body
            anchors.left: root.hasIcon ? iconTile.right : parent.left
            anchors.leftMargin: root.hasIcon ? root.gap : root.pad
            anchors.verticalCenter: parent.verticalCenter
            width: root.textWidth
            spacing: root.style.getRValue(3)

            Text {
                id: titleText
                objectName: "toastText"
                width: parent.width
                text: root.title
                color: "@@ink@@"
                font.family: "Manrope"
                font.pixelSize: root.style.getRValue(26)
                font.weight: root.subtitle !== "" ? Font.DemiBold : Font.Medium
                wrapMode: Text.Wrap
                elide: Text.ElideRight
                maximumLineCount: root.subtitle !== "" ? 1 : 3
            }
            Text {
                id: subText
                visible: root.subtitle !== ""
                width: parent.width
                text: root.subtitle
                color: "@@inkDim@@"
                font.family: "Manrope"
                font.pixelSize: root.style.getRValue(21)
                font.weight: Font.Normal
                wrapMode: Text.Wrap
                elide: Text.ElideRight
                maximumLineCount: 2
            }
        }

        Row {
            id: link
            visible: root.isLinkActive
            anchors.right: parent.right
            anchors.rightMargin: root.pad
            anchors.verticalCenter: parent.verticalCenter
            spacing: root.style.getRValue(10)
            Text {
                id: linkText
                visible: root.linkLabel !== ""
                text: root.linkLabel
                color: "@@accent@@"
                font.family: "Manrope"
                font.pixelSize: root.style.getRValue(24)
                font.weight: Font.DemiBold
                anchors.verticalCenter: parent.verticalCenter
            }
            Text {
                id: chevron
                text: "\u203a"
                color: "@@accent@@"
                font.family: "Manrope"
                font.pixelSize: root.style.getRValue(34)
                font.weight: Font.DemiBold
                anchors.verticalCenter: parent.verticalCenter
            }
        }

        WebOSMouseArea {
            id: mouseArea
            cursorVisible: notificationRoot.cursorVisible
            z: 10
            anchors.fill: parent
            enabled: !alertView.modalAlertsVisible && root.isLinkActive
            hoverEnabled: enabled
            propagateComposedEvents: true
            onClicked: {
                if (root.advancedToast) clickAnimation.restart()
            }
        }

        SequentialAnimation {
            id: clickAnimation
            PropertyAnimation { target: sheet; properties: "scale"; to: 1.0; duration: 50 }
            PauseAnimation { duration: 20 }
            PropertyAnimation { target: sheet; properties: "scale"; to: 1.02; duration: 50 }
            ScriptAction { script: root.activated() }
        }
    }

    state: "closed"
    Component.onCompleted: root.state = "opened"

    states: [
        State { name: "opened" },
        State { name: "closed" }
    ]

    transitions: [
        Transition {
            from: "closed"
            to: "opened"
            NumberAnimation { target: toast; property: "y"; from: -(root.height + root.style.getRValue(24)); to: 0; duration: 240; easing.type: Easing.OutCubic }
        },
        Transition {
            from: "opened"
            to: "closed"
            SequentialAnimation {
                NumberAnimation { target: toast; property: "y"; from: 0; to: -(root.height + root.style.getRValue(24)); duration: 200; easing.type: Easing.InCubic }
                ScriptAction { script: root.closed() }
            }
        }
    ]

    NumberAnimation {
        id: openAnimation
        target: toast
        property: "y"
        from: -(root.height + root.style.getRValue(24))
        to: 0
        duration: 240
        easing.type: Easing.OutCubic
    }

    function rotateToastView() {
        openAnimation.restart()
    }
}
