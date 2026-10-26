import QtQuick 2.4
import WebOSCompositorBase 1.0
import WebOSServices 1.0
import "../kit"
import TextFile 1.0
import "../ArtDeck.js" as Deck

// The screensaver and art mode (docs/mocks/21-screensaver-*). Drawn over everything when LG's idle timer ends: tvpower asks its screen
// saver list whether LG's saver may start and this surface, on that list while Home is in front, says no (`vote`, SaverVote.qml; the power
// state stays Active and LG's Flutter saver is never started, docs/research/saver-own-result.md). It is also drawn while the TV's power state
// is `Screen Saver` (`tvpower/power/getPowerState`, which the helper publishes as `screen.state`), which only happens when LG's saver was
// started anyway; under LG's compositor LG's window is above this one then, under LumaTV's it is below. The owner's options are
// `theme.screensaver` (Settings › Home & look › Screensaver): `style` clock, art or off; `own` (false: LG's saver is left alone, nothing
// is voted); `dim`; `glance`; `art.everyMin`, `art.order`, `art.caption`. (`afterMin` is not read here: the idle time is LG's timer, 180 s on
// this set; nothing here counts idle time.)
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
    readonly property bool keepOrder: artCfg.keepOrder !== false         // the shuffle bag is kept across restarts (ArtDeck.js); `art.keepOrder: false` turns it off
    readonly property bool caption: artCfg.caption !== false
    readonly property bool twelveHour: !(Lu.theme && Lu.theme.clock && Lu.theme.clock.twelveHour === false)
    // It sits on black, so it takes the dark material's inks whichever material Home is in.
    readonly property var darkRamp: (Lu.theme && Lu.theme.dark) || ({})
    readonly property color ink: Lu.asColor(darkRamp.ink || "white")
    readonly property color ink2: Lu.asColor(darkRamp.ink2 || "white")

    // The art on the TV's video decoder instead of the GPU (docs/research/videoplane-saver-result.md): the owner's switch, off until the TV steps
    // there have passed. The words are drawn at their one size (there is no text-size setting), so the clips' words and this surface's are the same.
    readonly property bool videoOn: cfg.video === true
    readonly property bool videoShell: cfg.videoOverlay === "shell"       // this surface draws the dim, the caption and the clock above the picture; else the player's window does
    readonly property bool videoUsable: videoOn && !forced && style === "art"
    // The art's shift, which the player makes too: whole pixels, a step every `shiftMs`.
    readonly property int driftAmpX: 30
    readonly property int driftAmpY: 20
    readonly property int driftPeriodX: 540
    readonly property int driftPeriodY: 380
    property int videoStartMs: 6000                                        // the player has this long to say it plays (a test shortens it, as it does the other timings)
    property int videoRenewMs: 10000
    property int videoStaleMs: 12000

    // ---- when it is up
    // Two ways in. LG's power state says `Screen Saver` (LG's saver was started: under LG's compositor its window is above this surface, so
    // this one is drawn but not seen; under LumaTV's compositor this one is above). Or `owned`: tvpower asked, while Home was in front and this
    // saver wanted, whether LG's saver may start, and `vote` (SaverVote.qml) said no. Then the power state stays `Active`, LG's Flutter saver
    // is never started, and this one draws over Home: it ends on any key (Home takes the key and nothing else happens), on the pointer, when
    // Home is no longer in front, when the TV is switched off, and when the owner turns the saver off.
    property bool forced: false                   // shown without the power state (a preview, a test)
    property bool hostActive: false               // Home is in front (Layers.qml)
    property bool owned: false                    // tvpower asked and was told no: this saver is the one the TV is showing
    readonly property bool powerState: !!(Helper.screen && Helper.screen.state === "Screen Saver")
    readonly property bool tvOff: !!(Helper.screen && Helper.screen.on === false)
    readonly property bool active: style !== "off" && (forced || powerState || owned)
    readonly property bool up: visible && active  // what every timer follows
    // The owner's switch for taking LG's saver over: on unless `theme.screensaver.own` is false (the way back to LG's saver, whose state and
    // window are then the TV's alone). It is the vote's only input besides Home being in front.
    readonly property bool takesOver: cfg.own !== false && style !== "off"
    onHostActiveChanged: if (!hostActive) owned = false
    onTvOffChanged: { if (tvOff) owned = false; else vote.renew() }      // (the TV came on: the subscription is made again, in case tvpower kept none through standby)
    onTakesOverChanged: if (!takesOver) owned = false

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
    // Qt's "h:mm" alone is 0-23 (the twelve hour face showed 23:39), so the twelve hour face folds the hour itself; the player's clock does the same.
    function clockText() {
        if (!twelveHour) return Qt.formatTime(now, "HH:mm")
        var h = now.getHours() % 12
        return (h === 0 ? 12 : h) + Qt.formatTime(now, ":mm")
    }

    // ---- keys: any key wakes the TV and nothing else happens. When LG's state is `Screen Saver` (or this is a preview) the TV is asked to turn
    // the screen on, as it always was; when this saver is the only one (`owned`, the state is `Active`) there is nothing to ask: it simply ends.
    signal woke()
    function wake() {
        var lg = powerState || forced
        owned = false
        if (lg) LS.adhoc.call("luna://com.webos.service.tvpower", "/power/turnOnScreen", "{}")
        woke()
    }
    function key(dir) { wake() }

    // ---- tvpower's question (SaverVote.qml): while Home is in front and the saver is wanted this surface is on tvpower's screen saver list and
    // says no to LG's saver; the question is the idle time's end, so the saver comes up in the Active state.
    SaverVote {
        id: vote
        want: saver.hostActive && saver.takesOver && !saver.forced
        onClaimed: saver.owned = true
    }

    property alias voter: vote             // (the tests answer through it)
    property alias videoPlayer: video      // (and give the player's mailbox a directory of their own)

    // A saver LG did not start has no power state to end it when the viewer moves the pointer or touches the screen, so it ends here. The first
    // position it hears is only where the pointer already is (a saver that comes up under a resting pointer gets one at once); a move of more
    // than a few pixels from there, a button or the wheel ends it. (LG's own state wakes on the pointer by itself, so this is only for `owned`.)
    property real pointerX: -1
    property real pointerY: -1
    function pointerAt(x, y) {
        if (pointerX < 0) { pointerX = x; pointerY = y; return }
        if (Math.abs(x - pointerX) + Math.abs(y - pointerY) > 12) wake()
    }
    MouseArea {
        id: pointer
        anchors.fill: parent
        enabled: saver.owned && saver.visible
        hoverEnabled: true
        acceptedButtons: Qt.AllButtons
        onEnabledChanged: { saver.pointerX = -1; saver.pointerY = -1 }
        onPositionChanged: saver.pointerAt(mouse.x, mouse.y)
        onPressed: saver.wake()
        onWheel: saver.wake()
    }
    // The panel is not left on a picture overnight.
    function standby() { LS.adhoc.call("luna://com.webos.service.tvpower", "/power/powerOff", JSON.stringify({ reason: "remoteKey" })) }
    Timer { id: standbyTimer; interval: Math.max(1, saver.standbyMs); running: saver.up && saver.standbyMs > 0; onTriggered: saver.standby() }

    onUpChanged: {
        if (up) {
            since = Date.now()
            refreshTime()
            minute.interval = msToNextMinute()
            clockBlock.pick()
            if (style === "art") startArt()
        } else {
            art.stop()
            video.end()
            clockBlock.fade = 1
        }
    }
    onStyleChanged: if (up) { if (style === "art") startArt(); else { art.stop(); video.end() } }
    onPeriodMsChanged: if (up && style === "art" && art.current >= 0 && !video.busy) lead.restart()

    // Whether the art goes to the player. The handlers below read it from what it is made of, not from `videoUsable`: a handler that runs because
    // `forced` changed (a preview) can run before the binding that follows `forced` has, and a preview must never ask the player.
    function videoAllowed() { return cfg.video === true && !forced && cfg.style === "art" }
    // The art is drawn by the GPU, as it always was, or by the decoder when the owner has asked for that and the player is there and plays:
    // black between the ask and the answer, the GPU's art if the player says no, does not answer or goes quiet. The order of the prints is
    // this surface's either way (its bag is kept across restarts), so the player is told the first print and each next one.
    function startArt() {
        if (!videoAllowed()) { art.start(); return }
        art.began = since
        art.loadIndex(function() {
            if (!(saver.up && saver.style === "art")) return
            if (!saver.videoAllowed()) { art.start(); return }      // turned off, or a preview, while the index loaded
            art.videoFirst(); saver.askVideo()
        })
    }
    function askVideo() {
        if (!up || !videoAllowed()) return
        video.spec = videoSpec()
        video.begin()
        if (!video.busy) { art.leaveVideo(); art.start() }      // no player on this TV
    }
    function hexOf(c) {
        function two(v) { var s = Math.round(v * 255).toString(16); return s.length < 2 ? "0" + s : s }
        return "#" + two(c.r) + two(c.g) + two(c.b)
    }
    // What the player is asked for: this surface's own tokens, so the two savers move alike.
    function videoSpec() {
        var s = { style: "art", everyMin: everyMin, shuffle: shuffle, dim: dim, dimTo: cfg.dimTo, dimOverMin: cfg.dimOverMin, moveMs: Math.max(20, shiftMs),
                  fadeMs: Math.max(20, Math.round(crossfadeMs / 2)), ampX: driftAmpX, ampY: driftAmpY, periodX: driftPeriodX, periodY: driftPeriodY,
                  drift: !Lu.reduced, clock: true, twelveHour: twelveHour, caption: caption, overlay: !videoShell, auto: art.pictures.length === 0,
                  ink: hexOf(ink), inkAlpha: Math.round(255 * 0.7 * ink.a), ink2: hexOf(ink2), ink2Alpha: Math.round(255 * 0.7 * ink2.a), since: since }
        if (art.videoMeta) s.clip = art.videoMeta.id
        return s
    }
    function videoChanged() {
        if (!up || style !== "art") return
        if (video.state === "failed") { art.leaveVideo(); art.start() }
        else if (video.state === "playing") vchange.restart()
    }
    // The owner turns the switch off while the picture is up: the player is let go (SaverVideo does it) and the GPU's art takes over, from black.
    onVideoUsableChanged: if (!videoUsable && up && style === "art" && !art.shown.meta && !art.idle.meta) { art.leaveVideo(); art.start() }
    SaverVideo {
        id: video
        want: saver.up && saver.videoUsable
        startMs: saver.videoStartMs; renewMs: saver.videoRenewMs; staleMs: saver.videoStaleMs
        onStateChanged: saver.videoChanged()
        // The player took a key or the pointer in its own window and asked tvpower for the screen: when LG's state is the Active one that ends the
        // saver (in LG's `Screen Saver` state the change of state does).
        onWoken: { saver.owned = false; saver.woke() }
    }
    readonly property bool holeOpen: video.playing && videoShell      // the picture is on the video plane and this surface is above it: nothing of its own under the dim
    // The shuffle bag is kept across restarts (ArtDeck.js): a small file in the data directory, and a second handle to ask whether a print's file is there.
    TextFile { id: deckFile; name: "file://{{dataDir}}/theme/saver-deck.json" }
    TextFile { id: deckProbe }

    // A preview (Settings' button, the scenes): shows it without the power state, optionally at a place and with a picture.
    function preview(x, y, pictureId) {
        forced = true
        if (x !== undefined) clockBlock.placeAt(x, y)
        if (pictureId) art.show(pictureId)
    }

    // Whether anything is running: nothing may be while it is hidden.
    readonly property bool timersRunning: standbyTimer.running || minute.running || move.running || lead.running || turn.running || grace.running || crossfade.running || shift.running || moveClock.running || video.timersRunning || vchange.running || vturn.running
    function snapshot() {
        return JSON.stringify({ up: up, active: active, owned: owned, style: style, level: level, timers: timersRunning,
                                vote: { want: vote.want, registered: vote.registered, asked: vote.asked, refused: vote.refused, timers: vote.timersRunning }, clock: { fx: clockBlock.fx, fy: clockBlock.fy, fade: clockBlock.fade, opacity: clockBlock.opacity },
                                art: { loaded: art.pictures.length, current: art.current, pending: art.pending, front: art.aFront, skipped: art.skipped, dx: art.dx, dy: art.dy,
                                       bag: art.bag.map(function(i) { return art.pictures[i].id }), last: art.lastId, key: art.deckKey,
                                       shown: art.shown.meta ? art.shown.meta.id : "", idle: art.idle.meta ? art.idle.meta.id : "", shownOpacity: art.shown.opacity, idleOpacity: art.idle.opacity },
                                video: { usable: videoUsable, state: video.state, reason: video.reason, hole: holeOpen, vfade: art.vfade, caption: art.captionMeta ? art.captionMeta.id : "", spec: video.spec } })
    }

    Timer {
        id: minute
        running: saver.up
        repeat: true
        onTriggered: { saver.refreshTime(); interval = saver.msToNextMinute() }
    }

    Rectangle { id: backdrop; anchors.fill: parent; color: "#000000"; visible: !saver.holeOpen }

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
                font.pixelSize: 152; font.weight: Font.Medium; font.letterSpacing: -152 * 0.03; line: 0.95
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
        property var bag: []                       // the prints still to deal in this round (indices of `pictures`)
        property string lastId: ""                 // the print dealt last: a new round does not start with it
        property string deckKey: ""                // the collection the bag belongs to (its folder)
        property int current: -1                   // the picture shown (an index of `pictures`)
        property int pending: -1                   // the one loading into the idle piece
        property bool aFront: true
        property int skipped: 0
        property int dx: 0                         // the shift, whole pixels
        property int dy: 0
        property double began: 0
        property var videoMeta: null               // video: the print the player shows (this surface's caption is made from it in shell mode)
        property real vfade: 0                     // video, shell mode: the page being turned, 0 clear to 1 black
        readonly property var captionMeta: shown.meta ? shown.meta : (saver.holeOpen ? videoMeta : null)
        readonly property Item shown: aFront ? pieceA : pieceB
        readonly property Item idle: aFront ? pieceB : pieceA

        function loadIndex(done) {
            if (loaded) { done(); return }
            var x = new XMLHttpRequest()
            x.onreadystatechange = function() {
                if (x.readyState !== 4) return
                try { art.pictures = (JSON.parse(x.responseText).pictures || []).filter(function(p) { return p.licence === "Public domain" }) } catch (e) { art.pictures = [] }
                art.loaded = true
                art.restoreDeck()
                done()
            }
            try { x.open("GET", saver.indexUrl); x.send() } catch (e) { loaded = true; pictures = []; done() }
        }
        function start() {
            began = Date.now()
            loadIndex(function() { if (saver.up && saver.style === "art" && art.current < 0) art.first() })
        }
        function stop() {
            lead.stop(); turn.stop(); grace.stop(); crossfade.stop(); shift.stop(); leaveVideo()
            pieceA.clear(); pieceB.clear()
            pieceA.opacity = 0; pieceB.opacity = 0
            current = -1; pending = -1; aFront = true; skipped = 0; dx = 0; dy = 0
        }
        // Video: the first print and the next in this surface's own order; the player is told which and shows it.
        function videoFirst() {
            var i = nextIndex()
            if (i < 0) return
            current = i
            videoMeta = pictures[i]
        }
        function videoSwitch() {
            var i = nextIndex()
            if (i < 0) return
            current = i
            videoMeta = pictures[i]
            video.next(pictures[i].id)
        }
        function leaveVideo() { vchange.stop(); vturn.stop(); vfade = 0; videoMeta = null; current = -1 }
        // The next print in the order the owner chose; the shuffle draws from a bag, so none repeats before all have been shown.
        function nextIndex() {
            var n = pictures.length
            if (n === 0) return -1
            // in order: after the print last tried, which is not the one shown while a broken print is being passed over
            if (!saver.shuffle) return ((pending >= 0 ? pending : current) + 1) % n
            if (bag.length === 0) bag = Deck.draw(n, [current, indexOfId(lastId)], Math.random)
            var out = bag[0]
            bag = bag.slice(1)
            lastId = pictures[out].id
            saveDeck()
            return out
        }
        // The bag across restarts: what was left of the round, less the prints whose file is gone, with the prints that have come since dealt in.
        function fileThere(p) { deckProbe.name = Lu.baseUrl + "art/" + p.file; return deckProbe.exists }       // where ArtPiece looks for it
        function readDeck() { return Deck.parse(deckFile.exists ? deckFile.read() : "") }
        function restoreDeck() {
            if (!saver.keepOrder) return
            deckKey = Deck.collectionKey(saver.indexUrl, Lu.baseUrl)
            try {
                var r = Deck.restore(readDeck(), deckKey, pictures, fileThere, Math.random)
                bag = r.bag
                lastId = r.last >= 0 ? pictures[r.last].id : ""
            } catch (e) { bag = []; lastId = ""; console.warn("[Screensaver] the saved order could not be read: " + e) }
        }
        function saveDeck() {
            if (!saver.keepOrder) return
            try { deckFile.write(Deck.stringify(Deck.put(readDeck(), deckKey, Deck.deckOf(pictures, bag, indexOfId(lastId), Date.now())))) }
            catch (e) { console.warn("[Screensaver] the order could not be saved: " + e) }
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
    // Video: the saver names each next print. In shell mode it also turns the page, as the dim is its own: out through black, the player told at
    // the dark, back in; in the player's own mode the player turns it through the dim it draws.
    Timer {
        id: vchange
        interval: Math.max(200, saver.periodMs)
        onTriggered: {
            if (art.pictures.length < 2) vchange.restart()
            else if (saver.videoShell) vturn.restart()
            else { art.videoSwitch(); vchange.restart() }
        }
    }
    SequentialAnimation {
        id: vturn
        NumberAnimation { target: art; property: "vfade"; to: 1; duration: Math.max(20, saver.crossfadeMs / 2); easing.type: Easing.InOutQuad }
        ScriptAction { script: art.videoSwitch() }
        PauseAnimation { duration: 400 }
        NumberAnimation { target: art; property: "vfade"; to: 0; duration: Math.max(20, saver.crossfadeMs / 2); easing.type: Easing.InOutQuad }
        ScriptAction { script: vchange.restart() }
    }
    // The shift: a step of a pixel every few seconds along a slow loop, so the picture is redrawn only when it moves.
    Timer {
        id: shift
        interval: Math.max(1, saver.shiftMs)
        running: saver.up && saver.style === "art" && !Lu.reduced && !(video.busy && !saver.videoShell)
        repeat: true
        onTriggered: {
            var t = (Date.now() - art.began) / 1000
            art.dx = Math.round(saver.driftAmpX * Math.sin(2 * Math.PI * t / saver.driftPeriodX))
            art.dy = Math.round(saver.driftAmpY * Math.sin(2 * Math.PI * t / saver.driftPeriodY))
        }
    }

    Item {
        id: wall
        visible: saver.style === "art"
        anchors.fill: parent
        Rectangle {
            anchors.fill: parent
            visible: !saver.holeOpen
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
            // the pieces are drawn at the print's own brightness: 78 % of the real one, all of it together; over the video (the clip has the 78 % in it) only the dim
            Rectangle { anchors.fill: parent; color: "#000000"; opacity: saver.holeOpen ? (1 - saver.level) + saver.level * art.vfade : 1 - saver.artLevel * saver.level; z: 1 }
            LuText {
                z: 2
                visible: saver.caption && art.captionMeta !== null
                x: saver.edgeX; y: 1080 - saver.safeY - height
                role: "label"; font.pixelSize: 24; color: saver.ink2; opacity: saver.holeOpen ? 0.7 * (1 - art.vfade) : 0.7
                textFormat: Text.StyledText
                text: art.captionMeta ? "<b><font color=\"" + saver.ink + "\">" + art.captionMeta.artist + "</font></b> \u00b7 " + art.captionMeta.title + ", " + art.captionMeta.year + " \u00b7 " + art.captionMeta.licence : ""
            }
            LuText {
                z: 2
                visible: !(video.busy && !saver.videoShell)                 // in the player's own mode its window draws the time
                x: 1920 - saver.edgeX - width; y: 1080 - saver.safeY - height
                role: "label"; font.pixelSize: 32; font.weight: Font.Medium; color: saver.ink2; opacity: 0.7
                text: saver.clockText()
            }
        }
    }
}
