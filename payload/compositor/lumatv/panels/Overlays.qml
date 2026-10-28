import QtQuick 2.4
import "../kit"
import "../Overlays.js" as Q

// The overlay system (docs/research/overlays-result.md, docs/mocks/overlays): what wants the person's eye over whatever plays, in one column of
// cards in the top right corner. A Resident of LumaOverlay, the layer that is up over Home and over every app, behind the switch `overlays.enabled`
// of theme.json (off until its TV check passes).
//
//   sources   the plugin broker's records, the ones LumaShow's devices draw too: a notification (`PluginHub.posted`, a new one) is a card; a
//             card at the `ringing` priority is one while it exists; the plugin ending its record takes the card away. What a record is (a
//             doorbell, an alarm, a timer, an alert, a score) is told from what the plugin calls its channel and icon, or from an explicit
//             `kind`; the doorbell's other source is the camera pop-up (panels/Cameras.qml), which is its own card: this surface only makes room
//             for it (`cameraUp`, `cameraBottom`) and takes in a ring that both of them say (Overlays.js `cameraShown`).
//   queue     Overlays.js: at most two cards, the most important class first (doorbell, alarm and timer; then an alert; then a score), none
//             below 75 % of the height, none while the host is busy (`dnd`: a PIN, an alert, the keyboard, a panel), each for its class's
//             time; one a person dismissed is not shown again until its record changes.
//   keys      OK acts (the record's first button, else the notification drawer opens) and Back dismisses, the top card's, once it has been up a
//             moment (`armed`); every other key is the app's. The layer asks the compositor for just those two keys while a card is up and
//             armed (LumaOverlay `takes`), so the app keeps the keyboard focus and the arrows.
//
// Nothing runs while no card is up and nothing waits: one timer, started for the next moment something changes, stopped otherwise. The two cards
// and the probe the heights are measured on are built at warm-up (LumaOverlay), never in a frame a card arrives in.
Item {
    id: ovs
    anchors.fill: parent

    property bool appFront: false             // an app's picture is under this layer: the glass lets it through to the reading alpha
    property bool dnd: false                  // the host is busy: nothing shows, what waits is kept
    property bool cameraUp: false             // the camera pop-up is up: it has a slot and the top of the column
    property real cameraBottom: 0             // and ends here
    property bool drawerReady: false          // OK can open the notification drawer here
    property bool warming: false              // the shell is warming its layer: the first card is drawn once with letters, nothing is shown
    signal openDrawer()

    readonly property var cfg: Q.config(Lu.theme ? Lu.theme.overlays : null)
    readonly property bool enabled: cfg.enabled
    property var queue: Q.create()
    property var shown: []                    // the cards on the screen, top first: snapshots of the queue's entries
    property string shownSig: ""
    property var held0: null                  // what each slot showed last, so a card fades out with its words
    property var held1: null
    property bool armed: false                // the top card has been up long enough for the keys to be its
    readonly property bool up: shown.length > 0
    readonly property bool live: up || slot0.visible || slot1.visible          // a card is drawn, or is fading out (the warm-up's is no card)
    property bool pending: false              // something waits or shows (the queue is not empty)
    readonly property real paneAlpha: appFront ? Lu.readingAlpha : 1
    readonly property real column: Math.round(width - cfg.right - cfg.width)
    // where the first of the queue's cards starts: the top of the column, or under the camera's card
    readonly property real firstY: cameraUp ? Math.round(cameraBottom + cfg.gap) : cfg.top
    readonly property var warmEntry: ({ key: "warm", cls: "alert", title: "The quick brown fox jumps over the lazy dog 0123456789", body: "THE QUICK BROWN FOX JUMPS OVER THE LAZY DOG - abcdefghijklmnopqrstuvwxyz : , . ! ? ' ( ) /",
                                         source: "Home Assistant OK Back Open Dismiss Clear Live", tone: "accent", actions: [], score: null, icon: "circle-alert", iconName: "" })

    // PluginHub hands new notifications here instead of toasting them while this surface is on
    Binding { target: PluginHub; property: "overlays"; value: ovs.enabled }
    readonly property bool hubOwns: PluginHub.overlays          // for the gate and the host's tests

    // ---- the sources
    function iconFor(e) {
        if (!e) return ""
        if (e.cls === "alert" && e.iconName !== "") {
            var n = PluginHub.icon(e.iconName)
            if (n !== "plug") return n
        }
        return e.icon
    }
    function take(entry) {
        if (!entry) return
        Q.offer(queue, entry, cfg, Date.now())
        refresh()
    }
    Connections {
        target: PluginHub
        // a notification that is news (PluginHub.announce: not one that was already there when the helper came up)
        onPosted: if (ovs.enabled) ovs.take(Q.fromNote(note, ovs.cfg, Date.now()))
        // a record the plugin ended (or that expired) takes its card with it
        onNotesChanged: ovs.reconcile()
        onCardsChanged: ovs.syncCards()
    }
    function reconcile() {
        if (!enabled) return
        var present = {}
        for (var i = 0; i < PluginHub.notes.length; i++) present["n:" + PluginHub.notes[i].pluginId + "/" + PluginHub.notes[i].id] = true
        var gone = false
        for (var j = queue.list.length - 1; j >= 0; j--) {
            var e = queue.list[j]
            if (e.src === "note" && !present[e.key]) { Q.withdraw(queue, e.key); gone = true }
        }
        // a record that is gone from the hub is also forgotten as dismissed, so that its id posted again is news
        for (var k in queue.dismissed) if (k.indexOf("n:") === 0 && !present[k]) delete queue.dismissed[k]
        if (gone) refresh()
    }
    function syncCards() {
        if (!enabled) return
        var now = Date.now(), present = {}, changed = false
        for (var i = 0; i < PluginHub.cards.length; i++) {
            var c = PluginHub.cards[i]
            if (c.priority !== "ringing") continue
            var e = Q.fromCard(c, cfg, now)
            if (!e) continue
            // one that started ringing long ago and was there before this surface is not news
            if (c.createdAt > 0 && now / 1000 - c.createdAt > 120 && !Q.find(queue, e.key)) continue
            present[e.key] = true
            var r = Q.offer(queue, e, cfg, now)
            if (r === "new" || r === "updated") changed = true
        }
        for (var j = queue.list.length - 1; j >= 0; j--) {
            var x = queue.list[j]
            if (x.src === "card" && !present[x.key]) { Q.withdraw(queue, x.key); changed = true }
        }
        if (changed) refresh()
    }
    // the camera pop-up showed a card: a doorbell record of the same ring is not a second card
    function cameraShown(kind) {
        if (!enabled) return
        Q.cameraShown(queue, kind, cfg, Date.now())
        refresh()
    }
    // A card made by hand (LumaOverlay's door file, for the gate and the TV steps): {class, title, body, source, actions, score, level, ttl (seconds), id}.
    // It takes the way of a plugin's record: classified, queued, kept clear of what is up, dismissed by Back.
    function door(spec) {
        if (!enabled || !spec) return false
        var cls = Q.CLASSES.indexOf(spec["class"]) >= 0 ? spec["class"] : "alert"
        var now = Date.now()
        take(Q.fromNote({ pluginId: "door", plugin: "Door", id: String(spec.id || "d" + now), channel: cls, channelTitle: String(spec.source || ""), channelIcon: "",
                          revision: (spec.revision | 0) || 1, expiresAt: spec.ttl > 0 ? now / 1000 + spec.ttl : 0, title: String(spec.title || ""), body: String(spec.body || ""),
                          actions: spec.actions || [], kind: cls, level: String(spec.level || ""), score: spec.score || null }, cfg, now))
        return true
    }

    // ---- what is on the screen
    // The height a card would have as it is drawn: the probe takes the entry's words and lays them out (nothing is drawn).
    function heightOf(e) {
        probe.entry = e
        probe.iconName = ""
        return probe.cardHeight
    }
    function snapshot(e) {
        return { key: e.key, src: e.src, pluginId: e.pluginId, id: e.id, cls: e.cls, title: e.title, body: e.body, source: e.source, tone: e.tone,
                 actions: e.actions, score: e.score, icon: e.icon, iconName: e.iconName }
    }
    function refresh() {
        wake.stop()
        if (!enabled) { clear(); return }
        var now = Date.now()
        pending = queue.list.length > 0
        var res = Q.select(queue, { dnd: dnd, camera: cameraUp, top: firstY, heightOf: heightOf }, cfg, now)
        var snap = []
        for (var i = 0; i < res.visible.length; i++) snap.push(snapshot(res.visible[i]))
        var sig = JSON.stringify(snap)
        if (sig !== shownSig) {
            // a second card that moves up to the first is not a card going away: its old slot does not fade
            var promoted = snap.length === 1 && shown.length === 2 && shown[1].key === snap[0].key
            if (promoted) { slot1.instant = true; Qt.callLater(function() { slot1.instant = false }) }
            shownSig = sig
            shown = snap
            if (snap.length > 0) held0 = snap[0]
            if (snap.length > 1) held1 = snap[1]
        }
        var a = Q.armed(res.visible, cfg, now)
        if (a !== armed) armed = a
        if (res.wakeAt > 0) { wake.interval = Math.max(20, res.wakeAt - now); wake.start() }
    }
    function clear() {
        queue = Q.create()
        shownSig = ""
        if (shown.length > 0) shown = []
        if (armed) armed = false
        wake.stop()
    }
    Timer { id: wake; onTriggered: ovs.refresh() }
    onDndChanged: refresh()
    onCameraUpChanged: refresh()
    onCameraBottomChanged: if (cameraUp) refresh()
    onEnabledChanged: { if (enabled) { syncCards(); refresh() } else clear() }
    onWidthChanged: refresh()

    // ---- keys: the top card's, once it has been up for `armMs`
    function key(dir) {
        if (!armed || shown.length === 0) return false
        if (dir === "back") { dismiss(shown[0]); return true }
        if (dir === "ok") { act(shown[0]); return true }
        return false
    }
    function dismiss(e) {
        Q.dismiss(queue, e.key, Date.now())
        refresh()
    }
    function act(e) {
        var ok = Q.okOf(e, drawerReady)
        dismiss(e)
        if (ok.action) PluginHub.act(e.pluginId, ok.action.action, ok.action.target)
        else if (drawerReady) openDrawer()
    }

    // ---- drawing: two slots, the second under the first at its real height
    OverlayCard {
        id: slot0
        x: ovs.column
        y: ovs.firstY
        cardWidth: ovs.cfg.width
        fillAlpha: ovs.paneAlpha
        drawerReady: ovs.drawerReady
        entry: ovs.shown.length > 0 ? ovs.shown[0] : ovs.held0
        iconName: ovs.iconFor(entry)
        shown: ovs.shown.length > 0
        Behavior on y { enabled: !Lu.reduced && slot0.shown; NumberAnimation { duration: Lu.cardMs; easing.type: Lu.cardEasing } }
    }
    OverlayCard {
        id: slot1
        x: ovs.column
        y: Math.round(slot0.y + slot0.cardHeight + ovs.cfg.gap)
        cardWidth: ovs.cfg.width
        fillAlpha: ovs.paneAlpha
        drawerReady: ovs.drawerReady
        entry: ovs.shown.length > 1 ? ovs.shown[1] : ovs.held1
        iconName: ovs.iconFor(entry)
        shown: ovs.shown.length > 1
        Behavior on y { enabled: !Lu.reduced && slot1.shown; NumberAnimation { duration: Lu.cardMs; easing.type: Lu.cardEasing } }
    }
    OverlayCard { id: probe; probe: true; cardWidth: ovs.cfg.width; drawerReady: ovs.drawerReady }
    // drawn once with letters and digits while the shell warms its layer (the compositor is not drawing it then: `live` leaves it out), so the
    // first card finds its glyphs cached, as the notifications' kept toast slot does
    OverlayCard {
        x: ovs.column; y: ovs.firstY
        cardWidth: ovs.cfg.width; drawerReady: true
        entry: ovs.warmEntry
        iconName: "circle-alert"
        shown: true
        instant: true
        visible: ovs.warming && ovs.enabled
    }

    // where the cards are as they are drawn, for the gate and the host's tests: the rectangles of the ones that are shown
    function geometry() {
        var out = []
        if (slot0.shown) out.push({ x: slot0.x, y: slot0.y, w: slot0.width, h: slot0.cardHeight })
        if (slot1.shown) out.push({ x: slot1.x, y: slot1.y, w: slot1.width, h: slot1.cardHeight })
        return out
    }

    Component.onCompleted: { Helper.start(); if (enabled) { syncCards(); refresh() } }
}
