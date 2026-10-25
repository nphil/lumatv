import QtQuick 2.4

// One status-bar entry: a 48px button (avatar for "profile", a built-in glyph, or a theme
// image), optional label, optional badge. Focus is the same material language as a focused
// card: a glass disc pops out under the glyph (lift, 1.08 scale, focus tint, edge, rim,
// sheen, shadow) and the glyph goes from ink2 to full ink.
Item {
    id: item

    property string kind: "generic"     // profile | notifications | settings | search | generic
    property string label: ""
    property string initial: ""
    property color avatarColor: "#7360E7"
    property int badge: 0
    property bool focused: false
    property var mat: ({ ink: "#F2EFF6", ink2: "#A8F2EFF6" })
    property bool isLight: false
    property url iconSource: ""
    property int focusMs: 150
    property int focusEasing: Easing.OutCubic
    property Item backdrop: null
    property real screenX: 0
    property real screenY: 0
    readonly property string iconDir: "file://{{dataDir}}/qml/WebOSCompositor/{{qmlModule}}/icons/"

    signal activated()
    signal hoverFocus()

    width: 48 + (labelText.visible ? labelText.implicitWidth + 12 : 0)
    height: 48

    property real f: focused ? 1 : 0
    Behavior on f { NumberAnimation { duration: item.focusMs; easing.type: item.focusEasing } }

    Item {
        // the whole button lifts and scales; the glass disc fades in under the glyph
        id: button
        width: 48; height: 48
        y: -3 * item.f
        scale: 1 + 0.1 * item.f
        transformOrigin: Item.Center
        Glass {
            mat: item.mat
            isLight: item.isLight
            cardRadius: 24
            focusMix: item.f
            opacity: item.f
            visible: opacity > 0.01
            screenX: item.screenX; screenY: item.screenY; screenW: 48; screenH: 48
            backdrop: item.backdrop
        }

        Rectangle {
            visible: item.kind === "profile" && item.iconSource == ""
            anchors.fill: parent
            radius: 24
            antialiasing: true
            gradient: Gradient {
                GradientStop { position: 0.0; color: Qt.lighter(item.avatarColor, 1.25) }
                GradientStop { position: 1.0; color: Qt.darker(item.avatarColor, 1.15) }
            }
            Text {
                x: Math.round((48 - implicitWidth) / 2); y: Math.round((48 - implicitHeight) / 2)
                text: item.initial || (item.label ? item.label.charAt(0).toUpperCase() : "?")
                color: "#ffffff"
                font.family: "Manrope"; font.weight: Font.Bold; font.pixelSize: 22
                renderType: Text.NativeRendering
            }
        }

        Image {
            // theme override, drawn at 26px like the built-in glyphs
            visible: item.iconSource != ""
            x: 11; y: 11; width: 26; height: 26
            source: item.iconSource
            sourceSize.width: 26; sourceSize.height: 26
            asynchronous: true
        }

        // built-in glyphs: ink2 at rest, full ink when focused (two pre-rendered PNGs, no tinting)
        Image {
            visible: item.kind !== "profile" && item.iconSource == ""
            x: 11; y: 11; width: 26; height: 26
            source: item.iconDir + item.glyphName() + (item.isLight ? "_l" : "_d") + ".png"
            opacity: 1 - item.f
        }
        Image {
            visible: item.kind !== "profile" && item.iconSource == "" && item.f > 0.01
            x: 11; y: 11; width: 26; height: 26
            source: item.iconDir + item.glyphName() + (item.isLight ? "_l" : "_d") + "_on.png"
            opacity: item.f
        }

        Rectangle {
            visible: item.badge > 0
            x: 48 - width; y: 2
            width: Math.max(22, badgeText.implicitWidth + 12); height: 22; radius: 11
            color: "#ff3b5c"
            border.width: item.isLight ? 2 : 0
            border.color: "#e6ffffff"
            antialiasing: true
            Text {
                id: badgeText
                x: Math.round((parent.width - implicitWidth) / 2); y: Math.round((parent.height - implicitHeight) / 2)
                text: item.badge > 99 ? "99+" : String(item.badge)
                color: "#ffffff"; font.family: "Manrope"; font.weight: Font.Bold; font.pixelSize: 15
                renderType: Text.NativeRendering
            }
        }
    }

    function glyphName() {
        if (kind === "notifications") return "bell"
        if (kind === "settings") return "settings"
        if (kind === "search") return "search"
        return "generic"
    }

    Text {
        id: labelText
        visible: item.label !== "" && item.kind === "profile"
        x: 60
        y: Math.round((48 - implicitHeight) / 2)
        text: item.label
        color: item.mat.ink
        font.family: "Manrope"; font.weight: Font.DemiBold; font.pixelSize: 24
        renderType: Text.NativeRendering
    }

    MouseArea {
        anchors.fill: parent
        hoverEnabled: true
        onEntered: item.hoverFocus()
        onClicked: item.activated()
    }
}
