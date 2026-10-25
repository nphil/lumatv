import QtQuick 2.4
import QtGraphicalEffects 1.0
import QtQuick.Window 2.2
import WebOSCompositorBase 1.0
import WebOSServices 1.0
import WebOS.Global 1.0
import "LumaBus.js" as Bus

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

    readonly property string themeDirUrl: "file://{{dataDir}}/theme/"
    readonly property string fontsDirUrl: "file://{{dataDir}}/fonts/"
    readonly property string moduleDirUrl: "file://{{dataDir}}/qml/WebOSCompositor/{{qmlModule}}/"

    property var theme: defaultTheme()
    property var layout: defaultLayout()
    property bool isLight: false
    property var mat: theme.dark
    property bool fpsEnabled: false
    property string lastThemeText: ""
    property string lastLayoutText: ""

    // ---- motion tokens ----
    function easingFromName(n) {
        if (!n) return Easing.OutCubic
        var v = Easing[n]
        return (v !== undefined) ? v : Easing.OutCubic
    }
    property var motion: theme.motion || {}
    property bool motionReduced: !!motion.reduced
    property int focusMs: motion.focusMs || 150
    property int focusEasing: easingFromName(motion.focusEasing)
    property int cardMs: motion.cardMs || 180
    property int cardEasing: easingFromName(motion.cardEasing)
    property int scrollMs: motion.scrollMs || 300
    property int scrollEasing: easingFromName(motion.scrollEasing)
    property int labelMs: motion.labelMs || 120
    property int layerMs: motion.layerMs || 220

    // ============================================================= defaults
    function defaultTheme() {
        return {
            version: 1, name: "{{displayName}} default", wallpaper: "wall_1080.png",
            material: "auto", materialThreshold: 0.55,
            dark: { ink: "#F2EFF6", ink2: "#A8F2EFF6", ink3: "#6BF2EFF6", glass: "#66120E1A", glassEdge: "#21FFFFFF",
                    glassFocus: "#8F2C263A", glassFocusEdge: "#33FFFFFF", scrim: [0.10, 0.18, 0.46] },
            light: { ink: "#17131F", ink2: "#A817131F", ink3: "#6B17131F", glass: "#85FFFFFF", glassEdge: "#D9FFFFFF",
                     glassFocus: "#B8FFFFFF", glassFocusEdge: "#E6FFFFFF", scrim: [0.06, 0.10, -0.10] },
            accent: "tile", radius: 26, tileRadius: 22, blurRadius: 30, saturation: 0.25,
            type: { clock: 92, temp: 84 },
            focus: { tileScale: 1.12, cardScale: 1.03, cardLift: 4, tileGlow: 0, ambient: 0.3 },
            motion: { focusMs: 150, focusEasing: "OutCubic", cardMs: 180, cardEasing: "OutCubic",
                      scrollMs: 300, scrollEasing: "OutCubic", labelMs: 120, layerMs: 220, reduced: false },
            tiles: { inset: 19, fullCanvasEdgeAlpha: 0.9, icons: {} },
            wallpaperMotion: { enabled: true, amplitude: 4, speed: 1, depth: 0.25, scale: 1, detail: 0.35, fps: 30,
                               vignette: 0, specular: 0, bloom: 0, saturation: 1, contrast: 1 },
            clock: { style: "digital", twelveHour: true, greeting: true, secondHand: "#ff3b5c" },
            weather: { zip: "30311", units: "fahrenheit", days: 5 },
            news: { slideMs: 10000, refreshMs: 900000 }
        }
    }
    function defaultLayout() {
        return {
            version: 1, safe: { x: 64, y: 54 }, grid: { cols: 12, gutter: 20, rows: [222, 238, 392] },
            statusBar: { height: 60, gap: 20,
                left: [ { type: "profile" } ],
                right: [ { type: "notifications" }, { type: "settings" }, { type: "search" } ] },
            widgets: [
                { id: "clock", type: "clock", col: 0, row: 0, span: 4, rows: 1 },
                { id: "home", type: "homeassistant", col: 0, row: 1, span: 4, rows: 1, title: "At home" },
                { id: "news", type: "news", col: 4, row: 0, span: 5, rows: 2 },
                { id: "weather", type: "weather", col: 9, row: 0, span: 3, rows: 2 },
                { id: "dock", type: "dock", col: 0, row: 2, span: 12, rows: 1, tile: 120, gap: 16, padY: 40, fade: 110 }
            ]
        }
    }

    function resolvePath(p) {
        if (!p) return ""
        if (/^[a-zA-Z][a-zA-Z0-9+.-]*:/.test(p)) return p
        if (p.charAt(0) === "/") return "file://" + p
        return themeDirUrl + p
    }

    // ============================================================= grid geometry
    // grid.rows is either a count (uniform rows filling the area) or an array of pixel
    // heights (the reference layout pins the mock's 222/238/392).
    function areaRect() {
        var x = layout.safe.x, y = layout.safe.y + layout.statusBar.height + layout.statusBar.gap
        return Qt.rect(x, y, 1920 - 2 * layout.safe.x, 1080 - 2 * layout.safe.y - layout.statusBar.height - layout.statusBar.gap)
    }
    function rowHeights() {
        var a = areaRect(), g = layout.grid
        if (g.rows && g.rows.length !== undefined) return g.rows
        var n = Math.max(1, g.rows || 8), h = (a.height - (n - 1) * g.gutter) / n, out = []
        for (var i = 0; i < n; i++) out.push(h)
        return out
    }
    function rowCount() { return rowHeights().length }
    function cellRect(entry) {
        var a = areaRect(), g = layout.grid, rows = rowHeights()
        var cellW = (a.width - (g.cols - 1) * g.gutter) / g.cols
        var y = a.y, h = 0
        for (var r = 0; r < entry.row && r < rows.length; r++) y += rows[r] + g.gutter
        for (var k = entry.row; k < entry.row + entry.rows && k < rows.length; k++) h += rows[k] + (k > entry.row ? g.gutter : 0)
        return Qt.rect(Math.round(a.x + entry.col * (cellW + g.gutter)), Math.round(y),
                       Math.round(entry.span * cellW + (entry.span - 1) * g.gutter), Math.round(h))
    }
    property var nonDockWidgets: (layout.widgets || []).filter(function(e) { return e.type !== "dock" })
    property var dockEntry: { var l = (layout.widgets || []).filter(function(e) { return e.type === "dock" }); return l.length ? l[0] : null }
    function widgetIndexById(id) {
        for (var i = 0; i < nonDockWidgets.length; i++) if (nonDockWidgets[i].id === id) return i
        return -1
    }
    function isTopRow(e) { return e.row === 0 }
    function isBottomRow(e) { return dockEntry ? (e.row + e.rows) === dockEntry.row : (e.row + e.rows) === rowCount() }

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
    function mergeDefaults(base, over) {
        if (!over || typeof over !== "object") return base
        var out = {}
        for (var k in base) out[k] = base[k]
        for (var k2 in over) out[k2] = over[k2]
        return out
    }
    function applyTheme(json, text) {
        root.theme = json ? mergeDefaults(defaultTheme(), json) : defaultTheme()
        if (!json) console.warn("[LumaHome] theme.json missing/invalid, using built-in defaults")
        root.lastThemeText = text || ""
        computeMaterial()
    }
    function applyLayout(json, text) {
        root.layout = json ? mergeDefaults(defaultLayout(), json) : defaultLayout()
        if (!json) console.warn("[LumaHome] layout.json missing/invalid, using built-in defaults")
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
            console.info("[LumaHome] theme.json write status " + x.status)
            if (x.status === 200 || x.status === 0) root.lastThemeText = text   // the poll sees its own write
            else console.warn("[LumaHome] theme.json write failed: " + x.status)
        }
        try { x.open("PUT", root.themeDirUrl + "theme.json"); x.send(text) }
        catch (e) { console.warn("[LumaHome] theme.json write failed: " + e) }
    }
    Timer {
        // live theme preview for the companion app: content compare every 2 s, no restart.
        // Only while Home is on screen; showing Home again catches up at once.
        interval: 2000; running: root.hostActive; repeat: true; triggeredOnStart: true
        onTriggered: {
            loadJson(root.themeDirUrl + "theme.json", function(json, text) {
                if ((text || "") === root.lastThemeText) return
                root.applyTheme(json, text)
            })
            loadJson(root.themeDirUrl + "layout.json", function(json, text) { if ((text || "") !== root.lastLayoutText) root.applyLayout(json, text) })
        }
    }

    // ============================================================= fonts
    FontLoader { source: root.fontsDirUrl + "Manrope-Medium.ttf" }
    FontLoader { source: root.fontsDirUrl + "Manrope-SemiBold.ttf" }
    FontLoader { source: root.fontsDirUrl + "Manrope-Bold.ttf" }
    Repeater { model: theme.fonts || []; delegate: FontLoader { source: root.resolvePath(modelData) } }

    // ============================================================= material (light/dark)
    function computeMaterial() {
        if (theme.material === "light") { root.isLight = true; root.mat = theme.light; return }
        if (theme.material === "dark") { root.isLight = false; root.mat = theme.dark; return }
        if (wallpaperImg.status === Image.Ready) lumaSampler.requestPaint()
    }
    Canvas {
        // "auto": mean luminance of the wallpaper against theme.materialThreshold
        id: lumaSampler
        width: 16; height: 9; visible: false
        onPaint: {
            var ctx = getContext("2d")
            ctx.drawImage(wallpaperImg, 0, 0, 16, 9)
            var d = ctx.getImageData(0, 0, 16, 9).data, sum = 0
            for (var i = 0; i < d.length; i += 4) sum += 0.299 * d[i] + 0.587 * d[i + 1] + 0.114 * d[i + 2]
            root.isLight = (sum / (16 * 9) / 255) > root.theme.materialThreshold
            root.mat = root.isLight ? root.theme.light : root.theme.dark
        }
    }
    function scrimColor(v) {
        if (!root.isLight) return Qt.rgba(0, 0, 0, v)
        return v >= 0 ? Qt.rgba(1, 1, 1, v) : Qt.rgba(0, 0, 0, -v)
    }

    // ============================================================= wallpaper + one cached blur
    property string wallpaperFit: (layout.background && layout.background.wallpaperFit) || "1:1"
    Rectangle { anchors.fill: parent; z: -3; color: (layout.background && layout.background.color) || "#0b0910" }
    Rectangle {
        // theme.<mat>.scrim: 3 stops at 0 / 55 / 100 %. Drawn by the live wallpaper's shader
        // while that is on, so the screen carries one full-screen quad, not two.
        anchors.fill: parent
        z: -1
        visible: !liveWall.visible
        gradient: Gradient {
            GradientStop { position: 0.0; color: root.scrimColor((root.mat.scrim || [0, 0, 0])[0]) }
            GradientStop { position: 0.55; color: root.scrimColor((root.mat.scrim || [0, 0, 0])[1]) }
            GradientStop { position: 1.0; color: root.scrimColor((root.mat.scrim || [0, 0, 0])[2]) }
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
        onStatusChanged: if (status === Image.Ready) root.computeMaterial()
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
        visible: !liveWall.visible && wallpaperImg.status === Image.Ready   // the still path draws the graded texture
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
    ShaderEffect {
        id: liveWall
        anchors.fill: parent
        z: -2
        visible: root.wallMotionOn && wallpaperImg.status === Image.Ready
        property variant src: gradedTex
        property real t: 0
        property real amp: (root.wallMotion.amplitude || 4) / 1920
        property real speed: root.wallMotion.speed || 1
        property real depth: root.wallMotion.depth === undefined ? 0.25 : root.wallMotion.depth
        property real waveScale: root.wallMotion.scale || 1          // wave frequency multiplier
        property real detail: root.wallMotion.detail === undefined ? 0.35 : root.wallMotion.detail   // finer second layer, 0..1
        property color s0: root.scrimColor((root.mat.scrim || [0, 0, 0])[0])
        property color s1: root.scrimColor((root.mat.scrim || [0, 0, 0])[1])
        property color s2: root.scrimColor((root.mat.scrim || [0, 0, 0])[2])
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
        Timer {
            interval: Math.round(1000 / (root.wallMotion.fps || 30))
            running: liveWall.visible && root.hostActive
            repeat: true
            onTriggered: liveWall.t += interval / 1000 * liveWall.speed
        }
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
        sourceItem: satPass
        width: 960; height: 540
        textureSize: Qt.size(960, 540)
        visible: false
        smooth: true
    }

    // ============================================================= focus model
    // "dock" | "widgets" | "statusbar" | "popover"
    property string focusLayer: "dock"
    property int dockRow: 0
    property int dockCol: 0
    property string widgetFocusId: nonDockWidgets.length ? nonDockWidgets[0].id : ""
    property int statusFocusIndex: 0
    property var statusItems: (layout.statusBar.left || []).concat(layout.statusBar.right || [])
    property bool popoverOpen: focusLayer === "popover"

    onHostActiveChanged: {
        if (hostActive) { focusLayer = "dock"; statusBarItem.requestClosePopover() }
        else if (miniFrame.open) { miniFrame.open = false; miniFrame.loading = false; miniSpec = null; frameAppWanted = false }
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
    // OK is decided on release: a press held past holdMs (no release yet) picks a dock tile
    // up for rearranging; a shorter press activates. Auto-repeat presses are ignored.
    property int holdMs: 600
    property bool okDown: false
    property bool okHeld: false
    Timer {
        id: holdTimer
        interval: root.holdMs
        onTriggered: {
            if (!root.okDown) return
            root.okHeld = true
            var d = dockLoader.item
            if (root.focusLayer === "dock" && d && !d.moving) d.beginMove()
        }
    }
    function key(k, pressed, autoRepeat, deviceId) {
        var dir = directionFor(k)
        if (!dir || !hostActive) return false
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
            if (dir === "back") return false
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
            if (dir === "back") return false
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
    Component.onCompleted: { Bus.setHandler(root.key); loadAll() }
    Component.onDestruction: Bus.setHandler(null)

    // Pointer: anything not over a card/tile lands here so the stock Home underneath never
    // sees the Magic Remote.
    MouseArea { anchors.fill: parent; hoverEnabled: true; z: 0 }


    // ============================================================= chrome
    StatusBar {
        id: statusBarItem
        z: 3
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
        backdrop: blurTex
        cardMs: root.cardMs
        cardEasing: root.cardEasing
        layerMs: root.layerMs
        onRequestOpenPopover: { root.statusFocusIndex = index; root.focusLayer = "popover" }
        onThemePatch: root.patchTheme(patch)
        onThemePersist: root.persistTheme(patch)
        onHoverFocus: { root.statusFocusIndex = index; if (root.focusLayer !== "popover") root.focusLayer = "statusbar" }
    }

    // Popover scrim: one full-screen quad between the content and the status bar. A modest
    // dim (~30%) marks the popover as the top layer while the page stays readable. The
    // wallpaper settings popover instead clears the page: widgets and dock fade out so the
    // wallpaper is seen bare while it is adjusted.
    property bool previewMode: root.popoverOpen && statusBarItem.popoverType === "settings"
    Rectangle {
        anchors.fill: parent
        z: miniFrame.open ? 3.5 : 2
        color: root.isLight ? "#ffffff" : "#000000"
        opacity: (root.popoverOpen && !root.previewMode) || miniFrame.open ? statusBarItem.popoverScrim : 0
        visible: opacity > 0.005
        Behavior on opacity { NumberAnimation { duration: root.layerMs; easing.type: Easing.OutCubic } }
        MouseArea { anchors.fill: parent; hoverEnabled: true; onClicked: { if (miniFrame.open) root.closeMini(); else { statusBarItem.requestClosePopover(); root.focusLayer = "statusbar" } } }
    }

    Item {
        id: widgetLayer
        anchors.fill: parent
        z: 1
        opacity: root.previewMode ? 0 : 1
        visible: opacity > 0.005
        Behavior on opacity { NumberAnimation { duration: root.layerMs; easing.type: Easing.OutCubic } }
        Repeater {
            id: widgetRepeater
            model: root.nonDockWidgets
            delegate: Widget {
                gridX: root.cellRect(modelData).x
                gridY: root.cellRect(modelData).y
                width: root.cellRect(modelData).width
                height: root.cellRect(modelData).height
                theme: root.theme
                mat: root.mat
                isLight: root.isLight
                layoutEntry: modelData
                themeDirUrl: root.themeDirUrl
                moduleDirUrl: root.moduleDirUrl
                focused: (root.focusLayer === "widgets" || root.focusLayer === "widget") && root.widgetFocusId === modelData.id
                dimmed: (root.focusLayer === "widgets" || root.focusLayer === "widget") && root.widgetFocusId !== modelData.id
                onOpenMini: root.openMini(spec, root.cellRect(modelData))
                backdrop: blurTex
                cardMs: root.cardMs
                cardEasing: root.cardEasing
                motionReduced: root.motionReduced
                live: root.hostActive
                onHoverFocus: { if (root.focusLayer === "widget" && root.widgetFocusId !== modelData.id) { var o = widgetRepeater.itemAt(root.widgetIndexById(root.widgetFocusId)); if (o) o.inner = false } root.widgetFocusId = modelData.id; if (root.focusLayer !== "widget" || root.widgetFocusId !== modelData.id) root.focusLayer = "widgets" }
            }
        }
    }

    Loader {
        id: dockLoader
        z: 1
        opacity: root.previewMode ? 0 : 1
        visible: opacity > 0.005
        Behavior on opacity { NumberAnimation { duration: root.layerMs; easing.type: Easing.OutCubic } }
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
            backdrop: blurTex
            scrollMs: root.scrollMs
            scrollEasing: root.scrollEasing
            focusMs: root.focusMs
            focusEasing: root.focusEasing
            labelMs: root.labelMs
            motionReduced: root.motionReduced
            onRowChanged: root.dockRow = row
            onColChanged: root.dockCol = col
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
        cardRadius: root.theme.radius || 26
        backdrop: blurTex
        motionMs: root.motion.frameMs || 260
        layerMs: root.layerMs
        toRect: {
            var w = 1520, h = 860
            return Qt.rect(Math.round((1920 - w) / 2), Math.round(layout.safe.y + layout.statusBar.height + layout.statusBar.gap + (1080 - layout.safe.y - layout.statusBar.height - layout.statusBar.gap - layout.safe.y - h) / 2), w, h)
        }
    }

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
