import QtQuick 2.12
import "../../Components"

// Luma: one of the no-signal screen's two actions (mock 18). LG's Selectable brings the OK key, the pointer's hover and click and the
// focus state; the look is drawn on black: a faint wash with a hairline, lit with a ring when it has the focus.
Selectable {

    id: root

    property string text: ""
    property string glyph: ""

    height: Math.max(styleSheet.guiSize(60), Math.ceil(label.implicitHeight + styleSheet.guiSize(16)))
    width: content.width + styleSheet.guiSize(44)

    Rectangle {

        anchors.fill: parent
        radius: lumaStyle.lumaRowRadius
        color: root.hasFocus ? lumaStyle.noSignalLitColor : lumaStyle.noSignalWashColor
        border.width: root.hasFocus ? 2 : 1
        border.color: root.hasFocus ? lumaStyle.noSignalRingColor : lumaStyle.noSignalEdgeColor
    }

    Row {

        id: content

        anchors.centerIn: parent
        spacing: styleSheet.guiSize(12)

        LumaGlyph {

            anchors.verticalCenter: parent.verticalCenter
            name: root.glyph
            size: styleSheet.guiSize(28)
            color: lumaStyle.noSignalButtonColor
        }

        Text {

            id: label

            anchors.verticalCenter: parent.verticalCenter
            text: root.text
            font: lumaStyle.semiBoldFont(24)
            color: lumaStyle.noSignalButtonColor
        }
    }
}
