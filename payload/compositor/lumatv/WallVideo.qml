import QtQuick 2.4
import TextFile 1.0
import "WallVideo.js" as Pick

// Home's side of the video-plane player's second role (vplane/, docs/research/videoplane-saver-result.md, "The moving wallpaper"): the
// wallpaper as a looping clip on the video plane, so Home's idle frames are not the GPU's. The mailbox is the saver's (SaverVideo.qml has
// its description): the shell writes `req.json`, the player writes `status.json`, neither needs the other to be there. The requests carry
// `role: "wall"` and the clip's name; the saver's carry none or "saver", and the player has one decoder: the saver takes it from the
// wallpaper, and the wallpaper never takes it from the saver.
//
// What Home needs to know is `up`: the GPU wallpaper may go. That is all of
//   want      Home asks for it (LumaHome: on screen, the switch on, a clip for this wallpaper, nothing over Home, ...),
//   playing   the player says the picture is on the plane (state `playing`),
//   settled   and has for `settleMs` (the first frames are on the plane before the GPU's go: no black between the two),
//   hole      and the compositor says the hole is under Home (LumaTV's `wall` topic: without it the plane's black would show).
// `up` goes false in the same moment any of them does, so the GPU wallpaper comes back at once; it is only ever drawn back over a plane
// that was showing the same picture. The other way, Home no longer wanting it, the player is told to stop `endMs` after `up` went false,
// when Home's own wallpaper has been drawn: the plane is not given up before the screen has something to show instead.
//
// State: off (nothing asked, no player, or the saver has it), starting (asked, not yet playing: for at most `startMs`; the GPU wallpaper
// is drawn meanwhile, so waiting costs nothing to see), playing, failed (the player said no, did not answer, went quiet or stopped: the GPU
// wallpaper stays until Home is next shown, and the player is told to stop). A player that is not on the TV is `off` with the reason "no
// player", asked again every 15 s while Home wants it. While it is asked, `renew` goes every `renewMs`: a player that is not renewed for
// `ttlMs` takes its window away by itself, so a shell that dies cannot leave a picture on the plane; the compositor ends the window the
// moment Home is not drawn (an app took the front), whatever this says.
QtObject {
    id: wv

    // ---- asked by Home
    property bool want: false
    property string clip: ""                    // the clip's name for the player: "wall-silk"
    property bool hole: false                   // the compositor says the wallpaper's hole is under Home
    property int startMs: 6000
    property int renewMs: 10000
    property int ttlMs: 30000
    property int staleMs: 12000                 // a playing player writes its status at least every 5 s
    property int idleStaleMs: 150000            // an idle one every 60 s
    property int settleMs: 250
    property int endMs: 250
    property int retryMs: 15000
    property string dirUrl: "file:///tmp/{{shortId}}-vplane/"
    property string clipsUrl: "file://{{dataDir}}/vplane/clips/"

    // ---- answered to it
    property string state: "off"
    property string reason: ""
    property var status: null
    property var manifest: null                 // the clips' manifest (wall.json), read by `loadManifest`
    property bool watch: false                  // the video wall is switched on and Home is shown: the manifest is read again every `manifestMs`
    property int manifestMs: 5000               // (the file is the package's, and `lumatv vplane install` puts it there while the shell runs)
    property string manifestText: ""
    property bool settled: false
    readonly property bool busy: state === "starting" || state === "playing"
    readonly property bool playing: state === "playing"
    readonly property bool up: want && playing && settled && hole

    property double seq: 0
    property double startSeq: 0                 // the seq of the `start` request: a status older than it is another session's
    property double askedAt: 0

    // (a QtObject has no default property, so its children are properties: as SaverVideo.qml does)
    property var reqFile: TextFile { name: wv.dirUrl + "req.json" }
    property var statusFile: TextFile { name: wv.dirUrl + "status.json" }
    property var manifestFile: TextFile { name: wv.clipsUrl + "wall.json" }

    // The clip for the wallpaper, the look and the scrim Home has now: {entry, why} (WallVideo.js).
    function pick(wallpaper, motion, scrim) { return Pick.pick(manifest, wallpaper, motion, scrim) }
    function loadManifest() {
        var t = manifestFile.exists ? manifestFile.read() : ""
        if (t === manifestText && manifest !== null) return         // unchanged: the picks stay as they are
        manifestText = t
        try { manifest = t ? JSON.parse(t) : null } catch (e) { manifest = null }
    }

    function readStatus() {
        var t = statusFile.exists ? statusFile.read() : ""
        try { return t ? JSON.parse(t) : null } catch (e) { return null }
    }
    function send(op, extra) {
        seq = Math.max(seq + 1, Date.now())
        var req = { seq: seq, sent: Date.now(), op: op, ttlMs: ttlMs, role: "wall" }
        for (var k in (extra || {})) req[k] = extra[k]
        reqFile.write(JSON.stringify(req))
        return seq
    }
    function fail(why) {
        if (state === "failed") return
        reason = why
        state = "failed"
        send("stop")
    }
    // The saver took the player (it starts over Home, and the player gives the wallpaper's session up for it): not a fault, and not ours to stop.
    function giveWay() { reason = "the saver has the player"; state = "off" }

    // Asks for the clip. Returns at once with `state` `starting`, or `off` when there is no player to ask or the saver has it.
    function begin() {
        var s = readStatus()
        if (!s || typeof s.ts !== "number" || Date.now() - s.ts > idleStaleMs) { reason = "no player"; state = "off"; return }
        if (s.state !== "idle" && s.state !== "error" && s.role !== "wall") { reason = "the saver has the player"; state = "off"; return }
        status = s
        reason = ""
        startSeq = send("start", { clip: clip })
        askedAt = Date.now()
        state = "starting"
    }
    function release() {
        if (state === "failed") { state = "off"; reason = ""; return }
        if (state !== "off") stopper.restart()
    }

    function tick() {
        var s = readStatus()
        status = s
        var now = Date.now()
        if (state === "starting") {
            if (s && s.seq >= startSeq && s.role === "wall" && s.state === "playing") { state = "playing"; reason = ""; settle.restart() }
            else if (s && s.seq >= startSeq && s.state === "error") fail(s.reason || "the player could not start")
            else if (s && s.role === "saver" && (s.state === "playing" || s.state === "opening")) giveWay()
            else if (now - askedAt > startMs) fail(s && s.seq >= startSeq ? "the player did not reach playing" : "the player did not answer")
        } else if (state === "playing") {
            if (!s || s.state !== "playing") fail(s && s.reason ? s.reason : "the player stopped")
            else if (s.role !== "wall") giveWay()
            else if (now - s.ts > staleMs) fail("the player went quiet")
        }
    }

    onWantChanged: {
        if (want) {
            stopper.stop()
            if (state === "off" || state === "failed") begin()        // (one still playing from a moment ago just goes on)
        } else release()
    }
    onStateChanged: if (state !== "playing") settled = false

    // While it plays the status is read twice a second, and while it is asked five times: a player that stops, or a window the compositor took
    // away, is the GPU wallpaper's again within a few hundred milliseconds even where the compositor does not say so.
    property var poll: Timer { interval: wv.state === "playing" ? 500 : 200; running: wv.want && wv.busy; repeat: true; onTriggered: wv.tick() }
    property var renewer: Timer { interval: wv.renewMs; running: wv.want && wv.busy; repeat: true; onTriggered: wv.send("renew") }
    property var settle: Timer { interval: wv.settleMs; onTriggered: wv.settled = wv.playing }
    // The player is told to stop only after Home has had the time to draw its own wallpaper again; a want that comes back before then keeps it.
    property var stopper: Timer {
        interval: wv.endMs
        onTriggered: { if (!wv.want && wv.state !== "off") { wv.send("stop"); wv.state = "off"; wv.reason = "" } }
    }
    property var retry: Timer {
        interval: wv.retryMs
        running: wv.want && wv.state === "off" && wv.reason === "no player"
        repeat: true
        onTriggered: wv.begin()
    }
    // Reads the manifest at once when the switch goes on and then every `manifestMs` while it is on: installing the player or the clips after the
    // shell started (TV round 13) needed a shell restart before.
    property var manifestTimer: Timer { interval: wv.manifestMs; running: wv.watch; repeat: true; triggeredOnStart: true; onTriggered: wv.loadManifest() }
    readonly property bool timersRunning: poll.running || renewer.running || settle.running || stopper.running || retry.running || manifestTimer.running
}
