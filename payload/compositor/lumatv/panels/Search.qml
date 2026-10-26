import QtQuick 2.4
import "../kit"
import "../kit/Spatial.js" as Spatial
import "../kit/Match.js" as Match

// Search and the on-screen keyboard (docs/mocks/09-search-keyboard): a sheet over Home with the text so far, up to four
// answers from what is on the set (apps, Settings pages, a web search) and a keyboard the D-pad types on. Nothing leaves
// the TV: the answers are matched here against the lists the host gives (kit/Match.js), and a web search is only a row
// that opens the browser with the text. The keyboard is the plain QWERTY of the mock: the number row, three letter rows,
// shift and backspace, "?123" (symbols), space and Done.
//
// It has two uses, the same keyboard in both:
//   search  `open({ sources: { apps: [{ id, title, iconUrl }], pages: [{ id, title, sub, icon }] }, voice })`: OK on a key
//           types it, on an answer picks it (`picked(kind, id, text)`); Done and Back close.
//   entry   `open({ mode: "entry", prompt: "Rename Netflix", text: "Netflix" })`: a name is typed; Done says
//           `entered(text)`, Back closes without it. No answers.
// `closeRequested()` follows every end, so the host closes the layer in one place. The mic hint shows only when `voice` is
// set, which is when something can hear (the voice bar is the recogniser's).
Item {
    id: search
    anchors.fill: parent
    property var sources: ({ apps: [], pages: [] })
    property string mode: "search"
    property string prompt: ""
    property bool voice: false
    property string text: ""
    property int index: 14                  // the focused key (keys come first), then the answers
    property bool shift: false
    property bool symbols: false
    property bool rested: true
    signal picked(string kind, string id, string text)
    signal entered(string text)
    signal closeRequested()
    readonly property bool built: true
    readonly property bool entry: mode === "entry"

    readonly property var letters: [["q","w","e","r","t","y","u","i","o","p"], ["a","s","d","f","g","h","j","k","l","'"], ["\u21e7","z","x","c","v","b","n","m",".","\u232b"]]
    readonly property var marks: [["1","2","3","4","5","6","7","8","9","0"], ["@","#","$","%","&","*","-","+","(",")"], ["\u21e7","!","?","/",":",";","\"","_",",","\u232b"]]
    readonly property var keyRows: [["1","2","3","4","5","6","7","8","9","0"]].concat(symbols ? marks.slice(1) : letters)
    // the bottom row: its three keys' left edge and width in the keyboard
    readonly property var lastRow: [{ k: "?123", x: 0, w: 178 }, { k: "space", x: 188, w: 554 }, { k: "Done", x: 752, w: 178 }]
    readonly property int nKeys: 40 + 3
    readonly property var answers: entry ? [] : Match.answers(text, sources.apps || [], sources.pages || [])

    function type(c) { text += shift ? c.toUpperCase() : c; shift = false }
    function press(k) {
        if (k === "\u232b") text = text.substring(0, text.length - 1)
        else if (k === "\u21e7") shift = !shift
        else if (k === "?123") symbols = !symbols
        else if (k === "space") text += " "
        else if (k === "Done") { if (entry) entered(text); closeRequested() }
        else type(k)
    }
    function keyAt(i) {
        if (i < 40) return keyRows[Math.floor(i / 10)][i % 10]
        return lastRow[i - 40].k
    }
    function open(spec) {
        var s = spec || {}
        sources = s.sources || sources
        mode = s.mode || "search"
        prompt = s.prompt || ""
        voice = !!s.voice
        text = s.text || ""
        index = 14; shift = false; symbols = false
    }
    // geometry for the D-pad, in the sheet's own coordinates: the keys, then the answers above them
    function boxes() {
        var out = []
        for (var i = 0; i < keysRep.count; i++) { var it = keysRep.itemAt(i); out.push(it ? { x: it.x + keys.x, y: it.y + keys.y, w: it.width, h: it.height } : null) }
        for (var a = 0; a < ansRep.count; a++) { var at = ansRep.itemAt(a); out.push(at ? { x: at.x + answersRow.x, y: at.y + answersRow.y, w: at.width, h: at.height } : null) }
        return out
    }
    function key(dir) {
        if (dir === "back") { closeRequested(); return }
        if (dir === "ok") {
            if (index < nKeys) press(keyAt(index))
            else { var a = answers[index - nKeys]; if (a) { picked(a.kind, a.id, text); closeRequested() } }
            return
        }
        var n = Spatial.nearest(boxes(), index, dir)
        if (n >= 0) index = n
        rested = false; restTimer.restart()
    }
    Timer { id: restTimer; interval: Lu.restMs; onTriggered: search.rested = true }

    Rectangle { anchors.fill: parent; color: Lu.scrim }
    LuPane {
        id: sheet
        x: 64; width: 1792; height: 704; y: 1080 - 56 - height
        cardRadius: Lu.radius("sheet")

        // ---- the field: what has been typed, the caret, and what could be said instead
        Rectangle {
            id: field
            x: 40; y: 28; width: parent.width - 80; height: 84; radius: Lu.radius("tile")
            color: Qt.rgba(Lu.ink.r, Lu.ink.g, Lu.ink.b, 0.06); border.width: 1; border.color: Lu.edge; antialiasing: true
            LuIcon { id: lens; x: 26; anchors.verticalCenter: parent.verticalCenter; name: search.entry ? "keyboard" : "search"; size: 32; color: Lu.ink2 }
            LuText { id: typed; x: 26 + 32 + 18; anchors.verticalCenter: parent.verticalCenter; role: "title"; font.pixelSize: 32; text: search.text }
            Rectangle { x: typed.x + typed.width + 4; anchors.verticalCenter: parent.verticalCenter; width: 3; height: 38; radius: 2; color: Lu.accent }
            Row {
                x: parent.width - width - 26; anchors.verticalCenter: parent.verticalCenter; spacing: Lu.space(2)
                visible: search.voice || search.entry
                LuIcon { visible: !search.entry; anchors.verticalCenter: parent.verticalCenter; name: "mic"; size: 26; color: Lu.ink3 }
                LuText { anchors.verticalCenter: parent.verticalCenter; role: "caption"; color: Lu.ink3; text: search.entry ? "Back cancels" : "Hold the mic key to speak" }
            }
        }

        // ---- the answers (search) or what is being named (entry)
        Row {
            id: answersRow
            x: 40; y: 134; spacing: 16
            visible: !search.entry
            Repeater {
                id: ansRep
                model: search.answers.length
                delegate: Item {
                    id: ans
                    readonly property var a: search.answers[index]
                    readonly property bool focused: search.index === search.nKeys + index
                    width: Math.floor((sheet.width - 80 - 3 * 16) / 4); height: 108
                    z: focused ? 1 : 0
                    FocusLift { id: alift; focused: ans.focused; rested: search.rested; reduced: Lu.reduced; duration: Lu.cardMs; easing: Lu.cardEasing }
                    Rectangle {
                        anchors.fill: parent; radius: Lu.radius("tile"); antialiasing: true
                        color: ans.focused ? Lu.wash("focus") : Qt.rgba(Lu.ink.r, Lu.ink.g, Lu.ink.b, 0.06)
                        border.width: 1; border.color: ans.focused ? Lu.edgeRaised : "transparent"
                        scale: 1 + (Lu.buttonScale - 1) * alift.value; transformOrigin: Item.Center      // the plate lifts; the words stay put
                    }
                    Rectangle {
                        id: lead
                        x: 22; anchors.verticalCenter: parent.verticalCenter; width: 60; height: 60; radius: 16; antialiasing: true
                        color: ans.a.iconUrl ? "transparent" : Lu.wash("selected")
                        border.width: ans.a.iconUrl ? 0 : 1; border.color: Lu.edge
                        Image { visible: !!ans.a.iconUrl; anchors.fill: parent; source: ans.a.iconUrl || ""; fillMode: Image.PreserveAspectFit; asynchronous: true; cache: true; smooth: true }
                        LuIcon { visible: !ans.a.iconUrl; anchors.centerIn: parent; name: ans.a.icon || ""; size: 32; color: Lu.ink }
                    }
                    Column {
                        x: lead.x + lead.width + 18; anchors.verticalCenter: parent.verticalCenter; width: parent.width - x - 22; spacing: Lu.space(1)
                        LuText { width: parent.width; role: "label"; font.weight: Font.DemiBold; elide: Text.ElideRight; textFormat: Text.StyledText
                                 color: ans.a.match ? Lu.ink2 : Lu.ink; text: Match.highlight(ans.a.title, ans.a.match, String(Lu.ink)) }      // the typed part in full ink: the accent is too pale on the light sheet
                        LuText { width: parent.width; role: "caption"; color: Lu.ink2; elide: Text.ElideRight; text: ans.a.sub || "" }
                    }
                    MouseArea { anchors.fill: parent; hoverEnabled: true; onEntered: search.index = search.nKeys + index; onClicked: { search.index = search.nKeys + index; search.key("ok") } }
                }
            }
        }
        Item {
            visible: search.entry
            x: 40; y: 134; width: parent.width - 80; height: 108
            Rectangle { anchors.fill: parent; radius: Lu.radius("tile"); color: Qt.rgba(Lu.ink.r, Lu.ink.g, Lu.ink.b, 0.06); antialiasing: true }
            LuTile { x: 22; anchors.verticalCenter: parent.verticalCenter; size: 60; icon: "keyboard" }
            Column {
                x: 22 + 60 + 18; anchors.verticalCenter: parent.verticalCenter; width: parent.width - x - 22; spacing: Lu.space(1)
                LuText { width: parent.width; role: "label"; font.weight: Font.DemiBold; elide: Text.ElideRight; text: search.prompt }
                LuText { width: parent.width; role: "caption"; color: Lu.ink2; text: "Done saves the name here and on the dock. An empty name brings back the one LG gave." }
            }
        }

        // ---- the keyboard
        Item {
            id: keys
            x: Math.round((sheet.width - 930) / 2); y: 272; width: 930; height: 400
            Repeater {
                id: keysRep
                model: search.nKeys
                delegate: Item {
                    id: keyItem
                    readonly property string label: search.keyAt(index)
                    readonly property bool wide: index >= 40
                    x: index < 40 ? (index % 10) * 94 : search.lastRow[index - 40].x
                    y: (index < 40 ? Math.floor(index / 10) : 4) * 82
                    width: index < 40 ? 84 : search.lastRow[index - 40].w; height: 72
                    readonly property bool focused: search.index === index
                    FocusLift { id: lift; focused: keyItem.focused; rested: search.rested; reduced: Lu.reduced; duration: Lu.cardMs; easing: Lu.cardEasing }
                    z: focused ? 1 : 0
                    Rectangle { anchors.fill: parent; radius: Lu.radius("control"); color: parent.focused ? Lu.glassRaised : Lu.glass; border.width: 1; border.color: parent.focused ? Lu.edgeRaised : Lu.edge; antialiasing: true
                                scale: 1 + (Lu.buttonScale - 1) * lift.value; transformOrigin: Item.Center }      // the key lifts; its glyph stays put
                    LuText {
                        anchors.centerIn: parent; role: parent.wide ? "label" : "body"; font.weight: parent.wide ? Font.DemiBold : Font.Medium
                        text: parent.label === "\u232b" || parent.label === "\u21e7" ? "" : parent.label === "?123" ? (search.symbols ? "abc" : "?123")
                              : (search.shift && parent.label.length === 1 ? parent.label.toUpperCase() : parent.label)
                    }
                    LuIcon { visible: parent.label === "\u232b"; anchors.centerIn: parent; name: "delete"; size: 30; color: Lu.ink }
                    LuIcon { visible: parent.label === "\u21e7"; anchors.centerIn: parent; name: "arrow-big-up"; size: 30; color: search.shift ? Lu.accent : Lu.ink }
                    MouseArea { anchors.fill: parent; hoverEnabled: true; onEntered: search.index = index; onClicked: { search.index = index; search.key("ok") } }
                }
            }
        }
    }
}
