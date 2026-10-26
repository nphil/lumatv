import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "AppKeys.js" as AK
import "Choice.js" as C
import "."

// Remote & pointer (docs/mocks/03-settings-remote). The Magic Remote's pointer is the TV's own switch
// (`general.remotePointer`, PLAN "Not owned"): off gives a button-only remote and removes accidental hover
// moves. Speed and size are the service's own settings (`option.pointerSpeed`, `option.pointerSize`). Holding Home
// for LG's Home stays on: it is the escape if LumaTV's Home ever misbehaves, so it is shown, not switchable.
// Whether the number keys open the dock's tiles is LumaTV's own (`remote.numberKeys` in theme.json, default on).
// Each app key (Netflix, Prime Video, Disney+, LG Channels) opens the chooser (PageAppKey) and keeps its answer in `remote.appKeys`
// (settings/AppKeys.js): LG's app where it is installed until the owner picks another app or nothing.
// Voice search is the mic key's: the helper's `base.voice` owner has its switch (off until the owner turns it on here), and the
// helper's `voice` topic says whether it can listen yet and, when it cannot, why. Which Home Assistant it speaks to is not a
// setting: it is linked from a command line on the TV (docs/research/voice-result.md).
PageList {
    id: page
    title: "Remote & pointer"
    tabs: ["Pointer", "Keys", "Voice"]
    HelperSettings { id: voiceSettings; owner: "base.voice" }
    readonly property var voiceTopic: Helper.topics.voice || null
    readonly property bool voiceOn: voiceSettings.known && voiceSettings.get("enabled") === true
    // What the helper says is missing, in words for the owner (the row's help); empty when voice search can listen.
    readonly property string voiceNeeds: {
        var r = voiceTopic ? voiceTopic.reason : ""
        return r === "voice_unlinked" ? "Home Assistant is not linked yet. Link it from the TV's command line (see the voice notes), then it listens."
             : r === "no_microphone" ? "No microphone is set in the helper's configuration."
             : r === "link_unreadable" ? "The saved Home Assistant link cannot be read. Link it again."
             : !voiceTopic ? "The TV's helper does not answer for voice search." : ""
    }
    // and in two words (the row's value)
    readonly property string voiceStatus: {
        var r = voiceTopic ? voiceTopic.reason : ""
        return r === "voice_unlinked" ? "Not linked" : r === "no_microphone" ? "No microphone" : r === "link_unreadable" ? "Link unreadable" : !voiceTopic ? "Not answering" : ""
    }
    readonly property var theme: Lu.theme || ({})
    readonly property bool pointerOn: String(Sio.get("general.remotePointer")) === "on"
    summary: Sio.has("general.remotePointer") ? (pointerOn ? "Pointer on" : "Pointer off") : ""

    function refresh() {
        voiceSettings.refresh()
        Sio.read("general", ["remotePointer"])
        Sio.read("option", ["pointerSpeed", "pointerSize", "remoteSlowInput", "longPressInterval", "repeatInterval"])
    }
    Component.onCompleted: refresh()
    onOpened: refresh()
    // LG's times for a long press and for the repeat: "Basic", or a number of seconds; both answer only while Extend Pressing Time is on
    readonly property var seconds: ({ basic: "Basic", "0.5": "0.5 s", "1": "1 s", "1.5": "1.5 s", "2": "2 s", "2.5": "2.5 s", "3": "3 s", "3.5": "3.5 s", "4": "4 s", "4.5": "4.5 s", "5": "5 s" })
    readonly property bool slowOn: String(Sio.get("option.remoteSlowInput")) === "on"
    function slow(row) {
        return R.over(row, { enabled: function() { return page.slowOn && Sio.usable(row.id) }, detail: function() { return !Sio.has(row.id) ? "TV not answering" : page.slowOn ? "" : "Turn Extend Pressing Time on" } })
    }
    // One row per app key: what it opens now, and OK opens the chooser (PageAppKey) for it.
    readonly property var keyCfg: AK.config(theme)
    readonly property var installed: AK.installed(Apps.launchPoints)
    function appKeyRow(k) {
        return { kind: "value", tab: 1, icon: k.icon, label: k.label + " key",
                 value: function() { var t = AK.target(page.keyCfg, k.id, page.installed); return t.installed ? t.title : "Nothing" },
                 act: function() { if (page.host) page.host.openPage("appkey", { key: k.id }) } }
    }

    rows: [
        R.toggle(Sio, { ds: "general.remotePointer", tab: 0, icon: "mouse-pointer-2", label: "Magic Remote pointer",
                        help: "Use the pointer on screen. Turn off if you only use a button remote." }),
        R.seg(Sio, { ds: "option.pointerSpeed", tab: 0, icon: "gauge", label: "Pointer speed",
                     options: ["Slow", "Normal", "Fast"], values: ["slow", "normal", "fast"] }),
        R.seg(Sio, { ds: "option.pointerSize", tab: 0, icon: "scan-eye", label: "Pointer size",
                     options: ["Small", "Medium", "Large"], values: ["small", "medium", "large"] }),

        { kind: "value", tab: 1, icon: "house", label: "Hold Home for LG's Home", info: true, chevron: false, value: "Always on",
          help: "Press and hold Home for 650 ms to open LG's own Home if LumaTV misbehaves." },
        { kind: "toggle", tab: 1, icon: "hash", label: "Number keys open dock tiles", help: "1 opens the first tile on the dock, 2 the second, and so on.",
          on: function() { return !page.theme.remote || page.theme.remote.numberKeys !== false },
          act: function(v) { if (page.host) page.host.patch({ remote: { numberKeys: v } }) } },
        { kind: "toggle", tab: 1, icon: "bell", label: "Hold the gear key for notifications", help: "Hold Settings for a moment to open the notifications. Off gives the hold back to LG's Settings.",
          on: function() { return !page.theme.remote || page.theme.remote.gearHold !== false },
          act: function(v) { if (page.host) page.host.patch({ remote: { gearHold: v } }) } },
        { kind: "toggle", tab: 1, icon: "layout-grid", label: "Quick panel on gear",
          on: function() { return !page.theme.remote || page.theme.remote.quickPanel !== false },
          act: function(v) { if (page.host) page.host.patch({ remote: { quickPanel: v } }) } },
        { kind: "toggle", tab: 1, icon: "cable", label: "Input picker on Input key",
          on: function() { return !page.theme.remote || page.theme.remote.inputPicker !== false },
          act: function(v) { if (page.host) page.host.patch({ remote: { inputPicker: v } }) } },
        R.head("App keys", 1)
    ].concat(AK.keys.map(function(k) { return page.appKeyRow(k) }), [
        R.toggle(Sio, { ds: "option.remoteSlowInput", tab: 1, icon: "timer", label: "Extend Pressing Time on Remote",
                        help: "For a slower hand: a key must be held longer to count as a long press, and repeats come slower." }),
        page.slow(C.choice(Sio, { ds: "option.longPressInterval", tab: 1, icon: "hourglass", label: "Activate time for long press", names: page.seconds, page: page,
                                  help: "How long a key is held before it counts as a long press." })),
        page.slow(C.choice(Sio, { ds: "option.repeatInterval", tab: 1, icon: "fast-forward", label: "Interval time for repeated actions", names: page.seconds, page: page,
                                  help: "How long between the repeats while a key is held." })),

        R.toggle(voiceSettings, { ds: "enabled", tab: 2, on: "true", off: "false", icon: "mic", label: "Search by voice",
                                  help: "Hold the mic key and speak; the words go into search. The mic is open only while held." }),
        { kind: "value", tab: 2, icon: "mic", label: "Voice search", info: true, chevron: false, value: function() { return page.voiceStatus },
          help: function() { return page.voiceNeeds }, visible: function() { return page.voiceOn && page.voiceNeeds !== "" } }
    ])
}
