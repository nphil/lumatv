import QtQuick 2.4
import "../kit"
import "."

// AI access (docs/mocks/04-settings-ai-access): what the helper's AI module says (the `ai` topic) and the
// changes the owner makes with the remote (`/v1/ai`). Principle 0 shapes it: the page can ask for anything
// and give less at once (off, pause, remove, a lower level, a lower volume limit, lock-out), but whatever gives
// more (turning it on, a pairing code, a higher level, resuming, a higher limit) only opens a prompt on the TV
// that the physical remote answers (AiPrompt.qml). The page has no way to answer the prompt, because the
// helper has no route that does; the helper itself reads OK and Back from the remote's own input devices
// (LGE RCU, LGE M-RCU) and ignores Smart Remote RCU Input and LGE Network Input, where Home Assistant's and
// every other network key arrives. While a prompt is up this page takes no key at all.
//
// Layout follows the mock: the switch across the top, the paired assistants and the recent activity on the
// left, what the chosen assistant may do on the right, the lock-out at the foot. Focus moves to the nearest
// control in the direction pressed; Left and Right first move inside a segmented control. Removing an
// assistant and the lock-out take two presses.
Item {
    id: page
    property var host
    readonly property var ai: Helper.ai
    readonly property bool up: Helper.up && ai !== null
    readonly property var assistants: ai && ai.assistants ? ai.assistants : []
    readonly property bool listenerOn: !!(ai && ai.listener && ai.listener.enabled)
    readonly property var pairing: ai ? ai.pairing : null
    readonly property bool asking: Helper.aiPrompt !== null
    readonly property bool rested: host ? host.rested : true
    readonly property bool active: host ? host.zone === "page" : false
    readonly property int nAssist: assistants.length
    readonly property var levelGroups: ai && ai.groups ? ai.groups : []
    readonly property var levelIds: ["always", "ask", "never"]
    readonly property var levelNames: ["Always", "Ask", "Never"]            // Ask: the TV asks, the remote answers (the note under the title)
    readonly property int maxVolume: ai && ai.limits && ai.limits.maxVolume !== undefined ? ai.limits.maxVolume : 98
    property int permIndex: 0
    readonly property var chosen: nAssist > 0 ? assistants[Math.min(permIndex, nAssist - 1)] : null
    property var audit: []
    property string armed: ""
    readonly property string summary: !up ? "Not running" : listenerOn ? "On" : "Off"

    // ---- focus: a list of the controls on screen, and the one the cursor is on
    property string fk: "listener"        // kind of the focused control
    property int fi: 0                    // its index among that kind
    property int seg: 0                   // segment inside it
    function items() {
        var out = [{ k: "listener", i: 0, it: rowListener, segs: 2 }, { k: "limit", i: 0, it: rowLimit, segs: 1 }]
        for (var a = 0; a < nAssist; a++) out.push({ k: "assistant", i: a, it: assistRepeater.itemAt(a), segs: 2 })
        out.push({ k: "pair", i: 0, it: rowPair, segs: 1 })
        if (nAssist > 1) out.push({ k: "chooser", i: 0, it: chooser, segs: nAssist })
        if (nAssist > 0) for (var g = 0; g < levelGroups.length; g++) out.push({ k: "level", i: g, it: levelRepeater.itemAt(g), segs: 3 })
        out.push({ k: "lock", i: 0, it: rowLock, segs: 1 })
        return out.filter(function(e) { return !!e.it })
    }
    function find(k, i) { var l = items(); for (var n = 0; n < l.length; n++) if (l[n].k === k && l[n].i === i) return l[n]; return null }
    function focus(k, i) {
        fk = k; fi = i; armed = ""
        if (host && host.hideHelp) host.hideHelp()
        seg = k === "listener" ? (listenerOn ? 1 : 0) : k === "chooser" ? permIndex
            : k === "level" ? Math.max(0, levelIds.indexOf(levelOf(levelGroups[i].id))) : 0
    }
    function pointAt(k, i) { if (host) host.zone = "page"; focus(k, i) }

    // ---- help: the explanation behind a row's info mark, as on every page (kit/LuHelp is the host's). The focused control's own row has it;
    // a control that is not a row (the assistant chooser) has none.
    function helpRow() { var c = find(fk, fi); return c && c.it && c.it.help !== undefined && c.it.help !== "" ? c.it : null }
    function helpReady() { return active && !asking && helpRow() !== null }
    function openHelp() {
        var r = helpRow()
        if (!r || !host || !host.showHelp) return false
        host.showHelp(r.label, r.help, r)
        return true
    }
    function reset() { if (!find(fk, fi)) focus("listener", 0); else if (fk === "listener") seg = listenerOn ? 1 : 0; refreshAudit() }
    function levelOf(group) { return chosen && chosen.levels ? chosen.levels[group] : "ask" }
    onNAssistChanged: { if (permIndex >= nAssist) permIndex = Math.max(0, nAssist - 1); if (!find(fk, fi)) focus("listener", 0) }
    onAskingChanged: armed = ""

    // The control nearest in the direction pressed: its middle must lie past the focused one's, and the sideways
    // gap counts three times (docs/mocks/mock.js, the same rule).
    function spatial(dir) {
        var cur = find(fk, fi)
        if (!cur) return false
        var a = cur.it.mapToItem(page, 0, 0)
        var ax = a.x + cur.it.width / 2, ay = a.y + cur.it.height / 2
        var best = null, bestScore = 1e9, bestAligned = null, alignedScore = 1e9
        var l = items()
        for (var n = 0; n < l.length; n++) {
            if (l[n] === cur || (l[n].k === cur.k && l[n].i === cur.i)) continue
            var b = l[n].it.mapToItem(page, 0, 0)
            var bx = b.x + l[n].it.width / 2, by = b.y + l[n].it.height / 2
            var dx = bx - ax, dy = by - ay
            var gap = dir === "up" || dir === "down" ? Math.max(0, Math.abs(dx) - (cur.it.width + l[n].it.width) / 2) : Math.max(0, Math.abs(dy) - (cur.it.height + l[n].it.height) / 2)
            if (dir === "up" && dy >= -4) continue
            if (dir === "down" && dy <= 4) continue
            if (dir === "left" && dx >= -4) continue
            if (dir === "right" && dx <= 4) continue
            var primary = dir === "up" || dir === "down" ? Math.abs(dy) : Math.abs(dx)
            var score = primary + 3 * gap
            if (score < bestScore) { best = l[n]; bestScore = score }
            // A control that overlaps the focused one across the direction pressed (the same column for Up and Down)
            // is preferred over a nearer one in the other column.
            if (gap === 0 && primary < alignedScore) { bestAligned = l[n]; alignedScore = primary }
        }
        if (bestAligned) best = bestAligned
        if (!best) return false
        focus(best.k, best.i)
        return true
    }

    function key(dir) {
        if (asking) return true                    // the prompt owns the keys; the remote's own OK and Back reach the helper, not this page
        var cur = find(fk, fi)
        if (!cur) { focus("listener", 0); return true }
        if (dir === "left" || dir === "right") {
            var d = dir === "left" ? -1 : 1
            if (cur.segs > 1 && seg + d >= 0 && seg + d < cur.segs) { seg += d; if (fk === "chooser") { permIndex = seg; relevel() } return true }
            if (fk === "limit") { adjustLimit(d); return true }
            if (spatial(dir)) return true
            return d < 0 ? false : true
        }
        if (dir === "up" || dir === "down") { if (spatial(dir)) return true; return dir === "up" ? false : true }
        if (dir === "ok") { act(); return true }
        return false
    }
    function relevel() { if (fk === "level") focus("level", fi) }

    function act() {
        if (!up) return
        if (fk === "listener") Helper.post("/v1/ai/listener", { enabled: seg === 1 })
        else if (fk === "pair") { if (!listenerOn) return; if (pairing) Helper.del("/v1/ai/pairing"); else Helper.post("/v1/ai/pairing", {}) }
        else if (fk === "assistant") {
            var a = assistants[fi]
            if (seg === 0) Helper.post("/v1/ai/assistants/" + a.id + (a.paused ? "/resume" : "/pause"), {})
            else if (armed === "remove:" + a.id) { armed = ""; Helper.del("/v1/ai/assistants/" + a.id) }
            else { armed = "remove:" + a.id; armTimer.restart() }
        } else if (fk === "level") {
            if (!chosen) return
            var levels = {}
            for (var g in chosen.levels) levels[g] = chosen.levels[g]
            levels[levelGroups[fi].id] = levelIds[seg]
            Helper.post("/v1/ai/assistants/" + chosen.id + "/levels", { levels: levels })
        } else if (fk === "lock") {
            if (armed === "lockout") { armed = ""; Helper.post("/v1/ai/lockout", {}) }
            else { armed = "lockout"; armTimer.restart() }
        }
    }
    function adjustLimit(d) { if (up) Helper.post("/v1/ai/limits", { maxVolume: Math.max(0, Math.min(98, maxVolume + d * 2)) }) }
    Timer { id: armTimer; interval: 5000; onTriggered: page.armed = "" }

    // ---- recent activity
    function refreshAudit() {
        Helper.request("GET", "/v1/ai/audit?limit=5", undefined, function(status, doc) {
            try { if (doc && doc.entries) page.audit = doc.entries.slice().reverse() } catch (e) {}
        })
    }
    Component.onCompleted: refreshAudit()
    Connections { target: Helper; onUpdated: if (topic === "ai") page.refreshAudit() }
    function ago(ms) {
        if (!ms) return ""
        var s = Math.max(0, Math.round((Date.now() - ms) / 1000))
        return s < 60 ? "just now" : s < 3600 ? Math.round(s / 60) + " min ago" : s < 86400 ? Math.round(s / 3600) + " h ago" : "yesterday"
    }
    function clock(ms) { return ms ? Qt.formatTime(new Date(ms), "h:mm") : "" }
    function outcomeName(o) { return o === "ok" ? "Allowed" : o === "denied" ? "Denied" : o === "expired" ? "Expired" : o === "never" ? "Blocked" : o === "paused" ? "Paused" : o }
    function outcomeColor(o) { return o === "ok" ? Lu.positive : o === "denied" || o === "never" ? Lu.danger : o === "expired" || o === "paused" ? Lu.warning : Lu.ink2 }
    // an assistant's state line: paused, else when it was last heard from (the request itself is in the activity list)
    function lastFor(a) {
        if (a.paused) return "Paused"
        for (var i = 0; i < audit.length; i++) if (audit[i].assistant && audit[i].assistant.id === a.id) return ago(audit[i].at)
        return a.lastSeenAt ? ago(a.lastSeenAt) : "Active"
    }
    // the helper's group titles, in the short form the column has room for
    readonly property var groupLabels: ({ state: "What's playing", playback: "Playback", launch: "Apps and inputs", power: "Power" })
    readonly property var groupNotes: ({ state: "Title, app, input, volume, on or off", playback: "Including mute", launch: "Only apps already installed",
                                         power: "Never turn it on. The receiver stays as it is." })

    // ---- head
    LuText { id: heading; role: "title"; text: "AI access" }
    LuText {
        visible: !page.up
        y: heading.height + Lu.space(6)
        width: parent.width
        role: "body"; color: Lu.ink2; wrapMode: Text.WordWrap
        text: "The helper is not running. Try again in a moment."
    }

    Item {
        id: body
        visible: page.up
        y: heading.height + Lu.space(4)
        width: parent.width
        height: parent.height - y

        LuRow {
            id: rowListener
            width: parent.width
            icon: "bot"
            label: "Allow AI assistants"
            detail: page.asking && Helper.aiPrompt.kind === "enable_listener" ? "Press OK on the remote" : ""
            help: "Off by default. Turning it on needs OK on the remote, and it reaches the TV from the home network only. AI can never change these settings."
            focused: page.active && page.fk === "listener"; rested: page.rested
            onHoverFocus: page.pointAt("listener", 0)
            onHelpAsked: { page.pointAt("listener", 0); page.openHelp() }
            trailing: LuSegmented {
                options: ["Off", "LAN only"]; current: page.listenerOn ? 1 : 0
                focusedIndex: page.active && page.fk === "listener" ? page.seg : -1
                onPicked: { page.pointAt("listener", 0); page.seg = index; page.act() }
            }
        }
        LuRow {
            id: rowLimit
            y: rowListener.height
            width: parent.width
            icon: "volume-2"
            label: "Volume limit"
            help: "The loudest an assistant may set the receiver. Lower is done at once; higher asks on the TV."
            focused: page.active && page.fk === "limit"; rested: page.rested
            onHoverFocus: page.pointAt("limit", 0)
            onHelpAsked: { page.pointAt("limit", 0); page.openHelp() }
            value: String(page.maxVolume)
        }

        // left: assistants, pairing, recent activity
        Column {
            id: colL
            y: rowListener.height + rowLimit.height + Lu.space(3)
            width: Math.round(parent.width * 0.46)
            LuText { role: "caption"; color: Lu.ink3; text: "Paired assistants" }
            Repeater {
                id: assistRepeater
                model: page.assistants
                delegate: LuRow {
                    width: colL.width
                    icon: "bot"
                    label: modelData.name
                    detail: page.armed === "remove:" + modelData.id ? "Press OK again" : page.lastFor(modelData)
                    focused: page.active && page.fk === "assistant" && page.fi === index; rested: page.rested
                    onHoverFocus: page.pointAt("assistant", index)
                    trailing: LuSegmented {
                        textRole: "caption"
                        options: [modelData.paused ? "Resume" : "Pause", page.armed === "remove:" + modelData.id ? "Sure?" : "Remove"]
                        current: -1
                        focusedIndex: page.active && page.fk === "assistant" && page.fi === index ? page.seg : -1
                        onPicked: { page.pointAt("assistant", index); page.seg = index; page.act() }
                    }
                }
            }
            LuRow {
                id: rowPair
                width: colL.width
                icon: "plus"
                label: page.pairing ? "Pairing code " + page.pairing.code : "Pair a new assistant"
                detail: page.pairing ? page.pairing.attemptsLeft + " tries left" : page.asking && Helper.aiPrompt.kind === "pair" ? "Press OK on the remote" : page.listenerOn ? "" : "Turn access on first"
                help: "A code shows here after OK on the remote. Give it to the assistant once; OK closes the window."
                enabled: page.listenerOn
                focused: page.active && page.fk === "pair"; rested: page.rested
                onActivated: { page.pointAt("pair", 0); page.act() }
                onHoverFocus: page.pointAt("pair", 0)
                onHelpAsked: { page.pointAt("pair", 0); page.openHelp() }
            }
            Item { width: 1; height: Lu.space(2) }
            LuText { visible: page.audit.length > 0; role: "caption"; color: Lu.ink3; text: "Recent activity" }
            Repeater {
                model: page.audit
                delegate: Item {
                    width: colL.width
                    height: Math.round(Lu.row * 0.5)
                    LuText { x: Lu.space(2); anchors.verticalCenter: parent.verticalCenter; role: "caption"; color: Lu.ink3; text: page.clock(modelData.at) }
                    LuText {
                        x: Lu.space(2) + 76; width: parent.width - x - 120; anchors.verticalCenter: parent.verticalCenter
                        role: "caption"; elide: Text.ElideRight
                        text: (modelData.assistant ? modelData.assistant.name + " \u00b7 " : "") + (modelData.message || modelData.tool)
                    }
                    LuText { x: parent.width - width - Lu.space(2); anchors.verticalCenter: parent.verticalCenter; role: "caption"; color: page.outcomeColor(modelData.outcome); text: page.outcomeName(modelData.outcome) }
                }
            }
        }

        // right: what the chosen assistant may do
        Column {
            id: colR
            x: colL.width + Lu.space(5)
            y: colL.y
            width: parent.width - x
            visible: page.nAssist > 0
            LuText { role: "caption"; color: Lu.ink3; text: "What assistants may do" }
            LuSegmented {
                id: chooser
                visible: page.nAssist > 1
                height: visible ? Math.round(Lu.target * 0.85) : 0
                textRole: "caption"
                options: page.assistants.map(function(a) { return a.name })
                current: page.permIndex
                focusedIndex: page.active && page.fk === "chooser" ? page.seg : -1
                onPicked: { page.pointAt("chooser", 0); page.seg = index; page.permIndex = index }
            }
            Repeater {
                id: levelRepeater
                model: page.nAssist > 0 ? page.levelGroups : []
                delegate: LuRow {
                    width: colR.width
                    label: page.groupLabels[modelData.id] || modelData.title
                    help: page.groupNotes[modelData.id] || ""
                    focused: page.active && page.fk === "level" && page.fi === index; rested: page.rested
                    onHoverFocus: page.pointAt("level", index)
                    onHelpAsked: { page.pointAt("level", index); page.openHelp() }
                    trailing: LuSegmented {
                        textRole: "caption"
                        options: page.levelNames
                        current: Math.max(0, page.levelIds.indexOf(page.levelOf(modelData.id)))
                        focusedIndex: page.active && page.fk === "level" && page.fi === index ? page.seg : -1
                        onPicked: { page.pointAt("level", index); page.seg = index; page.act() }
                    }
                }
            }
        }

        LuRow {
            id: rowLock
            y: parent.height - height
            width: colL.width
            icon: "ban"
            label: "Lock out all AI"
            detail: page.armed === "lockout" ? "Press OK again" : ""
            help: "Removes every assistant and turns access off, at once."
            focused: page.active && page.fk === "lock"; rested: page.rested
            onActivated: { page.pointAt("lock", 0); page.act() }
            onHoverFocus: page.pointAt("lock", 0)
            onHelpAsked: { page.pointAt("lock", 0); page.openHelp() }
        }
    }
}
