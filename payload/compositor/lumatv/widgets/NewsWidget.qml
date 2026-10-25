import QtQuick 2.4
import WebOSCompositorBase 1.0
import TextFile 1.0
import "../Saved.js" as Saved
import "../kit"

// Hero image, headline, meta row with page dots. Rows are the mock's grid (hero 262, gap 18,
// headline 34px/1.22 capped at two lines, meta 22px pinned to the bottom). OK opens the article.
Item {
    id: root
    property var theme: ({})
    property var entry: ({})
    property var mat: ({ ink: "#F2EFF6", ink2: "#A8F2EFF6", ink3: "#6BF2EFF6" })
    property bool isLight: false
    property var settings: ({})
    property bool focused: false
    property bool inner: false

    signal openMini(var spec)
    // OK on the card enters the inner layer (LumaHome does that); inside it Left/Right step
    // stories, OK opens the article in a mini frame, Back leaves (handled by LumaHome).
    function key(dir) {
        if (dir === "left") { step(-1); return true }
        if (dir === "right") { step(1); return true }
        if (dir === "ok") { activate(); return true }
        return false
    }
    function step(d) {
        if (items.length < 2) return
        index = (index + d + items.length) % items.length
        holdTimer.restart()
    }
    function activate() {
        var n = items[index]
        if (n && n.link) openMini({ kind: "web", url: n.link, title: n.title, subtitle: (n.src || "") + (n.pub ? " \u00b7 " + ago(n.pub) : ""), source: n.src, ago: ago(n.pub) })
    }
    // a manual step holds the slideshow for a while so the story is not swept away
    Timer { id: holdTimer; interval: 30000 }

    property int slideMs: (theme.news && theme.news.slideMs) || 10000
    property bool live: true            // Home is on screen: the slideshow runs only then
    property double fetchedAt: 0        // when the stories on screen were fetched

    property var items: []
    property int index: 0

    function ichefWidth(px) {
        var buckets = [480, 640, 800, 1024, 1280, 1536, 1920]
        for (var i = 0; i < buckets.length; i++) if (buckets[i] >= px) return buckets[i]
        return buckets[buckets.length - 1]
    }
    // The feed gives its hero in one size; the card asks for the width it draws at (the BBC's
    // image host has buckets). The saved stories keep the address as the feed gave it.
    function imgUrl(n) {
        if (!n || !n.img) return ""
        return n.img.replace(/\/standard\/[0-9]+\//, "/standard/" + ichefWidth(Math.round(root.width)) + "/")
    }
    function ago(pub) {
        var t = Date.parse(pub); if (isNaN(t)) return ""
        var m = Math.round((Date.now() - t) / 60000)
        if (m < 1) return "now"
        if (m < 60) return m + (m === 1 ? " minute ago" : " minutes ago")
        if (m < 1440) { var h = Math.round(m / 60); return h + (h === 1 ? " hour ago" : " hours ago") }
        var d = Math.round(m / 1440); return d + (d === 1 ? " day ago" : " days ago")
    }
    function advance() { if (items.length > 1) index = (index + 1) % items.length }

    // The stories come from the native helper (Helper.topics.news: fetched, saved, aged and dropped
    // there; the edition is its setting, base.news). Until the helper has answered, the document it
    // saved last time is read from {{dataDir}}/cache/news.json, so a cold start draws the last stories
    // at once. A document older than the one on screen, or past its six hours, is not drawn.
    TextFile { id: savedFile; name: "file://{{dataDir}}/cache/news.json" }
    function take(doc) {
        if (!Saved.newsUsable(doc, Date.now()) || doc.fetchedAt <= fetchedAt) return
        items = doc.items
        if (index >= items.length) index = 0
        fetchedAt = doc.fetchedAt
    }
    readonly property var topic: Helper.topics.news || null
    onTopicChanged: if (topic) take(topic.doc)
    Component.onCompleted: {
        take(Saved.parse(savedFile.exists ? savedFile.read() : ""))
        if (topic) take(topic.doc)
    }
    Timer { interval: slideMs; running: root.live && root.items.length > 1 && !root.inner && !holdTimer.running; repeat: true; onTriggered: root.advance() }

    // What is on screen. A story change is a cross-fade: the next hero image decodes into
    // the idle slot first (no placeholder flash), then the hero shader mixes A->B over
    // 420 ms while the text fades out, swaps and fades back in. Opacity on native text and
    // one mix uniform on a quad already being drawn: nothing extra per frame.
    property var current: ({ tag: "", src: "", title: "", summary: "", pub: "", img: "" })
    // rows: hero 196, gap 18, headline (2 lines of 41), gap 8, summary, meta 30 at the bottom
    property int heroH: 196
    property var pendingItem: null
    property int fadeMs: 420
    onIndexChanged: showItem(items[index])
    onItemsChanged: if (!current.title && items.length) showItem(items[index])
    function showItem(n) {
        if (!n) return
        if (!current.title) { current = n; heroA.source = imgUrl(n); return }
        pendingItem = n
        var slot = hero.frontIsA ? heroB : heroA
        if (imgUrl(n) && slot.source == imgUrl(n) && slot.status === Image.Ready) { beginFade(); return }
        slot.source = imgUrl(n)
        if (!n.img) beginFade()
    }
    function beginFade() {
        if (!pendingItem) return
        textFade.restart()
        heroFade.restart()
    }
    SequentialAnimation {
        id: textFade
        NumberAnimation { target: textBlock; property: "opacity"; to: 0; duration: root.fadeMs / 2; easing.type: Easing.InQuad }
        ScriptAction { script: { if (root.pendingItem) root.current = root.pendingItem } }
        NumberAnimation { target: textBlock; property: "opacity"; to: 1; duration: root.fadeMs / 2; easing.type: Easing.OutQuad }
    }
    SequentialAnimation {
        id: heroFade
        NumberAnimation { target: hero; property: "mixv"; to: hero.frontIsA ? 1 : 0; duration: root.fadeMs; easing.type: Easing.InOutQuad }
        ScriptAction { script: { hero.frontIsA = !hero.frontIsA; root.pendingItem = null } }
    }

    // hero: rounded 18, cover-cropped image at exactly the box size (no resampling later)
    Item {
        id: hero
        x: 0; y: 0
        width: parent.width
        height: root.heroH
        property bool frontIsA: true
        property real mixv: 0       // 0 shows A, 1 shows B
        Rectangle { anchors.fill: parent; radius: 18; color: root.isLight ? "#e9ecf3" : "#222222"; antialiasing: true }
        Image {
            id: heroA
            visible: false
            asynchronous: true; cache: true
            sourceSize.width: hero.width
            onStatusChanged: if (status === Image.Ready && root.pendingItem && !hero.frontIsA) root.beginFade()
        }
        Image {
            id: heroB
            visible: false
            asynchronous: true; cache: true
            sourceSize.width: hero.width
            onStatusChanged: if (status === Image.Ready && root.pendingItem && hero.frontIsA) root.beginFade()
        }
        ShaderEffect {
            // two cover-crops through a rounded mask, mixed by one uniform: one quad, no layer
            anchors.fill: parent
            visible: heroA.status === Image.Ready || heroB.status === Image.Ready
            property variant srcA: heroA
            property variant srcB: heroB
            property real mixv: hero.mixv
            property vector2d dims: Qt.vector2d(width, height)
            property real radius: 18
            function coverFor(img) {
                var ia = img.implicitHeight > 0 ? img.implicitWidth / img.implicitHeight : 1
                var ba = width / height
                return ia > ba ? Qt.vector2d(ba / ia, 1) : Qt.vector2d(1, ia / ba)
            }
            property vector2d coverA: coverFor(heroA)
            property vector2d coverB: coverFor(heroB)
            fragmentShader: "
                uniform sampler2D srcA;
                uniform sampler2D srcB;
                uniform mediump float mixv;
                uniform highp vec2 dims;
                uniform highp vec2 coverA;
                uniform highp vec2 coverB;
                uniform highp float radius;
                uniform lowp float qt_Opacity;
                varying highp vec2 qt_TexCoord0;
                void main() {
                    highp vec2 p = qt_TexCoord0 * dims;
                    highp vec2 hs = dims * 0.5;
                    highp vec2 q = abs(p - hs) - (hs - vec2(radius));
                    highp float d = length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - radius;
                    mediump float a = 1.0 - smoothstep(-0.75, 0.75, d);
                    lowp vec4 ca = texture2D(srcA, (qt_TexCoord0 - 0.5) * coverA + 0.5);
                    lowp vec4 cb = texture2D(srcB, (qt_TexCoord0 - 0.5) * coverB + 0.5);
                    gl_FragColor = mix(ca, cb, mixv) * a * qt_Opacity;
                }"
        }
        Rectangle {
            visible: root.current.src !== ""
            opacity: textBlock.opacity
            x: 18; y: 16
            width: kicker.implicitWidth + 24; height: 39; radius: 10
            color: root.isLight ? "#b3ffffff" : "#73000000"
            antialiasing: true
            Text {
                id: kicker
                x: 12; y: Math.round((parent.height - implicitHeight) / 2)
                text: root.current.src
                color: root.isLight ? "#17131F" : "#ffffff"
                font.family: "Manrope"; font.weight: Font.DemiBold; font.pixelSize: 20
                renderType: Text.NativeRendering
            }
        }
    }

    Item {
        id: textBlock
        anchors.fill: parent
    Text {
        id: headline
        x: 0; y: root.heroH + 18
        width: parent.width
        text: root.current.title
        color: root.mat.ink
        font.family: "Manrope"; font.weight: Font.DemiBold; font.pixelSize: 34
        font.letterSpacing: -0.34
        lineHeight: 41
        lineHeightMode: Text.FixedHeight
        wrapMode: Text.WordWrap
        maximumLineCount: 2
        elide: Text.ElideRight
        renderType: Text.NativeRendering
    }
    Text {
        id: summary
        x: 0; y: headline.y + headline.height + 8
        width: parent.width
        height: Math.max(0, meta.y - 12 - y)
        text: root.current.summary
        color: root.mat.ink2
        font.family: "Manrope"; font.weight: Font.Medium; font.pixelSize: 22
        lineHeight: 30
        lineHeightMode: Text.FixedHeight
        wrapMode: Text.WordWrap
        maximumLineCount: Math.max(1, Math.floor(height / 30))
        elide: Text.ElideRight
        renderType: Text.NativeRendering
    }

    Item {
        id: meta
        x: 0
        y: parent.height - 30
        width: parent.width
        height: 30
        Text {
            x: 0; y: Math.round((parent.height - implicitHeight) / 2)
            text: (root.current.tag ? root.current.tag + " \u00b7 " : "") + root.ago(root.current.pub)
            color: root.mat.ink2
            font.family: "Manrope"; font.weight: Font.Medium; font.pixelSize: 22
            renderType: Text.NativeRendering
        }
        Row {
            anchors.right: parent.right
            y: 11
            spacing: 8
            Repeater {
                model: root.items.length
                delegate: Rectangle {
                    height: 8; radius: 4
                    width: index === root.index ? (root.inner ? 30 : 22) : 8
                    color: index === root.index ? root.mat.ink : (root.inner ? root.mat.ink2 : root.mat.ink3)
                    Behavior on width { NumberAnimation { duration: 150 } }
                    Behavior on color { ColorAnimation { duration: 150 } }
                }
            }
        }
    }
    }
}
