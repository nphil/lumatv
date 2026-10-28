import QtQuick 2.4
import QtGraphicalEffects 1.0
import QtQuick.Window 2.2
import WebOSCompositorBase 1.0
import WebOSServices 1.0
import WebOS.Global 1.0
import TextFile 1.0
import "LumaBus.js" as Bus
import "Tint.js" as Tint
import "LayoutGrid.js" as Grid
import "WallVideo.js" as WallPick
import "kit"
import "kit/Match.js" as Match
import "settings/AppKeys.js" as AppKeys
import "settings/QuickAccess.js" as QA

// The compositor-drawn Home: wallpaper, glass material, status bar, widget grid and dock,
// driven by {{dataDir}}/theme/{theme.json,layout.json}. Keys arrive through
// LumaBus.js from the configd-registered key filter (payload/keyfilter/lumatv.js);
// pointer input arrives as ordinary QtQuick mouse events because this item sits above the
// Home surface.
Item {
    id: root
    anchors.fill: parent

    // set by StarfishFullscreenContainer: Home is this container's app and visible
    property bool hostActive: false
    // The Hz the compositor's output runs at: set by the shell client under LumaTV's compositor (compositor/shell/Shell.qml, the
    // `refresh` topic); 60 under LG's. Read through Lu.fast (kit/Lu.qml): the 120 Hz test mode.
    property int refreshHz: 60

    readonly property string themeDirUrl: "file://{{dataDir}}/theme/"
    readonly property string fontsDirUrl: "file://{{dataDir}}/fonts/"
    readonly property string moduleDirUrl: "file://{{dataDir}}/qml/WebOSCompositor/{{qmlModule}}/"

    // The shipped theme.json and layout.json are staged beside this file as the defaults (tools/
    // lumatv stage_luma) and read at once, so every binding below has a theme to read. The copies
    // in the data dir are merged over them when they load (applyTheme, applyLayout). The file
    // names are plain values, not bindings, so they are set before any binding runs.
    TextFile { id: themeDefaultsFile; name: "file://{{dataDir}}/qml/WebOSCompositor/{{qmlModule}}/theme.default.json" }
    TextFile { id: layoutDefaultsFile; name: "file://{{dataDir}}/qml/WebOSCompositor/{{qmlModule}}/layout.default.json" }
    function readDefaults(file) {
        try { return JSON.parse(file.read()) }
        catch (e) { console.warn("[LumaHome] " + file.name + " unreadable: " + e); return {} }
    }
    readonly property var themeDefaults: readDefaults(themeDefaultsFile)
    readonly property var layoutDefaults: readDefaults(layoutDefaultsFile)

    property var theme: themeDefaults
    property var layout: layoutDefaults
    property bool isLight: false
    property var mat: theme.dark
    property bool fpsEnabled: false
    property string lastThemeText: ""
    // theme.json is written by a PUT that is not atomic: a read that is out while one is out (or one that began before it ended) can see the file empty or cut
    // short, or as it was. `themeWritesOut` counts the PUTs not yet answered, `themeWriteSeq` the writes begun and ended.
    property int themeWritesOut: 0
    property int themeWriteSeq: 0
    property string lastLayoutText: ""

    // ---- motion tokens ----
    property var motion: theme.motion
    property bool motionReduced: !!motion.reduced
    property int focusMs: motion.focusMs
    property int focusEasing: Lu.easing(motion.focusEasing)
    property int cardMs: motion.cardMs
    property int cardEasing: Lu.easing(motion.cardEasing)
    property int scrollMs: motion.scrollMs
    property int scrollEasing: Lu.easing(motion.scrollEasing)
    property int labelMs: motion.labelMs
    property int layerMs: motion.layerMs

    // The kit's token layer takes what it draws with from here (kit/Lu.qml).
    Binding { target: Lu; property: "theme"; value: root.theme }
    Binding { target: Lu; property: "isLight"; value: root.isLight }
    Binding { target: Lu; property: "wallAccent"; value: root.wallTint ? root.wallAccent : "transparent" }
    Binding { target: Lu; property: "refreshHz"; value: root.refreshHz }
    // The glass samples the wallpaper's blur while Home is on screen: Home takes the backdrop at load and whenever it is shown again, since
    // over an app the overlay (LumaOverlay.qml) points it at a flat colour. An assignment and not a Binding: a Binding that switches off is
    // the one construct whose behaviour differs between Qt 5.12 (the set) and 5.15 (the offline host).
    Binding { target: Lu; property: "baseUrl"; value: root.moduleDirUrl }
    Binding { target: Lu; property: "homeWindow"; value: root.Window.window }
    // What the all-apps library lists and in which order: the owner's choices (theme.apps) and the dock's tiles (kit/Apps.qml).
    Binding { target: Apps; property: "prefs"; value: root.theme.apps || ({}) }
    Binding { target: Apps; property: "dockKey"; value: root.dockKey }
    readonly property string dockKey: dockLoader.item ? dockLoader.item.points.map(function(p) { return p.id }).join("|") : ""

    function resolvePath(p) {
        if (!p) return ""
        if (/^[a-zA-Z][a-zA-Z0-9+.-]*:/.test(p)) return p
        if (p.charAt(0) === "/") return "file://" + p
        return themeDirUrl + p
    }

    // ============================================================= grid geometry
    // grid.rows is either a count (uniform rows filling the area) or an array of pixel
    // heights (the reference layout pins the mock's 222/238/392). The model is LayoutGrid.js, the one
    // the Home widgets page and the layout editor change layouts with, so a card is drawn where they put it.
    function areaRect() { var a = Grid.area(layout); return Qt.rect(a.x, a.y, a.w, a.h) }
    function rowHeights() { return Grid.rowHeights(layout) }
    function rowCount() { return Grid.rowCount(layout) }
    function cellRect(entry) { var r = Grid.rectOf(layout, entry); return Qt.rect(r.x, r.y, r.w, r.h) }
    // A card the owner hid (Settings, Home widgets) is not drawn and takes no focus; the dock cannot be hidden.
    property var nonDockWidgets: (layout.widgets || []).filter(function(e) { return e.type !== "dock" && !e.hidden })
    // The ids of those cards, in order, behind a string: the list the widgets are built from is replaced only when a card comes or goes
    // or changes place in the order, not at every move and every size of the layout editor.
    readonly property string widgetKey: nonDockWidgets.map(function(e) { return e.id }).join("|")
    property var widgetIds: widgetKey === "" ? [] : widgetKey.split("|")
    property var dockEntry: { var l = (layout.widgets || []).filter(function(e) { return e.type === "dock" }); return l.length ? l[0] : null }
    function widgetIndexById(id) {
        for (var i = 0; i < nonDockWidgets.length; i++) if (nonDockWidgets[i].id === id) return i
        return -1
    }
    // Whether no card is above (below) another whose columns meet its own: Up from such a card reaches the status bar and Down the
    // dock, whatever the owner's layout leaves above or under it (a card off the first row, a gap above the dock).
    function cardBeside(e, dy) {
        for (var i = 0; i < nonDockWidgets.length; i++) {
            var o = nonDockWidgets[i]
            if (o.id === e.id || o.col >= e.col + e.span || e.col >= o.col + o.span) continue
            if (dy < 0 ? o.row + o.rows <= e.row : o.row >= e.row + e.rows) return true
        }
        return false
    }
    function isTopRow(e) { return !cardBeside(e, -1) }
    function isBottomRow(e) { return !cardBeside(e, 1) }

    // ============================================================= JSON load / reload
    function loadJson(url, cb) {
        var x = new XMLHttpRequest()
        x.onreadystatechange = function() {
            if (x.readyState !== 4) return
            if (x.responseText && (x.status === 200 || x.status === 0)) {
                try { cb(JSON.parse(x.responseText), x.responseText); return }
                catch (e) { console.warn("[LumaHome] invalid JSON at " + url + ": " + e) }
            }
            cb(null, null)
        }
        try { x.open("GET", url); x.send() } catch (e) { cb(null, null) }
    }
    // Objects merge key by key, everything else (numbers, strings, arrays) is replaced by the
    // override: a user's theme.json keeps only what it says, and a key it never set follows the
    // shipped file.
    function mergeDeep(base, over) {
        if (over === undefined) return base
        if (over === null || typeof over !== "object" || Array.isArray(over)) return over
        if (base === null || typeof base !== "object" || Array.isArray(base)) return over
        var out = {}
        for (var k in base) out[k] = base[k]
        for (var k2 in over) out[k2] = mergeDeep(base[k2], over[k2])
        return out
    }
    function applyTheme(json, text) {
        root.theme = json ? mergeDeep(themeDefaults, json) : themeDefaults
        if (!json) console.warn("[LumaHome] theme.json missing/invalid, using the shipped one")
        root.lastThemeText = text || ""
        restoreTint()
        computeMaterial()
    }
    function applyLayout(json, text) {
        root.layout = json ? mergeDeep(layoutDefaults, json) : layoutDefaults
        if (!json) console.warn("[LumaHome] layout.json missing/invalid, using the shipped one")
        root.lastLayoutText = text || ""
        if (widgetIndexById(widgetFocusId) < 0) widgetFocusId = nonDockWidgets.length ? nonDockWidgets[0].id : ""
    }
    function loadFpsSetting() {
        loadJson(moduleDirUrl + "settings.json", function(json) { root.fpsEnabled = !!(json && json.fps) })
    }
    function loadAll() {
        loadJson(themeDirUrl + "theme.json", applyTheme)
        loadJson(themeDirUrl + "layout.json", applyLayout)
        loadFpsSetting()
    }
    function reload() { loadAll() }
    // The poll's read of theme.json. A read that overlapped one of this layer's own writes is dropped, not applied: applied as an empty file it would put
    // the shipped theme on screen and clear `lastThemeText`, the text the next write starts from, so that write would keep none of the earlier choices. The
    // next poll finds the whole file.
    function pollTheme() {
        var seq = root.themeWriteSeq
        loadJson(root.themeDirUrl + "theme.json", function(json, text) {
            if (seq !== root.themeWriteSeq || root.themeWritesOut > 0) return
            if ((text || "") === root.lastThemeText) return
            root.applyTheme(json, text)
        })
    }
    // Popover settings: apply a patch to the live theme immediately and persist it through
    // the tool (theme set), which the 2 s poll then reads back as the same content.
    function patchTheme(patch) {
        var t = JSON.parse(JSON.stringify(root.theme))
        for (var k in patch) {
            if (typeof patch[k] === "object" && patch[k] !== null && typeof t[k] === "object" && t[k] !== null) { for (var k2 in patch[k]) t[k][k2] = patch[k][k2] }
            else t[k] = patch[k]
        }
        root.theme = t
        computeMaterial()
    }
    function persistTheme(patch) {
        // Qt 5.12's QML XMLHttpRequest may write local files, so the layer saves theme.json
        // itself: the on-disk object (not the merged defaults) plus the patch, one level deep.
        var file = {}
        try { file = JSON.parse(root.lastThemeText || "{}") } catch (e) { file = {} }
        for (var k in patch) {
            if (typeof patch[k] === "object" && patch[k] !== null) {
                if (typeof file[k] !== "object" || file[k] === null) file[k] = {}
                for (var k2 in patch[k]) file[k][k2] = patch[k][k2]
            } else file[k] = patch[k]
        }
        var text = JSON.stringify(file, null, 2) + "\n"
        console.info("[LumaHome] persist theme " + JSON.stringify(patch))
        var x = new XMLHttpRequest()
        x.onreadystatechange = function() {
            if (x.readyState !== 4) return
            root.themeWritesOut--
            root.themeWriteSeq++
            console.info("[LumaHome] theme.json write status " + x.status)
            if (x.status === 200 || x.status === 0) root.lastThemeText = text   // the poll sees its own write
            else console.warn("[LumaHome] theme.json write failed: " + x.status)
        }
        root.themeWritesOut++
        root.themeWriteSeq++
        try { x.open("PUT", root.themeDirUrl + "theme.json"); x.send(text) }
        catch (e) { root.themeWritesOut--; root.themeWriteSeq++; console.warn("[LumaHome] theme.json write failed: " + e) }
    }

    // ---- the owner's layout.json: what Settings' Home widgets page and the layout editor ask for through HomeLayout. `next` is the whole
    // layout wanted; the file keeps only what differs from the shipped one (Grid.forFile: the cut `theme.py minimize` makes at every
    // apply). No copy of the file it replaces is kept: the way back is "Reset to default" (the file is `{}` again), and the editor's own
    // Undo and Cancel before a save. TextFile writes at once, like the beat, and the layout is applied here before the poll could find
    // the file changed, so the poll reads back what was written and does nothing.
    TextFile { id: layoutFile; name: root.themeDirUrl + "layout.json" }
    readonly property bool layoutCustom: { var t = lastLayoutText.replace(/\s+/g, ""); return t !== "" && t !== "{}" }
    function persistLayout(next) { writeLayoutFile(Grid.forFile(next, layoutDefaults)) }
    function resetLayout() { writeLayoutFile({}) }
    function writeLayoutFile(file) {
        var text = JSON.stringify(file, null, 2) + "\n"
        if (text !== (layoutFile.exists ? layoutFile.read() : "")) {
            layoutFile.write(text)
            console.info("[LumaHome] layout.json written, " + (file.widgets ? file.widgets.length + " cards" : "the shipped layout"))
        }
        applyLayout(file, text)
    }
    Binding { target: HomeLayout; property: "layout"; value: root.layout }
    Binding { target: HomeLayout; property: "shipped"; value: root.layoutDefaults }
    Binding { target: HomeLayout; property: "customised"; value: root.layoutCustom }
    Connections {
        target: HomeLayout
        onChangeRequested: root.persistLayout(layout)
        onResetRequested: root.resetLayout()
    }
    Timer {
        // live theme preview for the companion app: content compare every 2 s, no restart.
        // Only while Home is on screen; showing Home again catches up at once.
        interval: 2000; running: root.hostActive; repeat: true; triggeredOnStart: true
        onTriggered: {
            root.pollTheme()
            loadJson(root.themeDirUrl + "layout.json", function(json, text) { if (!root.editing && (text || "") !== root.lastLayoutText) root.applyLayout(json, text) })
            // The development door: a layer named in this file is shown (the gate and the host use it for surfaces that no
            // Home key opens yet). The file is in /tmp, so a reboot clears it. A name that is no surface is ignored: it would
            // own every key and show nothing.
            var door = (layerFile.exists ? layerFile.read() : "").replace(/\s+/g, "")
            if (door !== "" && !layers.find(door)) { if (door !== root.badDoor) console.warn("[LumaHome] the layer door names no surface: " + door); root.badDoor = door; door = "" }
            if (door !== root.doorLayer) root.doorLayer = door
        }
    }
    TextFile { id: layerFile; name: "file:///tmp/{{shortId}}-layer" }
    property string doorLayer: ""             // a layer the development door asked for
    property string badDoor: ""               // the last name the door gave that is no surface, said once
    property string layerOpen: ""             // the layer that has the keys, one Home opened itself (the quick panel, the library, a menu)
    readonly property bool saverUp: layers.saverUp   // the screensaver is over Home (the shell reports it under LumaTV's compositor)

    // ============================================================= the moving wallpaper on the video plane
    // Home's wallpaper as a clip the TV's video decoder plays (vplane/; WallVideo.qml asks the player, WallVideo.js picks the clip; docs/research/
    // videoplane-saver-result.md, "The moving wallpaper"), so the idle Home's frames are not the GPU's. `theme.wallpaperMotion.source` is "gpu"
    // (the default: nothing here runs) or "video".
    //
    // It works under LumaTV's compositor only, whose wallpaper window (the player's) is under Home's: the shell sets `videoWallBackend` to "own"
    // and `videoWallHole` from the compositor's `wall` topic (compositor/shell/Shell.qml). Under LG's compositor neither is set and the GPU
    // wallpaper is drawn, as it always was.
    //
    // While the clip plays the GPU wallpaper is not drawn at all, so Home's window is transparent where it was: the plane shows through and
    // what Home draws above it (the glass, the widgets, the dock) is blended over the video by the display engine. Glass cannot blur a plane the
    // GPU cannot sample, so it keeps the blur of the picture the clip is made from (the clip is that picture in slow motion), or with
    // `theme.wallpaperMotion.glass: "tint"` it is a flat tint over the video, opaque enough for its ink to read over the brightest the clip gets
    // (the clip's `glass.alpha`, measured when it was made).
    // LumaTV's compositor grows an app's card out of its tile and takes it back (compositor/shell/Shell.qml sets this from the compositor's
    // `transitions` topic; under LG's compositor it stays false and a launch is the cut it was). Home tells it where the tile is, and the launch
    // screen (the splash) waits `splashWaitMs` so that the tile is what the card grows from; an app that has a window by then never shows it.
    property bool smoothLaunch: false
    readonly property int splashWaitMs: 400
    property string launchingId: ""
    function noteLaunchOrigin(appId, tile) {
        launchingId = appId
        if (!smoothLaunch || !tile) return
        var r = tile.rect, inset = tile.floating ? ((theme.tiles && theme.tiles.inset) || 19) : 0
        LS.adhoc.call("luna://com.webos.surfacemanager", "/setLaunchOrigin", JSON.stringify({ appId: appId, from: null,
            tile: { x: r.x + inset, y: r.y + inset, width: r.width - 2 * inset, height: r.height - 2 * inset, radius: Lu.radius("tile") } }))
    }
    function noteSplashOrigin(rect, radius) {
        if (!smoothLaunch || launchingId === "") return
        LS.adhoc.call("luna://com.webos.surfacemanager", "/setLaunchOrigin", JSON.stringify({ appId: launchingId, from: { x: rect.x, y: rect.y, width: rect.width, height: rect.height, radius: radius } }))
    }
    // An app has taken the front (the shell's `foreground` topic): a launch screen that was still waiting for its turn is not wanted.
    function appFronted(appId) { layers.cancelDelayedSplash() }
    property string videoWallBackend: ""
    property bool videoWallHole: false
    readonly property var wallPick: wallVideo.pick(theme.wallpaper, wallMotion, (theme.dark && theme.dark.wallScrim) || [])
    // The clip is the dark material's picture: the light material, another wallpaper or a look other than the clip's draws the GPU wallpaper.
    readonly property bool videoWallWanted: videoWallBackend === "own" && wallMotion.source === "video" && wallMotionOn && !isLight
                                            && hostActive && !idleHold && !saverUp && !!wallPick.entry
    readonly property bool videoWallUp: wallVideo.up
    readonly property alias videoWall: wallVideo       // the controller (state, reason, status): the host's tests and the TV steps read it
    // The switch is on and Home is shown: the clips' manifest is read again every few seconds (a player or clips installed after the shell started).
    readonly property bool videoWallWatch: videoWallBackend === "own" && wallMotion.source === "video" && wallMotionOn && !isLight && hostActive && !idleHold && !saverUp
    WallVideo { id: wallVideo; want: root.videoWallWanted; watch: root.videoWallWatch; clip: WallPick.clipName(root.wallPick.entry); hole: root.videoWallHole; Component.onCompleted: loadManifest() }
    Connections { target: root; onHostActiveChanged: if (root.hostActive) wallVideo.loadManifest() }
    // The GPU wallpaper fades out over the plane's first pictures (the plane shows the same one) and is back the moment the plane goes.
    property real gpuWall: 1
    onVideoWallUpChanged: { gpuFade.stop(); if (videoWallUp) { gpuFade.from = gpuWall; gpuFade.to = 0; gpuFade.start() } else gpuWall = 1 }
    NumberAnimation { id: gpuFade; target: root; property: "gpuWall"; duration: root.layerMs; easing.type: Easing.InOutQuad }
    readonly property bool wallGlassTint: videoWallUp && gpuWall < 0.001 && wallMotion.glass === "tint" && !!wallPick.entry
    Binding { target: Lu; property: "homeFill"; value: root.wallGlassTint ? root.wallPick.entry.glass.alpha : 1 }
    property bool liveWallOn: wallMotionOn && wallpaperImg.status === Image.Ready      // the GPU wallpaper is the warped one (else the graded still)

    // ============================================================= Home stub contract
    // With the Home stub (tools/lumatv homestub) this layer is the only Home, so
    // something outside the compositor has to notice when it stops drawing: a file
    // rewritten once a second for as long as this item lives, read by the tool's
    // guard. That one write is all that runs per tick, and it runs while Home is
    // hidden too, since a layer that has died is found out before the next Home key.
    // The files are written and read through LG's TextFile type, synchronously: the
    // same writes as XMLHttpRequest PUTs went through the network access manager's
    // queue and were twice seen to land 5 s late with the timer ticking on time.
    // The mode comes back the other way: "lg" means LG's own Home is meant to be on
    // screen (Home held, or the stub gave way) and the layer steps aside; "stub" and
    // "legacy" (no file) leave it covering Home. The second field of the file changes
    // with every decision, which ends an escape taken without the tool (legacy mode:
    // LG's Home is already underneath, so the layer just hides until the next apply).
    // A third field, trial=<seconds>, says the stub is on a trial that ends on its
    // own: the toast below asks for OK, and OK is the confirmation (a file the guard
    // reads), because a stub nobody can confirm from the sofa would never be kept.
    readonly property string beatUrl: "file:///tmp/{{shortId}}-beat"
    readonly property string modeUrl: "file:///tmp/{{shortId}}-homemode"
    readonly property string confirmUrl: "file:///tmp/{{shortId}}-confirm"
    property int beatSeq: 0
    property bool homeCurrent: false        // set by the container: Home is its surface
    property string homeMode: "legacy"
    property string modeToken: ""
    property bool escapedLocal: false
    readonly property bool lgHome: homeMode === "lg" || escapedLocal
    property double trialEndMs: 0
    property int trialLeft: 0
    property bool trialConfirming: false
    readonly property bool trialOn: homeMode === "stub" && trialLeft > 0
    TextFile { id: beatFile; name: root.beatUrl }
    TextFile { id: modeFile; name: root.modeUrl }
    TextFile { id: confirmFile; name: root.confirmUrl }
    // One beat: the in-engine one the key filter reads, then the file the guard reads. It is also written once at load, before any
    // timer has ticked, so a layer that is built while the panel is off leaves a beat and a log line behind even if the engine
    // does not run timers then, and a timer that went quiet shows as a gap in the log when it starts again.
    property double lastBeatMs: 0
    function beatOnce() {
        var now = Date.now()
        if (lastBeatMs > 0 && now - lastBeatMs > 3000) console.info("[LumaHome] the beat was silent for " + Math.round((now - lastBeatMs) / 1000) + " s")
        lastBeatMs = now
        Bus.beat()
        beatFile.write(String(++beatSeq))
    }
    Timer {
        interval: 1000; running: true; repeat: true
        onTriggered: {
            root.beatOnce()
            root.trialLeft = root.trialEndMs > 0 ? Math.max(0, Math.ceil((root.trialEndMs - Date.now()) / 1000)) : 0
        }
    }
    function readMode() {
        var t = ((modeFile.exists ? modeFile.read() : "") || "").replace(/\s+$/, "").split(" ")
        var token = t[1] || ""
        if (token !== root.modeToken) {
            root.modeToken = token
            root.escapedLocal = false
            root.trialConfirming = false
            var m = /^trial=([0-9]+)$/.exec(t[2] || "")
            root.trialEndMs = m ? Date.now() + parseInt(m[1], 10) * 1000 : 0
            root.trialLeft = m ? parseInt(m[1], 10) : 0
        }
        root.homeMode = (t[0] === "stub" || t[0] === "lg") ? t[0] : "legacy"
    }
    // Home becoming the foreground surface: the mode is read before the first frame.
    onHomeCurrentChanged: if (homeCurrent) readMode()
    function confirmTrial() {
        trialConfirming = true
        confirmFile.write(String(Date.now()))
        modeFast.ticks = 0
        modeFast.restart()
    }
    Timer {
        // the mode is only looked at while Home is the foreground surface
        interval: 2000; running: root.homeCurrent; repeat: true; triggeredOnStart: true
        onTriggered: root.readMode()
    }
    Timer {
        // after the Home key has been held, or OK confirmed a trial: follow the tool's
        // answer closely for 10 s
        id: modeFast
        property int ticks: 0
        interval: 250; repeat: true; running: false
        onTriggered: {
            root.readMode()
            if (++ticks >= 40 || root.homeMode === "lg") stop()
        }
    }
    function escapeRequested() {
        if (homeMode === "stub") { modeFast.ticks = 0; modeFast.restart() }   // the tool is on it
        else escapedLocal = true                                               // LG's Home is already underneath
    }

    // ============================================================= fonts
    // Faces named by theme.font.files (the family every kit component and every Text here draws
    // in), then any a theme brings of its own (theme.fonts). The shipped files are in
    // payload/fonts, copied to the data dir by stage_theme.
    Repeater { model: (theme.font && theme.font.files) || []; delegate: Item { FontLoader { source: root.fontsDirUrl + modelData } } }
    Repeater { model: theme.fonts || []; delegate: Item { FontLoader { source: root.resolvePath(modelData) } } }

    // ============================================================= material (light/dark) and tint
    // One 32x18 sample of the wallpaper gives both the mean luma, which decides light or dark
    // glass for an "auto" material, and the dominant colour that tints the accent and, when the
    // theme asks (tint.mix), the focused glass (Tint.js). A newer wallpaper cancels a sample that
    // has not painted yet: the paint checks that the picture it was asked about is still the one
    // loaded. The last result is kept in {{dataDir}}/theme/tint.json, so a cold start has it
    // before the first sample, and the tool's palette reads the same file for the web menus.
    property real wallLuma: -1                 // -1 until the wallpaper has been sampled
    property var wallTint: null                // {r, g, b} 0..255, or null
    property string sampledSource: ""          // the picture wallLuma and wallTint describe
    readonly property var wallAccentRgb: wallTint ? Tint.accentFrom(wallTint, isLight) : null
    readonly property color wallAccent: wallAccentRgb ? Qt.rgba(wallAccentRgb.r / 255, wallAccentRgb.g / 255, wallAccentRgb.b / 255, 1) : "#9a8cf0"
    function computeMaterial() {
        if (theme.material === "light") root.isLight = true
        else if (theme.material === "dark") root.isLight = false
        else if (wallLuma >= 0) root.isLight = wallLuma > theme.materialThreshold
        root.mat = tintedMaterial(root.isLight ? theme.light : theme.dark)
    }
    function tintedMaterial(m) {
        var mix = (theme.tint && theme.tint.mix) || 0
        if (!(mix > 0) || !wallTint || !m) return m
        var out = {}
        for (var k in m) out[k] = m[k]
        var c = Qt.lighter(m.glassFocus, 1.0), t = Tint.mixToward({ r: c.r, g: c.g, b: c.b, a: c.a }, wallTint, Math.min(1, mix))
        out.glassFocus = Qt.rgba(t.r, t.g, t.b, t.a)
        return out
    }
    // The sample is taken without a canvas: Canvas.drawImage paints nothing on this Qt build
    // (a loaded PNG or an Image item both read back as transparent). The wallpaper is decoded
    // again at 64x36 (area-averaged by the image reader), that item is grabbed at 32x18, and the
    // grab is saved as XPM, the one text image format Qt writes, which reads back through
    // TextFile (Tint.parseXpm).
    property int sampleGen: 0
    readonly property string sampleUrl: "file:///tmp/{{shortId}}-wallsample.xpm"
    TextFile { id: sampleFile; name: root.sampleUrl }
    Image {
        id: wallSmall
        visible: false
        asynchronous: true; cache: false; smooth: true
        sourceSize: Qt.size(64, 36)
        source: wallpaperImg.source
        onStatusChanged: if (status === Image.Ready) root.sampleWallpaper()
    }
    function sampleWallpaper() {
        if (wallSmall.status !== Image.Ready) return
        var gen = ++sampleGen, source = String(wallSmall.source)
        wallSmall.grabToImage(function(grab) {
            if (gen !== sampleGen) return        // a newer wallpaper cancelled this sample
            if (!grab.saveToFile("/tmp/{{shortId}}-wallsample.xpm")) { console.warn("[LumaHome] wallpaper sample could not be saved"); return }
            var px = Tint.parseXpm(sampleFile.read())
            if (!px) { console.warn("[LumaHome] wallpaper sample could not be read"); return }
            root.takeSample(source, Tint.luma(px.data, px.count), Tint.dominant(px.data, px.count))
        }, Qt.size(32, 18))
    }
    function takeSample(source, luma, tint) {
        sampledSource = source
        wallLuma = luma
        wallTint = tint
        console.info("[LumaHome] wallpaper sampled: luma " + luma.toFixed(3) + ", tint " + (tint ? Tint.hex(tint) + " (" + Math.round(tint.share * 100) + " %)" : "none"))
        computeMaterial()
        saveTint()
    }
    TextFile { id: tintFile; name: root.themeDirUrl + "tint.json" }
    property string tintText: ""
    function saveTint() {
        if (!wallTint) return
        var text = JSON.stringify({ wallpaper: sampledSource, tint: Tint.hex(wallTint), accent: Tint.hex(Tint.accentFrom(wallTint, isLight)),
                                    light: isLight, luma: Math.round(wallLuma * 1000) / 1000 })
        if (text === tintText) return
        tintText = text
        tintFile.write(text)
    }
    // A saved sample of the picture now loaded stands in until the real one lands.
    function restoreTint() {
        if (wallLuma >= 0) return
        var j = null
        try { j = JSON.parse(tintFile.exists ? tintFile.read() : "") } catch (e) { j = null }
        var t = j ? Tint.parseHex(j.tint) : null
        if (!t || typeof j.luma !== "number" || j.wallpaper !== String(wallpaperImg.source)) return
        sampledSource = j.wallpaper; wallLuma = j.luma; wallTint = t
        tintText = JSON.stringify(j)
    }
    function scrimColor(v) {
        if (!root.isLight) return Qt.rgba(0, 0, 0, v)
        return v >= 0 ? Qt.rgba(1, 1, 1, v) : Qt.rgba(0, 0, 0, -v)
    }

    // ============================================================= wallpaper + one cached blur
    property string wallpaperFit: (layout.background && layout.background.wallpaperFit) || "1:1"
    Rectangle { anchors.fill: parent; z: -3; color: (layout.background && layout.background.color) || "#0b0910"; visible: root.gpuWall > 0.001; opacity: root.gpuWall }
    Rectangle {
        // theme.<mat>.wallScrim: 3 stops at 0 / 55 / 100 %. Drawn by the live wallpaper's shader
        // while that is on, so the screen carries one full-screen quad, not two.
        anchors.fill: parent
        z: -1
        visible: !root.liveWallOn && root.gpuWall > 0.001
        opacity: root.gpuWall
        gradient: Gradient {
            GradientStop { position: 0.0; color: root.scrimColor((root.mat.wallScrim || [0, 0, 0])[0]) }
            GradientStop { position: 0.55; color: root.scrimColor((root.mat.wallScrim || [0, 0, 0])[1]) }
            GradientStop { position: 1.0; color: root.scrimColor((root.mat.wallScrim || [0, 0, 0])[2]) }
        }
    }
    Image {
        id: wallpaperImg
        anchors.fill: parent
        z: -2
        visible: false
        source: root.resolvePath(theme.wallpaper)
        asynchronous: true
        cache: false
        fillMode: root.wallpaperFit === "cover" ? Image.PreserveAspectCrop
                  : root.wallpaperFit === "contain" ? Image.PreserveAspectFit : Image.Stretch
    }
    // Colour grade (saturation / contrast) baked once into a texture the live pass, the still
    // path and the glass blur all read; re-baked only when the values change, so grading
    // costs nothing per frame.
    ShaderEffect {
        id: gradePass
        width: 1920; height: 1080
        visible: false
        property variant src: wallpaperImg
        property real saturationK: root.wallMotion.saturation === undefined ? 1 : root.wallMotion.saturation
        property real contrastK: root.wallMotion.contrast === undefined ? 1 : root.wallMotion.contrast
        onSaturationKChanged: gradedTex.scheduleUpdate()
        onContrastKChanged: gradedTex.scheduleUpdate()
        fragmentShader: "
            uniform sampler2D src;
            uniform mediump float saturationK;
            uniform mediump float contrastK;
            uniform lowp float qt_Opacity;
            varying highp vec2 qt_TexCoord0;
            void main() {
                lowp vec3 c = texture2D(src, qt_TexCoord0).rgb;
                mediump float l = dot(c, vec3(0.299, 0.587, 0.114));
                c = (mix(vec3(l), c, saturationK) - 0.5) * contrastK + 0.5;
                gl_FragColor = vec4(c, 1.0) * qt_Opacity;
            }"
    }
    ShaderEffectSource {
        id: gradedTex
        sourceItem: gradePass
        width: 1920; height: 1080
        textureSize: Qt.size(1920, 1080)
        live: false
        z: -2
        visible: !root.liveWallOn && wallpaperImg.status === Image.Ready && root.gpuWall > 0.001   // the still path draws the graded texture
        opacity: root.gpuWall
        Connections { target: wallpaperImg; onStatusChanged: if (wallpaperImg.status === Image.Ready) gradedTex.scheduleUpdate() }
    }

    // Live wallpaper (theme.wallpaperMotion): the same texture through a slow domain warp, a
    // few pixels of drift from two sine fields, with the warp also modulating brightness so
    // light appears to play over the waves (the "depth"). One quad, one read per pixel,
    // ticked at wallpaperMotion.fps by a timer rather than the animation clock, so idle
    // draw is bounded and a 60 fps animation on top stays at 60 (measured: a second
    // parallax read cost 10 fps at full layout). Nothing above it re-renders: the glass
    // blur is baked from the still image.
    property var wallMotion: theme.wallpaperMotion || {}
    property bool wallMotionOn: (wallMotion.enabled !== false) && !root.motionReduced
    // One clock for everything that moves at rest: the wallpaper phase and the clock's
    // hands advance on the same tick, so idle Home presents one frame per tick. Separate
    // 33 ms wallpaper and 1 s + 160 ms second-hand timers measured an uneven 37 fps with
    // a 50 ms hitch every 1.5 s (2026-10-23). The phase follows real elapsed time, so
    // timer jitter never shows as uneven motion; a gap over 200 ms (Home hidden) counts
    // as one tick.
    property int frameTick: 0
    property double lastTickMs: 0
    // Under LumaTV's compositor (compositor/shell/Shell.qml) Home is a window of its own that something may cover whole (an app's card,
    // a full-screen overlay): the tick stops then, so a Home nobody sees draws nothing. Never set under LG's compositor.
    property bool idleHold: false
    Timer {
        // With the clip on the plane the wallpaper has nothing to advance and the tick is the clock's alone (its second hand): once a second. The
        // tick stops under the saver as well: Home is not seen then.
        interval: root.videoWallUp ? 1000 : Math.round(1000 / (root.wallMotion.fps || 10))
        running: root.hostActive && !root.idleHold && !root.saverUp
        repeat: true
        onTriggered: {
            var now = Date.now()
            var dt = now - root.lastTickMs
            if (dt <= 0 || dt > 200) dt = interval
            root.lastTickMs = now
            if (root.liveWallOn && !root.videoWallUp) liveWall.t += dt / 1000 * liveWall.speed
            root.frameTick++
        }
    }
    ShaderEffect {
        id: liveWall
        anchors.fill: parent
        z: -2
        visible: root.liveWallOn && root.gpuWall > 0.001
        opacity: root.gpuWall
        property variant src: gradedTex
        property real t: 0
        property real amp: (root.wallMotion.amplitude || 4) / 1920
        property real speed: root.wallMotion.speed || 1
        property real depth: root.wallMotion.depth === undefined ? 0.25 : root.wallMotion.depth
        property real waveScale: root.wallMotion.scale || 1          // wave frequency multiplier
        property real detail: root.wallMotion.detail === undefined ? 0.35 : root.wallMotion.detail   // finer second layer, 0..1
        property color s0: root.scrimColor((root.mat.wallScrim || [0, 0, 0])[0])
        property color s1: root.scrimColor((root.mat.wallScrim || [0, 0, 0])[1])
        property color s2: root.scrimColor((root.mat.wallScrim || [0, 0, 0])[2])
        property vector4d scrimA: Qt.vector4d(s0.r, s0.g, s0.b, s0.a)
        property vector4d scrimB: Qt.vector4d(s1.r, s1.g, s1.b, s1.a)
        property vector4d scrimC: Qt.vector4d(s2.r, s2.g, s2.b, s2.a)
        // phases advanced on the CPU once per tick; the fragment only evaluates the fields
        property vector4d phase: Qt.vector4d(t * 0.70, t * 0.50, t * 0.40, t * 0.90)
        property vector4d params: Qt.vector4d(amp, depth, waveScale, detail)
        // look: vignette, specular, bloom, saturation | contrast
        property real vignette: root.wallMotion.vignette === undefined ? 0 : root.wallMotion.vignette
        property real specular: root.wallMotion.specular === undefined ? 0 : root.wallMotion.specular
        property real bloom: root.wallMotion.bloom === undefined ? 0 : root.wallMotion.bloom
        property vector4d look: Qt.vector4d(vignette, specular, bloom, 0)
        property variant glow: blurTex
        // The fields are at most 6 cycles across the screen, so they are evaluated per
        // vertex on a grid and interpolated; the fragment is one read plus the scrim.
        mesh: GridMesh { resolution: Qt.size(64, 36) }
        vertexShader: "
            uniform highp mat4 qt_Matrix;
            uniform highp vec4 phase;
            uniform highp vec4 params;   // amp, depth, scale, detail
            uniform mediump vec4 look;   // vignette, specular, bloom, saturation
            uniform sampler2D glow;
            attribute highp vec4 qt_Vertex;
            attribute highp vec2 qt_MultiTexCoord0;
            varying highp vec2 vUv;
            varying mediump float vLight;
            varying mediump vec3 vAdd;   // specular + bloom, added per fragment
            void main() {
                highp vec2 uv = qt_MultiTexCoord0;
                highp float k = params.z;
                mediump float w1 = sin(uv.y * 6.0 * k + phase.x) * cos(uv.x * 4.0 * k - phase.y);
                mediump float w2 = sin((uv.x + uv.y) * 5.0 * k - phase.z);
                mediump float w3 = sin(uv.x * 11.0 * k + phase.w) * sin(uv.y * 9.0 * k - phase.x * 0.8);
                highp vec2 disp = vec2(w1 + 0.5 * w2, 0.7 * w2 - 0.5 * w1) + params.w * 0.5 * vec2(w3, -w3);
                vUv = uv + params.x * disp;
                // light follows the slope of the displacement: crests brighten, troughs darken
                vLight = 1.0 + params.y * (0.3 * (w1 - 0.5 * w2) + params.w * 0.25 * w3);
                // glossy ridge: a narrow lobe on the crest of the main field
                mediump float crest = clamp(0.5 + 0.5 * (w1 + 0.4 * w3), 0.0, 1.0);
                mediump float spec = look.y * 0.35 * crest * crest * crest * crest * crest * crest;
                // vignette folded into the light term: quadratic falloff, aspect-corrected
                highp vec2 q = (uv - 0.5) * vec2(1.0, 0.5625);
                vLight *= 1.0 - look.x * 0.75 * smoothstep(0.12, 0.42, dot(q, q));
                // bloom from the already-baked blur, fetched here: the glow is 30px soft, so
                // sampling it per vertex and interpolating loses nothing and costs no fragment read
                mediump vec3 bl = look.z > 0.001 ? max(texture2DLod(glow, vUv, 0.0).rgb - 0.3, 0.0) * look.z : vec3(0.0);
                vAdd = vec3(spec) + bl;
                gl_Position = qt_Matrix * qt_Vertex;
            }"
        fragmentShader: "
            uniform sampler2D src;
            uniform mediump vec4 scrimA;
            uniform mediump vec4 scrimB;
            uniform mediump vec4 scrimC;
            uniform lowp float qt_Opacity;
            varying highp vec2 vUv;
            varying mediump float vLight;
            varying mediump vec3 vAdd;
            void main() {
                lowp vec3 c = texture2D(src, vUv).rgb * vLight + vAdd;
                mediump float y = vUv.y;
                mediump vec4 sc = y < 0.55 ? mix(scrimA, scrimB, y / 0.55) : mix(scrimB, scrimC, (y - 0.55) / 0.45);
                c = mix(c, sc.rgb, sc.a);
                gl_FragColor = vec4(c, 1.0) * qt_Opacity;
            }"
    }
    // The glass backdrop: wallpaper at half resolution, blurred and saturated once (the
    // sources are static, so the live chain re-renders only when the wallpaper changes).
    // Every Glass samples blurTex at its own screen rect; nothing here runs per frame.
    ShaderEffectSource {
        id: wallHalf
        sourceItem: gradedTex
        width: 960; height: 540
        textureSize: Qt.size(960, 540)
        visible: false
    }
    GaussianBlur {
        id: blurPass
        width: 960; height: 540
        source: wallHalf
        radius: Math.round((root.theme.blurRadius || 30) / 2)
        samples: radius * 2 + 1
        visible: false
    }
    HueSaturation {
        id: satPass
        width: 960; height: 540
        source: blurPass
        saturation: root.theme.saturation === undefined ? 0.25 : root.theme.saturation
        visible: false
    }
    ShaderEffectSource {
        id: blurTex
        sourceItem: root.wallGlassTint ? flatPlate : satPass
        width: 960; height: 540
        textureSize: Qt.size(960, 540)
        visible: false
        smooth: true
    }
    // The tint-only glass's backdrop: one flat colour, the mean of the clip's pictures (the pane then reads as the frosted look in colour).
    Rectangle { id: flatPlate; width: 960; height: 540; visible: false; color: root.wallPick.entry ? root.wallPick.entry.glass.flat : "#111018" }

    // ============================================================= focus model
    // "dock" | "widgets" | "statusbar" | "popover"
    property string focusLayer: "dock"
    property int dockRow: 0
    property int dockCol: 0
    property string widgetFocusId: nonDockWidgets.length ? nonDockWidgets[0].id : ""
    property int statusFocusIndex: 0
    property var statusItems: (layout.statusBar.left || []).concat(layout.statusBar.right || [])
    property bool popoverOpen: focusLayer === "popover"

    // Focus rest. The highlight moves with every key press; lift and scale start only once
    // the focused item has rested motion.restMs, and any further move cancels them. Lifting
    // glass per key press measured 16.8 ms against 127 ms mean frame time in a launcher
    // that did it, so a sweep over the dock is one redraw per press and nothing animates
    // until the user stops. Items take `rested` and animate their lift only on gaining it.
    // The identity is the item, not the layer: entering a card's inner layer keeps the card
    // lifted. With reduced motion everything lifts at once.
    property int restMs: motion.restMs || 150
    property bool focusRested: true
    readonly property string focusKey: focusLayer === "dock" ? "d" + dockRow + ":" + dockCol
                                     : (focusLayer === "widgets" || focusLayer === "widget") ? "w" + widgetFocusId
                                     : (focusLayer === "statusbar" || focusLayer === "popover") ? "s" + statusFocusIndex
                                     : focusLayer
    onFocusKeyChanged: {
        if (motionReduced) return
        focusRested = false
        restTimer.restart()
    }
    Timer { id: restTimer; interval: root.restMs; onTriggered: root.focusRested = true }

    onHostActiveChanged: {
        resetOk()
        if (hostActive) { Lu.backdrop = blurTex; focusLayer = "dock"; statusBarItem.requestClosePopover(); layers.recentsRead() }
        else {
            if (voiceBusy) voiceAbort()
            if (settingsOpen) closeSettings()
            if (editing) { if (editRes.item) previewLayout(editRes.item.orig); editEnd() }
            closeAllLayers()
            if (miniFrame.open) { miniFrame.open = false; miniFrame.loading = false; miniSpec = null; frameAppWanted = false }
        }
    }
    // A held OK or a tile picked up for rearranging never outlives the screen it belongs to:
    // the release of a press taken on Home can arrive after an app has opened, and Home must
    // not come back mid-pick-up.
    function resetOk() {
        holdTimer.stop()
        okDown = false; okHeld = false
        var d = dockLoader.item
        if (d && d.moving) d.cancelMove()
    }

    function widgetRectById(id) {
        var i = widgetIndexById(id)
        return i >= 0 ? cellRect(nonDockWidgets[i]) : null
    }
    function nearestWidgetByX(x) {
        var best = -1, bestD = Infinity
        for (var i = 0; i < nonDockWidgets.length; i++) {
            var r = cellRect(nonDockWidgets[i])
            var d = Math.abs((r.x + r.width / 2) - x)
            if (d < bestD) { bestD = d; best = i }
        }
        return best >= 0 ? nonDockWidgets[best].id : ""
    }
    function lowestWidgetByX(x) {
        // entering from the dock: prefer the card whose bottom edge is nearest the dock
        var best = -1, bestScore = Infinity
        for (var i = 0; i < nonDockWidgets.length; i++) {
            var r = cellRect(nonDockWidgets[i])
            var score = Math.abs((r.x + r.width / 2) - x) + (areaRect().y + areaRect().height - (r.y + r.height)) * 2
            if (score < bestScore) { bestScore = score; best = i }
        }
        return best >= 0 ? nonDockWidgets[best].id : ""
    }
    function nearestWidgetInDirection(fromId, dx, dy) {
        var fr = widgetRectById(fromId)
        if (!fr) return fromId
        var fcx = fr.x + fr.width / 2, fcy = fr.y + fr.height / 2
        var best = -1, bestScore = Infinity
        for (var i = 0; i < nonDockWidgets.length; i++) {
            var e = nonDockWidgets[i]
            if (e.id === fromId) continue
            var r = cellRect(e)
            var cx = r.x + r.width / 2, cy = r.y + r.height / 2
            var ddx = cx - fcx, ddy = cy - fcy
            if (dx !== 0 && ddx * dx <= 0) continue
            if (dy !== 0 && ddy * dy <= 0) continue
            var primary = dx !== 0 ? Math.abs(ddx) : Math.abs(ddy)
            var secondary = dx !== 0 ? Math.abs(ddy) : Math.abs(ddx)
            var score = primary + secondary * 2
            if (score < bestScore) { bestScore = score; best = i }
        }
        return best >= 0 ? nonDockWidgets[best].id : fromId
    }

    function directionFor(k) {
        if (k === Qt.Key_Left) return "left"
        if (k === Qt.Key_Right) return "right"
        if (k === Qt.Key_Up) return "up"
        if (k === Qt.Key_Down) return "down"
        if (k === Qt.Key_Return || k === Qt.Key_Enter || k === Qt.Key_Select) return "ok"
        if (k === Qt.Key_Back || k === WebOS.Key_webOS_Back) return "back"
        return ""
    }

    // Returns true when the key is consumed; false lets the stock chain deliver it to Home's
    // own surface (Back from the widget layer closes Home exactly as stock does).
    // OK is decided on release, never on press: a press only starts the hold timer. Held
    // motion.holdMs it picks a dock tile up for rearranging (the one layer with a held
    // action); anywhere else a long press is an ordinary one. Auto-repeat presses are
    // ignored. The key filter hands over the release of every press it took, wherever Home
    // is by then (payload/keyfilter/lumatv.js).
    property int holdMs: motion.holdMs || 700
    property bool okDown: false
    property bool okHeld: false
    // A held OK has a meaning on a dock tile and on a library icon (the options of that app) and, in Settings, on a row with
    // an info mark (its help); anywhere else a long press is an ordinary one.
    function holdAvailable() {
        if (layers.open === "apps" || layers.open === "recents") return true
        if (layers.open === "settings" || layers.open === "quick") return layers.holdReady()
        var d = dockLoader.item
        if (layers.open === "" && focusLayer === "widgets" && widgetFocusId !== "" && !editing) return true          // a card: Edit Home
        return layers.open === "" && focusLayer === "dock" && !!d && !d.moving
    }
    Timer {
        id: holdTimer
        interval: root.holdMs
        onTriggered: {
            if (!root.okDown || !root.holdAvailable()) return
            root.okHeld = true
            if (layers.open === "apps" || layers.open === "settings" || layers.open === "recents" || layers.open === "quick") layers.hold()
            else if (root.focusLayer === "widgets") root.editHome(root.widgetFocusId)
            else root.openTileMenu()
        }
    }
    // ---- the number keys open the dock's tiles (Settings, Remote & pointer, "Number keys open dock tiles": theme `remote.numberKeys`,
    // on unless the owner turned it off). 1 is the first tile, 2 the second and so on to 9; 0 is the tenth. Only on Home with nothing
    // open: anywhere else a digit means what LG's own handling makes of it, and so does a digit with no tile behind it. The tile takes
    // Home's focus, so it is as if the owner had walked to it and pressed OK. The press acts (a digit has no hold), and its repeats and
    // release are taken with it, wherever Home is by then.
    // Quick Access (theme `remote.quickAccess`, settings/QuickAccess.js, off until the TV check): with it on, a digit says what it opened in a
    // toast (panels/QuickAccess.qml), the owner's own choice for a number comes before the dock's tile, and a digit with nothing behind it
    // asks what to assign instead of being LG's.
    readonly property bool numberKeysOn: !theme.remote || theme.remote.numberKeys !== false
    readonly property bool quickAccessOn: QA.on(theme)
    property int numberEaten: 0
    function tileForKey(k) { return k >= Qt.Key_1 && k <= Qt.Key_9 ? k - Qt.Key_1 : k === Qt.Key_0 ? 9 : -1 }
    function dockIds() { var d = dockLoader.item; return d ? d.points.map(function(p) { return p.id }) : [] }
    function appById(id) { return Apps.byId(id) }
    function numberKey(k, tile, pressed, autoRepeat) {
        if (pressed && !autoRepeat) numberEaten = 0                // a new press supersedes one whose release never came
        if (numberEaten === k) {
            if (!pressed) numberEaten = 0
            return true
        }
        if (!pressed || autoRepeat) return false
        if (!hostActive || !numberKeysOn || aiPrompt.up || trialOn || focusLayer === "popover") return false
        if (layers.open === "access") return true                  // the question has the keys: a digit is not an answer, and is not LG's
        if (layers.open !== "") return false
        var digit = k === Qt.Key_0 ? "0" : String(k - Qt.Key_0)
        if (quickAccessOn) {
            var slot = QA.target(theme, digit, dockIds(), appById)
            var app = slot.kind === "slot" ? Apps.byId(slot.id) : null
            if (app) { numberEaten = k; focusLayer = "dock"; launchApp(app); layers.accessToast(digit, app.title || app.id, "Opening."); return true }
            if (slot.kind === "none") { numberEaten = k; accessAsk(digit); return true }
        }
        var d = dockLoader.item
        var t = d ? d.tileAt(tile) : null
        if (!d || !d.openTile(tile)) return false
        numberEaten = k
        focusLayer = "dock"
        if (quickAccessOn && t && layers.open === "") layers.accessToast(digit, t.title || t.id, "Opening.")
        return true
    }
    // The question for a number with nothing behind it: four apps no number opens yet, More and Not now.
    function accessAsk(digit) {
        layers.accessAsk(digit, QA.suggestions(theme, dockIds(), Apps.shown, 4))
        openLayer("access")
    }
    // What the question did: an app was picked (the choice is kept, and the app opens), or More was (Settings has every number).
    function accessAssigned(digit, appId) {
        var cfg = QA.chosen(theme, digit, appId)
        patchTheme({ remote: { quickAccess: cfg } })
        persistTheme({ remote: { quickAccess: cfg } })
        var app = Apps.byId(appId)
        if (app) { launchApp(app); layers.accessToast(digit, app.title || app.id, "Assigned. Opening.") }
    }
    function accessMore(digit) { openSettings(); layers.settingsGoTo("remote") }
    // ---- recent apps (Settings, Remote & pointer, "Recent apps on Back": theme `remote.recents`, off until the TV check). Back from the top of Home
    // opens the switcher (panels/Recents.qml) when LG's recents list holds an app, instead of leaving Home for the latest one; with the
    // switch off or the list empty Back is what it always was.
    readonly property bool recentsOn: !!theme.remote && theme.remote.recents === true
    function recentsBack() {
        if (!recentsOn || layers.recentsCount === 0) return false
        openLayer("recents")
        return true
    }
    function recentsPicked(appId) {
        var app = Apps.byId(appId)
        if (app) { launchApp(app); return }
        closeAllLayers()
        layers.launchSplash(appId, "")
        LS.adhoc.call(appMgr, "/launch", JSON.stringify({ id: appId }))
    }
    // ---- the gear key held (payload/keyfilter/lumatv.js times the hold): the notification drawer, from wherever Home is. While Home can
    // do this the filter stops LG's Settings long press (theme `remote.gearHold`, on unless the owner turned it off, hands the hold back to
    // LG). Held again while the drawer is up it closes it, as LG's own hold toggles Settings. The AI prompt and the saver own the remote:
    // the hold is taken and does nothing, so LG's Settings does not open behind them.
    readonly property bool gearHoldOn: !theme.remote || theme.remote.gearHold !== false
    function gearReady() { return hostActive && gearHoldOn }
    function gearHold() {
        if (!gearReady()) return false
        if (aiPrompt.up || layers.saverUp) return true
        if (layers.open === "drawer") { closeLayer(); return true }
        closeAllLayers()
        if (settingsOpen) closeSettings()
        openLayer("drawer")
        return true
    }
    // ---- the remote's app keys (Settings, Remote & pointer: theme `remote.appKeys`, the shape is settings/AppKeys.js). Netflix, Prime Video,
    // Disney+ and LG Channels open what the owner chose, or nothing; a key never chosen is LG's own target where that app is installed and
    // nothing where it is not, so no key starts LG's auto-install. The key filter asks in any foreground and the TV on; true is a key taken.
    // Pressed over the app it is set to open, the key does nothing: LG hands such a key to the app, which has no use for another's.
    function appKey(id, frontId) {
        var points = Apps.launchPoints
        if (!points || points.length === 0) return false         // the app list has not loaded: no key is swallowed on a guess
        var t = AppKeys.target(AppKeys.config(theme), id, AppKeys.installed(points))
        if (t.kind === "default") return !t.installed           // LG's own, where there is one to open
        if (t.kind === "app" && t.installed && t.appId !== frontId) openAppById(t.appId)
        return true
    }
    function openAppById(id) {
        var app = Apps.byId(id)
        if (hostActive && app) launchApp(app)                  // from Home: the splash, and the layers closed, as for a tile
        else LS.adhoc.call(appMgr, "/launch", JSON.stringify({ id: id }))
    }
    property int saverEaten: 0              // the key whose press the screensaver took: its release is taken too
    function key(k, pressed, autoRepeat, deviceId) {
        // A volume key press is LG's whatever is on screen. The display does not wait for it: it answers the audio service and the receiver
        // (panels/VolumeOsd.qml, in LumaOverlay). A saver LumaTV owns (the TV is Active, nothing else will end it) ends with it, as LG's own
        // `Screen Saver` state ends on any key; in LG's state the TV's change of state does that, and the saver is left to it.
        if (pressed && !autoRepeat && (k === Qt.Key_VolumeUp || k === Qt.Key_VolumeDown || k === Qt.Key_VolumeMute)) { layers.saverEnd(); return false }
        // The screensaver is up: any other key wakes the TV and does nothing else, so the key that wakes it never acts on Home;
        // the release goes with it (an OK release alone would open whatever is focused). The AI prompt is the exception: it is
        // drawn over the saver and only the physical remote answers it.
        if (hostActive && !aiPrompt.up && (layers.saverUp || (!pressed && saverEaten === k))) {
            if (pressed) { if (!autoRepeat) { saverEaten = k; layers.saverKey() } }
            else if (saverEaten === k) saverEaten = 0
            return true
        }
        // The layout editor has the D-pad, OK and Back while it is up (a confirmation it asks with is a layer and has the keys instead).
        if (editing && hostActive && !aiPrompt.up && layers.open === "") return editKey(k, pressed, autoRepeat)
        // The mic key (push-to-talk) is LumaTV's while voice search is ready: the press starts listening, the release ends it, and
        // LG's chain never sees either (LG's voice would send the speech to LG). When the helper does not say `ready` (voice search
        // is off, not linked, or has no microphone) the key goes down LG's chain as it always did.
        if (hostActive && !aiPrompt.up && k === WebOS.Key_webOS_Voice && (voiceBusy || (voiceReady && !settingsOpen))) {
            if (pressed && !autoRepeat) voiceKeyDown()
            else if (!pressed) voiceKeyUp()
            return true
        }
        // The Menu key: the options of the app under the focus (docs/mocks/16-context-menu: "hold OK or the Menu key on a dock
        // tile", and on a library icon); anywhere else LumaTV's own quick panel when the owner has turned it on (theme
        // `quickPanel.own`), else the library, which is where LG's "Home with My Apps" (this same key) goes.
        if (hostActive && k === Qt.Key_Menu && !aiPrompt.up) {
            if (pressed && !autoRepeat) menuKey()
            return true
        }
        // The on-screen remote key (123) opens LumaTV's remote; the Search key opens search (a surface that has the keys keeps them)
        if (hostActive && pressed && !autoRepeat && !aiPrompt.up && k === WebOS.Key_webOS_ScreenRemote) { remoteKeyPressed(); return true }
        if (hostActive && pressed && !autoRepeat && !aiPrompt.up && k === Qt.Key_Search && layers.open === "") { openSearch(); return true }
        var tile = tileForKey(k)
        if (tile >= 0) return numberKey(k, tile, pressed, autoRepeat)
        var dir = directionFor(k)
        if (!dir) return false
        // An AI prompt is up: the helper reads the answer from the physical remote itself, and nothing
        // else may act on these keys (not Home, not Settings behind it).
        if (hostActive && aiPrompt.up) return true
        if (!hostActive) {
            // the release of a press taken on Home can arrive after an app has opened: it
            // does nothing but clear the OK state
            if (dir === "ok" && !pressed) resetOk()
            return false
        }
        if (trialOn && dir === "ok") {
            // the trial's toast owns OK until the trial ends
            if (pressed && !autoRepeat && !trialConfirming) confirmTrial()
            return true
        }
        if (dir === "ok") {
            if (pressed) {
                if (autoRepeat || okDown) return true
                okDown = true; okHeld = false; holdTimer.restart()
                return true
            }
            holdTimer.stop()
            var wasHeld = okHeld
            okDown = false; okHeld = false
            if (wasHeld) return true
            return keyAction("ok")
        }
        if (!pressed) return true
        if (autoRepeat && dir === "back") return true
        return keyAction(dir)
    }
    function keyAction(dir) {
        if (layers.open !== "" && !layers.find(layers.open)) closeLayer()      // a name that is no surface owns no key
        // a camera pop-up or viewer is up over Home (LumaOverlay's surface): OK and Back of the pop-up, every key of the viewer, are its
        if (layers.open === "" && Bus.cameraKey(dir)) return true
        if (layers.open !== "") {
            // a modal surface owns the keys while it is up; Back closes the kit gallery, any other asks to close itself
            if (layers.open === "gallery" && dir === "back") closeLayer()
            else layers.key(dir)
            return true
        }
        var d = dockLoader.item

        if (focusLayer === "popover") {
            if (dir === "back") { statusBarItem.requestClosePopover(); focusLayer = "statusbar" }
            else if (dir === "ok") { statusBarItem.popoverActivate(); if (!statusBarItem.popoverOpen) focusLayer = "statusbar" }
            else if (dir === "up") statusBarItem.popoverMove(-1)
            else if (dir === "down") statusBarItem.popoverMove(1)
            else if (dir === "left") statusBarItem.popoverAdjust(-1)
            else if (dir === "right") statusBarItem.popoverAdjust(1)
            return true
        }
        if (focusLayer === "dock") {
            if (!d) return false
            if (dir === "left") d.moveLeft()
            else if (dir === "right") d.moveRight()
            else if (dir === "down") d.moveDown()
            else if (dir === "up") { if (!d.moveUp()) { widgetFocusId = lowestWidgetByX(d.focusedCenterX()); focusLayer = "widgets" } }
            else if (dir === "back") { if (d.moving) d.cancelMove(); else { widgetFocusId = lowestWidgetByX(d.focusedCenterX()); focusLayer = "widgets" } }
            else if (dir === "ok") { if (d.moving) d.endMove(); else d.activate() }
            return true
        }
        if (focusLayer === "frame") {
            // only reached while the overlay app is not yet up (loading); Back aborts
            if (dir === "back") closeMini()
            return true
        }
        if (focusLayer === "widget") {
            var wIdx = widgetIndexById(widgetFocusId)
            var w = wIdx >= 0 ? widgetRepeater.itemAt(wIdx) : null
            if (dir === "back" || !w) { if (w) w.inner = false; focusLayer = "widgets"; return true }
            w.innerKey(dir)
            return true
        }
        if (focusLayer === "widgets") {
            if (dir === "back") return recentsBack()
            var idx = widgetIndexById(widgetFocusId)
            var entry = idx >= 0 ? nonDockWidgets[idx] : null
            if (dir === "ok") {
                var wi = idx >= 0 ? widgetRepeater.itemAt(idx) : null
                if (!wi) return true
                if (wi.hasInner()) { wi.inner = true; focusLayer = "widget" } else wi.activate()
                return true
            }
            if (dir === "up" && entry && isTopRow(entry)) { statusFocusIndex = nearestStatusByX(cellRect(entry).x + cellRect(entry).width / 2); focusLayer = "statusbar"; return true }
            if (dir === "down" && entry && isBottomRow(entry) && d) {
                var r = cellRect(entry)
                d.col = Math.max(0, Math.min(d.cols - 1, Math.round((r.x + r.width / 2 - d.originX - d.padX - d.tileSize / 2) / (d.tileSize + d.gap))))
                d.row = d.scrollRow
                if (d.row * d.cols + d.col >= d.points.length) d.col = Math.max(0, d.points.length - 1 - d.row * d.cols)
                focusLayer = "dock"
                return true
            }
            var dx = dir === "left" ? -1 : dir === "right" ? 1 : 0
            var dy = dir === "up" ? -1 : dir === "down" ? 1 : 0
            if (dx !== 0 || dy !== 0) widgetFocusId = nearestWidgetInDirection(widgetFocusId, dx, dy)
            return true
        }
        if (focusLayer === "statusbar") {
            if (dir === "back") return recentsBack()
            if (dir === "left") statusFocusIndex = Math.max(0, statusFocusIndex - 1)
            else if (dir === "right") statusFocusIndex = Math.min(statusItems.length - 1, statusFocusIndex + 1)
            else if (dir === "down") { widgetFocusId = nearestWidgetByX(statusBarItem.focusedGlobalX()); focusLayer = "widgets" }
            else if (dir === "ok") statusBarItem.activate(statusFocusIndex)
            return true
        }
        return false
    }
    function nearestStatusByX(x) {
        var best = 0, bestD = Infinity
        for (var i = 0; i < statusItems.length; i++) {
            var it = statusBarItem.itemAt(i)
            if (!it) continue
            var cx = statusBarItem.x + it.parent.x + it.x + 24
            if (Math.abs(cx - x) < bestD) { bestD = Math.abs(cx - x); best = i }
        }
        return best
    }
    Component.onCompleted: {
        Lu.backdrop = blurTex
        Bus.setHandler(root.key); Bus.setEscapeHandler(root.escapeRequested); Bus.setGear({ ready: root.gearReady, hold: root.gearHold }); Bus.setAppKey(root.appKey)
        // what a camera pop-up must not cover: a prompt, a layer that has the keys (a PIN, a confirmation, Settings), the layout editor
        Bus.setHomeBusy(function() { return root.hostActive && (aiPrompt.up || layers.open !== "" || root.editing || root.trialOn) })
        Bus.setThemeWriter({ patch: root.patchTheme, persist: root.persistTheme })
        root.beatOnce()
        console.info("[LumaHome] loaded, first beat written")
        loadAll(); readMode(); Helper.start(); NoteLog.seenAt = Date.now()       // the log's own toast subscription starts when it is first read
    }
    Component.onDestruction: { Bus.setHandler(null); Bus.setEscapeHandler(null); Bus.setGear(null); Bus.setAppKey(null); Bus.setThemeWriter(null); Bus.setHomeBusy(null) }

    // Pointer: anything not over a card/tile lands here so the stock Home underneath never
    // sees the Magic Remote.
    MouseArea { anchors.fill: parent; hoverEnabled: true; z: 0 }


    // ============================================================= chrome
    StatusBar {
        id: statusBarItem
        z: 3
        visible: !root.settingsOpen && !root.appsUp && !root.editing
        x: layout.safe.x
        y: layout.safe.y
        width: 1920 - 2 * layout.safe.x
        height: layout.statusBar.height
        theme: root.theme
        mat: root.mat
        isLight: root.isLight
        items: root.statusItems
        leftCount: (layout.statusBar.left || []).length
        focusedLayer: root.focusLayer === "statusbar" || root.focusLayer === "popover"
        focusedIndex: root.statusFocusIndex
        rested: root.focusRested
        motionReduced: root.motionReduced
        backdrop: blurTex
        cardMs: root.cardMs
        cardEasing: root.cardEasing
        layerMs: root.layerMs
        onRequestOpenPopover: { root.statusFocusIndex = index; root.focusLayer = "popover" }
        onThemePatch: root.patchTheme(patch)
        onThemePersist: root.persistTheme(patch)
        onRequestOpenSettings: root.openSettings()
        onRequestSearch: root.openSearch()
        onRequestNotifications: root.openLayer("drawer")
        onHoverFocus: { root.statusFocusIndex = index; if (root.focusLayer !== "popover") root.focusLayer = "statusbar" }
    }

    // Popover scrim: one full-screen quad between the content and the status bar. A modest
    // dim (~30%) marks the popover as the top layer while the page stays readable. The
    // wallpaper settings popover instead clears the page: widgets and dock fade out so the
    // wallpaper is seen bare while it is adjusted.
    property bool previewMode: (root.popoverOpen && statusBarItem.popoverType === "settings") || root.settingsOpen || root.appsUp
    Rectangle {
        anchors.fill: parent
        z: miniFrame.open ? 3.5 : 2
        color: root.isLight ? "#ffffff" : "#000000"
        opacity: (root.popoverOpen && !root.previewMode) || miniFrame.open ? statusBarItem.popoverScrim : 0
        visible: opacity > 0.005
        Behavior on opacity { NumberAnimation { duration: root.layerMs; easing.type: Easing.OutCubic } }
        MouseArea { anchors.fill: parent; hoverEnabled: true; onClicked: { if (miniFrame.open) root.closeMini(); else { statusBarItem.requestClosePopover(); root.focusLayer = "statusbar" } } }
    }

    // At 120 Hz (Lu.fast) a panel sliding over Home draws the widgets and the dock from one texture each (the item layer, rendered once
    // when the slide starts and again only when something in them changes: the clock's second), not their nodes in every frame of the
    // slide: a frame of Home costs 11-13.5 ms of GPU at 60 and has 8.33 ms at 120. The dock's texture reaches past its rect for the
    // glass shadows, which the layer would otherwise clip. At 60 nothing is layered.
    readonly property real layerReach: 80 * Math.max(1, Lu.depth)
    Item {
        id: widgetLayer
        anchors.fill: parent
        z: 1
        opacity: root.previewMode ? 0 : 1
        visible: opacity > 0.005
        Behavior on opacity { NumberAnimation { duration: root.layerMs; easing.type: Easing.OutCubic } }
        layer.enabled: Lu.fast && layers.panelSliding
        Repeater {
            id: widgetRepeater
            model: root.widgetIds
            delegate: Widget {
                // the card of this id in the layout now: a new layout (a move in the editor, a width from Settings) moves and sizes this
                // delegate and builds nothing; one is built only for a card that comes, and dropped for one that goes
                readonly property var entry: { var i = root.widgetIndexById(modelData); return i >= 0 ? root.nonDockWidgets[i] : ({ id: modelData, type: "", col: 0, row: 0, span: 1, rows: 1 }) }
                readonly property var box: root.cellRect(entry)
                gridX: box.x
                gridY: box.y
                width: box.width
                height: box.height
                theme: root.theme
                mat: root.mat
                isLight: root.isLight
                layoutEntry: entry
                themeDirUrl: root.themeDirUrl
                moduleDirUrl: root.moduleDirUrl
                focused: (root.focusLayer === "widgets" || root.focusLayer === "widget") && root.widgetFocusId === modelData
                dimmed: (root.focusLayer === "widgets" || root.focusLayer === "widget") && root.widgetFocusId !== modelData
                onOpenMini: root.openMini(spec, box)
                onOpenSettingsPage: { root.openSettings(); layers.settingsGoTo(id) }
                backdrop: blurTex
                cardMs: root.cardMs
                cardEasing: root.cardEasing
                motionReduced: root.motionReduced
                rested: root.focusRested
                live: root.hostActive && !root.saverUp
                frameTick: root.frameTick
                onHoverFocus: { if (root.focusLayer === "widget" && root.widgetFocusId !== modelData) { var o = widgetRepeater.itemAt(root.widgetIndexById(root.widgetFocusId)); if (o) o.inner = false } root.widgetFocusId = modelData; if (root.focusLayer !== "widget" || root.widgetFocusId !== modelData) root.focusLayer = "widgets" }
            }
        }
    }

    Loader {
        id: dockLoader
        z: 1
        opacity: root.previewMode ? 0 : 1
        visible: opacity > 0.005
        Behavior on opacity { NumberAnimation { duration: root.layerMs; easing.type: Easing.OutCubic } }
        layer.enabled: Lu.fast && layers.panelSliding
        layer.sourceRect: Qt.rect(-root.layerReach, -root.layerReach, width + 2 * root.layerReach, height + 2 * root.layerReach)
        active: root.dockEntry !== null
        x: active ? root.cellRect(root.dockEntry).x : 0
        y: active ? root.cellRect(root.dockEntry).y : 0
        width: active ? root.cellRect(root.dockEntry).width : 0
        height: active ? root.cellRect(root.dockEntry).height : 0
        sourceComponent: Dock {
            theme: root.theme
            mat: root.mat
            isLight: root.isLight
            dockEntry: root.dockEntry || ({})
            originX: dockLoader.x
            originY: dockLoader.y
            focusedLayer: root.focusLayer === "dock"
            dimmed: root.focusLayer === "widgets"
            wallAccent: root.wallAccent
            backdrop: blurTex
            scrollMs: root.scrollMs
            scrollEasing: root.scrollEasing
            focusMs: root.focusMs
            focusEasing: root.focusEasing
            labelMs: root.labelMs
            motionReduced: root.motionReduced
            rested: root.focusRested
            onRowChanged: root.dockRow = row
            onColChanged: root.dockCol = col
            onLaunchingFrom: root.noteLaunchOrigin(appId, tile)
            onLaunching: layers.launchSplash(title, iconUrl)
            onAppsRequested: root.openApps()
            onMenuRequested: root.openTileMenu()
        }
        MouseArea {
            // hovering the dock card itself hands the layer to the dock
            anchors.fill: parent
            hoverEnabled: true
            z: -1
            onEntered: root.focusLayer = "dock"
        }
    }

    // ============================================================= mini apps
    // A card's content asks for a mini app with a spec {kind, title, subtitle, url, ...}.
    // The frame grows out of the card; once it has landed, "web" specs launch the overlay
    // app {{appId}}.frame with the interior rect, "qml" specs instantiate a
    // component in the frame. Closing: Back in the overlay makes the compositor close the
    // app; the running-apps subscription sees it go and the frame shrinks back into the
    // card. Back while still loading closes directly (and the app, if it started).
    readonly property string frameAppId: "{{appId}}.frame"
    property var miniSpec: null
    property bool frameAppRunning: false
    property bool frameAppWanted: false
    function openMini(spec, fromRect) {
        if (!spec) return
        miniSpec = spec
        miniFrame.fromRect = fromRect
        miniFrame.title = spec.title || ""
        miniFrame.subtitle = spec.subtitle || ""
        miniFrame.loading = true
        miniFrame.open = true
        focusLayer = "frame"
    }
    function launchFrameApp() {
        var spec = miniSpec
        if (!spec || spec.kind !== "web") return
        var r = miniFrame.interior
        frameAppWanted = true
        LS.adhoc.call("luna://com.webos.applicationManager", "/launch", JSON.stringify({
            id: frameAppId,
            params: { url: spec.url, title: spec.title || "", source: spec.source || "", ago: spec.ago || "",
                      mode: spec.mode || "auto", light: root.isLight,
                      rect: { x: r.x, y: r.y, w: r.width, h: r.height } }
        }))
    }
    function closeMini() {
        if (!miniFrame.open) return
        if (frameAppWanted) LS.adhoc.call("luna://com.webos.applicationManager", "/closeByAppId", JSON.stringify({ id: frameAppId }))
        frameAppWanted = false
        miniFrame.loading = false
        miniFrame.open = false
        miniSpec = null
        focusLayer = "widget"
    }
    Connections {
        target: miniFrame
        onLandedChanged: if (miniFrame.landed && miniFrame.open) root.launchFrameApp()
    }
    Service {
        // running-apps subscription: the overlay app appearing ends the spinner, its
        // disappearance (Back in the overlay) closes the frame
        id: runningSvc
        appId: LS.appId
        onResponse: {
            try {
                var msg = JSON.parse(payload)
                if (!msg.running) return
                var seen = false
                for (var i = 0; i < msg.running.length; i++) if (msg.running[i].id === root.frameAppId) seen = true
                if (seen && !root.frameAppRunning) { root.frameAppRunning = true; spinnerGrace.restart() }
                if (!seen && root.frameAppRunning) { root.frameAppRunning = false; if (root.frameAppWanted) { root.frameAppWanted = false; root.closeMini() } }
            } catch (e) {}
        }
        Component.onCompleted: call("luna://com.webos.applicationManager", "/running", JSON.stringify({ subscribe: true }))
    }
    Timer {
        // the app paints its own spinner once it is up; ours covers the launch latency
        id: spinnerGrace
        interval: 350
        onTriggered: miniFrame.loading = false
    }
    MiniFrame {
        id: miniFrame
        z: 4
        mat: root.mat
        isLight: root.isLight
        cardRadius: Lu.radius("card")
        backdrop: blurTex
        motionMs: root.motion.frameMs || 260
        layerMs: root.layerMs
        toRect: {
            var w = 1520, h = 860
            return Qt.rect(Math.round((1920 - w) / 2), Math.round(layout.safe.y + layout.statusBar.height + layout.statusBar.gap + (1080 - layout.safe.y - layout.statusBar.height - layout.statusBar.gap - layout.safe.y - h) / 2), w, h)
        }
    }

    // The Home stub's trial: shown only while the tool says one is running, so OK keeps it
    // (it reverts to LG's Home by itself otherwise).
    Rectangle {
        visible: root.trialOn || root.trialConfirming
        z: 50
        width: trialText.implicitWidth + 56; height: 56; radius: 28
        x: Math.round((parent.width - width) / 2); y: parent.height - height - 20
        color: root.isLight ? "#f2ffffff" : "#f21a1626"
        border.width: 1; border.color: root.isLight ? "#26000000" : "#33ffffff"
        Text {
            id: trialText
            x: 28; y: Math.round((56 - implicitHeight) / 2)
            color: root.isLight ? "#17131F" : "#F2EFF6"
            font.family: "Manrope"; font.weight: Font.DemiBold; font.pixelSize: 24; renderType: Text.NativeRendering
            text: root.trialConfirming ? "Keeping this Home..." : "Press OK to keep this Home. It goes back to LG's Home in " + root.trialLeft + " s."
        }
    }

    // ============================================================= resident surfaces
    // Settings, the kit gallery and the other panels that open over Home are built in the background
    // once Home has presented and kept (Layers.qml, Resident.qml), so opening one is a visibility flip
    // on a tree that is already laid out, never a build inside the frame the key waits for. Home decides
    // which one is open and gives it the keys; the layer decides nothing else.
    property bool settingsOpen: false
    readonly property bool settingsHelpOpen: layers.settingsHelpOpen
    readonly property var lunaCalls: LS.calls || []        // what the offline host's luna stand-in recorded; the set's bridge has none
    readonly property bool galleryOn: !!root.theme.kitGallery && root.hostActive
    function openSettings() {
        if (!hostActive) return
        statusBarItem.requestClosePopover()
        settingsOpen = true
    }
    function closeSettings() { settingsOpen = false; if (hostActive) focusLayer = "statusbar" }
    // ---- the layout editor (LayoutEdit.qml, kept built once the layers are, and drawn under them: the confirmation it asks "Save changes?"
    // with is a layer and stands above it). It is opened from Settings' Home widgets page (HomeLayout.edit) or by holding OK on a card.
    // While it is up it has the D-pad, OK and Back, Home draws the layout it is changing, and nothing is written until it says save.
    property bool editing: false
    property string editWanted: ""
    function editHome(id) {
        if (!hostActive || editing) return
        statusBarItem.requestClosePopover()
        settingsOpen = false
        closeAllLayers()
        editWanted = id || widgetFocusId
        editing = true
        focusLayer = "edit"
        if (editRes.item) editRes.item.start(editWanted)
    }
    function editEnd() {
        if (!editing) return
        var keep = editRes.item ? editRes.item.sel : ""
        editing = false
        if (hostActive) {
            if (widgetIndexById(keep) >= 0) widgetFocusId = keep
            focusLayer = dockEntry && dockEntry.id === keep ? "dock" : "widgets"
        }
    }
    function previewLayout(next) { root.layout = next }
    function editSave(next) { persistLayout(next); editEnd() }
    function editLeave(orig) { previewLayout(orig); editEnd() }
    // OK is decided on release, as everywhere on Home; the release of the hold that opened the editor is not an OK.
    function editKey(k, pressed, autoRepeat) {
        if (k === Qt.Key_Menu) { if (pressed && !autoRepeat && editRes.item) editRes.item.menu(); return true }
        var dir = directionFor(k)
        if (dir === "") return k >= Qt.Key_0 && k <= Qt.Key_9 || k === Qt.Key_Search || k === WebOS.Key_webOS_Voice || k === WebOS.Key_webOS_ScreenRemote
        if (dir === "ok") {
            if (pressed) { if (!autoRepeat) { okDown = true; okHeld = false }; return true }
            var armed = okDown && !okHeld
            okDown = false; okHeld = false
            if (armed && editRes.item) editRes.item.key("ok")
            return true
        }
        if (!pressed || (autoRepeat && dir === "back")) return true
        if (editRes.item) editRes.item.key(dir)
        return true
    }
    Resident {
        id: editRes
        name: "edit"
        source: "LayoutEdit.qml"
        anchors.fill: parent
        z: 59
        build: layers.built
        shown: root.editing && root.hostActive
        onItemChanged: if (item && root.editing) item.start(root.editWanted)
    }
    Connections {
        target: editRes.item
        ignoreUnknownSignals: true
        onPreview: root.previewLayout(layout)
        onSave: root.editSave(layout)
        onLeave: root.editLeave(layout)
        onAsk: root.showConfirm(spec)
    }
    Connections { target: HomeLayout; onEditRequested: root.editHome(id) }
    Binding { target: HomeLayout; property: "editing"; value: root.editing }
    // ---- layers: `layerOpen` has the keys. A surface opened over another (a menu over the library, the keyboard over the
    // library) pushes the one under it and closing pops it, so Back from the menu is the library again and Back from the
    // library is Home with the focus where it was.
    property var layerStack: []
    readonly property bool appsUp: layerOpen === "apps" || layerStack.indexOf("apps") >= 0
    function openLayer(id) {
        if (!hostActive) return
        if (!layers.find(id)) { console.warn("[LumaHome] no surface is called " + id); return }
        statusBarItem.requestClosePopover()
        if (layerOpen !== "" && layerOpen !== id) { var s = layerStack.slice(); s.push(layerOpen); layerStack = s }
        layerOpen = id
    }
    function closeLayer() {
        if (layerStack.length > 0) { var s = layerStack.slice(); layerOpen = s.pop(); layerStack = s; return }
        layerOpen = ""
        if (doorLayer !== "") { layerFile.write(""); doorLayer = "" }
        if (theme.kitGallery) { patchTheme({ kitGallery: false }); persistTheme({ kitGallery: false }) }
    }
    // A surface that ends by itself (the banner's timer, a picker that has acted) is dropped wherever it is: the top one is
    // closed, one under it is taken out of the stack, one that is not up is left alone.
    function closeLayerById(id) {
        if (layerOpen === id || (layerOpen === "" && doorLayer === id)) closeLayer()
        else if (layerStack.indexOf(id) >= 0) layerStack = layerStack.filter(function(x) { return x !== id })
    }
    function closeAllLayers() { layerStack = []; layerOpen = "" }
    // What the rest of Home asks of the small panels: a menu next to something, a question, the on-screen remote, the
    // launch splash, a spinner, the ring around a tile that is being held. Each is a call on a built item.
    property string menuKind: ""
    function showMenu(spec) { menuKind = spec.kind || ""; layers.openMenu(spec); openLayer("menu") }
    function showConfirm(spec) { layers.askConfirm(spec); openLayer("confirm") }
    function showRemote() { openLayer("remote") }
    function showSplash(name, icon) { layers.launchSplash(name, icon) }
    function showSpinner(text) { layers.spinner(text) }
    function showHold(rect, name, fraction) { layers.holdRing(rect, name, fraction) }
    Layers {
        id: layers
        z: 60
        home: root
        splashDelayMs: root.smoothLaunch ? root.splashWaitMs : 0
        onSplashShown: root.noteSplashOrigin(iconRect, radius)
        below: root.layerStack
        open: !root.hostActive ? "" : root.settingsOpen ? "settings" : root.layerOpen !== "" ? root.layerOpen : root.doorLayer !== "" ? root.doorLayer : root.galleryOn ? "gallery" : ""
        onCloseRequested: {
            if (id === "settings") root.closeSettings()
            else { if (id === "voice" && root.voiceBusy) root.voiceAbort(); root.closeLayerById(id) }
        }
        onOpenSettingsPage: { root.openSettings(); layers.settingsGoTo(id) }
        onAppLaunched: root.launchApp(app)
        onAppMenu: root.appMenu(app, "library", anchor, null)
        onSearchRequested: root.openSearch()
        onSearchPicked: root.searchPicked(kind, id, text)
        onEntered: root.entered(text)
        onMenuPicked: root.menuPicked(id, index)
        onConfirmAnswered: root.confirmAnswered(id)
        onRemoteKey: root.sendRemoteKey(name)
        onBannerPicked: root.bannerPicked(id)
        onAccessAssigned: root.accessAssigned(digit, appId)
        onAccessMore: root.accessMore(digit)
        onRecentsPicked: root.recentsPicked(appId)
    }
    // the gear's tap and the Input key: the quick panel and the input picker, from the key filter's second entry (HomePanels.qml)
    HomePanels { home: root; layers: layers; aiPrompt: aiPrompt }

    // ============================================================= what the panels ask Home to do
    // A panel never acts: it says what was chosen and Home does it, with the calls the rest of Home makes (luna through
    // LS, the theme through patchTheme and persistTheme, Settings through goTo). Each panel is a surface that is already built.
    readonly property string appMgr: "luna://com.webos.applicationManager"

    // ---- the library, search, an app's options
    function openApps() { layers.appsReset(); openLayer("apps") }
    function launchApp(app) {
        if (!app) return
        closeAllLayers()
        var d = dockLoader.item
        noteLaunchOrigin(app.id, d ? d.tileOf(app.id) : null)
        layers.launchSplash(app.title || app.id, app.iconUrl || "")
        LS.adhoc.call(appMgr, "/launch", JSON.stringify({ id: app.id, params: app.params || {} }))
    }
    function openSearch(text) {
        layers.searchOpen({ sources: { apps: Apps.shown, pages: layers.settingsPages() }, text: text || "", voice: voiceReady })
        openLayer("search")
    }
    function searchPicked(kind, id, text) {
        if (kind === "app") launchApp(Apps.byId(id))
        else if (kind === "page") { closeAllLayers(); openSettings(); layers.settingsGoTo(id) }
        else if (kind === "web") webSearch(text)
    }
    // The search engine is the owner's to name (theme `search.url`, the query address); the default keeps the question off Google.
    function webSearch(text) {
        var base = (theme.search && theme.search.url) || "https://duckduckgo.com/?q="
        closeAllLayers()
        LS.adhoc.call(appMgr, "/launch", JSON.stringify({ id: "com.webos.app.browser", params: { target: Match.webUrl(base, text) } }))
    }

    property var appTarget: null            // what the open menu or dialog is about: { app, source: "dock" | "library" }
    function menuKey() {
        if (layers.open === "apps" || layers.open === "settings") { layers.hold(); return }
        if (layers.open === "quick") { closeLayer(); return }
        if (layers.open !== "") return
        var d = dockLoader.item
        if (focusLayer === "dock" && d && !d.moving) { openTileMenu(); return }
        if (theme.quickPanel && theme.quickPanel.own) openLayer("quick"); else openApps()
    }
    function openTileMenu() {
        var d = dockLoader.item
        var t = d ? d.focusedTile() : null
        if (!t || t.unmovable || t.id === d.appsTileId) return            // the Apps tile has no options of its own
        var app = Apps.byId(t.id) || { id: t.id, title: t.title, iconUrl: t.iconUrl, params: t.params, launchPointId: t.launchPointId, onDock: true, dockIndex: t.index, removable: false, hidden: false }
        appMenu(app, "dock", t.rect, t)
    }
    // The options of one app, beside the tile or icon they belong to, which stays lit above the scrim. On the dock the
    // choices are the dock's (Move, Hide from the dock); in the library they are the library's (Move on the dock, Hide, Show again).
    function appMenu(app, source, anchor, tile) {
        appTarget = { app: app, source: source }
        var dock = source === "dock"
        var items = [{ id: "open", icon: "play", label: "Open" }]
        if (app.onDock && app.id !== Apps.storeId) items.push(dock ? { id: "move", icon: "layout-grid", label: "Move", hint: "then \u2190 \u2192" } : { id: "move", icon: "layout-grid", label: "Move on the dock" })
        if (dock) items.push({ id: "unpin", icon: "eye-off", label: "Hide from the dock" })
        else items.push(app.hidden ? { id: "show", icon: "eye", label: "Show again" } : { id: "hide", icon: "eye-off", label: "Hide" })
        items.push({ id: "rename", icon: "keyboard", label: "Rename" })
        if (app.removable) items.push({ id: "uninstall", icon: "trash-2", label: "Uninstall", danger: true })
        var raised = tile ? { x: anchor.x, y: anchor.y, width: anchor.width, height: anchor.height, iconUrl: tile.iconUrl, plate: tile.plate, inset: tile.floating ? ((theme.tiles && theme.tiles.inset) || 19) : 0, radius: Lu.radius("tile") }
                          : { x: anchor.x + Math.round((anchor.width - 152) / 2), y: anchor.y + 14, width: 152, height: 152, iconUrl: anchor.iconUrl, radius: 30 }
        showMenu({ kind: "app", title: app.title, sub: app.onDock ? "On the dock \u00b7 tile " + (app.dockIndex + 1) : app.hidden ? "Hidden" : "Not on the dock",
                   iconUrl: app.iconUrl, anchor: anchor, raised: raised, width: 520, items: items })
    }
    function menuPicked(id, index) {
        var t = appTarget
        if (menuKind !== "app" || !t) { closeLayer(); return }
        var app = t.app
        closeLayer()                                                    // the menu: back to the library, or to Home
        if (id === "open") launchApp(app)
        else if (id === "move") { closeAllLayers(); moveOnDock(app.id) }
        else if (id === "unpin") hideFromDock(app)
        else if (id === "hide") setAppHidden(app.id, true)
        else if (id === "show") setAppHidden(app.id, false)
        else if (id === "rename") renameApp(app)
        else if (id === "uninstall") askUninstall(app)
    }
    function moveOnDock(id) { var d = dockLoader.item; if (!d) return; focusLayer = "dock"; d.beginMoveOf(id) }
    // Hiding from the dock is LG's own list (homelaunchpoints); hiding from the library is the owner's (theme.apps.hidden).
    function hideFromDock(app) { LS.adhoc.call("luna://com.webos.service.homelaunchpoints", "/hideLaunchPoint", JSON.stringify({ launchPointId: app.launchPointId })) }
    function setAppHidden(id, hide) {
        var next = ((theme.apps && theme.apps.hidden) || []).filter(function(x) { return x !== id })
        if (hide) next.push(id)
        var patch = { apps: { hidden: next } }
        patchTheme(patch); persistTheme(patch)
    }
    // A new name is the owner's (theme.apps.names, one name per app id); an empty one brings back the name LG gave.
    property string renameId: ""
    function renameApp(app) {
        renameId = app.id
        layers.searchOpen({ mode: "entry", prompt: "Rename " + app.title, text: app.title })
        openLayer("search")
    }
    function entered(text) {
        if (renameId === "") return
        var id = renameId
        renameId = ""
        var names = JSON.parse(JSON.stringify((theme.apps && theme.apps.names) || {}))
        var name = String(text).trim()
        if (name === "") delete names[id]; else names[id] = name
        var patch = { apps: { names: names } }
        patchTheme(patch); persistTheme(patch)
    }
    function askUninstall(app) {
        appTarget = { app: app, source: "confirm" }
        showConfirm({ iconUrl: app.iconUrl, title: "Uninstall " + app.title + "?",
                      body: "The app and its data on this TV are removed. It can be installed again from where it came from.",
                      answers: [{ id: "keep", label: "Keep it" }, { id: "uninstall", label: "Uninstall", danger: true }], focus: 0 })
    }
    function confirmAnswered(id) {
        if (id.indexOf("edit-") === 0) { if (editRes.item) editRes.item.answer(id); return }
        var t = appTarget
        if (id === "uninstall" && t && t.source === "confirm" && t.app.removable)
            LS.adhoc.call("luna://com.webos.appInstallService", "/remove", JSON.stringify({ id: t.app.id, subscribe: false }))
        appTarget = null
    }

    // ---- the on-screen remote: its keys are the TV's own. LG's own on-screen remote presses them through the micom service
    // (`sendKeycode`, the remote's decimal keycode), which is also how the third-party launchers' keypads do it
    // (docs/research/home-launcher-replacements.md). The numbers are LG's published IR command numbers for its remotes
    // [INFERENCE: not yet run on this set]; a key the table lacks is not sent.
    readonly property var remoteCodes: ({ "0": 16, "1": 17, "2": 18, "3": 19, "4": 20, "5": 21, "6": 22, "7": 23, "8": 24, "9": 25,
                                          red: 114, green: 113, yellow: 99, blue: 97, play: 176, pause: 186, stop: 177, rewind: 143, forward: 142, info: 170 })
    function remoteKeyPressed() { if (layers.open === "remote") closeLayer(); else if (layers.open === "") showRemote() }
    function sendRemoteKey(name) {
        if (name === "back") { closeLayer(); return }                  // Back on the panel is leaving it
        var code = remoteCodes[name]
        if (code === undefined) return
        LS.adhoc.call("luna://com.webos.service.micomservice", "/sendKeycode", JSON.stringify({ keycode: code }))
    }

    // ---- the voice bar is Home's to draw while the helper listens (see "voice search" below). Voice only ever fills Search: the
    // bar has no options to pick and nothing here launches an app or opens a page from what was said.
    function voiceStart() { layers.voiceListen(); openLayer("voice") }
    // The microphone is open for something else (an app): the indicator says so and takes no key.
    function showMic(on) { layers.voiceMic(on) }
    // ---- voice search: hold the mic key and speak; the words are typed into Search and nothing acts on them. The helper listens
    // (`POST /v1/voice/listen` and `/stop`) and has Home Assistant's Assist pipeline turn the speech into text; what it knows is
    // its `voice` topic: ready, listening, thinking, then heard (the words) or error (why not).
    readonly property bool voiceReady: !!(Helper.topics.voice && Helper.topics.voice.ready === true)
    property bool voiceBusy: false              // from the key going down to the end of the request
    property bool voiceReleased: false
    property int voiceSession: 0                // the helper's number for the request, 0 until it has answered
    property int voiceRequest: 0                // which key press this is, so a late answer to an old one is told apart
    function voiceKeyDown() {
        if (voiceBusy) return
        var mine = ++voiceRequest
        voiceBusy = true; voiceReleased = false; voiceSession = 0
        voiceClear.stop()
        voiceStart()
        Helper.post("/v1/voice/listen", {}, function(status, doc) {
            var ok = status === 200 && doc && doc.ok
            // Back (or the app launched) came first: whatever started is ended again.
            if (mine !== voiceRequest || !voiceBusy) { if (ok) Helper.post("/v1/voice/cancel", {}); return }
            if (ok) {
                voiceSession = doc.session
                if (voiceReleased) Helper.post("/v1/voice/stop", {})
                voiceTopicChanged()
            } else voiceFail(voiceWhy(doc))
        })
    }
    // Why the helper would not listen. Qt's XMLHttpRequest reports no status and no body for a 409, so when there is no document
    // the helper's own `voice` topic (it carries a `reason` whenever it is not ready) says it.
    function voiceWhy(doc) {
        if (doc && doc.error && doc.error.message) return doc.error.message
        if (!Helper.up) return "The TV's helper is not running"
        var v = Helper.topics.voice, r = v ? v.reason : ""
        return r === "voice_off" ? "Voice search is switched off" : r === "voice_unlinked" ? "Voice search is not linked to Home Assistant"
             : r === "no_microphone" ? "No microphone is chosen for voice search" : "Voice search is busy or not available"
    }
    function voiceKeyUp() {
        if (!voiceBusy) return
        voiceReleased = true
        layers.voiceWorking()
        if (voiceSession !== 0) Helper.post("/v1/voice/stop", {})
    }
    // Back, or anything that takes Home away: the microphone is closed and nothing is reported.
    function voiceAbort() {
        voiceBusy = false; voiceSession = 0
        layers.voiceMic(false)
        Helper.post("/v1/voice/cancel", {})
    }
    function voiceTopicChanged() {
        var v = Helper.topics.voice
        if (!voiceBusy || voiceSession === 0 || !v || v.session !== voiceSession || v.state === "listening") return
        layers.voiceMic(false)                  // the helper says the microphone is closed
        if (v.state === "thinking") layers.voiceWorking()
        else if (v.state === "heard") voiceFill(v.text)
        else if (v.state === "error") voiceFail(v.error ? v.error.message : "Voice search failed")
        else voiceEnd()                         // idle: the helper ended it (switched off, or stopping)
    }
    Connections { target: Helper; onUpdated: if (topic === "voice") root.voiceTopicChanged() }
    function voiceFill(words) {
        voiceBusy = false
        layers.voiceIdle()
        closeLayerById("voice")
        openSearch(words)
    }
    function voiceFail(words) {
        voiceBusy = false
        layers.voiceFailed(words)
        voiceClear.restart()
    }
    function voiceEnd() { voiceBusy = false; layers.voiceIdle(); closeLayerById("voice") }
    // A reason stays on the bar long enough to read, then the bar goes (unless a new request has taken it).
    Timer { id: voiceClear; interval: 2600; onTriggered: { if (!root.voiceBusy && layers.up("voice")) root.voiceEnd() } }
    // ---- banners: what the TV says about what is on it. Only one with buttons takes the keys; the others show and hide.
    function showBanner(spec) {
        layers.bannerShow(spec)
        if (spec.kind === "nosignal" || spec.buttons) openLayer("banner")
    }
    function hideBanner() { layers.bannerHide() }
    function bannerPicked(id) {
        closeAllLayers()
        if (id === "inputs") openLayer("picker")
        else if (id === "picture") { openSettings(); layers.settingsGoTo("picture") }
        else if (id === "details") { openSettings(); layers.settingsGoTo("inputs") }
        else if (id === "home") LS.adhoc.call(appMgr, "/launchDefaultApp", JSON.stringify({ category: "home" }))
    }

    // ============================================================= AI prompt
    // The question AI access asks on the TV (AiPrompt.qml draws the helper's `ai.prompt` topic and has
    // no way to answer it; see there). Above Settings and the gallery, below nothing of ours.
    AiPrompt { id: aiPrompt; z: 90 }

    // ============================================================= startup warm-up
    // Draws the text styles, images and shader components the surfaces use once, hidden, after Home
    // has presented (Warmup.qml), so the first open of any of them rasterises and compiles nothing
    // in the frame the user is waiting for. theme.json "warmup" turns it off.
    Warmup {
        id: warmup
        tree: root
        enabled: !root.theme.warmup || root.theme.warmup.enabled !== false
        active: root.hostActive && layers.built
        startMs: (root.theme.warmup && root.theme.warmup.startMs) || 1500
        stepMs: (root.theme.warmup && root.theme.warmup.stepMs) || 50
    }
    // The icon shader is one program for every Lucide icon on any surface; drawn once here, none compiles it on show.
    Component { id: iconProbe; LuIcon { name: "check"; size: 8 } }
    QtObject { Component.onCompleted: Warm.request({ components: [iconProbe] }) }

    // ====================================================== frame pacing probe / overlay
    // onFrameSwapped fires once per presented frame of the compositor's own
    // window, so this counts everything the set draws - Home, and anything
    // composited over it, including the in-process system UI panels. An
    // average hides the thing that is actually visible: one 33ms frame in an
    // otherwise perfect second still reads as 59 fps. So the worst interval
    // and the number of intervals past a frame and a half are logged too.
    QtObject {
        id: fpsProbe
        property int frames: 0
        property int fps: 0
        property real last: 0
        property real worst: 0      // ms, longest gap between presents this second
        property int late: 0        // intervals over 25ms: a missed 60Hz deadline
        property real worstShown: 0
        property int lateShown: 0
    }
    Connections {
        target: root.Window.window
        onFrameSwapped: {
            fpsProbe.frames++
            var t = Date.now()
            if (fpsProbe.last > 0) {
                var dt = t - fpsProbe.last
                if (dt > fpsProbe.worst) fpsProbe.worst = dt
                if (dt > 25) fpsProbe.late++
                // One line per long gap, so a transition can be judged frame by
                // frame instead of through a one-second average that hides a
                // single dropped frame. Only while the probe is on, and only
                // for gaps that are visible as judder.
                if (root.fpsEnabled && dt > 20 && dt < 5000)
                    console.info("[FPSJANK] " + Math.round(dt))
            }
            fpsProbe.last = t
        }
    }
    Timer {
        interval: 1000; running: root.fpsEnabled; repeat: true
        onTriggered: {
            fpsProbe.fps = fpsProbe.frames
            fpsProbe.worstShown = fpsProbe.worst
            fpsProbe.lateShown = fpsProbe.late
            console.info("[FPSLOG] luma " + fpsProbe.fps + " fps worst " + Math.round(fpsProbe.worst) + "ms late " + fpsProbe.late)
            fpsProbe.frames = 0; fpsProbe.worst = 0; fpsProbe.late = 0
        }
    }
    Rectangle {
        visible: root.fpsEnabled
        x: parent.width - width - 24; y: 24; z: 100
        width: fpsText.implicitWidth + 28; height: 34; radius: 8; color: "#a0100e1a"
        Text {
            id: fpsText
            x: 14; y: Math.round((34 - implicitHeight) / 2)
            color: fpsProbe.lateShown > 0 ? "#eb6f92" : "#9ccfd8"
            font.family: "Manrope"; renderType: Text.NativeRendering; font.pixelSize: 18
            text: "luma " + fpsProbe.fps + " fps  " + Math.round(fpsProbe.worstShown) + "ms"
        }
    }
}
