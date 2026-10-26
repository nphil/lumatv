import QtQuick 2.4

// One day of the forecast as a row: the day, its icon, its low, a bar for how far its low and high reach on the week's scale, its high.
// The weather card draws one per day (a narrow card's rows are `compact`: the mock's 26 px icons and 23 px figures; a wide card's have
// room for the larger icon). It takes everything it draws from its properties.
Item {
    id: dr
    property var day: ({})                       // { day, kind, hi, lo, hiN, loN }
    property bool compact: false
    property var mat: ({ ink: "#F2EFF6", ink2: "#A8F2EFF6" })
    property bool isLight: false
    property string iconDir: ""
    property real weekMin: 0
    property real weekMax: 1
    property color inkBase: mat.ink || "#F2EFF6"
    readonly property color soft: Qt.rgba(inkBase.r, inkBase.g, inkBase.b, 0.8)

    readonly property int dayW: compact ? 70 : 84
    readonly property int numW: compact ? 56 : 60
    readonly property int icon: compact ? 26 : 34

    Text {
        x: 0; y: Math.round((parent.height - implicitHeight) / 2)
        width: dr.dayW
        text: dr.day.day || ""
        color: dr.mat.ink
        font.family: "Manrope"; font.weight: Font.Medium; font.pixelSize: 23
        renderType: Text.NativeRendering
    }
    Image {
        x: dr.dayW; y: Math.round((parent.height - dr.icon) / 2)
        width: dr.icon; height: dr.icon
        source: dr.compact ? dr.iconDir + "wx-" + dr.day.kind + (dr.isLight ? "_l" : "_d") + ".png" : dr.iconDir + "wxbig-" + dr.day.kind + ".png"
        smooth: true
    }
    Text {
        x: dr.dayW + dr.icon + 10; width: dr.numW
        y: Math.round((parent.height - implicitHeight) / 2)
        horizontalAlignment: Text.AlignRight
        text: dr.day.lo || ""
        color: dr.soft
        font.family: "Manrope"; font.weight: Font.Medium; font.pixelSize: 23
        renderType: Text.NativeRendering
    }
    RangeBar {
        x: dr.dayW + dr.icon + 10 + dr.numW + 14
        y: 0
        width: Math.max(0, parent.width - x - dr.numW - 14)
        height: parent.height
        lo: dr.day.loN; hi: dr.day.hiN; min: dr.weekMin; max: dr.weekMax
    }
    Text {
        x: parent.width - dr.numW; width: dr.numW
        y: Math.round((parent.height - implicitHeight) / 2)
        horizontalAlignment: Text.AlignRight
        text: dr.day.hi || ""
        color: dr.mat.ink
        font.family: "Manrope"; font.weight: Font.Medium; font.pixelSize: 23
        renderType: Text.NativeRendering
    }
}
