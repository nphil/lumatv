import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "Choice.js" as C
import "."

// Time & Date (LG: General › System › Time & Timers): Set Automatically, the time zone, and the time and date by hand. LG's own calls:
// `time.autoClock` with the system preference `useNetworkTime` (both, as its row writes them), the zone list from `getPreferenceValues`
// (the TV's zones for the country of the zone in force), the zone itself as the setting `time.timeZone` with the preference `timeZone`,
// and the clock by `time/setSystemTime {utc}`. The hour, minute and date rows answer only while Set Automatically is off; a change is sent
// a second after the last press, as LG's does, so holding a key does not send every step.
PageList {
    id: page
    title: "Time & Date"
    back: "System"
    property bool automatic: true                // the preference `useNetworkTime`
    property var zone: ({})                      // the zone in force (the preference `timeZone`)
    property var zones: []                       // every zone the TV lists
    property var draft: new Date()               // the clock as shown while it is being set
    property bool editing: false
    readonly property bool manual: !automatic
    readonly property string zoneName: zone && (zone.City || zone.Description || zone.ZoneID) ? (zone.City || zone.Description || zone.ZoneID) : ""
    summary: clockText(draft)

    LunaCall {
        id: luna
        onAnswered: {
            if (!doc || doc.returnValue === false) return
            if (tag === "prefs") {
                if (doc.useNetworkTime !== undefined) page.automatic = !!doc.useNetworkTime
                if (doc.timeZone) page.zone = doc.timeZone
            } else if (tag === "zones" && doc.timeZone) page.zones = doc.timeZone
            else if (tag === "time" && doc.adjustedUtc !== undefined && !page.editing) page.draft = new Date(doc.adjustedUtc * 1000)
        }
    }
    function refresh() {
        Sio.read("time", ["autoClock", "timeZone"])
        luna.subscribe("prefs", "luna://com.palm.systemservice", "getPreferences", { keys: ["useNetworkTime", "timeZone"] })
        luna.call("zones", "luna://com.palm.systemservice", "getPreferenceValues", { key: "timeZone" })
        luna.call("time", "luna://com.palm.systemservice/time", "getEffectiveBroadcastTime", {})
    }
    Component.onCompleted: Sio.read("time", ["autoClock", "timeZone"])
    onOpened: refresh()
    onVisibleChanged: if (!visible) luna.close("prefs")

    function two(n) { return (n < 10 ? "0" : "") + n }
    function clockText(d) { var h = d.getHours() % 12; return (h === 0 ? 12 : h) + ":" + two(d.getMinutes()) + (d.getHours() < 12 ? " AM" : " PM") }
    readonly property var months: ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"]
    function daysIn(y, m) { return new Date(y, m + 1, 0).getDate() }

    // One press on a field: the draft moves a step (a month or a day wraps within its own range) and goes to the TV a second later.
    function step(field, d) {
        var t = new Date(draft.getTime())
        var y = t.getFullYear(), mo = t.getMonth(), dd = t.getDate()
        if (field === "h") t.setHours((t.getHours() + d + 24) % 24)
        else if (field === "m") t.setMinutes((t.getMinutes() + d + 60) % 60)
        else if (field === "y") { y = Math.max(2020, Math.min(2037, y + d)); t = new Date(y, mo, Math.min(dd, daysIn(y, mo)), t.getHours(), t.getMinutes(), 0) }
        else if (field === "mo") { mo = (mo + d + 12) % 12; t = new Date(y, mo, Math.min(dd, daysIn(y, mo)), t.getHours(), t.getMinutes(), 0) }
        else if (field === "d") { dd = ((dd - 1 + d + daysIn(y, mo)) % daysIn(y, mo)) + 1; t = new Date(y, mo, dd, t.getHours(), t.getMinutes(), 0) }
        t.setSeconds(0)
        draft = t
        editing = true
        send.restart()
    }
    Timer {
        id: send
        interval: 1000
        onTriggered: { luna.act("luna://com.palm.systemservice", "time/setSystemTime", { utc: Math.floor(page.draft.getTime() / 1000) }); page.editing = false }
    }

    function nudge(which, label, icon, text) {
        return { kind: "nudge", tab: 0, id: which, icon: icon, label: label, value: text,
                 enabled: function() { return page.manual },
                 detail: function() { return page.manual ? "" : "Set Automatically is on" },
                 act: function(d) { page.step(which, d) } }
    }

    // The zone row has the shape of a choice row, so the one chooser page lists it.
    readonly property var zoneRow: ({
        kind: "value", tab: 0, id: "zone", icon: "globe", label: "Time Zone", chevron: true,
        enabled: function() { return page.zones.length > 0 },
        value: function() { return page.zoneName },
        options: function() {
            var cc = page.zone ? page.zone.CountryCode : ""
            var out = []
            for (var i = 0; i < page.zones.length; i++) {
                var z = page.zones[i]
                if (cc && z.CountryCode !== cc) continue
                out.push({ value: z.ZoneID, label: z.City || z.Description || z.ZoneID, enabled: true })
            }
            return out
        },
        current: function() { return page.zone ? String(page.zone.ZoneID) : "" },
        pick: function(i) {
            var o = page.zoneRow.options()[i]
            if (!o) return
            for (var k = 0; k < page.zones.length; k++) if (page.zones[k].ZoneID === o.value) {
                luna.act("luna://com.palm.systemservice", "setPreferences", { timeZone: page.zones[k] })
                Sio.writeMany("time", { timeZone: page.zones[k] })
                page.zone = page.zones[k]
            }
        },
        act: function() { page.host.openPage("choice", { row: page.zoneRow, back: page.title, backId: "clock" }) }
    })

    rows: [
        { kind: "toggle", tab: 0, id: "auto", icon: "clock", label: "Set Automatically",
          on: function() { return page.automatic },
          act: function(v) {
              page.automatic = v
              Sio.write("time.autoClock", v ? "on" : "off")
              luna.act("luna://com.palm.systemservice", "setPreferences", { useNetworkTime: v })
          } },
        zoneRow,
        nudge("h", "Hour", "clock", function() { var h = page.draft.getHours() % 12; return (h === 0 ? 12 : h) + (page.draft.getHours() < 12 ? " AM" : " PM") }),
        nudge("m", "Minute", "clock", function() { return page.two(page.draft.getMinutes()) }),
        nudge("y", "Year", "history", function() { return String(page.draft.getFullYear()) }),
        nudge("mo", "Month", "history", function() { return page.months[page.draft.getMonth()] }),
        nudge("d", "Day", "history", function() { return String(page.draft.getDate()) })
    ]
}
