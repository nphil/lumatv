import QtQuick 2.4
import "../kit"

// AI access (docs/mocks/04-settings-ai-access): what the helper's AI module says (the `ai` topic) and
// the changes the owner makes with the remote (`/v1/ai`). Principle 0 shapes it: the page can ask
// for anything and give less at once (off, pause, remove, a lower level, lock-out), but whatever
// gives more (turning it on, a pairing code, a higher level, resuming) only opens a prompt on the
// TV that the physical remote answers (AiPrompt.qml); the page has no way to answer it, because
// the helper has no route that does.
//
// The page is one list of stops. Up and Down move between them, Left and Right move inside the
// segmented ones, OK acts. Removing an assistant and the lock-out take two presses.
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
    readonly property string summary: !up ? "The helper is not running" : (listenerOn ? "LAN only" : "Off") + " \u00b7 " + assistants.length + (assistants.length === 1 ? " assistant" : " assistants")

    // ---- stops: 0 listener, 1 pairing, then the assistants, the permissions chooser and four groups
    // when there is an assistant, and the lock-out last.
    readonly property int nAssist: assistants.length
    readonly property int iPairing: 1
    readonly property int iAssist: 2
    readonly property int iPerm: iAssist + nAssist
    readonly property int iLevels: nAssist > 0 ? iPerm + 1 : iPerm
    readonly property int nLevels: nAssist > 0 ? levelGroups.length : 0
    readonly property int iLock: iLevels + nLevels
    readonly property int stops: iLock + 1
    property int stop: 0
    property int seg: 0                  // segment focus inside the stop's control
    property int permIndex: 0            // whose permissions the group rows show
    property string armed: ""            // "remove:<id>" or "lockout": the next OK confirms
    readonly property var levelGroups: ai && ai.groups ? ai.groups : []
    readonly property var levelIds: ["always", "ask", "never"]
    readonly property var levelNames: ["Always", "Ask on TV", "Never"]
    readonly property var chosen: nAssist > 0 ? assistants[Math.min(permIndex, nAssist - 1)] : null
    property var audit: []

    function kindOf(n) {
        if (n === 0) return "listener"
        if (n === iPairing) return "pairing"
        if (n >= iAssist && n < iAssist + nAssist) return "assistant"
        if (nAssist > 0 && n === iPerm) return "perm"
        if (n >= iLevels && n < iLevels + nLevels) return "level"
        return "lock"
    }
    function segCount(n) {
        var k = kindOf(n)
        return k === "listener" ? 2 : k === "assistant" ? 2 : k === "level" ? 3 : 1
    }
    function levelOf(group) { return chosen && chosen.levels ? chosen.levels[group] : "ask" }
    function enterStop(n) {
        stop = Math.max(0, Math.min(stops - 1, n))
        armed = ""
        var k = kindOf(stop)
        seg = k === "listener" ? (listenerOn ? 1 : 0) : k === "level" ? Math.max(0, levelIds.indexOf(levelOf(levelGroups[stop - iLevels].id))) : 0
    }
    function pointAt(n) { if (host) host.zone = "page"; enterStop(n) }
    onStopsChanged: if (stop >= stops) enterStop(stops - 1)
    onNAssistChanged: if (permIndex >= nAssist) permIndex = Math.max(0, nAssist - 1)
    onAskingChanged: armed = ""

    function key(dir) {
        if (asking) return true                    // the prompt owns the keys; the remote's own OK and Back reach the helper, not this page
        if (dir === "up") { if (stop === 0) return false; enterStop(stop - 1); return true }
        if (dir === "down") { enterStop(stop + 1); return true }
        var k = kindOf(stop)
        if (dir === "left") {
            if (k === "perm") { if (permIndex > 0) { permIndex--; enterStop(stop) } return true }
            if (seg === 0) return false
            seg--
            return true
        }
        if (dir === "right") {
            if (k === "perm") { if (permIndex < nAssist - 1) { permIndex++; enterStop(stop) } return true }
            seg = Math.min(segCount(stop) - 1, seg + 1)
            return true
        }
        if (dir === "ok") { act(k); return true }
        return false
    }

    function act(k) {
        if (!up) return
        if (k === "listener") {
            Helper.post("/v1/ai/listener", { enabled: seg === 1 })
        } else if (k === "pairing") {
            if (!listenerOn) return
            if (pairing) Helper.del("/v1/ai/pairing")
            else Helper.post("/v1/ai/pairing", {})
        } else if (k === "assistant") {
            var a = assistants[stop - iAssist]
            if (seg === 0) Helper.post("/v1/ai/assistants/" + a.id + (a.paused ? "/resume" : "/pause"), {})
            else if (armed === "remove:" + a.id) { armed = ""; Helper.del("/v1/ai/assistants/" + a.id) }
            else { armed = "remove:" + a.id; armTimer.restart() }
        } else if (k === "level") {
            if (!chosen) return
            var levels = {}
            for (var g in chosen.levels) levels[g] = chosen.levels[g]
            levels[levelGroups[stop - iLevels].id] = levelIds[seg]
            Helper.post("/v1/ai/assistants/" + chosen.id + "/levels", { levels: levels })
        } else if (k === "lock") {
            if (armed === "lockout") { armed = ""; Helper.post("/v1/ai/lockout", {}) }
            else { armed = "lockout"; armTimer.restart() }
        }
    }
    Timer { id: armTimer; interval: 5000; onTriggered: page.armed = "" }

    // ---- recent activity
    function refreshAudit() {
        Helper.request("GET", "/v1/ai/audit?limit=6", undefined, function(status, doc) {
            // the answer can arrive after the page has gone (another section was picked)
            try { if (doc && doc.entries) page.audit = doc.entries.slice().reverse() } catch (e) {}
        })
    }
    Component.onCompleted: { enterStop(0); refreshAudit() }
    onListenerOnChanged: if (stop === 0) seg = listenerOn ? 1 : 0
    Connections { target: Helper; onUpdated: if (topic === "ai") page.refreshAudit() }
    function ago(ms) {
        if (!ms) return ""
        var s = Math.max(0, Math.round((Date.now() - ms) / 1000))
        return s < 60 ? "just now" : s < 3600 ? Math.round(s / 60) + " min ago" : s < 86400 ? Math.round(s / 3600) + " h ago" : "yesterday"
    }
    function outcomeName(o) { return o === "ok" ? "Allowed" : o === "denied" ? "Denied" : o === "expired" ? "Expired" : o === "never" ? "Blocked" : o === "paused" ? "Paused" : o }

    // ---- scrolling: the focused row stays inside the viewport
    property real offset: 0
    function focusedItem() {
        var k = kindOf(stop)
        return k === "listener" ? rowListener : k === "pairing" ? rowPairing : k === "assistant" ? assistRepeater.itemAt(stop - iAssist)
             : k === "perm" ? rowPerm : k === "level" ? levelRepeater.itemAt(stop - iLevels) : rowLock
    }
    onStopChanged: scrollTimer.restart()
    Timer {
        id: scrollTimer
        interval: 1
        onTriggered: {
            var it = page.focusedItem()
            if (!it) return
            var top = it.mapToItem(list, 0, 0).y
            if (top - page.offset < 0) page.offset = top
            else if (top + it.height - page.offset > viewport.height) page.offset = top + it.height - viewport.height
        }
    }

    LuText { id: heading; role: "title"; text: "AI access" }
    LuText { anchors.baseline: heading.baseline; x: heading.width + Lu.space(4); role: "label"; color: Lu.ink2; text: page.summary }
    LuText {
        id: lockNote
        y: heading.height + Lu.space(2)
        width: parent.width
        role: "caption"; color: Lu.ink2; wrapMode: Text.WordWrap
        text: "AI can never change these settings, pair new assistants or answer its own requests. Only this remote can."
    }
    LuText {
        visible: !page.up
        y: lockNote.y + lockNote.height + Lu.space(6)
        width: parent.width
        role: "body"; wrapMode: Text.WordWrap
        text: "The helper that serves AI access is not running, so there is nothing to show or change yet. It starts with Home; try again in a moment."
    }

    Item {
        id: viewport
        visible: page.up
        y: lockNote.y + lockNote.height + Lu.space(4)
        width: parent.width
        height: parent.height - y
        clip: true
        Column {
            id: list
            width: parent.width
            y: -page.offset
            Behavior on y { enabled: !Lu.reduced; NumberAnimation { duration: Lu.cardMs; easing.type: Lu.cardEasing } }

            LuRow {
                id: rowListener
                width: list.width
                label: "Allow AI assistants"
                detail: page.asking && Helper.aiPrompt.kind === "enable_listener" ? "Press OK on the remote to allow it, Back to cancel."
                      : page.listenerOn ? "Reachable from the home network only." : "Off is the default. Turning it on needs OK on the remote."
                focused: page.active && page.stop === 0; rested: page.rested
                onHoverFocus: page.pointAt(0)
                trailing: LuSegmented {
                    options: ["Off", "LAN only"]; current: page.listenerOn ? 1 : 0
                    focusedIndex: page.active && page.stop === 0 ? page.seg : -1
                    onPicked: { page.pointAt(0); page.seg = index; page.act("listener") }
                }
            }
            LuRow {
                id: rowPairing
                width: list.width
                label: page.pairing ? "Pairing code " + page.pairing.code : "Pair a new assistant"
                detail: page.pairing ? "Give it to the assistant once. " + page.pairing.attemptsLeft + " tries left. OK closes the window."
                      : page.asking && Helper.aiPrompt.kind === "pair" ? "Press OK on the remote to show the code, Back to cancel."
                      : page.listenerOn ? "A one-time code shows here after OK on the remote." : "Turn AI access on first."
                enabled: page.listenerOn
                focused: page.active && page.stop === page.iPairing; rested: page.rested
                onActivated: { page.pointAt(page.iPairing); page.act("pairing") }
                onHoverFocus: page.pointAt(page.iPairing)
                value: page.pairing ? "Close" : ""
            }
            Repeater {
                id: assistRepeater
                model: page.assistants
                delegate: LuRow {
                    width: list.width
                    label: modelData.name
                    detail: (modelData.paused ? "Paused" : "Active") + " \u00b7 " + (modelData.calls || 0) + " calls" + (modelData.lastSeenAt ? " \u00b7 last seen " + page.ago(modelData.lastSeenAt) : "")
                          + (page.armed === "remove:" + modelData.id ? " \u00b7 press OK again to remove it" : "")
                    focused: page.active && page.stop === page.iAssist + index; rested: page.rested
                    onHoverFocus: page.pointAt(page.iAssist + index)
                    trailing: LuSegmented {
                        segmentWidth: Lu.target * 2
                        options: [modelData.paused ? "Resume" : "Pause", page.armed === "remove:" + modelData.id ? "Sure?" : "Remove"]
                        current: -1
                        focusedIndex: page.active && page.stop === page.iAssist + index ? page.seg : -1
                        onPicked: { page.pointAt(page.iAssist + index); page.seg = index; page.act("assistant") }
                    }
                }
            }
            LuRow {
                id: rowPerm
                visible: page.nAssist > 0
                height: visible ? implicitHeight : 0
                width: list.width
                label: "What " + (page.chosen ? page.chosen.name : "") + " may do"
                detail: page.nAssist > 1 ? "Left and Right choose the assistant." : "Each group is Always, Ask on TV or Never."
                value: page.nAssist > 1 ? "\u2039 " + (page.permIndex + 1) + " of " + page.nAssist + " \u203a" : ""
                focused: page.active && page.nAssist > 0 && page.stop === page.iPerm; rested: page.rested
                onHoverFocus: if (page.nAssist > 0) page.pointAt(page.iPerm)
            }
            Repeater {
                id: levelRepeater
                model: page.nAssist > 0 ? page.levelGroups : []
                delegate: LuRow {
                    width: list.width
                    label: modelData.title
                    focused: page.active && page.stop === page.iLevels + index; rested: page.rested
                    onHoverFocus: page.pointAt(page.iLevels + index)
                    trailing: LuSegmented {
                        options: page.levelNames
                        current: Math.max(0, page.levelIds.indexOf(page.levelOf(modelData.id)))
                        focusedIndex: page.active && page.stop === page.iLevels + index ? page.seg : -1
                        onPicked: { page.pointAt(page.iLevels + index); page.seg = index; page.act("level") }
                    }
                }
            }
            LuRow {
                id: rowLock
                width: list.width
                label: "Lock out all AI"
                detail: page.armed === "lockout" ? "Press OK again: every assistant is removed and access goes off."
                      : "Removes every assistant and turns access off. Takes effect at once."
                value: page.armed === "lockout" ? "Sure?" : ""
                focused: page.active && page.stop === page.iLock; rested: page.rested
                onActivated: { page.pointAt(page.iLock); page.act("lock") }
                onHoverFocus: page.pointAt(page.iLock)
            }
            Item {
                visible: page.audit.length > 0
                width: list.width
                height: visible ? Lu.space(5) + recentLabel.implicitHeight : 0
                LuText { id: recentLabel; y: Lu.space(5); role: "label"; color: Lu.ink2; text: "Recent activity" }
            }
            Repeater {
                model: page.audit
                delegate: LuRow {
                    width: list.width
                    height: Math.round(Lu.row * 0.7)
                    label: (modelData.assistant ? modelData.assistant.name + " \u00b7 " : "") + (modelData.message || modelData.tool)
                    value: page.outcomeName(modelData.outcome) + " \u00b7 " + page.ago(modelData.at)
                }
            }
        }
    }
}
