import QtQuick 2.4

// LG's clips `sourceItem` to the shape of `maskItem`; here it only holds the properties so the card loads.
Item {
    property Item sourceItem
    property Item maskItem
    property real radius: 0
}
