import QtQuick 2.4

// Fallback for a widget type nothing can resolve: shows the type name (and why, in the
// log) instead of leaving a blank card. A bad layout.json can never blank Home.
Item {
    id: root
    property var theme: ({})
    property var entry: ({})
    property var mat: ({ ink: "#F2EFF6", ink2: "#A8F2EFF6" })
    property bool isLight: false
    property var settings: ({})
    property bool focused: false
    property bool inner: false

    function activate() { /* nothing to do */ }

    Column {
        anchors.centerIn: parent
        spacing: 8
        width: parent.width
        Text {
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            text: "?"
            color: root.mat.ink2
            font.family: "Manrope"; font.pixelSize: 40
            renderType: Text.NativeRendering
        }
        Text {
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            text: settings.typeName || entry.type || "unknown widget"
            color: root.mat.ink2
            font.family: "Manrope"; font.pixelSize: 20
            renderType: Text.NativeRendering
            wrapMode: Text.WordWrap
        }
    }
}
