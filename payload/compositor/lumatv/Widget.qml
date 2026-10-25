import QtQuick 2.4

// Generic widget card: Glass chrome, the mock's focus treatment (lift 4px, chrome scaled
// 1.03, brighter tint/edge/sheen) and a dynamically resolved content component. Content is
// never scaled: it sits on the pixel grid while only the glass grows under it. Resolution
// order for a layout entry's type: built-in registry -> theme-dir "*.qml" -> installed pack
// "<packId>.<name>" -> UnknownWidget, so a bad entry never blanks Home. Content receives
// {theme, mat, isLight, entry, settings, width, height, focused} and may expose activate().
Item {
    id: card

    property var theme: ({})
    property var mat: ({})
    property bool isLight: false
    property var layoutEntry: ({})
    property string themeDirUrl: "file://{{dataDir}}/theme/"
    property string moduleDirUrl: "file://{{dataDir}}/qml/WebOSCompositor/{{qmlModule}}/"
    readonly property string packsDirUrl: "file://{{dataDir}}/widgets/"
    property bool focused: false
    property bool inner: false          // OK was pressed: keys go to the content
    property bool dimmed: false
    property Item backdrop: null
    property int cardMs: 180
    property int cardEasing: Easing.OutCubic
    property bool motionReduced: false
    property real gridX: 0
    property real gridY: 0
    // False while another app is in front: content stops its timers and fetches.
    property bool live: true
    // Home's shared animation tick; content that moves at rest steps on it.
    property int frameTick: 0

    readonly property real cardLift: (theme.focus && theme.focus.cardLift) || 4
    readonly property real cardScale: (theme.focus && theme.focus.cardScale) || 1.03
    readonly property real padTop: 28
    readonly property real padSide: 32

    signal hoverFocus()
    signal openMini(var spec)

    x: gridX
    y: gridY - cardLift * f
    z: focused ? 2 : 1
    opacity: dimmed ? 0.86 : 1.0
    Behavior on opacity { NumberAnimation { duration: card.cardMs; easing.type: card.cardEasing } }

    property real f: focused ? 1 : 0
    Behavior on f { enabled: !motionReduced; NumberAnimation { duration: card.cardMs; easing.type: card.cardEasing } }

    Glass {
        mat: card.mat
        isLight: card.isLight
        cardRadius: card.theme.radius || 26
        focusMix: card.f
        edgeFocus: card.inner ? 1 : 0
        scale: 1 + (card.cardScale - 1) * card.f
        transformOrigin: Item.Center
        screenX: card.gridX; screenY: card.gridY; screenW: card.width; screenH: card.height
        backdrop: card.backdrop
    }

    Image {
        // optional theme-provided image under the content (layout entry "background")
        anchors.fill: parent
        visible: source != ""
        source: card.layoutEntry.background ? card.resolveThemePath(card.layoutEntry.background) : ""
        fillMode: Image.PreserveAspectCrop
        asynchronous: true
        sourceSize.width: card.width; sourceSize.height: card.height
    }

    Item {
        // clip at the card edge (CSS overflow:hidden on the padding box), not the content box
        anchors.fill: parent
        clip: true
        Item {
            id: contentArea
            x: card.padSide; y: card.padTop
            width: parent.width - 2 * card.padSide
            height: parent.height - 2 * card.padTop
        }
    }

    property Item content: null
    property var packCache: ({})

    function resolveThemePath(p) {
        if (!p) return ""
        if (/^[a-zA-Z][a-zA-Z0-9+.-]*:/.test(p)) return p
        if (p.charAt(0) === "/") return "file://" + p
        return card.themeDirUrl + p
    }

    function activate() { if (content && typeof content.activate === "function") content.activate() }
    // true when the content handles keys itself (it gains an inner focus layer on OK)
    function hasInner() { return !!(content && typeof content.key === "function") }
    function innerKey(dir) { return hasInner() ? content.key(dir) === true : false }
    function reload() { loadContent() }

    function builtinUrl(type) {
        var map = { clock: "ClockWidget.qml", weather: "WeatherWidget.qml", news: "NewsWidget.qml", homeassistant: "HomeAssistantWidget.qml" }
        return map[type] ? (card.moduleDirUrl + "widgets/" + map[type]) : ""
    }

    function contentProps(settings) {
        return { theme: card.theme, mat: card.mat, isLight: card.isLight, entry: card.layoutEntry, settings: settings || {},
                 width: contentArea.width, height: contentArea.height, focused: card.focused, inner: card.inner, live: card.live,
                 frameTick: card.frameTick }
    }
    function instantiate(url, settings) {
        var comp = Qt.createComponent(url)
        function finish() {
            if (comp.status === Component.Ready) {
                if (card.content) { card.content.destroy(); card.content = null }
                var obj = comp.createObject(contentArea, card.contentProps(settings))
                if (obj) { card.content = obj; if (obj.openMini) obj.openMini.connect(function(spec) { card.openMini(spec) }) }
                else card.showUnknown(card.layoutEntry.type, "failed to instantiate")
            } else if (comp.status === Component.Error) {
                card.showUnknown(card.layoutEntry.type, comp.errorString())
            }
        }
        if (comp.status === Component.Loading) comp.statusChanged.connect(finish)
        else finish()
    }
    function showUnknown(type, errorText) {
        console.warn("[Widget] falling back to UnknownWidget for '" + type + "': " + errorText)
        if (card.content) { card.content.destroy(); card.content = null }
        var comp = Qt.createComponent(card.moduleDirUrl + "widgets/UnknownWidget.qml")
        function finish() {
            if (comp.status !== Component.Ready) return
            var obj = comp.createObject(contentArea, card.contentProps({ typeName: type, errorText: errorText || "" }))
            if (obj) card.content = obj
        }
        if (comp.status === Component.Loading) comp.statusChanged.connect(finish)
        else finish()
    }

    // ---- widget packs: {{dataDir}}/widgets/<packId>/pack.json, or a theme-bundled
    // <themeDir>/widgets/pack.json; loaded lazily on first reference and cached ----
    function loadPack(type, settingsOverride) {
        var packId = type.substring(0, type.indexOf("."))
        var candidates = [packsDirUrl + packId + "/pack.json", card.themeDirUrl + "widgets/pack.json"]
        tryPack(candidates, 0, type, settingsOverride)
    }
    function tryPack(paths, i, type, settingsOverride) {
        if (i >= paths.length) { card.showUnknown(type, "no installed pack provides this type"); return }
        var url = paths[i]
        var cached = card.packCache[url]
        if (cached) { if (!finishPack(cached, url, type, settingsOverride)) tryPack(paths, i + 1, type, settingsOverride); return }
        loadJsonUrl(url, function(json) {
            if (json) card.packCache[url] = json
            if (!json || !card.finishPack(json, url, type, settingsOverride)) card.tryPack(paths, i + 1, type, settingsOverride)
        })
    }
    function finishPack(pack, url, type, settingsOverride) {
        var entry = null
        for (var i = 0; i < (pack.widgets || []).length; i++) if (pack.widgets[i].type === type) entry = pack.widgets[i]
        if (!entry) return false
        var merged = {}
        var defs = entry.defaults || {}
        for (var k in defs) merged[k] = defs[k]
        for (var k2 in settingsOverride) merged[k2] = settingsOverride[k2]
        card.instantiate(url.replace(/pack\.json$/, "") + entry.qml, merged)
        return true
    }
    function loadJsonUrl(url, cb) {
        var x = new XMLHttpRequest()
        x.onreadystatechange = function() {
            if (x.readyState !== 4) return
            if (x.responseText && (x.status === 200 || x.status === 0)) {
                try { cb(JSON.parse(x.responseText)); return } catch (e) {}
            }
            cb(null)
        }
        try { x.open("GET", url); x.send() } catch (e) { cb(null) }
    }

    function loadContent() {
        var type = card.layoutEntry.type || ""
        var builtin = builtinUrl(type)
        if (builtin) { instantiate(builtin, card.layoutEntry.settings || {}); return }
        if (/\.qml$/.test(type)) { instantiate(card.themeDirUrl + type, card.layoutEntry.settings || {}); return }
        if (type.indexOf(".") > 0) { loadPack(type, card.layoutEntry.settings || {}); return }
        showUnknown(type, "unrecognised widget type")
    }

    onFocusedChanged: if (content && "focused" in content) content.focused = card.focused
    onInnerChanged: if (content && "inner" in content) content.inner = card.inner
    onLiveChanged: if (content && "live" in content) content.live = card.live
    onFrameTickChanged: if (content && "frameTick" in content) content.frameTick = card.frameTick
    onThemeChanged: if (content && "theme" in content) content.theme = card.theme
    onMatChanged: if (content) { if ("mat" in content) content.mat = card.mat; if ("isLight" in content) content.isLight = card.isLight }
    onWidthChanged: if (content) content.width = contentArea.width
    onHeightChanged: if (content) content.height = contentArea.height

    Component.onCompleted: loadContent()

    MouseArea {
        anchors.fill: parent
        hoverEnabled: true
        onEntered: card.hoverFocus()
        onClicked: card.activate()
    }
}
