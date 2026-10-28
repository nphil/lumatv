import QtQuick 2.4
import "../kit"

// Quick Access, the remote's number keys (docs/mocks/31-quick-access-toast, 31-quick-access-assign). Two things, one resident:
//   toast   what a digit opened, in a glass pill at the top right where every toast goes: the number on a plate, "4 · Netflix" and
//           "Opening." Two seconds, no key, over the launch screen as well (it is the layer above the splash).
//   ask     a digit with nothing behind it: "Nothing on 4 yet. Assign it?", four apps nobody has a number for yet, More and Not now.
//           It owns the keys while it is up (Left and Right choose, Down reaches Not now, OK answers, Back is Not now).
// The host (LumaHome, theme `remote.quickAccess`, settings/QuickAccess.js) decides when either shows; this item draws them and answers
// with a signal, never writes a file or calls a service. Nothing is built for a toast: the pill and the card exist, hidden, and a hidden
// item costs nothing.
Item {
    id: qa
    anchors.fill: parent
    readonly property bool built: true
    property bool rested: true
    property string digit: ""
    // the toast
    property string title: ""
    property string sub: ""
    property bool toasting: false
    // the question
    property bool asking: false
    property var picks: []                    // [{ id, title, iconUrl }]
    property int index: 0                     // 0..picks.length - 1 the apps, picks.length More, picks.length + 1 Not now
    readonly property int moreIndex: picks.length
    readonly property int laterIndex: picks.length + 1
    signal assigned(string digit, string appId)
    signal moreRequested(string digit)
    signal closeRequested()
    visible: toasting || asking

    function toast(d, name, line) { digit = d; title = d + " \u00b7 " + name; sub = line || "Opening."; toasting = true; hideTimer.restart() }
    function ask(d, list) { digit = d; picks = list || []; index = 0; asking = true; toasting = false; hideTimer.stop() }
    function key(dir) {
        if (!asking) return
        if (dir === "left") { if (index <= moreIndex) index = Math.max(0, index - 1) }
        else if (dir === "right") { if (index < moreIndex) index += 1 }
        else if (dir === "down") { if (index <= moreIndex) index = laterIndex }
        else if (dir === "up") { if (index === laterIndex) index = moreIndex }
        else if (dir === "ok") answer()
        else if (dir === "back") { asking = false; closeRequested() }
        rested = false
        restTimer.restart()
    }
    function answer() {
        var i = index
        asking = false
        if (i < picks.length) assigned(digit, picks[i].id)
        else if (i === moreIndex) moreRequested(digit)
        closeRequested()
    }
    Timer { id: hideTimer; interval: 2400; onTriggered: qa.toasting = false }
    Timer { id: restTimer; interval: Lu.restMs; onTriggered: qa.rested = true }

    // ---- the toast
    Rectangle {
        id: pill
        visible: qa.toasting
        x: qa.width - 64 - width
        y: 137
        width: Math.max(560, 2 * Lu.space(5) + plate.width + Lu.space(5) + Math.max(titleText.implicitWidth, subText.implicitWidth))
        height: 2 * Lu.space(5) + plate.height
        radius: Lu.radius("row")
        color: Lu.sheet
        border.width: 1
        border.color: Lu.edge
        Rectangle {
            id: plate
            x: Lu.space(5); y: Lu.space(5)
            width: 64; height: 64
            radius: Lu.radius("control")
            color: Lu.wash("selected")
            border.width: 1
            border.color: Lu.edge
            LuText { anchors.centerIn: parent; role: "title"; text: qa.digit }
        }
        Column {
            x: plate.x + plate.width + Lu.space(5)
            anchors.verticalCenter: parent.verticalCenter
            LuText { id: titleText; role: "label"; font.weight: Font.DemiBold; text: qa.title }
            LuText { id: subText; role: "caption"; color: Lu.ink2; text: qa.sub }
        }
    }

    // ---- the question
    Rectangle { anchors.fill: parent; visible: qa.asking; color: Lu.scrimHide }       // a question hides the page behind it
    Rectangle {
        id: card
        visible: qa.asking
        width: 820
        height: body.height + 2 * Lu.space(6)
        x: Math.round((qa.width - width) / 2); y: Math.round((qa.height - height) / 2)
        radius: Lu.radius("sheet")
        color: Lu.sheet
        border.width: 1
        border.color: Lu.edge
        Column {
            id: body
            x: Lu.space(6); y: Lu.space(6)
            width: parent.width - 2 * Lu.space(6)
            spacing: Lu.space(5)
            Row {
                spacing: Lu.space(4)
                Rectangle {
                    width: 64; height: 64; radius: Lu.radius("control")
                    color: Lu.wash("selected"); border.width: 1; border.color: Lu.edge
                    LuText { anchors.centerIn: parent; role: "title"; text: qa.digit }
                }
                LuText { anchors.verticalCenter: parent.verticalCenter; role: "title"; text: "Nothing on " + qa.digit + " yet. Assign it?" }
            }
            LuText { width: parent.width; role: "body"; line: 0.99; color: Lu.ink2; wrapMode: Text.WordWrap; text: "Pressing " + qa.digit + " will open what you pick. The dock stays as it is." }
            Row {
                spacing: Lu.space(3)
                Repeater {
                    model: qa.picks.length + 1
                    delegate: Item {
                        id: pick
                        readonly property bool isMore: index === qa.moreIndex
                        readonly property bool isFocus: qa.asking && qa.index === index
                        width: 140; height: 150
                        FocusLift { id: lift; focused: pick.isFocus; rested: qa.rested; reduced: Lu.reduced; duration: Lu.cardMs; easing: Lu.cardEasing }
                        LuHalo { anchors.centerIn: parent; width: parent.width * 1.2; height: parent.height * 1.5; z: -1; on: pick.isFocus }
                        Rectangle {
                            anchors.fill: parent
                            radius: Lu.radius("row")
                            color: pick.isFocus ? Lu.wash("focus") : "transparent"
                            border.width: pick.isFocus ? 1 : 0
                            border.color: Lu.edgeRaised
                            antialiasing: true
                            scale: 1 + (Lu.rowScale - 1) * lift.value
                        }
                        LuAppIcon { visible: !pick.isMore; x: Math.round((pick.width - width) / 2); y: Lu.space(3); width: 76; height: 76; radius: 18; source: pick.isMore ? "" : (qa.picks[index] ? qa.picks[index].iconUrl || "" : "") }
                        LuTile { visible: pick.isMore; x: Math.round((pick.width - width) / 2); y: Lu.space(3); size: 76; icon: "ellipsis" }
                        LuText {
                            x: Lu.space(1); y: Lu.space(3) + 76 + Lu.space(2)
                            width: pick.width - 2 * Lu.space(1)
                            horizontalAlignment: Text.AlignHCenter
                            role: "caption"
                            elide: Text.ElideRight
                            text: pick.isMore ? "More\u2026" : (qa.picks[index] ? qa.picks[index].title : "")
                        }
                        MouseArea {
                            anchors.fill: parent
                            hoverEnabled: true
                            onEntered: qa.index = index
                            onClicked: { qa.index = index; qa.answer() }
                        }
                    }
                }
            }
            Item {
                width: parent.width
                height: later.height
                LuButton {
                    id: later
                    x: parent.width - width
                    text: "Not now"
                    focused: qa.asking && qa.index === qa.laterIndex
                    rested: qa.rested
                    onActivated: { qa.index = qa.laterIndex; qa.answer() }
                    onHoverFocus: qa.index = qa.laterIndex
                }
            }
        }
    }
}
