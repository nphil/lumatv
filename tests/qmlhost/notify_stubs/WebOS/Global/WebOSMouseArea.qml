import QtQuick 2.4

// LG's wrapper turns hover into a focus highlight while the pointer is hidden; same rule here.
MouseArea {
    hoverEnabled: true
    property bool cursorVisible: false
    property bool highlight: (cursorVisible && containsMouse) || (!cursorVisible && parent.activeFocus)
}
