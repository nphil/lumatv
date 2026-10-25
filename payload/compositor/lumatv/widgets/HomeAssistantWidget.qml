import QtQuick 2.4

// Home Assistant card, placeholder data until the integration lands. Chips follow the mock's
// grid (2 columns, 12px gap, 16px under the title) with the icon slot the mock left empty
// filled in: a 36px tinted disc, 22px glyph, text 14px to its right and centred as a block.
Item {
    id: root
    property var theme: ({})
    property var entry: ({})
    property var mat: ({ ink: "#F2EFF6", ink2: "#A8F2EFF6" })
    property bool isLight: false
    property var settings: ({})
    property bool focused: false
    property bool inner: false
    readonly property string iconDir: "file://{{dataDir}}/qml/WebOSCompositor/{{qmlModule}}/icons/"

    function activate() {}

    property var chips: [
        { icon: "ha-bulb",   tint: "#38ffc45c", title: "Living room",  sub: "3 lights on" },
        { icon: "ha-thermo", tint: "#3378beff", title: "72\u00b0 cooling", sub: "Set to 70\u00b0" },
        { icon: "ha-lock",   tint: "#2e78f0aa", title: "Front door",   sub: "Locked" },
        { icon: "ha-music",  tint: "#2eff78c8", title: "Kitchen",      sub: "Playing \u00b7 Sonos" }
    ]

    Text {
        id: title
        x: 0; y: 0
        text: root.entry.title || "At home"
        color: root.mat.ink2
        font.family: "Manrope"; font.weight: Font.DemiBold; font.pixelSize: 24
        renderType: Text.NativeRendering
    }

    property int chipTop: 49
    property int chipGap: 12
    property int chipW: Math.floor((width - chipGap) / 2)
    property int chipH: Math.floor((height - chipTop - chipGap) / 2)

    Repeater {
        model: root.chips
        delegate: Item {
            x: (index % 2) * (root.chipW + root.chipGap)
            y: root.chipTop + Math.floor(index / 2) * (root.chipH + root.chipGap)
            width: root.chipW
            height: root.chipH
            Rectangle {
                anchors.fill: parent
                radius: 18
                antialiasing: true
                color: root.isLight ? "#8cffffff" : "#12ffffff"
                border.width: 1
                border.color: root.isLight ? "#e6ffffff" : "#14ffffff"
            }
            Rectangle {
                id: disc
                x: 16; y: Math.round((parent.height - 36) / 2)
                width: 36; height: 36; radius: 18
                antialiasing: true
                color: modelData.tint
                Image {
                    x: 7; y: 7
                    width: 22; height: 22
                    source: root.iconDir + modelData.icon + ".png"
                }
            }
            Item {
                x: 66
                y: Math.round((parent.height - 58) / 2)
                width: parent.width - 66 - 12
                height: 58
                Text {
                    x: 0; y: 0
                    width: parent.width
                    text: modelData.title
                    elide: Text.ElideRight
                    color: root.mat.ink
                    font.family: "Manrope"; font.weight: Font.DemiBold; font.pixelSize: 24
                    renderType: Text.NativeRendering
                }
                Text {
                    x: 0; y: 30
                    width: parent.width
                    text: modelData.sub
                    elide: Text.ElideRight
                    color: root.mat.ink2
                    font.family: "Manrope"; font.weight: Font.Medium; font.pixelSize: 20
                    renderType: Text.NativeRendering
                }
            }
        }
    }
}
