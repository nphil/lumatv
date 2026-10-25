import QtQuick 2.12

// The three glass panes of assets/icon.svg, drawn with rounded rectangles and no tile behind
// them (only the panes are lit). Geometry is icon.svg's, in its 200-unit box, shifted so the item
// is the panes' bounding box (142 x 146 units, origin at 27,39) and scaled by `unit` pixels per
// unit. Each pane has a position `p` (0 apart, 1 assembled) and a visibility `k`.
Item {
    id: mark

    property real unit: 1.25
    property color love: "#EB6F92"
    property color foam: "#9CCFD8"
    property color iris: "#C4A7E7"
    property color sheen: "#E0DEF4"
    property real brightness: 1             // overall multiplier on every pane

    property real pLove: 1
    property real pFoam: 1
    property real pIris: 1
    property real kLove: 1
    property real kFoam: 1
    property real kIris: 1
    property real travel: 1                 // 0 keeps the panes in place and only fades them

    width: 142 * unit
    height: 146 * unit

    // Pane centre in icon.svg units minus the box origin, and where each pane comes from.
    LogoPane {
        tint: mark.love; alphaTop: 0.8; alphaBottom: 0.22
        width: 104 * mark.unit; height: width; radius: 26 * mark.unit
        x: (90 - 52 - 27 - 74 * (1 - mark.pLove) * mark.travel) * mark.unit
        y: (122 - 52 - 39 + 46 * (1 - mark.pLove) * mark.travel) * mark.unit
        rotation: -14 - 22 * (1 - mark.pLove) * mark.travel
        opacity: mark.kLove * mark.brightness
    }
    LogoPane {
        tint: mark.foam; alphaTop: 0.85; alphaBottom: 0.28
        width: 104 * mark.unit; height: width; radius: 26 * mark.unit
        x: (110 - 52 - 27 + 74 * (1 - mark.pFoam) * mark.travel) * mark.unit
        y: (98 - 52 - 39 - 44 * (1 - mark.pFoam) * mark.travel) * mark.unit
        rotation: 8 + 22 * (1 - mark.pFoam) * mark.travel
        opacity: mark.kFoam * mark.brightness
    }
    LogoPane {
        id: front
        tint: mark.iris; alphaTop: 0.92; alphaBottom: 0.35
        edgeColor: mark.sheen; edgeAlpha: 0.45
        width: 112 * mark.unit; height: width; radius: 28 * mark.unit
        x: (104 - 56 - 27 + 12 * (1 - mark.pIris) * mark.travel) * mark.unit
        y: (110 - 56 - 39 - 26 * (1 - mark.pIris) * mark.travel) * mark.unit
        rotation: -4 + 8 * (1 - mark.pIris) * mark.travel
        opacity: mark.kIris * mark.brightness
    }
    // The sheen across the front pane's top-left corner.
    Rectangle {
        x: front.x + 6 * mark.unit
        y: front.y + 8 * mark.unit
        width: 64 * mark.unit; height: 22 * mark.unit; radius: 10 * mark.unit
        rotation: front.rotation - 4
        antialiasing: true
        opacity: mark.kIris * mark.brightness
        gradient: Gradient {
            GradientStop { position: 0.0; color: Qt.rgba(mark.sheen.r, mark.sheen.g, mark.sheen.b, 0.5) }
            GradientStop { position: 1.0; color: Qt.rgba(mark.sheen.r, mark.sheen.g, mark.sheen.b, 0.0) }
        }
    }
}
