import QtQuick 2.4
import WebOSCompositorBase 1.0
import WebOSServices 1.0
import "../kit"

// The screensaver and art mode (docs/mocks/21-screensaver-*). Drawn over everything while the TV's power state is `Screen Saver`
// (`tvpower/power/getPowerState`, which the helper publishes as `screen.state`; LG's own saver is not a window of anything LumaTV
// owns, so this one is drawn in the compositor on that state), and gone the moment it changes. The owner's options are
// `theme.screensaver` (Settings › Home & look › Screensaver): `style` clock, art or off; `dim`; `glance`; `art.everyMin`,
// `art.order`, `art.caption`. (`afterMin` is when the TV's own power manager starts the state; nothing here counts idle time.)
// The rest of `theme.screensaver` is the burn-in tokens (docs/PLAN.md, Screensaver; docs/mocks/BATCH3.md): `clockLevel`, `artLevel`,
// `dimTo` over `dimOverMin`, `moveEverySec`, `fadeMs`, `crossfadeMs`, `preloadMs`, `graceMs`, `shiftEverySec`, `standbyHours`.
//
// Burn-in rules: the screen is black but for what is shown, everything shown moves, and nothing is bright.
// - clock: the block (time, date, a glance line) sits at `clockLevel` of full brightness on black and moves to another spot of the
//   screen every `moveEverySec`, fading out and in (`fadeMs` each: a black fade of twice that) so the move is never seen as motion;
//   with `dim` its brightness runs down to `dimTo` of that over `dimOverMin`.
// - art: a public-domain print in a matte and a frame on a dark wall at `artLevel`, the caption and a small clock at the corners.
//   All of it shifts a few pixels in a slow loop (one pixel at a time, every `shiftEverySec`, so nothing is redrawn between steps),
//   and a new print crossfades in (`crossfadeMs`) every `everyMin` minutes. The next print is loaded in the background `preloadMs`
//   before its turn, and one that has not loaded `graceMs` after its turn is skipped. The prints are on the set (`art/`, with their
//   licence checks: tools/screensaver_art.py), never fetched.
// - After `standbyHours` in the saver the TV is put in standby (0: never): the panel is not left on a picture overnight.
// - No timer runs and no image is kept while it is hidden: a surface that is not shown costs nothing.
//
// The host (Layers.qml) builds it once and shows it with `visible` while `active`. A key while it is up is the host's to hand to
// `key()`: it asks the TV to turn the screen on and does nothing else, so the key that wakes the TV never acts on Home.
Item {
    id: saver
    anchors.fill: parent
    readonly property bool built: true

    // ---- what the owner chose
    readonly property var cfg: (Lu.theme && Lu.theme.screensaver) || ({})
    readonly property var artCfg: cfg.art || ({})
    readonly property string style: cfg.style === "art" ? "art" : cfg.style === "off" ? "off" : "clock"
    readonly property bool dim: cfg.dim !== false
    readonly property bool glance: cfg.glance !== false
    readonly property int everyMin: artCfg.everyMin || 15
    readonly property bool shuffle: artCfg.order !== "sequence"
    readonly property bool caption: artCfg.caption !== false
    readonly property bool twelveHour: !(Lu.theme && Lu.theme.clock && Lu.theme.clock.twelveHour === false)
    // It sits on black, so it takes the dark material's inks whichever material Home is in.
    readonly property var darkRamp: (Lu.theme && Lu.theme.dark) || ({})
    readonly property color ink: Lu.asColor(darkRamp.ink || "white")
    readonly property color ink2: Lu.asColor(darkRamp.ink2 || "white")

    // ---- when it is up
    property bool forced: false                   // shown without the power state (a preview, a test)
    readonly property bool powerState: !!(Helper.screen && Helper.screen.state === "Screen Saver")
    readonly property bool active: style !== "off" && (forced || powerState)
    readonly property bool up: visible && active  // what every timer follows

    // ---- the burn-in tokens of theme.json (the mock's numbers: docs/mocks/lucent.css `.ss-clock`, `.art`)
    readonly property real clockLevel: cfg.clockLevel
    readonly property real artLevel: cfg.artLevel
    readonly property int crossfadeMs: Lu.reduced ? Lu.pressMs : cfg.crossfadeMs
    readonly property int graceMs: cfg.graceMs
    // Where things sit: the TV's safe area, and the corners of the art mode.
    readonly property int safeX: 96
    readonly property int safeY: 54
    readonly property int edgeX: 64
    // How often things happen. A test shortens them.
    property int fadeMs: Lu.reduced ? Lu.pressMs : cfg.fadeMs                 // the clock block fades out, and in, this long
    property int moveEveryMs: cfg.moveEverySec * 1000      // the clock block moves
    property int periodMs: everyMin * 60000                // a new print comes
    property int leadMs: cfg.preloadMs                     // the next print loads this long before its turn
    property int shiftMs: cfg.shiftEverySec * 1000         // the art takes a step along its loop
    property double standbyMs: cfg.standbyHours * 3600000  // in the saver this long: standby
    property string indexUrl: Lu.baseUrl + "art/index.json"

    property double since: 0                      // when it came up
    property date now: new Date()
    property real level: 1                        // 1 at first, down to `dimTo` over `dimOverMin` (with `dim`)

    function refreshTime() {
        now = new Date()
        var minutes = since > 0 ? (Date.now() - since) / 60000 : 0
        level = dim ? 1 - (1 - cfg.dimTo) * Math.min(1, minutes / cfg.dimOverMin) : 1
    }
    function msToNextMinute() { var d = new Date(); return 60000 - (d.getSeconds() * 1000 + d.getMilliseconds()) + 20 }
    function clockText() { return Qt.formatTime(now, twelveHour ? "h:mm" : "HH:mm") }

    // ---- keys: any key wakes the TV and nothing else happens
    signal woke()
    function wake() {
        LS.adhoc.call("luna://com.webos.service.tvpower", "/power/turnOnScreen", "{}")
        woke()
    }
    function key(dir) { wake() }
    // The panel is not left on a picture overnight.
    function standby() { LS.adhoc.call("luna://com.webos.service.tvpower", "/power/powerOff", JSON.stringify({ reason: "remoteKey" })) }
    Timer { id: standbyTimer; interval: Math.max(1, saver.standbyMs); running: saver.up && saver.standbyMs > 0; onTriggered: saver.standby() }

    onUpChanged: {
        if (up) {
            since = Date.now()
            refreshTime()
            minute.interval = msToNextMinute()
            clockBlock.pick()
            if (style === "art") art.start()
        } else {
            art.stop()
            clockBlock.fade = 1
        }
    }
    onStyleChanged: if (up) { if (style === "art") art.start(); else art.stop() }
    onPeriodMsChanged: if (up && style === "art" && art.current >= 0) lead.restart()

    // A preview (Settings' button, the scenes): shows it without the power state, optionally at a place and with a picture.
    function preview(x, y, pictureId) {
        forced = true
        if (x !== undefined) clockBlock.placeAt(x, y)
        if (pictureId) art.show(pictureId)
    }

    // Whether anything is running: nothing may be while it is hidden.
    readonly property bool timersRunning: standbyTimer.running || minute.running || move.running || lead.running || turn.running || grace.running || crossfade.running || shift.running || moveClock.running
    function snapshot() {
        return JSON.stringify({ up: up, active: active, style: style, level: level, timers: timersRunning, clock: { fx: clockBlock.fx, fy: clockBlock.fy, fade: clockBlock.fade, opacity: clockBlock.opacity },
                                art: { loaded: art.pictures.length, current: art.current, pending: art.pending, front: art.aFront, skipped: art.skipped, dx: art.dx, dy: art.dy,
                                       shown: art.shown.meta ? art.shown.meta.id : "", idle: art.idle.meta ? art.idle.meta.id : "", shownOpacity: art.shown.opacity, idleOpacity: art.idle.opacity } })
    }

    Timer {
        id: minute
        running: saver.up
        repeat: true
        onTriggered: { saver.refreshTime(); interval = saver.msToNextMinute() }
    }

    Rectangle { anchors.fill: parent; color: "#000000" }

    // ================================================================= clock
    Item {
        id: clockBlock
        visible: saver.style === "clock"
        // where the block is, as fractions of the room it may move in
        property real fx: 0.36
        property real fy: 0.58
        property real fade: 1
        readonly property real roomW: 1920 - 2 * saver.safeX - block.width
        readonly property real roomH: 1080 - 2 * saver.safeY - block.height
        x: Math.round(saver.safeX + fx * Math.max(0, roomW))
        y: Math.round(saver.safeY + fy * Math.max(0, roomH))
        width: block.width; height: block.height
        opacity: saver.clockLevel * saver.level * fade

        function placeAt(x, y) { fx = x; fy = y }
        // Another spot, far enough from this one to be a different place.
        function pick() {
            for (var i = 0; i < 12; i++) {
                var nx = Math.random(), ny = Math.random()
                var dx = (nx - fx) * roomW, dy = (ny - fy) * roomH
                if (Math.sqrt(dx * dx + dy * dy) > 420) { fx = nx; fy = ny; return }
            }
            fx = fx < 0.5 ? 0.85 : 0.15
        }
        SequentialAnimation {
            id: move
            NumberAnimation { target: clockBlock; property: "fade"; to: 0; duration: saver.fadeMs; easing.type: Easing.InOutQuad }
            ScriptAction { script: clockBlock.pick() }
            NumberAnimation { target: clockBlock; property: "fade"; to: 1; duration: saver.fadeMs; easing.type: Easing.InOutQuad }
        }
        Timer {
            id: moveClock
            interval: saver.moveEveryMs
            running: saver.up && saver.style === "clock"
            repeat: true
            onTriggered: if (Lu.reduced) { clockBlock.pick() } else move.restart()
        }
        Column {
            id: block
            spacing: 10
            LuText {
                role: "display"
                font.pixelSize: 152; font.weight: Font.Medium; font.letterSpacing: -152 * 0.03; lineHeight: 0.95
                color: saver.ink; text: saver.clockText()
            }
            LuText { role: "title"; font.pixelSize: 30; font.weight: Font.Medium; color: saver.ink2; text: Qt.formatDate(saver.now, "dddd, MMMM d") }
            Item {
                visible: saver.glance && (weatherText.text !== "" || homeText.text !== "")
                width: glanceRow.width; height: glanceRow.height + 10
                Row {
                    id: glanceRow
                    y: 10
                    spacing: 22
                    LuIcon { visible: weatherText.text !== ""; y: 4; size: 26; name: saver.weatherIcon; color: saver.ink2 }
                    LuText { id: weatherText; role: "label"; font.pixelSize: 24; color: saver.ink2; text: saver.weatherLine }
                    LuText { visible: weatherText.text !== "" && homeText.text !== ""; role: "label"; font.pixelSize: 24; color: saver.ink2; text: "\u00b7" }
                    LuIcon { visible: homeText.text !== ""; y: 4; size: 26; name: saver.homeIcon; color: saver.ink2 }
                    LuText { id: homeText; role: "label"; font.pixelSize: 24; color: saver.ink2; text: saver.homeLine }
                }
            }
        }
    }

    // What the glance line says: the weather plugin's card (the temperature and the conditions) and the first card of any other
    // plugin (what the home says: a door locked). Both are what the plugins published, read only while the saver is up.
    readonly property var weatherCard: up ? PluginHub.card("weather", "weather") : null
    readonly property var homeCard: up && PluginHub.homeCards.length ? PluginHub.homeCards[0] : null
    readonly property string weatherLine: {
        var f = weatherCard && weatherCard.content.facts
        return f && f.length ? f[0].value + (f.length > 1 ? " " + f[1].value : "") : ""
    }
    readonly property string weatherIcon: weatherCard ? PluginHub.icon(weatherCard.content.icon) : ""
    readonly property string homeLine: {
        if (!homeCard) return ""
        var g = PluginHub.glance(homeCard)
        return g.sub !== "" ? g.title + " \u00b7 " + g.sub : g.title
    }
    readonly property string homeIcon: homeCard ? PluginHub.icon(homeCard.content.icon) : ""

    // ================================================================== art
    QtObject {
        id: art
        property var pictures: []                  // art/index.json, public-domain pictures only
        property bool loaded: false
        property var bag: []
        property int current: -1                   // the picture shown (an index of `pictures`)
        property int pending: -1                   // the one loading into the idle piece
        property bool aFront: true
        property int skipped: 0
        property int dx: 0                         // the shift, whole pixels
        property int dy: 0
        property double began: 0
        readonly property Item shown: aFront ? pieceA : pieceB
        readonly property Item idle: aFront ? pieceB : pieceA

        function loadIndex(done) {
            if (loaded) { done(); return }
            var x = new XMLHttpRequest()
            x.onreadystatechange = function() {
                if (x.readyState !== 4) return
                try { art.pictures = (JSON.parse(x.responseText).pictures || []).filter(function(p) { return p.licence === "Public domain" }) } catch (e) { art.pictures = [] }
                art.loaded = true
                done()
            }
            try { x.open("GET", saver.indexUrl); x.send() } catch (e) { loaded = true; pictures = []; done() }
        }
        function start() {
            began = Date.now()
            loadIndex(function() { if (saver.up && saver.style === "art" && art.current < 0) art.first() })
        }
        function stop() {
            lead.stop(); turn.stop(); grace.stop(); crossfade.stop(); shift.stop()
            pieceA.clear(); pieceB.clear()
            pieceA.opacity = 0; pieceB.opacity = 0
            current = -1; pending = -1; aFront = true; skipped = 0; dx = 0; dy = 0
        }
        // The next print in the order the owner chose; the shuffle draws from a bag, so none repeats before all have been shown.
        function nextIndex() {
            var n = pictures.length
            if (n === 0) return -1
            // in order: after the print last tried, which is not the one shown while a broken print is being passed over
            if (!saver.shuffle) return ((pending >= 0 ? pending : current) + 1) % n
            if (bag.length === 0) {
                var b = []
                for (var i = 0; i < n; i++) b.push(i)
                for (var j = n - 1; j > 0; j--) { var k = Math.floor(Math.random() * (j + 1)); var t = b[j]; b[j] = b[k]; b[k] = t }
                if (n > 1 && b[0] === current) { var s = b[0]; b[0] = b[1]; b[1] = s }
                bag = b
            }
            var out = bag[0]
            bag = bag.slice(1)
            return out
        }
        function indexOfId(id) { for (var i = 0; i < pictures.length; i++) if (pictures[i].id === id) return i; return -1 }
        function showIndex(i) {
            current = i; pending = -1
            shown.put(pictures[i]); shown.opacity = 1
            idle.clear(); idle.opacity = 0
            lead.interval = Math.max(200, saver.periodMs - saver.leadMs)
            lead.restart()
        }
        function first() { var i = nextIndex(); if (i >= 0) showIndex(i) }
        function show(id) { loadIndex(function() { var i = art.indexOfId(id); if (i >= 0) art.showIndex(i) }) }
        // Six seconds before its turn the next print starts loading into the idle piece.
        function preload() {
            var i = nextIndex()
            if (i < 0) return
            pending = i
            idle.put(pictures[i])
            turn.restart()
        }
        // Its turn: crossfade if it is there, skip it if it could not be loaded, give it 300 ms if it is still loading.
        function arrive() {
            if (idle.ready) { crossfade.restart(); return }
            if (idle.broken) { skip(); return }
            grace.restart()
        }
        function late() { if (idle.ready) crossfade.restart(); else skip() }
        function skip() {
            skipped++
            if (skipped > pictures.length) { skipped = 0; lead.restart(); return }
            var i = nextIndex()
            if (i < 0) return
            pending = i
            idle.put(pictures[i])
            grace.restart()
        }
        function swapped() {
            aFront = !aFront
            idle.clear(); idle.opacity = 0
            current = pending; skipped = 0
            lead.interval = Math.max(200, saver.periodMs - saver.leadMs)
            lead.restart()
        }
    }
    Timer { id: lead; running: false; onTriggered: art.preload() }
    Timer { id: turn; interval: saver.leadMs; onTriggered: art.arrive() }
    Timer { id: grace; interval: saver.graceMs; onTriggered: art.late() }
    ParallelAnimation {
        id: crossfade
        NumberAnimation { target: art.idle; property: "opacity"; to: 1; duration: saver.crossfadeMs; easing.type: Easing.InOutQuad }
        NumberAnimation { target: art.shown; property: "opacity"; to: 0; duration: saver.crossfadeMs; easing.type: Easing.InOutQuad }
        onStopped: if (art.idle && art.idle.opacity >= 0.99) art.swapped()
    }
    // The shift: a step of a pixel every few seconds along a slow loop, so the picture is redrawn only when it moves.
    Timer {
        id: shift
        interval: Math.max(1, saver.shiftMs)
        running: saver.up && saver.style === "art" && !Lu.reduced
        repeat: true
        onTriggered: {
            var t = (Date.now() - art.began) / 1000
            art.dx = Math.round(30 * Math.sin(2 * Math.PI * t / 540))
            art.dy = Math.round(20 * Math.sin(2 * Math.PI * t / 380))
        }
    }

    Item {
        id: wall
        visible: saver.style === "art"
        anchors.fill: parent
        Rectangle {
            anchors.fill: parent
            gradient: Gradient {
                GradientStop { position: 0.0; color: "#100d12" }
                GradientStop { position: 0.7; color: "#060507" }
                GradientStop { position: 1.0; color: "#030203" }
            }
        }
        Item {
            x: art.dx; y: art.dy
            width: parent.width; height: parent.height
            ArtPiece { id: pieceA; opacity: 0 }
            ArtPiece { id: pieceB; opacity: 0 }
            // the pieces are drawn at the print's own brightness: 78 % of the real one, all of it together
            Rectangle { anchors.fill: parent; color: "#000000"; opacity: 1 - saver.artLevel * saver.level; z: 1 }
            LuText {
                z: 2
                visible: saver.caption && art.shown.meta !== null
                x: saver.edgeX; y: 1080 - saver.safeY - height
                role: "label"; font.pixelSize: 24; color: saver.ink2; opacity: 0.7
                textFormat: Text.StyledText
                text: art.shown.meta ? "<b><font color=\"" + saver.ink + "\">" + art.shown.meta.artist + "</font></b> \u00b7 " + art.shown.meta.title + ", " + art.shown.meta.year + " \u00b7 " + art.shown.meta.licence : ""
            }
            LuText {
                z: 2
                x: 1920 - saver.edgeX - width; y: 1080 - saver.safeY - height
                role: "label"; font.pixelSize: 32; font.weight: Font.Medium; color: saver.ink2; opacity: 0.7
                text: saver.clockText()
            }
        }
    }
}
