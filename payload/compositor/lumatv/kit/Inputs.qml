pragma Singleton
import QtQuick 2.4
import WebOSServices 1.0

// What the TV knows about its inputs, for the input picker and the Settings pages: each HDMI port's name
// (the device names itself over HDMI-CEC) and whether anything is connected, from the external input
// manager (`com.webos.service.eim/getAllInputStatus`, PLAN "Reference: ecklf/lg-luma"), and which app is
// in front (`getForegroundAppInfo`), which is the current input when its id is an input's. The picker
// asks for a refresh when it opens; the service is also asked every 30 s while a surface that shows
// inputs is up (`watch`), so a cable plugged in reads as connected without a restart.
//
// The device icon follows the name the device gave itself, then the port's own label; the owner's choice
// per input (Auto, Always, Hidden) lives in the helper's settings and is applied by the surfaces.
QtObject {
    id: inputs

    property var list: []                  // [{id, appId, port, label, name, connected, icon}]
    property string foreground: ""         // app id in front
    property int watchers: 0               // surfaces that want the list kept fresh
    readonly property var connected: list.filter(function(i) { return i.connected })
    readonly property var current: {
        for (var i = 0; i < list.length; i++) if (list[i].appId === foreground) return list[i]
        return null
    }

    // The icon for what is on the port: the device's own name first.
    function iconFor(name, port) {
        var n = String(name || "").toLowerCase()
        if (/play ?station|ps[345]|xbox|switch|nintendo|console|steam deck/.test(n)) return "gamepad-2"
        if (/apple tv|chromecast|fire ?tv|roku|shield|streamer|nvidia|google tv/.test(n)) return "tv-minimal-play"
        if (/denon|marantz|yamaha|onkyo|receiver|avr|soundbar|sonos|bose|speaker|arc/.test(n)) return "speaker"
        if (/blu-?ray|dvd|player|disc|magnavox/.test(n)) return "disc-3"
        if (/pc|computer|laptop|mac|desktop|monitor/.test(n)) return "monitor"
        if (/antenna|tuner|cable box|set.?top/.test(n)) return "antenna"
        return n === "" ? "unplug" : "cable"
    }
    // What a tile is for: the receiver (the sound output when the sound goes out over eARC) is told from its name.
    function isReceiver(name) { return /denon|marantz|yamaha|onkyo|receiver|avr/i.test(String(name || "")) }
    // The owner's choice for an input (theme `inputs.<port>`: auto, always, hidden): Auto shows it only while something is connected.
    function visibleFor(choices, key, connected) {
        var c = (choices && choices[key]) || "auto"
        return c === "always" || (c === "auto" && connected)
    }
    // The tiles of the picker and of Home's Inputs row (PLAN, "Inputs on Home"), one shape for both: Live TV first, then each HDMI
    // port the owner has not hidden. `choices` is theme.inputs. A port with nothing connected is there only when it is Always.
    //   { appId, key, port, icon, name, state, on, connected, front, sound }
    function tiles(choices) {
        var out = []
        if (visibleFor(choices, "livetv", true))
            out.push({ appId: "com.webos.app.livetv", key: "livetv", port: "LIVE TV", icon: "antenna", name: "Antenna", state: "Live TV", on: true, connected: true, front: foreground === "com.webos.app.livetv", sound: false })
        for (var i = 0; i < list.length; i++) {
            var e = list[i]
            var key = "hdmi" + (/hdmi(\d)/.exec(e.appId) || [0, i + 1])[1]
            if (!visibleFor(choices, key, e.connected)) continue
            var front = e.appId === foreground
            var sound = e.connected && isReceiver(e.label)
            out.push({ appId: e.appId, key: key, port: e.port.toUpperCase() + (sound ? " \u00b7 eARC" : ""),
                       icon: e.icon, name: e.connected ? (e.label || e.port) : "Nothing connected",
                       state: !e.connected ? "Plug in a device to use it" : front ? "In front" : "Connected", on: e.connected, connected: e.connected, front: front, sound: sound })
        }
        return out
    }
    function portName(appId) {
        var m = /hdmi(\d)/.exec(String(appId))
        return m ? "HDMI " + m[1] : appId === "com.webos.app.livetv" ? "Live TV" : String(appId)
    }

    function parse(payload) {
        var doc = null
        try { doc = JSON.parse(payload) } catch (e) { return }
        if (!doc || doc.returnValue === false) return
        var src = doc.devices || doc.inputs || doc.deviceList || []
        var out = []
        for (var i = 0; i < src.length; i++) {
            var d = src[i]
            var appId = d.appId || d.id || ""
            var label = d.label || d.deviceLabel || d.name || ""
            var port = portName(appId)
            var up = d.connected === true || d.activate === true || d.connected === "true"
            out.push({ id: d.id || appId, appId: appId, port: port, label: label, name: up ? (label || "") : "", connected: up,
                       icon: iconFor(up ? label : "", port) })
        }
        // A refresh usually repeats the subscription's last answer; replacing it would rebind every picker tile.
        var changed = out.length !== list.length
        for (var j = 0; !changed && j < out.length; j++) {
            var a = list[j], b = out[j]
            changed = a.id !== b.id || a.appId !== b.appId || a.port !== b.port || a.label !== b.label
                   || a.name !== b.name || a.connected !== b.connected || a.icon !== b.icon
        }
        if (changed) list = out
    }
    function parseForeground(payload) {
        try {
            var doc = JSON.parse(payload)
            var f = doc.foregroundAppInfo
            foreground = f && f.length ? (f[0].appId || "") : (doc.appId || "")
        } catch (e) { }
    }

    // Each ask is a new subscription: LG's `Service.callService` returns the call's token and keeps the call open until it is cancelled
    // (libwebosserviceplugin.so), so the one before is let go first. Without that every panel opening, every Settings page and every 30 s
    // watch left one more open call per service, each answering every input change and every app switch again.
    property double eimToken: 0                // LSMessageToken is an unsigned long: an int could turn it negative
    property double fgToken: 0
    function resubscribe(svc, token) {
        if (token > 0) svc.cancel(token)
        var t = svc.callService({ subscribe: true })
        return typeof t === "number" && t > 0 ? t : 0
    }
    function refresh() {
        eimToken = resubscribe(eim, eimToken)
        fgToken = resubscribe(fg, fgToken)
    }
    function watch(on) { watchers = Math.max(0, watchers + (on ? 1 : -1)) }

    property Service eim: Service {
        appId: "com.webos.surfacemanager"
        service: "luna://com.webos.service.eim"
        method: "getAllInputStatus"
        onResponse: inputs.parse(payload)
    }
    property Service fg: Service {
        appId: "com.webos.surfacemanager"
        service: "luna://com.webos.service.applicationmanager"
        method: "getForegroundAppInfo"
        onResponse: inputs.parseForeground(payload)
    }
    property Timer keep: Timer {
        interval: 30000
        repeat: true
        running: inputs.watchers > 0
        onTriggered: inputs.refresh()
    }
    Component.onCompleted: refresh()
}
