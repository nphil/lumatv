import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "Choice.js" as C
import "."

// One Bluetooth device (LG: the device's own screen under Registered Devices): connect or disconnect it, or delete it from the paired
// list. Opened with `host.openPage("btdevice", { kind, address, name, connected, back, backId })` by PageBluetooth; `kind` is "audio"
// (`btaudiosrc`: connect, disconnect, removePairedDevice, all by `address`) or "hid" (`bthidmanager`: requestConnect with the name and
// address, requestDisconnect and requestDelete by `deviceAddr`). Disconnecting and deleting ask first; both answers are done here at
// the remote, and the page goes back to the list.
PageList {
    id: page
    readonly property var dev: arg && arg.address ? arg : null
    readonly property bool audio: !dev || dev.kind === "audio"
    readonly property string svc: audio ? "luna://com.webos.service.btaudiosrc" : "luna://com.webos.service.bthidmanager"
    title: dev ? String(dev.name) : ""
    back: dev && dev.back ? dev.back : ""
    backId: dev && dev.backId ? dev.backId : ""

    LunaCall { id: luna }
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

    rows: [
        { kind: "value", id: "toggle", icon: "plug", chevron: false, label: dev && dev.connected ? "Disconnect" : "Connect",
          act: function() {
              if (!page.dev) return
              if (page.dev.connected) C.ask(page, { icon: "unplug", title: "Disconnect " + page.dev.name + "?", yes: "Disconnect", no: "Keep it" }, page.disconnectNow)()
              else page.connectNow()
          } },
        C.action({ id: "delete", icon: "trash-2", label: "Delete Device", button: "Delete", danger: true,
                   act: C.ask(page, { icon: "trash-2", title: "Delete " + (dev ? dev.name : "") + "?", body: "It is removed from the paired list.", yes: "Delete", no: "Keep it", danger: true }, page.deleteNow) })
    ]
}
