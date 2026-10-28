import QtQuick 2.4
import "../kit"
import "../Overlays.js" as Q

// One card of the overlay system (panels/Overlays.qml), docs/mocks/overlays: a glass pane 560 wide with a tile for what it is, the words, and
// the keys under them. `entry` is the queue's entry (Overlays.js): its class decides the tile and the tone, a score with two sides is drawn as a
// score, anything else as a title and a line. The card takes no focus and has no buttons: OK and Back act on it, and what OK does is said under
// the words ("OK Clear", "OK Open"), the first action of the record or the notification drawer.
//
// Its height follows its words (two lines of each at most), so a second card is placed under the first at its real height and a card that would
// end below the limit is known before it is shown (the overlay measures a probe of this same card). Nothing runs while it is out of sight: no
// timer, no animation; it fades and slides in from a touch to the right as the camera pop-up does (motion.cardMs) and out quicker.
Item {
    id: card
    property var entry: null                  // the queue's entry, or null
    property string iconName: ""              // the tile's glyph, resolved by the host (PluginHub.icon for a plugin's own)
    property real cardWidth: 560
    property real fillAlpha: 1                // below 1 over video: the glass lets the picture through to the reading alpha
    property bool drawerReady: false          // OK can open the notification drawer here
    property bool shown: false                // on the screen: in, else out (and invisible when it has gone)
    property bool probe: false                // only measured: never drawn, nothing animates
    property bool instant: false              // comes and goes at once (a card that moves up to the first place does not fade from the second)
    readonly property var e: entry || ({})
    readonly property var score: e.score || null
    readonly property real cardHeight: pane.height
    readonly property real padX: Lu.space(5)                  // 20, as the camera pop-up
    readonly property int lead: 56
    readonly property real whoX: padX + lead + Lu.space(4)
    readonly property real whoW: cardWidth - whoX - padX

    // the tone's glyph colour, pushed where the material needs it to read (Lu.warningInk, Lu.dangerInk; the light accent is 2.4:1 on the glass over
    // the worst frame, so it is darkened there as the status colours are)
    readonly property color toneInk: e.tone === "danger" ? Lu.dangerInk : e.tone === "warning" ? Lu.warningInk
                                     : (Lu.isLight ? Qt.darker(Lu.accent, 1.6) : Lu.accent)
    readonly property var keys: Q.keysOf(e, drawerReady)

    width: cardWidth
    height: cardHeight
    visible: !probe && opacity > 0
    opacity: probe ? 1 : (shown ? 1 : 0)
    scale: probe || shown ? 1 : 0.96
    transform: Translate { x: card.probe || card.shown ? 0 : 24 }
    Behavior on opacity { enabled: !card.probe && !card.instant && !Lu.reduced; NumberAnimation { duration: card.shown ? Lu.cardMs : Lu.restMs; easing.type: Lu.cardEasing } }
    Behavior on scale { enabled: !card.probe && !card.instant && !Lu.reduced; NumberAnimation { duration: Lu.cardMs; easing.type: Lu.cardEasing } }
    onOpacityChanged: if (opacity === 1) pane.locate()

    LuPane {
        id: pane
        width: card.cardWidth
        height: foot.y + foot.height + card.padX
        cardRadius: Lu.radius("card")
        fillAlpha: card.fillAlpha

        // what it is: the tile
        Rectangle {
            x: card.padX; y: card.padX
            width: card.lead; height: card.lead
            radius: Math.round(card.lead * 0.32)
            color: Lu.wash("selected")
            border.width: 1
            border.color: Lu.edge
            antialiasing: true
            LuIcon { anchors.centerIn: parent; size: 32; name: card.iconName; color: card.toneInk }
        }

        // a card of words: a title (two lines at most) and a line under it
        LuText {
            id: titleText
            visible: !card.score
            x: card.whoX; y: card.padX
            width: card.whoW
            role: "body"
            font.weight: Font.DemiBold
            wrapMode: Text.WordWrap
            maximumLineCount: 2
            elide: Text.ElideRight
            text: card.score ? "" : (card.e.title || "")
        }
        LuText {
            id: descText
            visible: !card.score && text !== ""
            x: card.whoX; y: titleText.y + titleText.height + Lu.space(1)
            width: card.whoW
            role: "label"
            color: Lu.ink2
            wrapMode: Text.WordWrap
            maximumLineCount: 2
            elide: Text.ElideRight
            text: card.score ? "" : (card.e.body || "")
        }

        // a score: the competition and the clock (with a live dot), then a line a side with the score at its end, the visitors first and the home side
        // under them as scores are read; the side that is ahead is in the full ink and semibold, the other a step down (one emphasis, no colour needed)
        Item {
            id: board
            visible: !!card.score
            x: card.whoX; y: card.padX
            width: card.whoW
            height: card.score ? metaText.height + Lu.space(2) + firstRow.height + secondRow.height : 0
            readonly property int firstN: card.score ? parseInt(card.score.away.score, 10) : 0
            readonly property int secondN: card.score ? parseInt(card.score.home.score, 10) : 0
            Rectangle {
                id: liveDot
                visible: !!card.score && card.score.live
                width: 12; height: 12; radius: 6
                color: Lu.live
                x: board.width - liveText.implicitWidth - Lu.space(2) - width
                y: Math.round((liveText.implicitHeight - height) / 2)
            }
            LuText {
                id: liveText
                visible: liveDot.visible
                x: board.width - implicitWidth
                role: "label"
                color: Lu.ink2
                text: "Live"
            }
            LuText {
                id: metaText
                width: board.width - (liveDot.visible ? liveText.implicitWidth + Lu.space(2) + liveDot.width + Lu.space(3) : 0)
                role: "label"
                color: Lu.ink2
                wrapMode: Text.WordWrap
                maximumLineCount: 2
                elide: Text.ElideRight
                text: !card.score ? "" : (card.score.league !== "" && card.score.status !== "" ? card.score.league + "  \u00b7  " + card.score.status : card.score.league + card.score.status)
            }
            Item {
                id: firstRow
                y: metaText.height + Lu.space(2)
                width: board.width
                height: Math.max(firstName.implicitHeight, firstScore.implicitHeight)
                LuText { id: firstName; y: Math.round((parent.height - implicitHeight) / 2); width: parent.width - firstScore.implicitWidth - Lu.space(4); role: "body"; elide: Text.ElideRight
                         font.weight: board.firstN > board.secondN ? Font.DemiBold : Font.Medium; color: board.firstN > board.secondN ? Lu.ink : Lu.ink2
                         text: card.score ? card.score.away.name : "" }
                LuText { id: firstScore; x: parent.width - implicitWidth; y: Math.round((parent.height - implicitHeight) / 2); role: "title"; text: card.score ? card.score.away.score : "" }
            }
            Item {
                id: secondRow
                y: firstRow.y + firstRow.height
                width: board.width
                height: Math.max(secondName.implicitHeight, secondScore.implicitHeight)
                LuText { id: secondName; y: Math.round((parent.height - implicitHeight) / 2); width: parent.width - secondScore.implicitWidth - Lu.space(4); role: "body"; elide: Text.ElideRight
                         font.weight: board.secondN > board.firstN ? Font.DemiBold : Font.Medium; color: board.secondN > board.firstN ? Lu.ink : Lu.ink2
                         text: card.score ? card.score.home.name : "" }
                LuText { id: secondScore; x: parent.width - implicitWidth; y: Math.round((parent.height - implicitHeight) / 2); role: "title"; text: card.score ? card.score.home.score : "" }
            }
        }

        // under the words: who said it, and the keys. On one line when they fit, else the keys take a line of their own, and at large text
        // each key takes one (a key's words are never cut).
        readonly property real whoH: card.score ? board.height : titleText.height + (descText.visible ? Lu.space(1) + descText.height : 0)
        Item {
            id: foot
            x: card.padX
            y: card.padX + Math.max(card.lead, pane.whoH) + Lu.space(3)
            width: card.cardWidth - 2 * card.padX
            readonly property int sp: Lu.space(5)
            readonly property bool hasSrc: src.text !== ""
            readonly property bool two: k2.text !== ""
            readonly property real keysW: k1.implicitWidth + (two ? sp + k2.implicitWidth : 0)
            readonly property bool oneLine: hasSrc ? src.implicitWidth + sp + keysW <= width : keysW <= width
            readonly property bool stacked: keysW > width                // the two keys on a line each
            height: (hasSrc && !oneLine ? src.implicitHeight + Lu.space(1) : 0) + (stacked ? k1.implicitHeight + Lu.space(1) + k2.implicitHeight : k1.implicitHeight)
            LuText {
                id: src
                width: Math.min(implicitWidth, foot.width)
                role: "caption"
                color: Lu.ink2
                elide: Text.ElideRight
                text: card.score ? "" : (card.e.source || "")
            }
            LuText {
                id: k1
                x: foot.oneLine && !foot.stacked ? foot.width - foot.keysW : 0
                y: foot.hasSrc && !foot.oneLine ? src.implicitHeight + Lu.space(1) : 0
                role: "caption"
                color: Lu.ink2
                text: card.keys.length > 0 ? card.keys[0].key + " " + card.keys[0].what : ""
            }
            LuText {
                id: k2
                x: foot.stacked ? 0 : k1.x + k1.implicitWidth + foot.sp
                y: foot.stacked ? k1.y + k1.implicitHeight + Lu.space(1) : k1.y
                role: "caption"
                color: Lu.ink2
                text: card.keys.length > 1 ? card.keys[1].key + " " + card.keys[1].what : ""
            }
        }
    }
}
