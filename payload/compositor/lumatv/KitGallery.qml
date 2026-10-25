import QtQuick 2.4
import "kit"

// Every kit component in every state, on one sheet, for looking at and measuring. Not part of
// Home: LumaHome builds it only while theme.json says `"kitGallery": true` (set it with `lumatv
// theme set '{"kitGallery":true}'`, clear it with false) and Home is on screen. The D-pad moves
// through it the way a Settings page would: Up and Down between rows, Left and Right inside a
// slider, the segmented control and the button pair, OK on a row's switch or a segment, Back leaves.
Item {
    id: gallery
    anchors.fill: parent

    property var backdropItem: Lu.backdrop
    // focus stops: 0-5 rows, 6 the segmented control, 7 the buttons
    property int stop: 0
    property int segFocus: 0
    property int segCurrent: 1
    property int buttonFocus: 0
    property bool rested: true
    property real amount: 4

    readonly property int sheetW: 1240
    readonly property int sheetH: 840
    readonly property int sheetX: Math.round((1920 - sheetW) / 2)
    readonly property int sheetY: Math.round((1080 - sheetH) / 2)

    onStopChanged: { rested = false; restTimer.restart() }
    onSegFocusChanged: { rested = false; restTimer.restart() }
    onButtonFocusChanged: { rested = false; restTimer.restart() }
    Timer { id: restTimer; interval: Lu.restMs; onTriggered: gallery.rested = true }

    // returns true when the key is consumed; Back is left to the host
    function key(dir) {
        if (dir === "up") { stop = Math.max(0, stop - 1); return true }
        if (dir === "down") { stop = Math.min(7, stop + 1); return true }
        if (stop === 2 && (dir === "left" || dir === "right")) { slider.adjust(dir === "left" ? -1 : 1); return true }
        if (stop === 6 && dir === "left") { segFocus = Math.max(0, segFocus - 1); return true }
        if (stop === 6 && dir === "right") { segFocus = Math.min(2, segFocus + 1); return true }
        if (stop === 7 && dir === "left") { buttonFocus = 0; return true }
        if (stop === 7 && dir === "right") { buttonFocus = 1; return true }
        if (dir === "ok") {
            if (stop === 0) toggleA.checked = !toggleA.checked
            else if (stop === 1) toggleB.checked = !toggleB.checked
            else if (stop === 6) segCurrent = segFocus
            return true
        }
        return false
    }

    Rectangle { anchors.fill: parent; color: Lu.scrim }
    Item {
        id: sheet
        x: gallery.sheetX; y: gallery.sheetY
        width: gallery.sheetW; height: gallery.sheetH
        Glass {
            mat: Lu.material
            isLight: Lu.isLight
            cardRadius: Lu.radius("sheet")
            focusMix: 0
            screenX: sheet.x; screenY: sheet.y; screenW: sheet.width; screenH: sheet.height
            backdrop: Lu.backdrop
        }
    }

    Item {
        x: gallery.sheetX + Lu.space(8)
        y: gallery.sheetY + Lu.space(7)
        width: gallery.sheetW - 2 * Lu.space(8)
        height: gallery.sheetH - 2 * Lu.space(7)

        LuText { id: heading; role: "title"; text: "Kit" }
        Row {
            anchors.verticalCenter: heading.verticalCenter
            x: heading.width + Lu.space(6)
            spacing: Lu.space(3)
            LuChip { text: "Live" ; tone: "live" }
            LuChip { text: "On" ; tone: "positive" }
            LuChip { text: "Low" ; tone: "warning" }
            LuChip { text: "Failed" ; tone: "danger" }
            LuChip { text: "Info" ; tone: "info" }
            LuChip { text: "Plain" }
        }

        Column {
            id: rows
            y: heading.height + Lu.space(5)
            width: 720
            LuRow {
                width: rows.width; label: "Wallpaper motion"; detail: "Waves on the wallpaper"
                focused: gallery.stop === 0; rested: gallery.rested
                onActivated: toggleA.checked = !toggleA.checked
                trailing: LuToggle { id: toggleA; checked: true }
            }
            LuRow {
                width: rows.width; label: "Dim at night"
                focused: gallery.stop === 1; rested: gallery.rested
                onActivated: toggleB.checked = !toggleB.checked
                trailing: LuToggle { id: toggleB; checked: false }
            }
            LuRow {
                width: rows.width; label: "Amount"
                focused: gallery.stop === 2; rested: gallery.rested
                trailing: LuSlider { id: slider; from: 0; to: 16; step: 1; value: gallery.amount; active: gallery.stop === 2
                                     onMoved: gallery.amount = value }
                value: Math.round(gallery.amount) + " px"
            }
            LuRow {
                width: rows.width; label: "Language"; value: "English"
                focused: gallery.stop === 3; rested: gallery.rested
            }
            LuRow {
                width: rows.width; label: "Selected row"; detail: "Stays lit without focus"; selected: true
                focused: gallery.stop === 4; rested: gallery.rested
            }
            LuRow {
                width: rows.width; label: "Disabled row"; detail: "Not available while recording"; value: "Off"
                enabled: false
                focused: gallery.stop === 5; rested: gallery.rested
            }
        }

        LuSegmented {
            id: segmented
            x: 0
            y: rows.y + rows.height + Lu.space(6)
            options: ["Subtle", "Normal", "Strong"]
            current: gallery.segCurrent
            focusedIndex: gallery.stop === 6 ? gallery.segFocus : -1
            onPicked: { gallery.stop = 6; gallery.segFocus = index; gallery.segCurrent = index }
        }
        Row {
            x: 0
            y: segmented.y + segmented.height + Lu.space(6)
            spacing: Lu.space(5)
            LuButton { text: "Apply"; primary: true; focused: gallery.stop === 7 && gallery.buttonFocus === 0; rested: gallery.rested }
            LuButton { text: "Cancel"; focused: gallery.stop === 7 && gallery.buttonFocus === 1; rested: gallery.rested }
            LuButton { text: "Unavailable"; enabled: false }
        }

        Column {
            x: rows.width + Lu.space(8)
            y: heading.height + Lu.space(5)
            width: parent.width - x
            spacing: Lu.space(3)
            LuText { role: "display"; text: "Display" }
            LuText { role: "title"; text: "Title 40" }
            LuText { role: "body"; text: "Body 28, half the room away" }
            LuText { role: "label"; text: "Label 26" }
            LuText { role: "caption"; text: "Caption 24"; color: Lu.ink2 }
            Rectangle { width: parent.width; height: 1; color: Lu.edge }
            Row {
                spacing: Lu.space(3)
                Repeater {
                    model: ["accent", "danger", "warning", "positive", "info"]
                    delegate: Rectangle {
                        width: Lu.target; height: Lu.target; radius: Lu.radius("control")
                        color: modelData === "accent" ? Lu.accent : Lu.role(modelData)
                        border.width: 1; border.color: Lu.edge
                        antialiasing: true
                    }
                }
            }
            Row {
                spacing: Lu.space(3)
                Rectangle { width: Lu.target; height: Lu.target; radius: Lu.radius("control"); color: Lu.accent
                            LuText { anchors.centerIn: parent; role: "label"; text: "Aa"; color: Lu.accentInk } }
                Rectangle { width: Lu.target; height: Lu.target; radius: Lu.radius("control"); color: Lu.glassRaised; border.width: 1; border.color: Lu.edgeRaised
                            LuText { anchors.centerIn: parent; role: "label"; text: "Aa" } }
                Rectangle { width: Lu.target; height: Lu.target; radius: Lu.radius("control"); color: Lu.sheet; border.width: 1; border.color: Lu.edge
                            LuText { anchors.centerIn: parent; role: "label"; text: "Aa" } }
            }
        }
    }
}
