import QtQuick 2.4

// Only what the input popup reads of LG's animated close button.
Item {
    property color color: "white"
    property color backgroundColor: "black"
    property int frameDuration: 40
    property real buttonWidth: 50
    property real buttonHeight: 50
    signal entered()
    signal exited()
    signal clicked()
    Rectangle { anchors.centerIn: parent; width: parent.buttonWidth; height: parent.buttonHeight; radius: width / 2; color: parent.backgroundColor }
}
