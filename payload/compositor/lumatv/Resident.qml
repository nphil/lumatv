import QtQuick 2.4
import "kit"

// A surface that is built once, in the background, and kept. Showing it flips `visible`; nothing is
// created, compiled or laid out for the first time in the frame the key is waiting for (the PLAN's
// first-frame rule: Quick Settings opens about 44 ms after the release because LG keeps it built; the
// gallery's Loader tore down 72 ms after Back). The build is asynchronous: Qt compiles the file on its
// loader thread and instantiates it in slices of a few milliseconds between frames, so a build in
// progress is not one long frame. `build` is the host's cue to start (Layers builds them one after
// another once Home has presented); a surface asked for before its turn is built at once and shows when
// it is ready. A file that fails to load logs once and stays absent; the host carries on.
//
// A surface that builds parts of itself in the background (Settings' pages) has a `built` property and
// is finished when that is true; `done()` is the signal the host waits for.
//
// A panel that slides in and out (panels/PanelMotion.qml) is told when it is wanted, in its `presented` property, and says `leaving` while
// it is on its way out, so `shown` going false does not cut the slide back short: it is drawn until it has gone. It says `held` while it is
// parked after the slide: still visible, so its nodes stay in the scene graph and the next opening joins nothing (PanelMotion has the numbers).
Loader {
    id: res
    property string name: ""
    property int step: 0                      // the host's build order
    property bool shown: false                // the host wants it on screen
    property bool build: false                // the host's turn to build it
    property bool wanted: false               // sticky: once built it is kept, however the host's flags move
    property bool announced: false
    readonly property bool ready: status === Loader.Ready
    signal done()                             // built, or failed: the host moves on to the next
    // The offline host sets theme `perf.syncResidents` so a render does not wait on software GL frames to incubate a tree.
    asynchronous: !(Lu.theme && Lu.theme.perf && Lu.theme.perf.syncResidents)
    active: wanted
    // a panel that moves keeps itself drawn while it leaves, and in the scene while it is parked
    readonly property bool leaving: !!item && item.leaving === true
    readonly property bool held: !!item && item.held === true
    visible: ready && (shown || leaving || held)
    onBuildChanged: if (build) wanted = true
    onShownChanged: if (shown) wanted = true
    onItemChanged: { if (item && item.presented !== undefined) item.presented = Qt.binding(function() { return res.shown }) }
    Component.onCompleted: if (build || shown) wanted = true

    // `done()` goes out on the next turn of the event loop, not inside the call that built the surface: with a synchronous build
    // (the offline host) that call is the host's own `build` binding, and the host moving its step on from inside it is a binding loop.
    function finish() { Qt.callLater(function() { res.done() }) }
    function check() {
        if (announced || status !== Loader.Ready || !item) return
        if (item.built === false) return
        announced = true
        console.info("[Resident] " + name + " built")
        finish()
    }
    onStatusChanged: {
        if (status === Loader.Ready) check()
        else if (status === Loader.Error && !announced) { announced = true; console.warn("[Resident] " + name + " did not load: " + source); finish() }
    }
    Connections {
        target: res.item
        ignoreUnknownSignals: true
        onBuiltChanged: res.check()
    }
}
