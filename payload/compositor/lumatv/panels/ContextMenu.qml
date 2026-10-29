import QtQuick 2.4
import "../kit"

// A list that opens next to something (docs/mocks/16-context-menu, 17-power-menu, 33-pointer-menu): the options on
// a dock tile (Open, Move, Hide from the dock, Rename, Uninstall), the power menu, the menu under the Magic
// Remote's pointer. One built item that is given a spec when it opens, so nothing is created per open:
//
//   spec = { title, sub, icon (Lucide) | iconUrl (a picture), x, y (where it opens, in the 1920x1080 scene),
//            anchor: { x, y, width, height, place } (instead of x, y: opens beside that rectangle, level with its bottom edge,
//            on the left of it when it does not fit on the right; place "below" opens under it, left edges level, above it
//            when it does not fit),
//            raised: { x, y, width, height, iconUrl, plate, inset, radius } (the thing the menu belongs to, drawn above the
//            scrim so it stays lit: a dock tile or a library icon),
//            items: [{ id, icon, label, detail, hint, danger, options: [...], index }], hint }
//
// An item with `options` is a segmented row (the sleep timer): Left and Right move inside it. Up and Down move,
// OK picks (`picked(id, index)`), Back closes. Danger items (Uninstall) are in the danger colour and are
// asked about twice by whoever receives them. The panel stays on screen: it is moved back inside the safe area when
// the given place would put it outside.
Item {
    id: menu
    anchors.fill: parent
    property var spec: ({ items: [] })
    property int index: 0
    property int seg: 0
    property bool rested: true
    signal picked(string id, int index)
    signal closeRequested()
    readonly property bool built: true
    readonly property var items: spec.items || []
    readonly property int pad: Lu.space(3)
    readonly property int rowH: Math.round(Lu.row * 0.8)
    readonly property int rowH2: Lu.row + Lu.space(2)         // a row with a line of detail under the label
    readonly property int bodyH: { var h = 0; for (var i = 0; i < items.length; i++) h += items[i].detail ? rowH2 : rowH; return h }
    readonly property bool hasHead: !!(spec.title || spec.sub)

    function open(s) {
        spec = s
        index = 0
        for (var i = 0; i < items.length; i++) if (!items[i].disabled) { index = i; break }
        seg = items[index] && items[index].options ? (items[index].index || 0) : 0
    }
    function key(dir) {
        var it = items[index]
        if (dir === "up" || dir === "down") {
            var n = index + (dir === "up" ? -1 : 1)
            while (n >= 0 && n < items.length && items[n].disabled) n += dir === "up" ? -1 : 1
            if (n >= 0 && n < items.length) { index = n; seg = items[n].options ? (items[n].index || 0) : 0 }
        } else if ((dir === "left" || dir === "right") && it && it.options) {
            seg = Math.max(0, Math.min(it.options.length - 1, seg + (dir === "left" ? -1 : 1)))
        } else if (dir === "ok" && it) picked(it.id, seg)
        else if (dir === "back") closeRequested()
        rested = false
        restTimer.restart()
    }
    Timer { id: restTimer; interval: Lu.restMs; onTriggered: menu.rested = true }

    Rectangle { anchors.fill: parent; color: spec.scrim === false ? "transparent" : Lu.scrim; opacity: 0.6 }
    // the thing the menu belongs to, lit above the scrim
    Item {
        visible: !!menu.spec.raised
        readonly property var r: menu.spec.raised || ({ x: 0, y: 0, width: 0, height: 0 })
        x: r.x; y: r.y; width: r.width; height: r.height
        LuHalo { anchors.centerIn: parent; width: parent.width * 1.9; height: parent.height * 1.9; z: -1; on: visible }
        Rectangle { visible: !!parent.r.plate; anchors.fill: parent; radius: parent.r.radius || Lu.radius("tile"); color: parent.r.plate || "transparent"; border.width: 1; border.color: Lu.edgeRaised; antialiasing: true }
        Image {
            x: parent.r.inset || 0; y: parent.r.inset || 0; width: parent.width - 2 * (parent.r.inset || 0); height: parent.height - 2 * (parent.r.inset || 0)
            source: parent.r.iconUrl || ""; fillMode: Image.PreserveAspectFit; asynchronous: false; cache: true; smooth: true
        }
    }
    LuPane {
        id: card
        width: spec.width || 520
        height: (menu.hasHead ? Math.round(Lu.row * 1.0) : 0) + menu.bodyH + 2 * menu.pad + (spec.hint ? Math.round(Lu.row * 0.55) : 0)
        readonly property var anchor: menu.spec.anchor || null
        readonly property bool below: !!anchor && anchor.place === "below"
        readonly property real wantX: !anchor ? (spec.x === undefined ? Math.round((1920 - width) / 2) : spec.x)
                                      : below ? anchor.x
                                      : (anchor.x + anchor.width + Lu.space(4) + width + 48 <= 1920 ? anchor.x + anchor.width + Lu.space(4) : anchor.x - Lu.space(4) - width)
        readonly property real wantY: !anchor ? (spec.y === undefined ? Math.round((1080 - height) / 2) : spec.y)
                                      : below ? (anchor.y + anchor.height + Lu.space(1) + height + 48 <= 1080 ? anchor.y + anchor.height + Lu.space(1) : anchor.y - Lu.space(1) - height)
                                      : anchor.y + anchor.height - height
        x: Math.max(48, Math.min(1920 - 48 - width, wantX))
        y: Math.max(48, Math.min(1080 - 48 - height, wantY))
        cardRadius: Lu.radius("card")
        onXChanged: locate()
        onYChanged: locate()

        Item {
            id: head
            visible: menu.hasHead
            x: menu.pad; y: menu.pad; width: parent.width - 2 * menu.pad; height: menu.hasHead ? Math.round(Lu.row * 1.0) : 0
            Image {
                visible: !!menu.spec.iconUrl
                x: Lu.space(3); anchors.verticalCenter: parent.verticalCenter; width: 48; height: 48; source: menu.spec.iconUrl || ""
                fillMode: Image.PreserveAspectFit; asynchronous: false; cache: true
            }
            LuTile { visible: !!menu.spec.icon; x: Lu.space(3); anchors.verticalCenter: parent.verticalCenter; size: 48; icon: menu.spec.icon || "" }
            Column {
                x: Lu.space(3) + 48 + Lu.space(4); anchors.verticalCenter: parent.verticalCenter; width: parent.width - x
                LuText { width: parent.width; role: "label"; text: menu.spec.title || ""; elide: Text.ElideRight }
                LuText { width: parent.width; role: "caption"; color: Lu.ink2; text: menu.spec.sub || ""; visible: !!menu.spec.sub; elide: Text.ElideRight }
            }
        }
        Column {
            x: menu.pad; y: menu.pad + head.height; width: parent.width - 2 * menu.pad
            Repeater {
                model: menu.items.length
                delegate: LuRow {
                    readonly property var it: menu.items[index]
                    width: parent.width
                    height: it.detail ? menu.rowH2 : menu.rowH
                    icon: it.icon || ""
                    label: it.label || ""
                    danger: !!it.danger
                    detail: it.detail || ""
                    focused: menu.index === index
                    rested: menu.rested
                    enabled: !it.disabled
                    onPointerFocus: { menu.index = index; menu.seg = it.options ? (it.index || 0) : 0 }
                    onActivated: { menu.index = index; menu.key("ok") }
                    trailing: Item {
                        width: it.options ? seg.width : hintText.width
                        height: it.options ? seg.height : hintText.height
                        LuText { id: hintText; visible: !it.options && !!it.hint; role: "caption"; color: Lu.ink2; text: it.hint || "" }
                        LuSegmented {
                            id: seg
                            visible: !!it.options
                            textRole: "caption"
                            options: it.options || []
                            current: it.index || 0
                            focusedIndex: menu.index === index ? menu.seg : -1
                        }
                    }
                }
            }
        }
        LuText {
            visible: !!menu.spec.hint
            x: menu.pad + Lu.space(3); y: parent.height - Math.round(Lu.row * 0.55) + Lu.space(1); width: parent.width - 2 * menu.pad
            role: "caption"; color: Lu.ink3; text: menu.spec.hint || ""; elide: Text.ElideRight
        }
    }
}
