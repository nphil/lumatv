import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "Choice.js" as C
import "Names.js" as N
import "."

// One Bluetooth device (LG: the device's own screen under Registered Devices): connect or disconnect it, or delete it from the paired
// list. Opened with `host.openPage("btdevice", { kind, address, name, connected, surround, back, backId })` by PageBluetooth; `kind` is "audio"
// (`btaudiosrc`: connect, disconnect, removePairedDevice, all by `address`) or "hid" (`bthidmanager`: requestConnect with the name and
// address, requestDisconnect and requestDelete by `deviceAddr`). Disconnecting and deleting ask first; both answers are done here at
// the remote, and the page goes back to the list.
// A connected speaker has LG's Position row (`btaudiosrc setChannel {address, from, to}`, LG's SpeakerSoundItemPanel): a chooser of the two
// positions that go with the one it has (Left and Right, otherwise Front and Rear), live only while the sound out is the Bluetooth speaker
// with the TV's. LG asks before Left and Right swap, since the speaker that holds the other one takes this one's place.
PageList {
    id: page
    readonly property var dev: arg && arg.address ? arg : null
    readonly property bool audio: !dev || dev.kind === "audio"
    readonly property string svc: audio ? "luna://com.webos.service.btaudiosrc" : "luna://com.webos.service.bthidmanager"
    property string position: "none"        // the speaker's position as getPairedDevices says it
    property bool connected: false          // as getPairedDevices says it now; the list's answer until then
    readonly property bool withTv: String(Sio.get("sound.soundOutput")) === "tv_speaker_bluetooth"
    title: dev ? String(dev.name) : ""
    back: dev && dev.back ? dev.back : ""
    backId: dev && dev.backId ? dev.backId : ""

    LunaCall {
        id: luna
        onAnswered: {
            if (!doc || doc.returnValue === false) return
            if (tag === "channel") luna.call("paired", page.svc, "getPairedDevices", {})        // as LG's panel does, once the service has answered
            else if (tag === "paired" && page.dev) {
                var d = doc.devices || []
                for (var i = 0; i < d.length; i++) if (d[i].address === page.dev.address) {
                    page.position = d[i].surround || "none"
                    page.connected = d[i].state === "connected"
                }
            }
        }
    }
    onOpened: {
        position = dev && dev.surround ? String(dev.surround) : "none"
        connected = !!dev && dev.connected === true
        if (!audio || !dev) return
        Sio.read("sound", ["soundOutput"])
        luna.call("paired", svc, "getPairedDevices", {})
    }

    function done() { if (host) host.up() }
    function connectNow() {
        if (!dev) return
        if (audio) luna.act(svc, "connect", { address: dev.address })
        else luna.act(svc, "requestConnect", { name: dev.name, address: dev.address })
        done()
    }
    function disconnectNow() {
        if (!dev) return
        luna.act(svc, audio ? "disconnect" : "requestDisconnect", audio ? { address: dev.address } : { deviceAddr: dev.address })
        done()
    }
    function deleteNow() {
        if (!dev) return
        luna.act(svc, audio ? "removePairedDevice" : "requestDelete", audio ? { address: dev.address } : { deviceAddr: dev.address })
        done()
    }

    function positions() {
        var pair = position === "left" || position === "right" ? ["left", "right"] : ["front", "rear"]
        return pair.map(function(v) { return { value: v, label: N.name("speakerPosition", v), enabled: true } })
    }
    function setPosition(to) {
        if (!dev || to === position) return
        var go = function() { luna.call("channel", page.svc, "setChannel", { address: page.dev.address, from: page.position, to: to }) }
        if ((position === "left" && to === "right") || (position === "right" && to === "left"))
            C.ask(page, { icon: "speaker", title: "Swap Left and Right?", body: "The other speaker takes this position.", yes: "Swap", no: "Keep it" }, go)()
        else go()
    }
    function positionRow() {
        var row = {
            kind: "value", id: "position", icon: "speaker", label: "Position", chevron: true,
            visible: function() { return page.audio && page.connected },
            enabled: function() { return page.withTv },
            detail: function() { return page.withTv ? "" : "Needs TV + Bluetooth" },
            value: function() { return N.name("speakerPosition", page.position) },
            options: function() { return page.positions() },
            current: function() { return page.position },
            pick: function(i) { var o = page.positions()[i]; if (o) page.setPosition(o.value) },
            act: function() { page.host.openPage("choice", { row: row, back: page.title, backId: page.host.current }) }
        }
        return row
    }

    rows: [
        { kind: "value", id: "toggle", icon: "plug", chevron: false, label: page.connected ? "Disconnect" : "Connect",
          act: function() {
              if (!page.dev) return
              if (page.connected) C.ask(page, { icon: "unplug", title: "Disconnect " + page.dev.name + "?", yes: "Disconnect", no: "Keep it" }, page.disconnectNow)()
              else page.connectNow()
          } },
        positionRow(),
        C.action({ id: "delete", icon: "trash-2", label: "Delete Device", button: "Delete", danger: true,
                   act: C.ask(page, { icon: "trash-2", title: "Delete " + (dev ? dev.name : "") + "?", body: "It is removed from the paired list.", yes: "Delete", no: "Keep it", danger: true }, page.deleteNow) })
    ]
}
