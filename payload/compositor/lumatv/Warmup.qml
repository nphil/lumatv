import QtQuick 2.4
import QtQuick.Window 2.2
import "kit"

// The startup warm-up. Qt rasterises a glyph set the first time a font at a given size is drawn and
// compiles a ShaderEffect's program the first time it is drawn, both inside the frame that is
// waiting for them. Here every text style the surfaces use, every shipped image and every component
// that carries a shader of its own is drawn once, hidden at 1 % opacity, one step at a time, after
// Home has presented; the items then stay (hidden) so the font engines and pixmaps stay referenced
// and Qt's caches do not drop them. A step advances only after two frames were presented, so each
// item is really drawn, and only while Home is on screen, since this item is part of its tree.
//
// The text styles come from three places: the type roles of theme.json at their weights, the text
// of the item tree this host is given (Home's own, walked once so the list cannot drift from the
// surfaces), and whatever surfaces ask for through Warm.request(). Both lists only grow: a request
// that arrives later is drawn in a second pass and nothing already drawn is built again.
Item {
    id: host

    property Item tree: null
    property bool enabled: true
    property bool active: false             // Home is on screen; steps wait while it is not
    property int startMs: 1500              // after Home first presents
    property int stepMs: 50
    property int perStep: 4                 // images and components drawn per step

    property string stage: "idle"           // idle -> waiting -> text -> other -> done
    readonly property bool finished: stage === "done"
    property var styles: []                 // [{weight, size, spacing}], append-only
    property var others: []                 // [{kind: "image", url} | {kind: "component", component}], append-only
    property var seen: ({})
    property int textShown: 0               // styles drawn so far; the one on screen is textShown - 1
    property int otherShown: 0
    property double startedAt: 0
    property int frames: 0
    property int frameMark: 0
    signal done(string summary)

    // Basic Latin, Latin-1 and the typographic marks the feeds and weather text use.
    readonly property string glyphs: {
        var s = ""
        for (var c = 32; c < 127; c++) s += String.fromCharCode(c)
        for (var d = 160; d < 256; d++) s += String.fromCharCode(d)
        return s + "\u2018\u2019\u201C\u201D\u2013\u2014\u2026\u2022\u00B7\u2122\u20AC"
    }

    Connections { target: host.Window.window; onFrameSwapped: host.frames++ }

    function addStyle(s) {
        var k = s.weight + "|" + s.size + "|" + (s.spacing || 0)
        if (seen[k] || !(s.size > 0)) return
        seen[k] = true
        var list = styles.slice()
        list.push({ weight: s.weight, size: s.size, spacing: s.spacing || 0 })
        styles = list
    }
    // Every Text-like item below `item`: it has a font, a text and a render type.
    function harvest(item, depth) {
        if (!item || depth > 48) return
        var f = item.font
        if (f !== undefined && item.text !== undefined && item.renderType !== undefined && f.pixelSize > 0)
            addStyle({ weight: f.weight, size: f.pixelSize, spacing: f.letterSpacing || 0 })
        var kids = item.children
        if (kids) for (var i = 0; i < kids.length; i++) harvest(kids[i], depth + 1)
    }
    function collect(withTree) {
        var roles = ["display", "title", "body", "label", "caption"]
        for (var i = 0; i < roles.length; i++) {
            var t = Lu.type(roles[i])
            addStyle({ weight: t.weight, size: t.size, spacing: t.tracking })
        }
        if (withTree) harvest(tree, 0)
        for (var j = 0; j < Warm.texts.length; j++) addStyle(Warm.texts[j])
        // Text styles of surfaces that live outside this item tree and cannot ask for themselves: the
        // notification view's toast (Manrope 26, 21, 24 and 34 px) and Quick Settings' tiles. Both draw
        // through the same process's glyph caches, so a style drawn here is hot there.
        var listed = Lu.theme && Lu.theme.warmup && Lu.theme.warmup.texts ? Lu.theme.warmup.texts : []
        for (var q = 0; q < listed.length; q++) addStyle({ weight: Lu.weightOf(listed[q].weight), size: listed[q].size, spacing: listed[q].spacing || 0 })
        var more = others.slice()
        for (var k = 0; k < Warm.images.length; k++) {
            var ik = "i|" + Warm.images[k]
            if (!seen[ik]) { seen[ik] = true; more.push({ kind: "image", url: Warm.images[k] }) }
        }
        for (var m = 0; m < Warm.components.length; m++) {
            if (more.some(function(e) { return e.component === Warm.components[m] })) continue
            more.push({ kind: "component", component: Warm.components[m] })
        }
        others = more
    }

    function begin() {
        if (!enabled || stage !== "idle" || !active) return
        stage = "waiting"
        startTimer.restart()
    }
    onActiveChanged: begin()
    onEnabledChanged: begin()
    Component.onCompleted: begin()

    Timer {
        id: startTimer
        interval: host.startMs
        onTriggered: {
            try { host.collect(true) }
            catch (e) { console.warn("[Warmup] could not list what to draw: " + e); host.stage = "done"; return }
            host.startedAt = Date.now()
            host.stage = "text"
            host.frameMark = host.frames
            stepTimer.restart()
        }
    }
    // A request that arrives after the first pass is drawn in a second one.
    Connections {
        target: Warm
        onRequested: {
            if (host.stage !== "done") return
            host.collect(false)
            host.stage = "text"
            host.frameMark = host.frames
            stepTimer.restart()
        }
    }
    Timer {
        id: stepTimer
        interval: host.stepMs
        repeat: true
        onTriggered: {
            if (!host.active || host.frames < host.frameMark + 2) return
            host.frameMark = host.frames
            if (host.stage === "text") {
                if (host.textShown < host.styles.length) host.textShown++
                else host.stage = "other"
            } else if (host.stage === "other") {
                if (host.otherShown < host.others.length) host.otherShown = Math.min(host.others.length, host.otherShown + host.perStep)
                else host.finish()
            }
        }
    }
    function finish() {
        stepTimer.stop()
        stage = "done"
        var ms = Date.now() - startedAt
        var summary = styles.length + " text styles and " + others.length + " images and components in " + ms + " ms"
        console.info("[Warmup] " + summary)
        done(summary)
    }

    // One item per style, on screen for its own step. The text is the whole glyph set wrapped to the
    // window: the glyph cache is filled by drawing, not by laying out.
    Repeater {
        model: host.styles.length
        delegate: Text {
            readonly property var spec: host.styles[index]
            width: 1920
            wrapMode: Text.WrapAnywhere
            maximumLineCount: 24
            text: host.glyphs
            color: "white"
            font.family: Lu.family
            font.weight: spec.weight
            font.pixelSize: spec.size
            font.letterSpacing: spec.spacing
            renderType: Text.NativeRendering
            opacity: 0.01
            visible: host.stage === "text" && index === host.textShown - 1
        }
    }
    // Images and shader components, in groups of `perStep`; the newest group is on screen.
    Repeater {
        model: host.otherShown
        delegate: Loader {
            readonly property var entry: host.others[index]
            opacity: 0.01
            visible: host.stage === "other" && index >= host.otherShown - host.perStep
            sourceComponent: entry.kind === "image" ? imageDelegate : entry.component
            Component {
                id: imageDelegate
                Image { source: entry.url; asynchronous: false; cache: true; width: 64; height: 64; fillMode: Image.PreserveAspectFit }
            }
        }
    }
}
