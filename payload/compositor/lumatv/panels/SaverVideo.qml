import QtQuick 2.4
import TextFile 1.0

// The saver's side of the video-plane player (vplane/, docs/research/videoplane-saver-result.md): a mailbox of two small files. The shell
// writes `req.json` (start, renew, next, stop), the player writes `status.json`; neither needs the other to be there. The files are in
// /tmp, read and written through the compositor's TextFile like the other doors (Home's heartbeat, the overlay's door).
//
// The state is what the saver needs to choose between the GPU and the decoder:
//   off       nothing asked (the saver is down, or the player is not on the TV: no status file, or one that has stopped being rewritten);
//   starting  the player was asked and has not said it plays: the saver shows black, for at most `startMs`;
//   playing   the picture is on the video plane: the GPU draws none of it;
//   failed    the player said it could not (its reason is `reason`), did not answer, or stopped being heard from: the saver draws the art
//             itself, as it always did, and the player is asked to stop.
// While it is asked, `renew` goes every `renewMs`: a player that is not renewed for `ttlMs` takes its window away by itself, so a shell
// that dies cannot leave a picture on the screen. Nothing runs while `want` is false.
QtObject {
    id: vp

    // ---- asked by the saver
    property bool want: false
    property var spec: ({})                      // the request's parameters (Screensaver.qml fills them from its tokens)
    property int startMs: 6000
    property int renewMs: 10000
    property int ttlMs: 30000
    property int staleMs: 12000                  // a playing player writes its status at least every 5 s
    property int idleStaleMs: 150000             // an idle one every 60 s
    property string dirUrl: "file:///tmp/{{shortId}}-vplane/"

    // ---- answered to it
    property string state: "off"
    property string reason: ""
    property var status: null
    readonly property bool busy: state === "starting" || state === "playing"
    readonly property bool playing: state === "playing"

    property double seq: 0
    property double startSeq: 0                  // the seq of the `start` request: a status older than it is another session's
    property double askedAt: 0

    // (a QtObject has no default property, so its children are properties: as LumaKeyBridge.qml does)
    property var reqFile: TextFile { name: vp.dirUrl + "req.json" }
    property var statusFile: TextFile { name: vp.dirUrl + "status.json" }

    function readStatus() {
        var t = statusFile.exists ? statusFile.read() : ""
        try { return t ? JSON.parse(t) : null } catch (e) { return null }
    }
    function send(op, extra) {
        seq = Math.max(seq + 1, Date.now())
        var req = { seq: seq, sent: Date.now(), op: op, ttlMs: ttlMs }
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

    // Asks for the picture. Returns at once with `state` `starting`, or `off` when there is no player to ask.
    function begin() {
        var s = readStatus()
        if (!s || typeof s.ts !== "number" || Date.now() - s.ts > idleStaleMs || (s.state !== "idle" && s.state !== "error")) {
            reason = s ? "the player is not idle" : "no player"
            state = "off"
            return
        }
        status = s
        reason = ""
        startSeq = send("start", vp.spec)
        askedAt = Date.now()
        state = "starting"
    }
    // Lets it go. The player is told once; a request that is not delivered is a ttl away from the same thing.
    function end() {
        if (state !== "off") send("stop")
        state = "off"
        reason = ""
    }
    // Turns to the next print now: `clip` names it (the shell's own order), else the player's.
    function next(clip) { if (state === "playing") send("next", clip ? { clip: clip } : {}) }

    // The player's own wake (a key or the pointer reached its window: it asked tvpower for the screen and took itself away) is the viewer
    // asking for the saver to end, not a failure: the saver is told, and does not fall back to the GPU's art. Its reason is the player's own text.
    signal woken()
    function wokenBy(s) { return !!s && s.state === "idle" && s.seq >= startSeq && String(s.reason || "").indexOf("a key or the pointer") === 0 }

    function tick() {
        var s = readStatus()
        status = s
        var now = Date.now()
        if (busy && wokenBy(s)) { state = "off"; reason = "woken"; woken(); return }
        if (state === "starting") {
            if (s && s.seq === seq && s.state === "playing") { state = "playing"; reason = "" }
            else if (s && s.seq === seq && s.state === "error") fail(s.reason || "the player could not start")
            else if (now - askedAt > startMs) fail(s && s.seq === seq ? "the player did not reach playing" : "the player did not answer")
        } else if (state === "playing") {
            if (!s || s.state !== "playing") fail(s && s.reason ? s.reason : "the player stopped")
            else if (now - s.ts > staleMs) fail("the player went quiet")
        }
    }

    onWantChanged: if (!want && state !== "off") end()

    // While it plays the status is read twice a second: the player's own wake (a key in its window) has to end the saver in the Active state at once,
    // and a read is one small file and one parse.
    property var poll: Timer { interval: vp.state === "playing" ? 500 : 200; running: vp.want && vp.busy; repeat: true; onTriggered: vp.tick() }
    property var renewer: Timer { interval: vp.renewMs; running: vp.want && vp.busy; repeat: true; onTriggered: vp.send("renew") }
    readonly property bool timersRunning: poll.running || renewer.running
}
