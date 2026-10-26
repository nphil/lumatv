import QtQuick 2.4
import "../kit"
import "../kit/Spatial.js" as Spatial

// All Apps (docs/mocks/12-all-apps, 12-all-apps-menu): the library behind the Apps tile. A rail down the left (All apps,
// a section per kind of app, On the dock, Hidden) and a grid of icons beside it with the name under each. The D-pad
// moves in the grid by the nearest icon and from its left edge into the rail; the rail changes the grid as the focus
// moves over it. OK opens an app, a hold of OK or the Menu key (the host's `hold()`) asks for the options
// (`menuRequested(app, anchor)`: Open, Move on the dock, Hide, Rename, Uninstall; the host draws and acts), the Search
// button at the top asks for search, Back closes. Two rows are in view; the grid scrolls a row when the focus passes the
// second.
//
// The apps are the library's (kit/Apps.qml: what the TV can launch, in dock order then by name, with the owner's hidden and
// renamed apps laid over). Sections: `theme.apps.categories` (id -> section) first, the word lists below for what the
// theme does not name, "Other" for the rest.
Item {
    id: page
    anchors.fill: parent
    property var apps: Apps.list              // [{ id, title, iconUrl, params, hidden, onDock, ... }]
    property string section: "all"
    property int railIndex: 0
    property int cellIndex: 0
    property int scrollRow: 0
    property bool inGrid: true
    property bool searchFocused: false
    property bool rested: true
    signal launched(var app)
    signal menuRequested(var app, var anchor)
    signal searchRequested()
    signal closeRequested()
    readonly property bool built: true

    readonly property var words: ({
        streaming: /netflix|youtube|plex|plx|apple ?tv|crunchyroll|lg movies|live ?tv|disney|hulu|prime|amazon|hbo|max\b|paramount|peacock|spotify|tubi|pluto|twitch|vudu|sling|philo/i,
        gaming: /geforce|game|xbox|stadia|luna|boosteroid|pool|steam|moonlight|punktfunk/i,
        utilities: /browser|hue|homebrew|hyperhdr|settings|file|usb|cast|share|miracast|airplay|photo|music|input|guide|support|manual|calculator|store|update|bridge|hub/i
    })
    readonly property var names: ({ streaming: "Streaming", gaming: "Gaming", utilities: "Utilities", other: "Other" })
    readonly property var subs: ({ all: "Everything on this TV", streaming: "Films, series and live TV", gaming: "Cloud gaming and games", utilities: "Tools and connections", other: "Everything else", dock: "In the order of the dock", hidden: "Kept out of the way, not removed" })
    readonly property var icons: ({ all: "layout-grid", streaming: "clapperboard", gaming: "gamepad-2", utilities: "wrench", other: "ellipsis", dock: "star", hidden: "eye-off" })
    function categoryOf(a) {
        var map = (Lu.theme.apps && Lu.theme.apps.categories) || {}
        if (map[a.id]) return map[a.id]
        var t = a.title || a.id
        return words.streaming.test(t) ? "streaming" : words.gaming.test(t) ? "gaming" : words.utilities.test(t) ? "utilities" : "other"
    }
    readonly property var shown: apps.filter(function(a) { return !a.hidden })
    readonly property var hiddenApps: apps.filter(function(a) { return a.hidden })
    // The rail: only the sections that have apps in them, always All apps first and Hidden (when there are any) last.
    readonly property var rail: {
        var out = [{ id: "all", title: "All apps", sub: shown.length + " installed" }]
        var kinds = ["streaming", "gaming", "utilities", "other"]
        for (var k = 0; k < kinds.length; k++) {
            var mine = shown.filter(function(a) { return page.categoryOf(a) === kinds[k] })
            if (!mine.length) continue
            // a count, not the names: three titles wrapped to a second line and ran into the next row on the set
            out.push({ id: kinds[k], title: names[kinds[k]], sub: mine.length + (mine.length === 1 ? " app" : " apps") })
        }
        var dock = shown.filter(function(a) { return a.onDock })
        out.push({ id: "dock", title: "On the dock", sub: dock.length + " tiles \u00b7 reorder here" })
        if (hiddenApps.length) out.push({ id: "hidden", title: "Hidden", sub: hiddenApps.length + (hiddenApps.length === 1 ? " app" : " apps") + " \u00b7 restore any time" })
        return out
    }
    readonly property var list: section === "all" ? shown : section === "dock" ? shown.filter(function(a) { return a.onDock })
        : section === "hidden" ? hiddenApps : shown.filter(function(a) { return page.categoryOf(a) === section })
    readonly property int cols: 5
    readonly property int visibleRows: 3
    readonly property int rowPitch: 237
    readonly property var heading: { for (var i = 0; i < rail.length; i++) if (rail[i].id === section) return rail[i]; return rail[0] }
    onListChanged: { if (cellIndex >= list.length) cellIndex = Math.max(0, list.length - 1); scrollRow = Math.min(scrollRow, Math.max(0, Math.floor((list.length - 1) / cols))) }
    onRailChanged: { if (railIndex >= rail.length) { railIndex = rail.length - 1; section = rail[railIndex].id } }

    function open() { section = "all"; railIndex = 0; cellIndex = 0; scrollRow = 0; inGrid = true; searchFocused = false }
    function showCell() {
        var row = Math.floor(cellIndex / cols)
        if (row < scrollRow) scrollRow = row
        else if (row > scrollRow + visibleRows - 1) scrollRow = row - visibleRows + 1
    }
    function boxes() {
        var out = []
        for (var i = 0; i < cellRep.count; i++) { var it = cellRep.itemAt(i); out.push(it ? { x: it.x, y: it.y, w: it.width, h: it.height } : null) }
        return out
    }
    function setSection(i) { railIndex = i; section = rail[i].id; cellIndex = 0; scrollRow = 0 }
    function key(dir) {
        if (dir === "back") { closeRequested(); return }
        if (searchFocused) {
            if (dir === "down") searchFocused = false
            else if (dir === "left") { searchFocused = false; inGrid = false }
            else if (dir === "ok") searchRequested()
        } else if (!inGrid) {
            if (dir === "up") setSection(Math.max(0, railIndex - 1))
            else if (dir === "down") setSection(Math.min(rail.length - 1, railIndex + 1))
            else if (dir === "right" || dir === "ok") { if (list.length) inGrid = true }
        } else {
            if (dir === "ok") { if (list[cellIndex]) launched(list[cellIndex]) }
            else {
                var n = Spatial.nearest(boxes(), cellIndex, dir)
                if (n >= 0) { cellIndex = n; showCell() }
                else if (dir === "left") inGrid = false
                else if (dir === "up") searchFocused = true
            }
        }
        rested = false; restTimer.restart()
    }
    // The long press of OK (or the Menu key) on an icon: the host opens the options beside it.
    function hold() {
        var c = cellRep.itemAt(cellIndex)
        if (!inGrid || searchFocused || !list[cellIndex] || !c) return
        var p = c.mapToItem(null, 0, 0)
        menuRequested(list[cellIndex], { x: Math.round(p.x), y: Math.round(p.y), width: c.width, height: c.height, iconUrl: list[cellIndex].iconUrl, place: "below" })
    }
    Timer { id: restTimer; interval: Lu.restMs; onTriggered: page.rested = true }

    Rectangle { anchors.fill: parent; color: Lu.scrim }
    // ---- the bar
    LuPane {
        x: 64; y: 56; width: 1792; height: 56; cardRadius: height / 2
        Row {
            x: Lu.space(5); anchors.verticalCenter: parent.verticalCenter; spacing: Lu.space(3)
            LuIcon { anchors.verticalCenter: parent.verticalCenter; name: "chevron-left"; size: 28; color: Lu.ink2 }
            LuText { anchors.verticalCenter: parent.verticalCenter; role: "body"; font.weight: Font.DemiBold; text: "Apps" }
        }
        Row {
            x: parent.width - width - Lu.space(6); anchors.verticalCenter: parent.verticalCenter; spacing: Lu.space(5)
            LuIcon { anchors.verticalCenter: parent.verticalCenter; name: "wifi"; size: 28; color: Lu.ink2 }
            LuIcon { anchors.verticalCenter: parent.verticalCenter; name: "mouse-pointer-2"; size: 28; color: Lu.ink2 }
            LuText { anchors.verticalCenter: parent.verticalCenter; role: "label"; color: Lu.ink2; text: Qt.formatTime(page.clock, "h:mm AP") }
        }
    }
    property date clock: new Date()
    // triggeredOnStart: the page is built long before it is shown, so the clock has to be read again when it is
    Timer { interval: 15000; running: page.visible; repeat: true; triggeredOnStart: true; onTriggered: page.clock = new Date() }

    // ---- the rail
    LuPane {
        id: railPane
        x: 64; y: 136; width: 400; height: 888
        Column {
            x: 16; y: 20; width: parent.width - 32; spacing: 4
            Repeater {
                model: page.rail.length
                delegate: Item {
                    readonly property var r: page.rail[index]
                    visible: r.id !== "hidden"
                    width: parent.width; height: visible ? Math.max(80, railRow.implicitHeight) : 0      // a detail that wraps grows its row
                    LuRow {
                        id: railRow
                        anchors.fill: parent
                        icon: page.icons[r.id] || "layout-grid"
                        label: r.title
                        detail: r.sub
                        selected: page.section === r.id
                        tileAccent: page.section === r.id
                        focused: !page.inGrid && !page.searchFocused && page.railIndex === index
                        rested: page.rested
                        onHoverFocus: { page.inGrid = false; page.searchFocused = false; page.setSection(index) }
                        onActivated: { page.setSection(index); page.inGrid = true }
                    }
                    Rectangle { visible: page.section === r.id; x: -6; y: 22; width: 5; height: parent.height - 44; radius: 3; color: Lu.accent; antialiasing: true }
                }
            }
        }
        // Hidden is the last entry of the rail and sits at the foot of it
        Item {
            visible: page.hiddenApps.length > 0
            x: 16; y: parent.height - 20 - height; width: parent.width - 32; height: Math.max(80, hiddenRow.implicitHeight)
            readonly property int at: page.rail.length - 1
            LuRow {
                id: hiddenRow
                anchors.fill: parent
                icon: "eye-off"
                label: "Hidden"
                detail: page.rail[parent.at].sub
                selected: page.section === "hidden"
                tileAccent: page.section === "hidden"
                focused: !page.inGrid && !page.searchFocused && page.railIndex === parent.at
                rested: page.rested
                onHoverFocus: { page.inGrid = false; page.searchFocused = false; page.setSection(parent.at) }
                onActivated: { page.setSection(parent.at); page.inGrid = true }
            }
            Rectangle { visible: page.section === "hidden"; x: -6; y: 22; width: 5; height: parent.height - 44; radius: 3; color: Lu.accent; antialiasing: true }
        }
    }

    // ---- the grid
    LuPane {
        id: gridPane
        x: 488; y: 136; width: 1368; height: 888
        Item {
            id: head
            x: 40; y: 28; width: parent.width - 80; height: 56
            Row {
                anchors.verticalCenter: parent.verticalCenter; spacing: Lu.space(4)
                LuText { anchors.verticalCenter: parent.verticalCenter; role: "title"; text: page.heading.title }
                LuText { anchors.verticalCenter: parent.verticalCenter; role: "caption"; color: Lu.ink2; text: page.subs[page.section] || "" }
            }
            BannerButton {
                x: parent.width - width; anchors.verticalCenter: parent.verticalCenter
                icon: "search"; text: "Search apps"; height: 56
                focused: page.searchFocused; rested: page.rested
                onHoverFocus: { page.searchFocused = true }
                onActivated: page.searchRequested()
            }
        }
        Item {
            id: gridArea
            x: 40; y: 28 + 56 + 22; width: parent.width - 80; height: parent.height - y - 70
            clip: true
            Item {
                id: gridSheet
                y: -page.scrollRow * page.rowPitch
                Behavior on y { enabled: !Lu.reduced; NumberAnimation { duration: Lu.cardMs; easing.type: Lu.cardEasing } }
                Repeater {
                    id: cellRep
                    model: page.list.length
                    delegate: Item {
                        id: cell
                        readonly property var a: page.list[index]
                        readonly property bool focused: page.inGrid && !page.searchFocused && page.cellIndex === index
                        x: (index % page.cols) * 262; y: Math.floor(index / page.cols) * page.rowPitch
                        width: 242; height: 221
                        z: focused ? 1 : 0
                        FocusLift { id: lift; focused: cell.focused; rested: page.rested; reduced: Lu.reduced; duration: Lu.cardMs; easing: Lu.cardEasing }
                        // the plate and the icon lift; the name stays put and sharp
                        Rectangle {
                            anchors.fill: parent; radius: Lu.radius("tile"); antialiasing: true
                            color: cell.focused ? Lu.wash("focus") : "transparent"; border.width: cell.focused ? 1 : 0; border.color: Lu.edgeRaised
                            scale: 1 + (Lu.buttonScale - 1) * lift.value; transformOrigin: Item.Center
                        }
                        Image {
                            x: Math.round((parent.width - 152) / 2); y: 14; width: 152; height: 152
                            source: cell.a ? cell.a.iconUrl : ""; fillMode: Image.PreserveAspectFit; asynchronous: true; cache: true; smooth: true
                            opacity: cell.a && cell.a.hidden ? 0.5 : 1
                            scale: 1 + (Lu.buttonScale - 1) * lift.value; transformOrigin: Item.Center
                        }
                        LuText { x: 12; y: 14 + 152 + 12; width: parent.width - 24; horizontalAlignment: Text.AlignHCenter; role: "label"; font.weight: Font.DemiBold; elide: Text.ElideRight; text: cell.a ? cell.a.title : "" }
                        MouseArea {
                            anchors.fill: parent; hoverEnabled: true
                            onEntered: { page.inGrid = true; page.searchFocused = false; page.cellIndex = index }
                            onClicked: { page.cellIndex = index; page.key("ok") }
                            onPressAndHold: { page.cellIndex = index; page.hold() }
                        }
                    }
                }
            }
        }
        // the hints under the grid
        Row {
            x: parent.width / 2 - width / 2; y: parent.height - 56; spacing: Lu.space(7)
            Repeater {
                model: [{ k: "OK", t: "Open" }, { k: "Hold OK", t: "Move, hide, rename, uninstall" }]
                delegate: Row {
                    spacing: Lu.space(2)
                    Rectangle { anchors.verticalCenter: parent.verticalCenter; width: kt.implicitWidth + 2 * Lu.space(2); height: 30; radius: 8; color: "transparent"; border.width: 1; border.color: Lu.edge; antialiasing: true
                                LuText { id: kt; anchors.centerIn: parent; role: "caption"; font.weight: Font.DemiBold; color: Lu.ink2; text: modelData.k } }
                    LuText { anchors.verticalCenter: parent.verticalCenter; role: "caption"; color: Lu.ink3; text: modelData.t }
                }
            }
        }
        LuText { visible: page.list.length === 0; anchors.centerIn: parent; role: "label"; color: Lu.ink2; text: "Nothing here" }
    }
}
