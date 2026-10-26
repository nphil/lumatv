import QtQuick 2.4
import WebOSCompositorBase 1.0
import WebOSServices 1.0

// tvpower's question before it starts LG's own screen saver (docs/research/saver-own-result.md). When the TV has been idle long enough
// (LG's idle timer `com.webos.service.nop`, 180 s on this set) tvpower asks every client on its screen saver list whether the saver may
// start, each with the same `timestamp`, and starts it unless one of them says no. The stock clients that say no are the ones the saver would
// spoil: the input apps, Netflix and YouTube while they play, the cover screen and the user list (all `ack: false`); `com.webos.service.screenmanager`
// is the one on the list that says yes. LG's compositor registers for its power requests the same way (controllers/base/StarfishPowerService.qml:
// `registerPreparePowerOff` with `{clientName, subscribe: true}`, the answer a call with `{clientName, timestamp, ack}`).
//
// This object puts LumaTV's saver on that list while it can draw: Home in front and the owner's saver on (`want`). A request that arrives
// then is answered no, so LG's Flutter saver is never started (the power state stays Active: nothing in the TV's power machine is skipped
// or forced), and `claimed()` tells the saver to draw. A request that arrives when it cannot draw (`want` has just gone false) is answered
// yes, as if this client were not there. Nothing here is a setting of the TV: the subscription belongs to the compositor's connection, so
// when the compositor is not running (the shadow module off, a crash, a restart) the TV's list has no such client and LG's saver starts as
// it always did.
//
// Stock clients use two categories, `/power2` (Netflix, the input apps) and `/power` (the browser, YouTube, the cover screen); tvpower keeps
// a list for each (`getClientList`: `ListScreenSaverRequests`, `/power2` has screenmanager, `/power` the browser). It is not known from the
// outside which list a request from the idle timer goes to, so the object is on both and answers each question it hears.
QtObject {
    id: vote

    // ---- asked by the saver
    property bool want: false
    readonly property string clientName: "{{appId}}"
    readonly property string tvpower: "luna://com.webos.service.tvpower"
    readonly property var lists: ["/power2", "/power"]
    // A list that has not taken the subscription (tvpower refused it, or did not answer) is asked again after `retryMs`, `retryFast` times, then once
    // in `retrySlowMs` for as long as the saver is wanted: LG's input apps ask every second until tvpower takes them, as it is not ready in every
    // power state, and a subscription that was never made is a saver LG draws.
    property int retryMs: 3000
    property int retryFast: 5
    property int retrySlowMs: 60000

    // ---- what has happened (read by the tests and the log)
    property int asked: 0                          // questions heard
    property int refused: 0                        // ... answered no
    signal claimed()                               // a question was answered no: LumaTV's saver is to draw now

    // per list: { token, live, fails }
    property var subs: ({ "/power2": { token: 0, live: false, fails: 0 }, "/power": { token: 0, live: false, fails: 0 } })
    property int version: 0                        // bumped when a subscription changes: `registered` follows it (the objects in `subs` are changed in place)
    readonly property bool registered: { var v = version; return subs["/power2"].live || subs["/power"].live }

    property var service: Service {
        appId: LS.appId
        onResponse: vote.heard(method, payload, token)
    }

    // The list an answer belongs to: by the token of the call that asked, else by the method it names (with or without its leading slash).
    function listOf(method, token) {
        for (var i = 0; i < lists.length; i++) if (token > 0 && subs[lists[i]].token === token) return lists[i]
        var m = String(method)
        if (m.charAt(0) !== "/") m = "/" + m
        return m.indexOf("/power2/") === 0 ? "/power2" : "/power"
    }

    // A refusal that goes on is said at the first three and then every tenth, not once in each minute of a TV that will not have it.
    function note(list, what) {
        var n = subs[list].fails
        if (n <= 3 || n % 10 === 0) console.warn("[SaverVote] " + list + " " + what + " (" + n + ")")
    }

    function open() {
        for (var i = 0; i < lists.length; i++) {
            var s = subs[lists[i]]
            if (s.token > 0) continue
            var t = service.call(tvpower, lists[i] + "/registerScreenSaverRequest", JSON.stringify({ clientName: clientName, subscribe: true }))
            if (t > 0) s.token = t
            else { s.fails++; version++; note(lists[i], "could not ask tvpower for the subscription") }
        }
        arm()
    }
    function close() {
        for (var i = 0; i < lists.length; i++) {
            var s = subs[lists[i]]
            if (s.token > 0 && typeof service.cancel === "function") service.cancel(s.token)
            s.token = 0; s.live = false; s.fails = 0
        }
        version++
        retry.stop()
    }
    // The TV has come on again after being off: the subscription is made anew, so a list that tvpower did not keep through standby has the client.
    function renew() { if (want) { close(); open() } }
    function pending() {
        for (var i = 0; i < lists.length; i++) if (!subs[lists[i]].live) return true
        return false
    }
    function delay() {
        for (var i = 0; i < lists.length; i++) { var s = subs[lists[i]]; if (!s.live && s.fails < retryFast) return retryMs }
        return retrySlowMs
    }
    readonly property int retryDelay: { var v = version; return delay() }       // follows the knobs and the fails, so a test can shorten both
    function arm() {
        if (!(want && pending())) retry.stop()
        else if (!retry.running) retry.start()
    }
    onWantChanged: { if (want) open(); else close() }
    Component.onCompleted: if (want) open()

    function answer(list, timestamp, ack) {
        LS.adhoc.call(tvpower, list + "/responseScreenSaverRequest", JSON.stringify({ clientName: clientName, timestamp: timestamp, ack: ack }))
    }

    function heard(method, payload, token) {
        var list = listOf(method, token)
        var s = subs[list]
        var m = null
        try { m = JSON.parse(payload) } catch (e) { return }
        if (!m) return
        if (m.returnValue === false) {
            // tvpower said no (not up yet, not in a state to take it, or the call is not allowed): the subscription is not live, and is asked for again
            s.token = 0; s.live = false; s.fails++
            note(list, "refused the subscription: " + payload)
            version++
            arm()
            return
        }
        if (!s.live) { s.live = true; s.fails = 0; version++; console.info("[SaverVote] on tvpower's list " + list) }
        arm()
        // the subscription's own answer has no timestamp; a question has one
        if (m.timestamp === undefined || m.timestamp === null || m.timestamp === "") return
        var yes = !want
        asked++
        if (!yes) refused++
        console.info("[SaverVote] " + list + " asked: " + payload + " -> " + (yes ? "yes (not ours to draw)" : "no (LumaTV's saver draws)"))
        answer(list, m.timestamp, yes)
        if (!yes) claimed()
    }

    // A list that has not answered in its time is asked again; the call that got no answer is let go first.
    property var retry: Timer {
        interval: vote.retryDelay
        repeat: true
        running: false
        onTriggered: {
            for (var i = 0; i < vote.lists.length; i++) {
                var s = vote.subs[vote.lists[i]]
                if (s.live || s.token <= 0) continue
                if (typeof vote.service.cancel === "function") vote.service.cancel(s.token)
                s.token = 0
                s.fails++
            }
            vote.version++
            vote.open()
            if (!vote.pending()) stop()
        }
    }
    readonly property bool timersRunning: retry.running
}
