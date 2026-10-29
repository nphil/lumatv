import QtQuick 2.4
import "../kit"
import "KeyboardLayouts.js" as Kb

// LumaTV's own on-screen keyboard (docs/research/keyboard-result.md): a sheet along the bottom of the system layer that a text field's app
// gets instead of LG's maliit while `lumatv keyboard luma` is on and this client is live. It is drawn here and only drawn: the compositor
// (server/ime.rs) holds the text protocol, tells the shell which field is asking (`field`, the `ime` topic) and hands over the remote's
// navigation keys (`imeKey`, the `imekey` events), and what is typed leaves as signals the shell posts back (compositor/shell/Shell.qml).
//
// What a field gets is KeyboardLayouts.js's: the letters, the e-mail and url letters, the symbols or a number pad, with the field's enter key
// as the one accent key. The look is the one of LG's keys in the Lucent restyle (tools/maliit_patch.py): an opaque sheet of the material's
// glass, key plates one hairline wash above it, the function keys one wash below, the focus a lit plate with a ring in the ink and the same
// ink, the accent on the enter key only. Nothing typed is ever drawn here, in any field.
//
// Cost: the plates come first, the one focus plate over them, then every label and picture over that, so that the renderer can batch the
// plates and the words and the plate that moves is the only thing that changes when the remote does (no cached texture: a focus move would draw
// it again). The 51 plates, 51 labels and 8 pictures a layout needs are made once and a layout change rebinds them. Hidden, the whole item is
// invisible and nothing in it runs (`busy` is false); the shell reports `live` false and the compositor draws no frame for the layer. What the
// frames cost on the set is the gate's to say (docs/research/keyboard-result.md).
//
// Motion: the sheet slides from the bottom edge in 110 ms and back (panels/PanelMotion.qml, a transform, nothing laid out while it moves). The
// focus plate moves at once; after 150 ms of rest it lifts (kit/FocusLift.qml), and a key pressed shrinks it for 90 ms. Reduced motion: a
// fade, no lift, no shrink.
Item {
    id: kb
    anchors.fill: parent
    visible: live

    // ---- what the shell hands in
    property var field: ({})                // the `ime` topic: {active, show, model, hint, purpose, enter, max, text}
    property bool voiceOk: false            // LG's voice can be asked (system.supportVoiceRecognition)
    property string voiceNote: ""           // words on the space bar while LG's voice is asked: "Listening", "Working", "Voice isn't available"
    property bool listening: false          // the microphone is open: Back ends that and not the keyboard

    // ---- what it hands out
    signal typed(string text)               // characters for the field
    signal fieldAction(string name)         // "backspace", "enter", "left", "right", "clear"
    signal hideAsked()                      // Back, or a key that finishes the field: the keyboard goes away, the field stays
    signal voiceAsked()
    signal voiceCancelled()
    signal shownAt(int x, int y, int w, int h)   // where the sheet is on the screen (it is on its way in)

    // ---- the field
    readonly property bool wanted: field.active === true && field.show === true
    readonly property int fieldId: field.model | 0
    readonly property int fPurpose: field.purpose | 0
    readonly property int fHint: field.hint | 0
    readonly property int fEnter: field.enter | 0
    readonly property bool hasText: field.text === true
    readonly property string baseId: Kb.layoutFor(fPurpose, fHint)

    property bool symbols: false
    property int shift: 0                   // 0 off, 1 one capital, 2 caps lock
    property int focusAt: 0                 // the key with the focus
    property real column: 0                 // the column the focus came to, kept while it only moves up and down
    property bool rested: true              // the focus has stood still for Lu.restMs
    property var gate: ({})                 // the remote's repeats (KeyboardLayouts.js gate)
    readonly property string layoutId: symbols && !Kb.isPad(baseId) ? "symbols" : baseId
    readonly property var layout: Kb.build(layoutId, { enter: fEnter, voice: voiceOk })

    // ---- motion: the sheet is on its way in or out, or at rest
    PanelMotion { id: motion; presented: kb.wanted; lingerMs: 0 }
    readonly property bool live: motion.presented || motion.leaving
    readonly property bool settled: motion.settled
    readonly property real rise: Lu.reduced ? 0 : (sheetH + 64) * (1 - motion.progress)

    // ---- geometry: 1080p pixels, the TV profile (target 64 and up, the safe area 54 at the foot)
    readonly property real tscale: Math.max(1, Math.min(1.5, Lu.type("title").size / 40))      // the sheet grows with larger text, to a point
    readonly property bool padded: layout.pad
    readonly property int gap: padded ? 12 : 10
    readonly property int unit: padded ? Math.min(Math.round(176 * tscale), 220) : Math.min(Math.round(104 * tscale), 134)
    readonly property int keyH: Math.round((padded ? 84 : 68) * tscale)
    readonly property var grid: Kb.rects(layout, unit, keyH, gap)
    readonly property int topPad: Lu.space(5)
    readonly property int bottomPad: 54
    readonly property int sheetW: padded ? grid.width + 128 : width
    readonly property int sheetH: topPad + grid.height + bottomPad
    readonly property int gridX: Math.round((sheetW - grid.width) / 2)
    readonly property int keyRadius: Lu.radius("row")
    readonly property int charSize: Math.round(Lu.type("title").size * (padded ? 1.2 : 1))
    readonly property int wordSize: Lu.type("label").size
    readonly property var focusRect: focusAt >= 0 && focusAt < grid.rects.length ? grid.rects[focusAt] : null
    readonly property var focusKey: focusAt >= 0 && focusAt < layout.keys.length ? layout.keys[focusAt] : null

    // ---- colour: opaque roles of the material, as the restyle of LG's keys computes them (tools/maliit_patch.py roles)
    function over(a, b, alpha) { return Qt.rgba(b.r * alpha + a.r * (1 - alpha), b.g * alpha + a.g * (1 - alpha), b.b * alpha + a.b * (1 - alpha), 1) }
    readonly property color sheetC: Qt.rgba(Lu.glass.r, Lu.glass.g, Lu.glass.b, 1)
    readonly property color washC: Lu.isLight ? "#000000" : "#ffffff"
    readonly property real hair: Lu.isLight ? 0.12 : Math.min(0.22, Math.max(0.10, Lu.edge.a))
    readonly property color tileC: over(sheetC, washC, hair)
    readonly property color tileEdgeC: over(tileC, washC, hair)
    readonly property color fnC: over(sheetC, over(sheetC, washC, 0.10), 0.35)
    readonly property color litC: over(sheetC, washC, Lu.isLight ? 0.22 : 0.28)
    function plateColor(k) { return !k ? "transparent" : k.accent ? Lu.accent : k.fn ? fnC : tileC }
    function edgeColor(k) { return !k ? "transparent" : k.accent ? Lu.accent : k.fn ? tileC : tileEdgeC }
    function inkOf(k) { return k && k.accent ? Lu.accentInk : Lu.ink }
    function enabled(k) { return !k || !k.needsText || hasText }

    // ---- what a key does
    function centreOf(i) { var k = layout.keys[i]; return k ? k.col + k.span / 2 : 0 }
    function rest() { rested = false; restTimer.restart() }
    function setFocus(i) {
        if (i < 0 || i >= layout.keys.length) return
        focusAt = i
        column = centreOf(i)
        rest()
    }
    function move(dir) {
        var m = Kb.move(layout, focusAt, dir, column)
        focusAt = m.index
        column = m.want
        rest()
    }
    function flash() { if (!Lu.reduced) pressAnim.restart() }
    // `i` pressed: the model says what happens, this carries it out
    function activate(i) {
        var k = layout.keys[i]
        if (!k || !enabled(k)) return
        flash()
        var e = Kb.press(layout, i, { shift: shift })
        if (e.mode !== undefined) {
            var was = layout.keys[i]
            symbols = e.mode === "symbols"
            var j = Kb.at(layout, was.row, was.col)
            setFocus(j >= 0 ? j : Kb.home(layout))
            return
        }
        if (e.shift !== undefined) shift = e.shift
        if (e.text !== undefined) typed(e.text)
        if (e.action !== undefined) fieldAction(e.action)
        if (e.voice === true) { if (listening) voiceCancelled(); else voiceAsked() }
        if (e.hide === true) hideAsked()
    }
    function back() {
        if (listening) voiceCancelled()
        else hideAsked()
    }
    // A field opens: the first layout, the first focus. From the topic itself, not from the bindings that follow it (they may not have caught up
    // when the change that calls this is announced).
    function begin() {
        var id = Kb.layoutFor(fPurpose, fHint)
        var lay = Kb.build(id, { enter: fEnter, voice: voiceOk })
        symbols = false
        shift = Kb.initialShift(fPurpose, fHint)
        focusAt = Kb.home(lay)
        column = lay.keys[focusAt].col + lay.keys[focusAt].span / 2
        rested = true
        restTimer.stop()
        gate = ({})
        Qt.callLater(announce)
    }
    function announce() { if (wanted) shownAt(Math.round((width - sheetW) / 2), height - sheetH, sheetW, sheetH) }
    onWantedChanged: { if (wanted) begin(); else restTimer.stop() }
    onFieldIdChanged: if (wanted) begin()
    onBaseIdChanged: if (wanted) begin()
    onSheetHChanged: if (wanted) Qt.callLater(announce)
    onSheetWChanged: if (wanted) Qt.callLater(announce)

    // The remote: a key of the compositor's `imekey` events. The arrows move, OK presses the key, a digit types itself, Back puts the keyboard
    // away. The press acts (a keyboard's key is its press; the release was taken too, so none reaches the app), the remote's repeats only for
    // the keys that repeat and at the keyboard's own pace.
    readonly property var repeatCfg: ({ delay: Lu.focus.repeatDelay || 300, interval: Lu.focus.repeatInterval || 100, fast: 60, fastAfter: 8 })
    function imeKey(name, down, repeat) {
        if (down) Lu.pointerHide()
        if (!wanted) return
        if (name.length === 1 && name >= "0" && name <= "9") {
            if (down && !repeat) { flash(); typed(name) }
            return
        }
        if (!down) return
        if (name === "back") { if (!repeat) back(); return }
        if (repeat && !Kb.repeats(name, layout, focusAt)) return
        if (!Kb.gate(gate, name, repeat, Date.now(), repeatCfg)) return
        if (name === "ok") activate(focusAt)
        else move(name)
    }

    // The pointer (Magic Remote): hover shows the key under it (`hoverAt`, drawn as the hover cue) and moves nothing; a press moves the
    // focus, and a press that is let go on the same key types it.
    property int downAt: -1
    property int hoverAt: -1
    function pointerHover(x, y) { hoverAt = Kb.hit(grid, x, y) }
    function pointerDown(x, y) {
        downAt = Kb.hit(grid, x, y)
        if (downAt >= 0) setFocus(downAt)
    }
    function pointerUp(x, y) {
        var i = Kb.hit(grid, x, y)
        if (i >= 0 && i === downAt) activate(i)
        downAt = -1
    }

    Timer { id: restTimer; interval: Lu.restMs; onTriggered: kb.rested = true }
    property real pressK: 1
    NumberAnimation { id: pressAnim; target: kb; property: "pressK"; from: Lu.pressScale; to: 1; duration: Lu.pressMs; easing.type: Easing.OutCubic }
    FocusLift { id: lift; focused: kb.wanted && kb.settled; rested: kb.rested; reduced: Lu.reduced; duration: Lu.focusMs; easing: Lu.focusEasing }

    // ---- the sheet
    Item {
        id: sheet
        x: Math.round((kb.width - kb.sheetW) / 2)
        y: kb.height - kb.sheetH + kb.rise
        width: kb.sheetW
        height: kb.sheetH
        opacity: Lu.reduced ? motion.progress : 1

        // the face: its top corners are the sheet's, the bottom ones and (full width) the sides are off the screen
        Rectangle {
            x: kb.padded ? 0 : -4
            y: 0
            width: parent.width + (kb.padded ? 0 : 8)
            height: parent.height + radius
            radius: Lu.radius("sheet")
            color: kb.sheetC
            border.width: 1
            border.color: Lu.edge
        }

        // the plates
        Repeater {
            model: Kb.MAX_KEYS
            delegate: Rectangle {
                readonly property var k: index < kb.layout.keys.length ? kb.layout.keys[index] : null
                readonly property var r: index < kb.grid.rects.length ? kb.grid.rects[index] : null
                visible: k !== null
                x: kb.gridX + (r ? r.x : 0)
                y: kb.topPad + (r ? r.y : 0)
                width: r ? r.w : 0
                height: r ? r.h : 0
                radius: kb.keyRadius
                color: kb.plateColor(k)
                border.width: 1
                border.color: kb.edgeColor(k)
            }
        }

        // the key under the pointer (not the focused one: that is the focus plate)
        Item {
            readonly property var r: kb.hoverAt >= 0 && kb.hoverAt < kb.grid.rects.length ? kb.grid.rects[kb.hoverAt] : null
            x: kb.gridX + (r ? r.x : 0)
            y: kb.topPad + (r ? r.y : 0)
            width: r ? r.w : 0
            height: r ? r.h : 0
            LuHover { on: ptr.hovered && r !== null && kb.hoverAt !== kb.focusAt && kb.enabled(kb.layout.keys[kb.hoverAt]); radius: kb.keyRadius }
        }

        // the focus: one plate that moves; the accent key keeps its fill, the others are lit; the ring is the ink
        Rectangle {
            readonly property var r: kb.focusRect
            visible: kb.wanted && r !== null
            x: kb.gridX + (r ? r.x : 0)
            y: kb.topPad + (r ? r.y : 0)
            width: r ? r.w : 0
            height: r ? r.h : 0
            radius: kb.keyRadius
            color: kb.focusKey && kb.focusKey.accent ? Lu.accent : kb.litC
            border.width: 3
            border.color: Lu.ink
            transformOrigin: Item.Center
            scale: kb.pressK * (1 + (Lu.buttonScale - 1) * lift.value)
        }

        // the words of the keys (characters, function words), over the focus plate
        Repeater {
            model: Kb.MAX_KEYS
            delegate: Text {
                readonly property var k: index < kb.layout.keys.length ? kb.layout.keys[index] : null
                readonly property var r: index < kb.grid.rects.length ? kb.grid.rects[index] : null
                visible: k !== null && !k.icon && k.kind !== "space" && text !== ""
                // function words keep Lucent's 8 px of air from the key's sides (a larger type shrinks into the key, never to its edge)
                x: kb.gridX + (r ? r.x + (k && k.kind !== "char" ? 8 : 0) : 0)
                y: kb.topPad + (r ? r.y : 0)
                width: r ? r.w - (k && k.kind !== "char" ? 16 : 0) : 0
                height: r ? r.h : 0
                text: k ? Kb.labelOf(kb.layout, index, kb.shift) : ""
                color: kb.inkOf(k)
                opacity: kb.enabled(k) ? 1 : Lu.disabledOpacity
                font.family: Lu.family
                font.weight: Font.Medium
                font.pixelSize: k && k.kind !== "char" ? kb.wordSize : kb.charSize
                fontSizeMode: k && k.kind !== "char" ? Text.Fit : Text.FixedSize
                minimumPixelSize: 12
                horizontalAlignment: Text.AlignHCenter
                verticalAlignment: Text.AlignVCenter
                renderType: Text.NativeRendering
            }
        }

        // the pictures of the keys that have one (delete, shift, the enter arrow, microphone, hide, the cursor)
        Repeater {
            model: Kb.MAX_ICONS
            delegate: LuIcon {
                readonly property int ki: index < kb.iconKeys.length ? kb.iconKeys[index] : -1
                readonly property var k: ki >= 0 ? kb.layout.keys[ki] : null
                readonly property var r: ki >= 0 ? kb.grid.rects[ki] : null
                visible: k !== null
                name: k ? k.icon : ""
                size: Math.round(32 * kb.tscale)
                x: kb.gridX + (r ? r.x + Math.round((r.w - size) / 2) : 0)
                y: kb.topPad + (r ? r.y + Math.round((r.h - size) / 2) : 0)
                color: kb.inkOf(k)
                opacity: kb.listening && k && k.kind === "voice" ? 1 : kb.enabled(k) ? 1 : Lu.disabledOpacity
            }
        }

        // Shift's state: a dot when one capital is waiting, a dot and a bar for caps lock (the ink, so it reads on every plate)
        Rectangle {
            readonly property int si: kb.layout.shiftable ? kb.indexOfId("shift") : -1
            readonly property var r: si >= 0 ? kb.grid.rects[si] : null
            visible: r !== null && kb.shift > 0
            x: kb.gridX + (r ? r.x + 12 : 0); y: kb.topPad + (r ? r.y + 10 : 0)
            width: 10; height: 10; radius: 5
            color: Lu.ink
        }
        Rectangle {
            readonly property int si: kb.layout.shiftable ? kb.indexOfId("shift") : -1
            readonly property var r: si >= 0 ? kb.grid.rects[si] : null
            visible: r !== null && kb.shift === 2
            x: kb.gridX + (r ? r.x + Math.round((r.w - 36) / 2) : 0); y: kb.topPad + (r ? r.y + r.h - 16 : 0)
            width: 36; height: 4; radius: 2
            color: Lu.ink
        }

        // the space bar: a short line, or what LG's voice is doing
        Rectangle {
            readonly property int si: kb.indexOfId("space")
            readonly property var r: si >= 0 ? kb.grid.rects[si] : null
            visible: r !== null && kb.voiceNote === ""
            x: kb.gridX + (r ? r.x + Math.round((r.w - 96) / 2) : 0); y: kb.topPad + (r ? r.y + Math.round(r.h / 2) + 6 : 0)
            width: 96; height: 4; radius: 2
            color: Lu.ink2
        }
        Text {
            readonly property int si: kb.indexOfId("space")
            readonly property var r: si >= 0 ? kb.grid.rects[si] : null
            visible: r !== null && kb.voiceNote !== ""
            x: kb.gridX + (r ? r.x : 0); y: kb.topPad + (r ? r.y : 0)
            width: r ? r.w : 0; height: r ? r.h : 0
            text: kb.voiceNote
            color: Lu.ink
            font.family: Lu.family
            font.weight: Font.Medium
            font.pixelSize: kb.wordSize
            fontSizeMode: Text.Fit
            minimumPixelSize: 12
            horizontalAlignment: Text.AlignHCenter
            verticalAlignment: Text.AlignVCenter
            renderType: Text.NativeRendering
        }

        LuPointer {
            id: ptr
            x: kb.gridX; y: kb.topPad
            width: kb.grid.width; height: kb.grid.height
            acceptedButtons: Qt.LeftButton
            onPositionChanged: kb.pointerHover(mouse.x, mouse.y)
            onPressed: kb.pointerDown(mouse.x, mouse.y)
            onReleased: kb.pointerUp(mouse.x, mouse.y)
        }
    }

    readonly property var iconKeys: {
        var out = []
        for (var i = 0; i < layout.keys.length; i++) if (layout.keys[i].icon) out.push(i)
        return out
    }
    function indexOfId(id) {
        for (var i = 0; i < layout.keys.length; i++) if (layout.keys[i].id === id) return i
        return -1
    }

    // What the shell reads: this panel can be shown (the compositor only serves a field here once the client says so)
    readonly property bool ready: true
    // Anything of this panel that is running: nothing is while the keyboard is hidden (the gate and the host test read it)
    readonly property bool busy: restTimer.running || pressAnim.running || lift.anim.running || motion.moving
}
