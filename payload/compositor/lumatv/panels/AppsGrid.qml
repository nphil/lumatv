import QtQuick 2.4
import "../kit"
import "../kit/Spatial.js" as Spatial

// All Apps (docs/mocks/12-all-apps, 12-all-apps-menu): the library behind the Apps tile. A rail down the left (All apps,
// a section per kind of app, On the dock, Hidden) and a grid of icons beside it with the name under each. The D-pad
// moves in the grid by the nearest icon and from its left edge into the rail; the rail changes the grid as the focus
// moves over it. OK opens an app, a hold of OK (the host's long-press) asks for the menu (`menuRequested(app, x, y)`),
// the Search button at the top asks for search, Back closes.
//
//   apps = [{ id, title, iconUrl, params, hidden, onDock }], categories from `theme.apps.categories` (id -> section) with the
//   built-in word lists below for what the theme does not name, "Other" for the rest.
Item {
    id: page
    anchors.fill: parent
    property var apps: []
    property string section: "all"
    property int railIndex: 0
    property int cellIndex: 0
    property bool inGrid: true
    property bool searchFocused: false
    property bool rested: true
    signal launched(var app)
    signal menuRequested(var app, int x, int y)
    signal searchRequested()
    signal closeRequested()
    readonly property bool built: true

    readonly property var words: ({
        streaming: /netflix|youtube|plex|apple ?tv|crunchyroll|lg movies|live ?tv|disney|hulu|prime|amazon|hbo|max\b|paramount|peacock|spotify|tubi|pluto|twitch|vudu|sling|philo/i,
        gaming: /geforce|game|xbox|stadia|luna|boosteroid|pool|steam|moonlight/i,
        utilities: /browser|hue|homebrew|settings|file|usb|cast|share|miracast|airplay|photo|music|input|guide|support|manual|calculator/i
    })
    readonly property var names: ({ streaming: "Streaming", gaming: "Gaming", utilities: "Utilities", other: "Other" })
    readonly property var subs: ({ streaming: "Films, series and live TV", gaming: "Cloud gaming and games", utilities: "Tools and connections", other: "Everything else" })
    readonly property var icons: ({ all: "layout-grid", streaming: "clapperboard", gaming: "gamepad-2", utilities: "cog", other: "ellipsis", dock: "star", hidden: "eye-off" })
    function categoryOf(a) {
        var map = (Lu.theme.apps && Lu.theme.apps.categories) || {}
        if (map[a.id]) return map[a.id]
        var t = a.title || a.id
        return words.streaming.test(t) ? "streaming" : words.gaming.test(t) ? "gaming" : words.utilities.test(t) ? "utilities" : "other"
    }
    readonly property var shown: apps.filter(function(a) { return !a.hidden })
    // The rail: only the sections that have apps in them, always All apps first.
    readonly property var rail: {
        var out = [{ id: "all", title: "All apps", sub: shown.length + " installed" }]
        var kinds = ["streaming", "gaming", "utilities", "other"]
        for (var k = 0; k < kinds.length; k++) {
            var mine = shown.filter(function(a) { return page.categoryOf(a) === kinds[k] })
            if (mine.length) out.push({ id: kinds[k], title: names[kinds[k]], sub: mine.length + (mine.length === 1 ? " app" : " apps") })
        }
        var dock = shown.filter(function(a) { return a.onDock })
        out.push({ id: "dock", title: "On the dock", sub: dock.length + " tiles \u00b7 reorder here" })
        var hid = apps.filter(function(a) { return a.hidden })
        if (hid.length) out.push({ id: "hidden", title: "Hidden", sub: hid.length + (hid.length === 1 ? " app" : " apps") + " \u00b7 restore any time" })
        return out
    }
    readonly property var list: section === "all" ? shown : section === "dock" ? shown.filter(function(a) { return a.onDock })
        : section === "hidden" ? apps.filter(function(a) { return a.hidden }) : shown.filter(function(a) { return page.categoryOf(a) === section })
    readonly property int cols: 5
    readonly property var heading: { for (var i = 0; i < rail.length; i++) if (rail[i].id === section) return rail[i]; return rail[0] }

    function open() { section = "all"; railIndex = 0; cellIndex = 0; inGrid = true; searchFocused = false }
    function boxes() {
        var out = []
        for (var i = 0; i < cellRep.count; i++) { var it = cellRep.itemAt(i); out.push(it ? { x: it.x, y: it.y, w: it.width, h: it.height } : null) }
        return out
    }
    function key(dir) {
        if (dir === "back") { closeRequested(); return }
        if (searchFocused) {
            if (dir === "down") searchFocused = false
            else if (dir === "left") { searchFocused = false; inGrid = false }
            else if (dir === "ok") searchRequested()
        } else if (!inGrid) {
            if (dir === "up") { railIndex = Math.max(0, railIndex - 1); section = rail[railIndex].id; cellIndex = 0 }
            else if (dir === "down") { railIndex = Math.min(rail.length - 1, railIndex + 1); section = rail[railIndex].id; cellIndex = 0 }
            else if (dir === "right" || dir === "ok") { if (list.length) inGrid = true }
        } else {
            if (dir === "ok") { if (list[cellIndex]) launched(list[cellIndex]) }
            else {
                var n = Spatial.nearest(boxes(), cellIndex, dir)
                if (n >= 0) cellIndex = n
                else if (dir === "left") inGrid = false
                else if (dir === "up") searchFocused = true
            }
        }
        rested = false; restTimer.restart()
    }
    // the long press of OK on an icon
    function hold() { var c = cellRep.itemAt(cellIndex); if (inGrid && list[cellIndex] && c) menuRequested(list[cellIndex], Math.round(c.x + gridArea.x + c.width * 0.5), Math.round(c.y + gridArea.y + c.height)) }
    Timer { id: restTimer; interval: Lu.restMs; onTriggered: page.rested = true }

    Rectangle { anchors.fill: parent; color: Lu.scrim }
    // ---- the bar
    LuPane {
        x: 64; y: 40; width: 1792; height: 72; cardRadius: Lu.radius("pill")
        Row {
            x: Lu.space(6); anchors.verticalCenter: parent.verticalCenter; spacing: Lu.space(3)
            LuIcon { anchors.verticalCenter: parent.verticalCenter; name: "chevron-left"; size: 28; color: Lu.ink2 }
            LuText { anchors.verticalCenter: parent.verticalCenter; role: "label"; text: "Apps" }
        }
        Row {
            x: parent.width - width - Lu.space(6); anchors.verticalCenter: parent.verticalCenter; spacing: Lu.space(4)
            LuIcon { anchors.verticalCenter: parent.verticalCenter; name: "wifi"; size: 26; color: Lu.ink2 }
            LuText { anchors.verticalCenter: parent.verticalCenter; role: "label"; color: Lu.ink2; text: Qt.formatTime(clock.now, "h:mm AP") }
        }
    }
    QtObject { id: clock; property date now: new Date() }
    Timer { interval: 15000; running: page.visible; repeat: true; onTriggered: clock.now = new Date() }

    // ---- the rail
    LuPane {
        id: railPane
        x: 64; y: 136; width: 380; height: 1080 - 136 - 64
        Column {
            x: Lu.space(3); y: Lu.space(3); width: parent.width - 2 * Lu.space(3)
            Repeater {
                model: page.rail.length
                delegate: LuRow {
                    readonly property var r: page.rail[index]
                    width: parent.width
                    icon: page.icons[r.id] || "layout-grid"
                    label: r.title
                    detail: r.sub
                    selected: page.section === r.id
                    tileAccent: page.section === r.id
                    focused: !page.inGrid && !page.searchFocused && page.railIndex === index
                    rested: page.rested
                    onHoverFocus: { page.inGrid = false; page.railIndex = index; page.section = r.id; page.cellIndex = 0 }
                    onActivated: { page.railIndex = index; page.section = r.id; page.inGrid = true }
                }
            }
        }
    }

    // ---- the grid
    LuPane {
        id: gridPane
        x: 468; y: 136; width: 1920 - 468 - 64; height: 1080 - 136 - 64
        Item {
            x: Lu.space(7); y: Lu.space(5); width: parent.width - 2 * Lu.space(7); height: 72
            Row {
                anchors.verticalCenter: parent.verticalCenter; spacing: Lu.space(4)
                LuText { anchors.verticalCenter: parent.verticalCenter; role: "title"; text: page.heading.title }
                LuText { anchors.verticalCenter: parent.verticalCenter; role: "caption"; color: Lu.ink2; text: page.subs[page.section] || "" }
            }
            LuButton {
                x: parent.width - width; anchors.verticalCenter: parent.verticalCenter
                text: "Search apps"; focused: page.searchFocused; rested: page.rested
                onHoverFocus: { page.searchFocused = true }
                onActivated: page.searchRequested()
            }
        }
        Item {
            id: gridArea
            x: Lu.space(7); y: Lu.space(5) + 72 + Lu.space(4); width: parent.width - 2 * Lu.space(7); height: parent.height - y - 72
            readonly property int cellW: Math.floor(width / page.cols)
            readonly property int cellH: 232
            Repeater {
                id: cellRep
                model: page.list.length
                delegate: Item {
                    readonly property var a: page.list[index]
                    readonly property bool focused: page.inGrid && !page.searchFocused && page.cellIndex === index
                    x: (index % page.cols) * gridArea.cellW + Math.round((gridArea.cellW - width) / 2)
                    y: Math.floor(index / page.cols) * gridArea.cellH
                    width: 176; height: 216
                    z: focused ? 1 : 0
                    FocusLift { id: lift; focused: parent.focused; rested: page.rested; reduced: Lu.reduced; duration: Lu.cardMs; easing: Lu.cardEasing }
                    scale: 1 + (Lu.buttonScale - 1) * lift.value
                    transformOrigin: Item.Center
                    Rectangle {
                        anchors.fill: parent; radius: Lu.radius("card")
                        color: parent.focused ? Lu.wash("focus") : "transparent"; border.width: parent.focused ? 1 : 0; border.color: Lu.edgeRaised; antialiasing: true
                    }
                    Image {
                        x: 12; y: 12; width: 152; height: 152; source: a ? a.iconUrl : ""; fillMode: Image.PreserveAspectFit; asynchronous: true; cache: true; smooth: true
                        opacity: a && a.hidden ? 0.5 : 1
                    }
                    LuText { x: 8; y: 172; width: parent.width - 16; horizontalAlignment: Text.AlignHCenter; role: "caption"; elide: Text.ElideRight; text: a ? a.title : "" }
                    MouseArea { anchors.fill: parent; hoverEnabled: true; onEntered: { page.inGrid = true; page.searchFocused = false; page.cellIndex = index } onClicked: { page.cellIndex = index; page.key("ok") } }
                }
            }
        }
        // the hints under the grid
        Row {
            x: parent.width / 2 - width / 2; y: parent.height - 56; spacing: Lu.space(6)
            Repeater {
                model: [{ k: "OK", t: "Open" }, { k: "Hold OK", t: "Move, hide, rename, uninstall" }, { k: "1-9", t: "Open a dock tile" }]
                delegate: Row {
                    spacing: Lu.space(2)
                    Rectangle { anchors.verticalCenter: parent.verticalCenter; width: kt.implicitWidth + 2 * Lu.space(3); height: 28; radius: 8; color: Lu.wash("hover"); border.width: 1; border.color: Lu.edge
                                LuText { id: kt; anchors.centerIn: parent; role: "caption"; text: modelData.k } }
                    LuText { anchors.verticalCenter: parent.verticalCenter; role: "caption"; color: Lu.ink3; text: modelData.t }
                }
            }
        }
        LuText { visible: page.list.length === 0; anchors.centerIn: parent; role: "label"; color: Lu.ink2; text: "Nothing here" }
    }
}
