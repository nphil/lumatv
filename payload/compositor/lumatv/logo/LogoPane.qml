import QtQuick 2.12

// One glass pane of the mark: a rounded square that fades from `alphaTop` to `alphaBottom` down
// its height (a vertical gradient is the one Qt 5.12 Rectangle draws on every build), tinted with
// one brand colour.
Rectangle {
    property color tint: "white"
    property real alphaTop: 0.9
    property real alphaBottom: 0.3
    property color edgeColor: "white"
    property real edgeAlpha: 0          // rim of the front pane; the two back panes have none

    antialiasing: true
    border.width: edgeAlpha > 0 ? 2 : 0
    border.color: Qt.rgba(edgeColor.r, edgeColor.g, edgeColor.b, edgeAlpha)
    gradient: Gradient {
        GradientStop { position: 0.0; color: Qt.rgba(tint.r, tint.g, tint.b, alphaTop) }
        GradientStop { position: 1.0; color: Qt.rgba(tint.r, tint.g, tint.b, alphaBottom) }
    }
}
