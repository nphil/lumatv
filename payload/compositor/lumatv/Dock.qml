import QtQuick 2.4
import WebOSCompositorBase 1.0
import WebOSServices 1.0

// The app dock: its own focus layer with a row-scrolling tile grid, an accent ambient glow
// behind the focused tile and a neutral bottom fade. Geometry follows the mock: tiles 120,
// gap 16, grid inset 56/40, fade 110; the column count is whatever fits the card width.
Item {
    id: dock

    property var theme: ({})
    property var mat: ({ ink: "#F2EFF6", glass: "#66120E1A", glassEdge: "#21FFFFFF" })
    property bool isLight: false
    property var dockEntry: ({})
    property bool focusedLayer: false
    property bool dimmed: false
    property real originX: 0
    property real originY: 0
    property int row: 0
    property int col: 0
    property Item backdrop: null
    property int scrollMs: 300
    property int scrollEasing: Easing.OutCubic
    property int focusMs: 150
    property int focusEasing: Easing.OutCubic
    property int labelMs: 120
    property bool motionReduced: false
    // LumaHome's focus-rest flag; tiles lift and show their name only once it is true.
    property bool rested: true

    property int tileSize: dockEntry.tile || 120
    property int gap: dockEntry.gap || 16
    property int padY: dockEntry.padY || 40
    property int fade: dockEntry.fade || 110
    // Columns: as many as fit at the mock's minimum inset; the grid is then centred so
    // whatever width is left over splits evenly instead of piling up on the right.
    property int minPadX: dockEntry.padX || 40
    property int cols: Math.max(1, Math.floor((width - 2 * minPadX + gap) / (tileSize + gap)))
    property int padX: Math.round((width - (cols * tileSize + (cols - 1) * gap)) / 2)
    property int visibleRows: Math.max(1, Math.floor((height - padY + gap) / (tileSize + gap)))
    property int totalRows: cols > 0 ? Math.ceil(points.length / cols) : 0
    property int scrollRow: 0
    property bool scrollable: totalRows > visibleRows
    // all rows fit: centre them; otherwise the mock's top inset with the fade over the overflow
    property int gridTop: scrollable ? Math.max(padY, 64) : Math.round((height - (totalRows * tileSize + (totalRows - 1) * gap)) / 2)

    property var points: []
    // The Repeater's model: a ListModel so a rearrange is a move() and the existing delegates
    // animate to their new slots (a replaced JS array would re-create them all).
    ListModel { id: tileModel }
    function syncModel() {
        tileModel.clear()
        for (var i = 0; i < points.length; i++) tileModel.append(rowFor(points[i]))
    }
    function rowFor(p) {
        return { id: p.id, launchPointId: p.launchPointId, title: p.title, paramsJson: JSON.stringify(p.params || {}),
                 iconUrl: p.iconUrl, plate: String(p.plate), floating: p.floating, neutralPlate: p.neutralPlate, unmovable: p.unmovable }
    }
    property var plateCache: ({})
    property var plateQueue: []
    property bool sampling: false

    // ---------------------------------------------------------------- launch points
    function iconFor(p) { return p.extraLargeIcon || p.largeIcon || p.icon || p.mediumLargeIcon || "" }
    function iconUrl(p) {
        if (!p) return ""
        if (/^[a-zA-Z][a-zA-Z0-9+.-]*:/.test(p)) return p
        return "file://" + p
    }
    // Launch points come from homelaunchpoints (the user-ordered list Home itself shows,
    // with `unmovable`); applicationManager's list is the fallback until it answers.
    property var hlpList: null
    function refresh() {
        var list = hlpList
        if (!list) {
            try { list = (JSON.parse(LS.applicationManager.launchPointsList).launchPoints || []) }
            catch (e) { console.warn("[Dock] launchPointsList parse failed: " + e); return }
        }
        list = list.filter(function(p) { return !p.hidden })
        var neutral = ["#ffffff", "#fff", "#000000", "#000", "#060606", "#1e1e1e"]
        var overrides = (theme.tiles && theme.tiles.icons) || {}
        var plates = (theme.tiles && theme.tiles.plates) || {}
        points = list.map(function(p) {
            var raw = String(p.iconColor || p.bgColor || "#2a2833").toLowerCase()
            var forced = plates[p.id] ? String(plates[p.id]).toLowerCase() : ""
            var icon = overrides[p.id] ? dock.themeIcon(overrides[p.id]) : dock.iconUrl(iconFor(p))
            return { id: p.id, launchPointId: p.launchPointId, title: p.title || p.id, params: p.params || {},
                     iconUrl: icon, plate: forced || raw, forced: forced !== "", floating: false,
                     neutralPlate: !forced && neutral.indexOf(raw) >= 0, unmovable: !!p.unmovable }
        })
        syncModel()
        for (var i = 0; i < points.length; i++) queueSample(points[i].iconUrl, i)
        if (row * cols + col >= points.length) { row = 0; col = 0; scrollRow = 0 }
    }
    Service {
        id: hlp
        appId: LS.appId
        onResponse: {
            try {
                var msg = JSON.parse(payload)
                if (msg.launchPoints && msg.launchPoints.length) { dock.hlpList = msg.launchPoints; if (!dock.moving) dock.refresh() }
            } catch (e) {}
        }
        Component.onCompleted: call("luna://com.webos.service.homelaunchpoints", "/listLaunchPoints", JSON.stringify({ subscribe: true }))
    }
    property string themeDirUrl: "file://{{dataDir}}/theme/"
    function themeIcon(p) {
        if (/^[a-zA-Z][a-zA-Z0-9+.-]*:/.test(p)) return p
        if (p.charAt(0) === "/") return "file://" + p
        return themeDirUrl + p
    }

    Connections { target: LS.applicationManager; onLaunchPointsListChanged: if (!dock.hlpList) dock.refresh() }
    onThemeChanged: refresh()   // theme.json lands after first paint; plates/icon overrides live there
    Component.onCompleted: dock.refresh()

    // ---------------------------------------------------------------- plate derivation
    // Icons whose canvas is fully painted get a plate matching their own edge colour and
    // are drawn edge to edge; icons with transparent edges float inset on a plate in their
    // dominant hue. Sampled once per icon at 32x32, off the render path.
    function queueSample(url, index) {
        if (!url) return
        if (plateCache[url]) { applyPlate(index, plateCache[url]); return }
        plateQueue.push({ url: url, index: index })
        if (!sampling) processNext()
    }
    function processNext() {
        if (plateQueue.length === 0) { sampling = false; return }
        sampling = true
        sampler.pending = plateQueue.shift()
        if (sampler.isImageLoaded(sampler.pending.url)) sampler.requestPaint()
        else sampler.loadImage(sampler.pending.url)
    }
    function applyPlate(index, result) {
        if (index < 0 || index >= points.length || !result) return
        var p = points[index]
        var copy = points.slice()
        var plate = p.forced ? p.plate : (p.neutralPlate || result.floating) ? result.color : p.plate
        copy[index] = { id: p.id, launchPointId: p.launchPointId, title: p.title, params: p.params, iconUrl: p.iconUrl,
                        plate: plate, forced: p.forced, floating: result.floating, neutralPlate: p.neutralPlate, unmovable: p.unmovable }
        points = copy
        if (index < tileModel.count) { tileModel.setProperty(index, "plate", String(plate)); tileModel.setProperty(index, "floating", result.floating) }
    }
    function analyzeEdges(data, w, h) {
        var sumA = 0, n = 0, sumR = 0, sumG = 0, sumB = 0
        var hx = 0, hy = 0, hn = 0
        for (var y = 0; y < h; y++) {
            for (var x = 0; x < w; x++) {
                var i = (y * w + x) * 4
                var a = data[i + 3] / 255
                // edge test on the middle half of each side, so rounded-square icons
                // (transparent corners, opaque sides) count as full-canvas artwork
                var onEdge = (x === 0 || y === 0 || x === w - 1 || y === h - 1)
                var mid = (x >= w / 4 && x < w * 3 / 4) || (y >= h / 4 && y < h * 3 / 4)
                if (onEdge && mid) {
                    sumA += a; n++
                    if (a > 0.5) { sumR += data[i]; sumG += data[i + 1]; sumB += data[i + 2] }
                }
                if (a > 0.5) {
                    var mx = Math.max(data[i], data[i + 1], data[i + 2]), mn = Math.min(data[i], data[i + 1], data[i + 2])
                    if (mx - mn > 12) { var hue = hueOf(data[i], data[i + 1], data[i + 2]); hx += Math.cos(hue); hy += Math.sin(hue); hn++ }
                }
            }
        }
        var edgeAlpha = n > 0 ? sumA / n : 0
        var threshold = (theme.tiles && theme.tiles.fullCanvasEdgeAlpha) || 0.9
        if (edgeAlpha >= threshold) {
            var cnt = Math.max(1, n)
            return { color: Qt.rgba(sumR / 255 / cnt, sumG / 255 / cnt, sumB / 255 / cnt, 1), floating: false }
        }
        if (hn > 0) return { color: hslColor(Math.atan2(hy, hx), 0.45, 0.55), floating: true }
        return { color: Qt.rgba(0.16, 0.16, 0.19, 1), floating: true }
    }
    function hueOf(r, g, b) {
        r /= 255; g /= 255; b /= 255
        var mx = Math.max(r, g, b), mn = Math.min(r, g, b), d = mx - mn
        if (d === 0) return 0
        var h
        if (mx === r) h = ((g - b) / d) % 6
        else if (mx === g) h = (b - r) / d + 2
        else h = (r - g) / d + 4
        return h * (Math.PI / 3)
    }
    function hslColor(hueRad, s, l) {
        var h = ((hueRad / (Math.PI * 2)) + 1) % 1
        var c = (1 - Math.abs(2 * l - 1)) * s
        var x = c * (1 - Math.abs(((h * 6) % 2) - 1))
        var m = l - c / 2
        var r, g, b
        if (h < 1/6) { r = c; g = x; b = 0 } else if (h < 2/6) { r = x; g = c; b = 0 }
        else if (h < 3/6) { r = 0; g = c; b = x } else if (h < 4/6) { r = 0; g = x; b = c }
        else if (h < 5/6) { r = x; g = 0; b = c } else { r = c; g = 0; b = x }
        return Qt.rgba(r + m, g + m, b + m, 1)
    }
    Canvas {
        id: sampler
        width: 32; height: 32; visible: false
        property var pending: null
        onImageLoaded: if (pending) requestPaint()
        onPaint: {
            if (!pending) return
            if (!isImageLoaded(pending.url)) { loadImage(pending.url); return }
            var ctx = getContext("2d")
            ctx.clearRect(0, 0, 32, 32)
            var result = null
            try {
                ctx.drawImage(pending.url, 0, 0, 32, 32)
                result = dock.analyzeEdges(ctx.getImageData(0, 0, 32, 32).data, 32, 32)
            } catch (e) {}
            var item = pending
            pending = null
            if (result) { dock.plateCache[item.url] = result; dock.applyPlate(item.index, result) }
            dock.processNext()
        }
    }

    // ---------------------------------------------------------------- accent
    function accentFor(p) {
        if (!p) return "#9a8cf0"
        var t = theme.accent || "tile"
        if (/^#/.test(t)) return t
        if (p.neutralPlate) return /netflix/i.test(p.title || p.id) ? "#e50914" : "#9a8cf0"
        return p.plate
    }
    property color currentAccent: accentFor(points[row * cols + col])

    // ---------------------------------------------------------------- chrome
    property var dockMat: {
        var m = {}
        for (var k in mat) m[k] = mat[k]
        m.glassFocusEdge = isLight ? "#ffffff" : "#38ffffff"   // .dock.active edge
        m.glassFocus = mat.glass
        return m
    }
    Glass {
        mat: dock.dockMat
        isLight: dock.isLight
        cardRadius: dock.theme.radius || 26
        focusMix: 0
        edgeFocus: dock.focusedLayer ? 1 : 0
        screenX: dock.originX; screenY: dock.originY; screenW: dock.width; screenH: dock.height
        backdrop: dock.backdrop
    }

    function focusCenter() {
        return Qt.point(padX + col * (tileSize + gap) + tileSize / 2,
                        gridTop + (row - scrollRow) * (tileSize + gap) + tileSize / 2)
    }

    Item {
        // tiles scroll inside the card rect; the clip lives here so the Glass shadow outside
        // the card is never cut off
        id: viewport
        anchors.fill: parent
        clip: true
        // radial-gradient(440px 230px at tile centre, accent .5 -> 0 at 70%), opacity .75
        ShaderEffect {
            id: ambient
            width: 440; height: 230
            x: Math.round(dock.focusCenter().x - width / 2)
            y: Math.round(dock.focusCenter().y - height / 2)
            // The glow follows the highlight at once, like it: a glow that glides draws a frame
            // per key press. Only its fade, when the dock gains or loses focus, animates.
            opacity: dock.focusedLayer ? ((dock.theme.focus && dock.theme.focus.ambient !== undefined) ? dock.theme.focus.ambient : 0.3) : 0
            visible: opacity > 0.01
            Behavior on opacity { NumberAnimation { duration: 180 } }
            property vector4d tintC: Qt.vector4d(dock.currentAccent.r, dock.currentAccent.g, dock.currentAccent.b, 0.5)
            fragmentShader: "
                uniform highp vec4 tintC;
                uniform lowp float qt_Opacity;
                varying highp vec2 qt_TexCoord0;
                void main() {
                    highp float r = length((qt_TexCoord0 - 0.5) * 2.0);
                    highp float a = tintC.a * (1.0 - smoothstep(0.0, 0.7, r));
                    gl_FragColor = vec4(tintC.rgb * a, a) * qt_Opacity;
                }"
        }
        Item {
            id: grid
            x: dock.padX
            y: dock.gridTop - dock.scrollRow * (dock.tileSize + dock.gap)
            width: dock.cols * dock.tileSize + (dock.cols - 1) * dock.gap
            height: Math.max(1, dock.totalRows * dock.tileSize + (dock.totalRows - 1) * dock.gap)
            Behavior on y { enabled: !dock.motionReduced; NumberAnimation { duration: dock.scrollMs; easing.type: dock.scrollEasing } }
    
            Repeater {
                model: tileModel
                delegate: Tile {
                    x: (index % dock.cols) * (dock.tileSize + dock.gap)
                    y: Math.floor(index / dock.cols) * (dock.tileSize + dock.gap)
                    // while rearranging every tile eases to its slot: the held one glides quickly,
                    // displaced neighbours take a little longer so the gap visibly closes behind it
                    Behavior on x { enabled: dock.moving; NumberAnimation { duration: moving ? 120 : 220; easing.type: Easing.OutCubic } }
                    Behavior on y { enabled: dock.moving; NumberAnimation { duration: moving ? 120 : 220; easing.type: Easing.OutCubic } }
                    moving: dock.moving && index === (dock.row * dock.cols + dock.col)
                    size: dock.tileSize
                    tileRadius: dock.theme.tileRadius || 22
                    inset: (dock.theme.tiles && dock.theme.tiles.inset) || 19
                    ink: dock.mat.ink
                    isLight: dock.isLight
                    launchPoint: ({ title: model.title, iconUrl: model.iconUrl })
                    plate: model.plate
                    floating: model.floating
                    accent: dock.accentFor({ neutralPlate: model.neutralPlate, plate: model.plate, title: model.title, id: model.id })
                    focused: dock.focusedLayer && index === (dock.row * dock.cols + dock.col)
                    focusScale: (dock.theme.focus && dock.theme.focus.tileScale) || 1.12
                    focusMs: dock.focusMs
                    focusEasing: dock.focusEasing
                    labelMs: dock.labelMs
                    motionReduced: dock.motionReduced
                    rested: dock.rested
                    labelAbove: Math.floor(index / dock.cols) === dock.scrollRow
                    glowStrength: (dock.theme.focus && dock.theme.focus.tileGlow) || 0
                    onHoverFocus: { dock.row = Math.floor(index / dock.cols); dock.col = index % dock.cols; dock.ensureRowVisible() }
                    onActivated: { dock.row = Math.floor(index / dock.cols); dock.col = index % dock.cols; if (dock.moving) dock.endMove(); else dock.activate() }
                    onHoldActivated: { dock.row = Math.floor(index / dock.cols); dock.col = index % dock.cols; dock.beginMove() }
                }
            }
        }
    }

    // .more: neutral fade over the last visible row, masked by the card's own rounded rect
    ShaderEffect {
        anchors.left: parent.left; anchors.right: parent.right; anchors.bottom: parent.bottom
        height: dock.fade
        z: 4
        visible: dock.scrollable
        property color base: dock.isLight ? "#f4f2f8" : "#120e1a"
        property vector4d fadeC: Qt.vector4d(base.r, base.g, base.b, dock.isLight ? 0.94 : 0.9)
        property real mid: dock.isLight ? 0.78 : 0.72
        property vector2d dims: Qt.vector2d(width, height)
        property vector2d card: Qt.vector2d(dock.width, dock.height)
        property real radius: dock.theme.radius || 26
        fragmentShader: "
            uniform highp vec4 fadeC;
            uniform highp float mid;
            uniform highp vec2 dims;
            uniform highp vec2 card;
            uniform highp float radius;
            uniform lowp float qt_Opacity;
            varying highp vec2 qt_TexCoord0;
            void main() {
                highp float t = qt_TexCoord0.y;
                highp float a = t < 0.7 ? mix(0.0, mid, t / 0.7) : mix(mid, fadeC.a, (t - 0.7) / 0.3);
                highp vec2 p = vec2(qt_TexCoord0.x * dims.x, card.y - dims.y + qt_TexCoord0.y * dims.y);
                highp vec2 hs = card * 0.5;
                highp vec2 q = abs(p - hs) - (hs - vec2(radius));
                highp float d = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - radius;
                a *= 1.0 - smoothstep(-0.75, 0.75, d);
                gl_FragColor = vec4(fadeC.rgb * a, a) * qt_Opacity;
            }"
    }

    // ---------------------------------------------------------------- rearrange
    // Hold OK on a tile to pick it up; Left/Right/Up/Down swap it with the neighbour in that
    // direction (the neighbour slides across); OK or Back drops it. The final order is
    // written through homelaunchpoints/moveLaunchPoint, the same list stock Home reads, so
    // it persists and both homes agree. Position 0 (LG's Apps) is unmovable.
    property bool moving: false
    property int moveOrigin: -1
    function beginMove() {
        var i = row * cols + col
        if (!points[i] || points[i].unmovable) return
        moving = true
        moveOrigin = i
    }
    function endMove() {
        if (!moving) return
        moving = false
        var i = row * cols + col
        var p = points[i]
        if (p && i !== moveOrigin)
            LS.adhoc.call("luna://com.webos.service.homelaunchpoints", "/moveLaunchPoint",
                          JSON.stringify({ launchPointId: p.launchPointId, position: i }))
        moveOrigin = -1
    }
    function cancelMove() {
        if (!moving) return
        moving = false
        var i = row * cols + col
        if (i !== moveOrigin && moveOrigin >= 0) {
            var copy = points.slice(); var p = copy.splice(i, 1)[0]; copy.splice(moveOrigin, 0, p); points = copy
            tileModel.move(i, moveOrigin, 1)
            row = Math.floor(moveOrigin / cols); col = moveOrigin % cols; ensureRowVisible()
        }
        moveOrigin = -1
    }
    function swapTo(j) {
        var i = row * cols + col
        if (j < 0 || j >= points.length || j === i || points[j].unmovable) return
        var copy = points.slice(); var p = copy.splice(i, 1)[0]; copy.splice(j, 0, p); points = copy
        tileModel.move(i, j, 1)
        row = Math.floor(j / cols); col = j % cols; ensureRowVisible()
    }

    // ---------------------------------------------------------------- navigation
    function ensureRowVisible() {
        if (row < scrollRow) scrollRow = row
        else if (row > scrollRow + visibleRows - 1) scrollRow = row - visibleRows + 1
        scrollRow = Math.max(0, Math.min(scrollRow, Math.max(0, totalRows - visibleRows)))
    }
    function moveLeft() { if (moving) { swapTo(row * cols + col - 1); return } if (col > 0) col-- }
    function moveRight() { if (moving) { swapTo(row * cols + col + 1); return } if (col < cols - 1 && (row * cols + col + 1) < points.length) col++ }
    function moveUp() {
        if (moving) { swapTo(row * cols + col - cols); return true }
        if (row === 0) return false
        row--
        ensureRowVisible()
        return true
    }
    function moveDown() {
        if (moving) { swapTo(Math.min(points.length - 1, row * cols + col + cols)); return }
        if ((row + 1) * cols >= points.length) return
        row++
        if (row * cols + col >= points.length) col = (points.length - 1) - row * cols
        ensureRowVisible()
    }
    function activate() {
        var p = points[row * cols + col]
        if (!p) return
        LS.adhoc.call("luna://com.webos.applicationManager", "/launch", JSON.stringify({ id: p.id, params: p.params || {} }))
    }
    function focusedCenterX() { return originX + focusCenter().x }
}
