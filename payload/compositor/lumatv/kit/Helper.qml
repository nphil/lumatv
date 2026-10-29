pragma Singleton
import QtQuick 2.4

// The client of lumatv-helper's loopback API (helper/, API v1 in docs/research/native-helper-result.md).
// One state read, then a long poll for changes; `topics` holds every topic's latest value (screen,
// weather, news, receiver) and `updated(topic)` fires when one changes, so a surface binds to
// `Helper.topics.weather` and never fetches or polls anything itself. A surface that has a value of
// its own on disk (the weather and news cards) draws that until the first answer.
//
// The helper being down is normal (it starts a moment after the compositor, and `lumatv revert`
// stops it): `up` is false, `topics` keeps what it had, and the client retries at 2, 4, 8 and then
// every 10 s. A long poll that gets no answer in 35 s is abandoned and the state read again.
QtObject {
    id: helper

    readonly property string base: "http://127.0.0.1:18731"
    readonly property string subscribed: "screen,weather,news,receiver,ai,ai.prompt,plugins,ha.link,updateblock,openapps,panel,cameras,cameras.popup,cameras.live,profiles"
    // Plugin topics are named per plugin and a long poll takes at most 16 names: the fixed ones, then the cards, tiles and
    // notifications of each plugin that is connected, the helper's own (`firstParty`, listed apart in `plugins`) included;
    // with more than fit, which is the usual case since the helper's own are four, there is no filter (every topic) rather
    // than a wrong one. A plugin that connects changes the `plugins` topic, which ends the poll in flight, so the next one
    // has it.
    function filter() {
        var list = subscribed.split(","), roster = topics.plugins || {}
        var p = (roster.plugins || []).concat(roster.firstParty || [])
        for (var i = 0; i < p.length; i++) {
            if (p[i].state !== "connected") continue
            list.push("plugin." + p[i].id + ".cards", "plugin." + p[i].id + ".tiles", "plugin." + p[i].id + ".notifications", "plugin." + p[i].id + ".facts")
        }
        return list.length > 16 ? "" : list.join(",")
    }
    function pluginTopic(id, kind) { return topics["plugin." + id + "." + kind] || null }
    function pluginAction(id, action, target) { return post("/v1/plugins/" + id + "/actions/" + action, target ? { target: target } : {}) }

    property bool up: false
    property var topics: ({})           // topic -> latest value (weather and news: {doc, status, ...})
    property double boot: 0             // the helper's start; a different value means it restarted
    property double seq: 0              // last event applied
    readonly property var receiver: topics.receiver || null
    readonly property var screen: topics.screen || null
    readonly property var ai: topics.ai || null                  // AI access: listener, pairing, assistants, limits
    readonly property var aiPrompt: topics["ai.prompt"] || null  // the open prompt, or null
    // Local profiles (helper/app/src/profiles.rs): who is active, who owns the TV, each person's name, letter, colour and picture. `ready` is false until the helper has made them.
    readonly property var profiles: topics.profiles || null
    readonly property var updateBlock: topics.updateblock || null  // LG's system updates: the Homebrew Channel's flag and what /etc/hosts maps now
    readonly property var openApps: topics.openapps || null        // open apps without updating: the setting and LG's list of apps waiting for an update (helper/app/src/openapps.rs)
    readonly property var panel: topics.panel || null            // LG's panel refresh: runs so far and when the last one was (helper/app/src/refreshguard.rs)
    // "last run 3 h ago, 12 runs" from the panel topic; "" until LG has answered (never a made-up number)
    readonly property string panelRefresh: {
        var p = panel
        if (!p || !p.read) return ""
        var ago = Math.max(0, Math.round(p.usageHours - p.lastRunHours))
        return "last run " + ago + " h ago, " + p.offrsCount + (p.offrsCount === 1 ? " run" : " runs")
    }
    signal updated(string topic)
    // The receiver's own controls (sound mode, Audyssey, levels, zones) exist only when the helper says so:
    // its `receiver` topic carries `controls: true` once it can send the Denon's commands. Until then the
    // Settings page shows what the topic has and dims the rest.
    readonly property bool receiverControls: !!(receiver && receiver.controls)
    function receiverSet(what, value) { return post("/v1/receiver/set", { what: what, value: value }) }

    property bool started: false
    property int failures: 0
    property int generation: 0          // bumped to drop answers to requests that were abandoned
    property var pending: null          // the long poll in flight
    property double polledAt: 0
    property bool refused: false

    // Idempotent; the first caller starts the loop.
    function start() {
        if (started) return
        started = true
        sync()
    }

    // callback(status, document|null). status 0: the helper is not listening.
    function request(method, path, body, done) {
        var x = new XMLHttpRequest()
        x.onreadystatechange = function() {
            if (x.readyState !== 4) return
            var doc = null
            if (x.status === 200 || x.status === 202 || x.status === 409) { try { doc = JSON.parse(x.responseText) } catch (e) { doc = null } }
            if (x.status === 403 && !helper.refused) {
                helper.refused = true
                console.warn("[Helper] refused: " + x.responseText.substring(0, 160))
            }
            if (done) done(x.status, doc)
        }
        x.open(method, base + path)
        if (method !== "GET") {
            x.setRequestHeader("X-Luma-Client", "compositor")
            x.setRequestHeader("Content-Type", "application/json")
        }
        x.send(body === undefined ? null : JSON.stringify(body))
        return x
    }
    function post(path, body, done) { return request("POST", path, body === undefined ? {} : body, done) }
    function del(path, done) { return request("DELETE", path, undefined, done) }

    function sync() {
        var gen = ++generation
        pending = null
        watchdog.stop()
        request("GET", "/v1/state", undefined, function(status, doc) {
            if (gen !== generation) return
            if (!doc || !doc.ok) { fail(); return }
            failures = 0
            up = true
            boot = doc.boot
            seq = doc.seq
            var old = topics, next = {}
            for (var name in doc.topics) next[name] = doc.topics[name]
            topics = next
            for (var n in next) if (JSON.stringify(old[n]) !== JSON.stringify(next[n])) updated(n)
            poll(gen)
        })
    }

    function poll(gen) {
        polledAt = Date.now()
        watchdog.restart()
        var names = filter()
        pending = request("GET", "/v1/events?after=" + seq + "&boot=" + boot + "&timeout=25" + (names ? "&topics=" + names : ""), undefined, function(status, doc) {
            if (gen !== generation) return
            watchdog.stop()
            if (!doc || !doc.ok) { fail(); return }
            if (doc.reset || doc.boot !== boot) { sync(); return }
            seq = doc.seq
            var changed = [], next = null
            for (var i = 0; i < doc.events.length; i++) {
                var e = doc.events[i]
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
        pending = null
        watchdog.stop()
        retry.interval = Math.min(10000, 2000 * Math.pow(2, Math.min(failures - 1, 3)))
        retry.restart()
    }

    property Timer retry: Timer { onTriggered: helper.sync() }
    property Timer pollAgain: Timer { interval: 1000; onTriggered: helper.poll(helper.generation) }
    property Timer watchdog: Timer {
        interval: 35000
        onTriggered: {
            var p = helper.pending
            helper.fail()
            if (p) p.abort()
        }
    }
}
