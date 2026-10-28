pragma Singleton
import QtQuick 2.4
import TextFile 1.0

// The shell's end of the UI channel (compositor/crates/lumatv-compositor/src/shell.rs): the compositor's loopback HTTP
// server, read the way LumaTV's helper is (kit/Helper.qml): one state read, then a long poll for events. Every request
// carries the token the compositor wrote for this session (a root-only file named in the launch parameters).
//
// Topics (`topics`, `updated(name)`): foreground {app, home}, screen {on}, keyboard {up}, cursor {visible}, wall {hole} (the moving wallpaper's
// hole is under Home), audio (the
// audio service's volume state). Events of their own: `show` (a panel the gear or Input asked for), `homeKey`, and the
// answers to luna calls, which `luna()` routes to the callback of the call's tag. The compositor restarting restarts this
// process with it, so a `reset` only ever means the client fell behind: the state is read again.
QtObject {
    id: link

    // qml-runner's launch JSON (its first argument): {"params": {port, tokenFile, dataDir, moduleDir, runtimeDir, socket}}.
    readonly property var params: {
        var args = Qt.application.arguments
        for (var i = 1; i < args.length; i++) {
            try { var j = JSON.parse(args[i]); if (j && j.params) return j.params } catch (e) {}
        }
        return {}
    }
    readonly property string base: "http://127.0.0.1:" + (params.port || 18750)
    property TextFile tokenFile: TextFile { name: "file://" + (link.params.tokenFile || "/tmp/xdg/lumatv-shell.token") }
    readonly property string token: { try { return String(tokenFile.read()).replace(/\s+/g, "") } catch (e) { return "" } }

    property bool up: false
    property var topics: ({})
    property double seq: 0
    property double boot: 0
    signal updated(string topic)
    signal show(string kind, bool warm)
    signal homeKey()

    property bool started: false
    property int failures: 0
    property int generation: 0
    property double polledAt: 0
    property int events: 0

    function start() {
        if (started) return
        started = true
        sync()
    }

    // callback(status, document|null). status 0: the compositor is not listening.
    function request(method, path, body, done) {
        var x = new XMLHttpRequest()
        x.onreadystatechange = function() {
            if (x.readyState !== 4) return
            var doc = null
            if (x.status === 200) { try { doc = JSON.parse(x.responseText) } catch (e) { doc = null } }
            if (x.status === 403) console.warn("[Link] the compositor refused the token")
            if (done) done(x.status, doc)
        }
        x.open(method, base + path)
        x.setRequestHeader("X-Luma-Token", token)
        if (method !== "GET") x.setRequestHeader("Content-Type", "application/json")
        x.send(body === undefined ? null : JSON.stringify(body))
        return x
    }
    function post(path, body, done) { return request("POST", path, body === undefined ? {} : body, done) }

    function sync() {
        var gen = ++generation
        watchdog.stop()
        request("GET", "/v1/state", undefined, function(status, doc) {
            if (gen !== generation) return
            if (!doc || !doc.ok) { fail(); return }
            failures = 0
            up = true
            boot = doc.boot
            seq = doc.seq
            var old = topics, next = {}
            for (var name in doc.topics) if (!streamed(name)) next[name] = doc.topics[name]
            topics = next
            for (var n in next) if (JSON.stringify(old[n]) !== JSON.stringify(next[n])) updated(n)
            poll(gen)
        })
    }
    // Events that are not a state: not kept as topics, not replayed from the state read.
    function streamed(name) { return name === "luna" || name === "show" || name === "homekey" }

    function poll(gen) {
        polledAt = Date.now()
        watchdog.restart()
        request("GET", "/v1/events?after=" + seq + "&timeout=25", undefined, function(status, doc) {
            if (gen !== generation) return
            watchdog.stop()
            if (!doc || !doc.ok) { fail(); return }
            if (doc.reset || doc.boot !== boot) { sync(); return }
            seq = doc.seq
            var changed = [], next = null
            for (var i = 0; i < doc.events.length; i++) {
                var e = doc.events[i]
                events++
                if (e.topic === "luna") { deliver(e.data.tag, e.data.payload); continue }
                if (e.topic === "show") { show(String(e.data.kind || ""), e.data.warm === true); continue }
                if (e.topic === "homekey") { homeKey(); continue }
                if (!next) { next = {}; for (var k in topics) next[k] = topics[k] }
                next[e.topic] = e.data
                if (changed.indexOf(e.topic) < 0) changed.push(e.topic)
            }
            if (next) topics = next
            for (var j = 0; j < changed.length; j++) updated(changed[j])
            // An answer with nothing in it that came back at once is not a long poll: wait a second.
            if (!doc.events.length && Date.now() - polledAt < 500) pollAgain.restart()
            else poll(gen)
        })
    }

    function fail() {
        up = false
        failures++
        generation++
        watchdog.stop()
        retry.interval = Math.min(10000, 1000 * Math.pow(2, Math.min(failures - 1, 3)))
        retry.restart()
    }
    property Timer retry: Timer { onTriggered: link.sync() }
    property Timer pollAgain: Timer { interval: 1000; onTriggered: link.poll(link.generation) }
    property Timer watchdog: Timer { interval: 35000; onTriggered: { console.warn("[Link] a poll got no answer in 35 s"); link.fail() } }

    // ---- luna through the compositor's bridge: `cb(payload)` per answer (every push of a subscription)
    property int nextTag: 0
    property var callbacks: ({})
    function luna(uri, payload, subscribe, cb) {
        var tag = ++nextTag
        callbacks[tag] = { cb: cb, subscribe: subscribe === true }
        post("/v1/luna", { tag: tag, uri: uri, payload: payload || {}, subscribe: subscribe === true })
        return tag
    }
    function cancel(tag) {
        if (!callbacks[tag]) return
        delete callbacks[tag]
        post("/v1/cancel", { tag: tag })
    }
    function deliver(tag, payload) {
        var c = callbacks[tag]
        if (!c) return
        if (!c.subscribe) delete callbacks[tag]
        try { if (c.cb) c.cb(payload) } catch (e) { console.warn("[Link] the answer to luna call " + tag + " threw: " + e) }
    }

    // ---- what the UI tells the compositor: can [...], wants, state {...}
    function report(doc) { post("/v1/ui", doc) }
}
