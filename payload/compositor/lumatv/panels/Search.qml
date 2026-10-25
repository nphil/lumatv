import QtQuick 2.4
import "../kit"
import "../kit/Spatial.js" as Spatial

// Search and the on-screen keyboard (docs/mocks/09-search-keyboard): a sheet over Home with the text so far, up to four
// answers from what is on the set (apps, Settings pages, a web search, AI access) and a keyboard the D-pad types on.
// Nothing leaves the TV: the answers are matched here against the lists the host gives (`sources`), and a web search is
// only a row that opens the browser with the text. The keyboard is the plain QWERTY of the mock: the number row, three
// letter rows, shift and backspace, "?123" (symbols), space and Done.
//
//   sources = { apps: [{ id, title, iconUrl }], pages: [{ id, title, sub }] }
// OK on a key types it, on an answer picks it (`picked(kind, id, text)`), Back closes; the mic key's voice search is the
// VoiceBar's.
Item {
    id: search
    anchors.fill: parent
    property var sources: ({ apps: [], pages: [] })
    property string text: ""
    property int index: 0                   // the focused key (keys come first), then the answers
    property bool shift: false
    property bool symbols: false
    property bool rested: true
    signal picked(string kind, string id, string text)
    signal closeRequested()
    readonly property bool built: true

    readonly property var letters: [["q","w","e","r","t","y","u","i","o","p"], ["a","s","d","f","g","h","j","k","l","'"], ["\u21e7","z","x","c","v","b","n","m",".","\u232b"]]
    readonly property var marks: [["1","2","3","4","5","6","7","8","9","0"], ["@","#","$","%","&","*","-","+","(",")"], ["\u21e7","!","?","/",":",";","\"","_",",","\u232b"]]
    readonly property var keyRows: [["1","2","3","4","5","6","7","8","9","0"]].concat(symbols ? marks.slice(1) : letters)
    readonly property var bottom: [{ k: "?123", w: 2 }, { k: "space", w: 4 }, { k: "Done", w: 2 }]
    readonly property int nKeys: 40 + 3
    // Answers: apps whose name starts with a word of the text first, then pages, then the web and AI access.
    readonly property var answers: {
        var t = text.toLowerCase().trim(), out = []
        if (t === "") return out
        var apps = sources.apps || [], pages = sources.pages || []
        for (var i = 0; i < apps.length && out.length < 2; i++) if (String(apps[i].title).toLowerCase().indexOf(t) === 0) out.push({ kind: "app", id: apps[i].id, title: apps[i].title, sub: "App \u00b7 open", iconUrl: apps[i].iconUrl, icon: "clapperboard" })
        for (var j = 0; j < pages.length && out.length < 3; j++) if (String(pages[j].title).toLowerCase().indexOf(t) === 0) out.push({ kind: "page", id: pages[j].id, title: pages[j].title, sub: pages[j].sub || "Settings", icon: "wifi" })
        out.push({ kind: "web", id: "web", title: "Web search \"" + text + "\"", sub: "Opens the browser", icon: "globe" })
        return out.slice(0, 4)
    }
    function type(c) { text += shift ? c.toUpperCase() : c; shift = false }
    function press(k) {
        if (k === "\u232b") text = text.substring(0, text.length - 1)
        else if (k === "\u21e7") shift = !shift
        else if (k === "?123") symbols = !symbols
        else if (k === "space") text += " "
        else if (k === "Done") closeRequested()
        else type(k)
    }
    function keyAt(i) {
        if (i < 40) return keyRows[Math.floor(i / 10)][i % 10]
        return bottom[i - 40].k
    }
    function open(s) { sources = s || sources; text = ""; index = 14; shift = false; symbols = false }
    // geometry for the D-pad: the key boxes, then the answers above them
    function boxes() {
        var out = []
        for (var i = 0; i < keysRep.count; i++) { var it = keysRep.itemAt(i); out.push(it ? { x: it.x, y: it.y + keys.y, w: it.width, h: it.height } : null) }
        for (var a = 0; a < ansRep.count; a++) { var at = ansRep.itemAt(a); out.push(at ? { x: at.x, y: at.y + answersRow.y, w: at.width, h: at.height } : null) }
        return out
    }
    function key(dir) {
        if (dir === "back") { closeRequested(); return }
        if (dir === "ok") {
            if (index < nKeys) press(keyAt(index))
            else { var a = answers[index - nKeys]; if (a) picked(a.kind, a.id, text) }
            return
        }
        var n = Spatial.nearest(boxes(), index, dir)
        if (n >= 0) index = n
        rested = false; restTimer.restart()
    }
    Timer { id: restTimer; interval: Lu.restMs; onTriggered: search.rested = true }

    Rectangle { anchors.fill: parent; color: Lu.scrim }
    LuPane {
        x: 64; y: 120; width: 1792; height: 840
        // the field
        Rectangle {
            id: field
            x: Lu.space(6); y: Lu.space(6); width: parent.width - 2 * Lu.space(6); height: 76; radius: Lu.radius("control")
            color: Lu.wash("selected"); border.width: 1; border.color: Lu.edge; antialiasing: true
            LuIcon { x: Lu.space(5); anchors.verticalCenter: parent.verticalCenter; name: "search"; size: 30; color: Lu.ink2 }
            LuText { x: Lu.space(5) + 50; anchors.verticalCenter: parent.verticalCenter; role: "body"; text: search.text + "|" }
            Row {
                x: parent.width - width - Lu.space(5); anchors.verticalCenter: parent.verticalCenter; spacing: Lu.space(2)
                LuIcon { anchors.verticalCenter: parent.verticalCenter; name: "mic"; size: 24; color: Lu.ink3 }
                LuText { anchors.verticalCenter: parent.verticalCenter; role: "caption"; color: Lu.ink3; text: "Hold the mic key to speak" }
            }
        }
        // the answers
        Row {
            id: answersRow
            x: Lu.space(6); y: field.y + field.height + Lu.space(4); spacing: Lu.space(3)
            Repeater {
                id: ansRep
                model: search.answers.length
                delegate: LuCard {
                    readonly property var a: search.answers[index]
                    width: Math.floor((parent.parent.width - 2 * Lu.space(6) - 3 * Lu.space(3)) / 4)
                    icon: a.icon; title: a.title; sub: a.sub
                    focused: search.index === search.nKeys + index
                    rested: search.rested
                    onHoverFocus: search.index = search.nKeys + index
                    onActivated: { search.index = search.nKeys + index; search.key("ok") }
                }
            }
        }
        // the keyboard
        Item {
            id: keys
            x: Math.round((parent.width - 1180) / 2); y: answersRow.y + Math.round(Lu.row * 0.95) + Lu.space(6); width: 1180; height: 5 * 84
            Repeater {
                id: keysRep
                model: search.nKeys
                delegate: Item {
                    readonly property string label: search.keyAt(index)
                    readonly property int row: index < 40 ? Math.floor(index / 10) : 4
                    readonly property int bx: index < 40 ? (index % 10) : (index === 40 ? 0 : index === 41 ? 2 : 6)
                    readonly property int bw: index < 40 ? 1 : search.bottom[index - 40].w
                    x: bx * 118; y: row * 84; width: bw * 118 - 10; height: 74
                    readonly property bool focused: search.index === index
                    FocusLift { id: lift; focused: parent.focused; rested: search.rested; reduced: Lu.reduced; duration: Lu.cardMs; easing: Lu.cardEasing }
                    scale: 1 + (Lu.buttonScale - 1) * lift.value
                    transformOrigin: Item.Center
                    z: focused ? 1 : 0
                    Rectangle { anchors.fill: parent; radius: Lu.radius("control"); color: parent.focused ? Lu.glassRaised : Lu.glass; border.width: 1; border.color: parent.focused ? Lu.edgeRaised : Lu.edge; antialiasing: true }
                    LuText { anchors.centerIn: parent; role: "label"; text: parent.label === "\u232b" || parent.label === "\u21e7" ? "" : (search.shift && parent.label.length === 1 ? parent.label.toUpperCase() : parent.label) }
                    LuIcon { visible: parent.label === "\u232b"; anchors.centerIn: parent; name: "delete"; size: 32; color: Lu.ink }
                    LuIcon { visible: parent.label === "\u21e7"; anchors.centerIn: parent; name: "arrow-big-up"; size: 32; color: search.shift ? Lu.accent : Lu.ink }
                    MouseArea { anchors.fill: parent; hoverEnabled: true; onEntered: search.index = index; onClicked: { search.index = index; search.key("ok") } }
                }
            }
        }
    }
}
