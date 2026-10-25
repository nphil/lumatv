import QtQuick 2.4

// A plain rectangle where LG's draws an angled card: enough to load and place what is in it.
Rectangle {
    property real angle: 0
    property real offset: height * Math.tan(angle * Math.PI / 180)
}
