pragma Singleton
import QtQuick 2.4
import WebOSCompositorBase 1.0
import WebOSServices 1.0

// The two power facts Settings and the quick panel show that the settings service does not hold: the sleep timer and
// the hours the panel has run. Both are LG services LG's own Settings app calls (research/lgsettings/main.js).
//
// Sleep timer: `tvReservationAgent` `timer/sleep/getRemainTime` answers {sleepTimeMinute, sleepTimeSecond} (what is left; an
// idle timer answers 0 seconds and no minutes) and `timer/sleep/add {sleepTimeMinute: "30"}` sets it ("0" clears it). LG's
// choices are Off, 10, 20, 30, 60, 90, 120, 180 and 240 minutes, and its row shows the first choice that is not shorter than
// what is left. Panel hours: `panelcontroller` `getPanelUsageTime` answers {panelUsageTime}, LG shows floor(t / 6) hours
// (the counter ticks every ten minutes); LG shows the row only where the config `tv.conti.supportUsedTime` is true,
// read with `com.webos.service.config/getConfigs`. Neither reply has been read on this TV; until one arrives the sleep
// timer shows Off and the panel hours show nothing, never a made-up number.
QtObject {
    id: power

    // ---- sleep timer
    readonly property var sleepMinutes: [0, 10, 20, 30, 60, 90, 120, 180, 240]
    readonly property var sleepLabels: ["Off", "10 min", "20 min", "30 min", "1 h", "90 min", "2 h", "3 h", "4 h"]
    property real sleepSeconds: 0                     // what is left; 0 when none runs or the service has not said
    readonly property int sleepIndex: {
        if (sleepSeconds <= 0) return 0
        var m = Math.floor(sleepSeconds / 60)           // LG compares the whole minutes left
        for (var i = 1; i < sleepMinutes.length; i++) if (m <= sleepMinutes[i]) return i
        return sleepMinutes.length - 1
    }
    // "Off in 23 min" while one runs; the words for a timer that is not set otherwise.
    readonly property string sleepLeft: sleepSeconds <= 0 ? "" : sleepSeconds < 60 ? "Off in under a minute" : "Off in " + Math.round(sleepSeconds / 60) + " min"

    function readSleep() { sleepSvc.callService({}) }
    function setSleep(i) {
        var m = sleepMinutes[i]
        sleepSeconds = m * 60                         // shown at once; the service's answer replaces it
        LS.adhoc.call("luna://com.webos.service.tvReservationAgent", "/timer/sleep/add", JSON.stringify({ sleepTimeMinute: String(m) }))
        sleepAgain.restart()
    }
    function parseSleep(payload) {
        var doc = null
        try { doc = JSON.parse(payload) } catch (e) { return }
        if (!doc || doc.returnValue === false) return
        sleepSeconds = (Number(doc.sleepTimeMinute) || 0) * 60 + (Number(doc.sleepTimeSecond) || 0)
    }
    property Service sleepSvc: Service {
        appId: "com.webos.surfacemanager"
        service: "luna://com.webos.service.tvReservationAgent/timer/sleep"
        method: "getRemainTime"
        onResponse: power.parseSleep(payload)
    }
    // the service takes a moment to start the countdown: read it back once after a change
    property Timer sleepAgain: Timer { interval: 700; onTriggered: power.readSleep() }

    // ---- panel hours
    property real panelUsage: -1                      // LG's counter (ten-minute ticks); -1 until the TV says
    property var supported: undefined                 // tv.conti.supportUsedTime once read
    readonly property bool panelShown: supported !== false
    readonly property string panelHours: panelUsage < 0 ? "" : grouped(Math.floor(panelUsage / 6)) + " h"
    // 1705 -> "1,705" (a lookahead regex gave "1,,705" in the offline host's QML engine)
    function grouped(n) {
        var s = String(n), out = ""
        for (var i = 0; i < s.length; i++) out += (i > 0 && (s.length - i) % 3 === 0 ? "," : "") + s.charAt(i)
        return out
    }
    property bool panelAsked: false

    function readPanel() {
        if (panelAsked) return
        panelAsked = true
        configSvc.callService({ configNames: ["tv.conti.supportUsedTime"] })
        panelSvc.callService({ subscribe: true })      // LG subscribes; the counter then arrives by itself
    }
    function parsePanel(payload) {
        var doc = null
        try { doc = JSON.parse(payload) } catch (e) { doc = null }
        if (!doc || doc.returnValue === false || typeof doc.panelUsageTime !== "number") { panelAsked = false; return }
        panelUsage = doc.panelUsageTime
    }
    function parseConfig(payload) {
        var doc = null
        try { doc = JSON.parse(payload) } catch (e) { return }
        if (!doc || doc.returnValue === false || !doc.configs) return
        var v = doc.configs["tv.conti.supportUsedTime"]
        supported = !(v === "false" || v === false || v === 0 || v === null || v === undefined)
    }
    property Service panelSvc: Service {
        appId: "com.webos.surfacemanager"
        service: "luna://com.webos.service.panelcontroller"
        method: "getPanelUsageTime"
        onResponse: power.parsePanel(payload)
    }
    property Service configSvc: Service {
        appId: "com.webos.surfacemanager"
        service: "luna://com.webos.service.config"
        method: "getConfigs"
        onResponse: power.parseConfig(payload)
    }
}
