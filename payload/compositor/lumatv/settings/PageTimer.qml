import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "Choice.js" as C
import "."

// The Power On Timer and the Power Off Timer (LG: General › System › Time & Timers › Timers), one page with two sets of LG's keys
// (`time.onTimer*` and `time.offTimer*`; PagePowerOn and PagePowerOff set `which`). The time is LG's hour and minute, the days are LG's
// weekday mask (bit 0 Sunday to bit 6 Saturday, 0 for none, as its day picker reads and writes it), and the power-on timer also takes the
// input to switch to and the volume to start at. The channel LG asks for when the input is Live TV is a pick from the TV's channel
// list; with no tuner in use the input list offers the HDMI inputs.
PageList {
    id: page
    property string which: "on"
    readonly property string pre: "time." + which + "Timer"
    readonly property var days: ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"]
    readonly property int mask: Math.max(0, Sio.num(pre + "Weekday", 0))
    title: which === "on" ? "Power On Timer" : "Power Off Timer"
    back: "System"
    tabs: ["Time", "Days"]

    function refresh() {
        Sio.read("time", [which + "TimerEnable", which + "TimerHour", which + "TimerMinute", which + "TimerWeekday"].concat(which === "on" ? ["onTimerSource", "onTimerVolume"] : []))
    }
    Component.onCompleted: refresh()
    onOpened: refresh()

    function hourText(h) { var n = h % 12; return (n === 0 ? 12 : n) + (h < 12 ? " AM" : " PM") }
    function minuteText(m) { return (m < 10 ? "0" : "") + m }
    function nudge(ds, label, icon, max, text) {
        return { kind: "nudge", tab: 0, id: ds, icon: icon, label: label, visible: function() { return C.shown(Sio, ds) }, enabled: function() { return Sio.usable(ds) },
                 value: function() { return Sio.has(ds) ? text(Sio.num(ds)) : "" },
                 act: function(d) { Sio.write(ds, (Sio.num(ds) + d + max) % max) } }
    }
    function day(i) {
        return { kind: "toggle", tab: 1, id: "day" + i, icon: "clock", label: page.days[i], enabled: function() { return Sio.usable(page.pre + "Weekday") },
                 on: function() { return (page.mask & (1 << i)) !== 0 },
                 act: function(v) { Sio.write(page.pre + "Weekday", v ? (page.mask | (1 << i)) : (page.mask & ~(1 << i))) } }
    }

    rows: [
        R.toggle(Sio, { ds: pre + "Enable", tab: 0, icon: "power", label: "Timer" }),
        nudge(pre + "Hour", "Hour", "clock", 24, hourText),
        nudge(pre + "Minute", "Minute", "clock", 60, minuteText),
        R.over(C.choice(Sio, { ds: "time.onTimerSource", tab: 0, icon: "cable", label: "Input", names: "timerInput", page: page,
                               values: ["hdmi1", "hdmi2", "hdmi3", "hdmi4"] }), { visible: function() { return page.which === "on" && C.shown(Sio, "time.onTimerSource") } }),
        R.over(R.slider(Sio, { ds: "time.onTimerVolume", tab: 0, min: 0, max: 100, step: 1, icon: "volume-2", label: "Volume" }),
               { visible: function() { return page.which === "on" && C.shown(Sio, "time.onTimerVolume") } }),
        day(0), day(1), day(2), day(3), day(4), day(5), day(6)
    ]
}
