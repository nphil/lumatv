import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "AppKeys.js" as AK
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
    HelperSettings { id: voiceSettings; owner: "base.voice" }
    readonly property var voiceTopic: Helper.topics.voice || null
    readonly property bool voiceOn: voiceSettings.known && voiceSettings.get("enabled") === true
    // What the helper says is missing, in words for the owner; empty when voice search can listen.
    readonly property string voiceNeeds: {
        var r = voiceTopic ? voiceTopic.reason : ""
        return r === "voice_unlinked" ? "Home Assistant is not linked yet. Link it from the TV's command line (see the voice notes), then it listens."
             : r === "no_microphone" ? "No microphone is set in the helper's configuration."
             : r === "link_unreadable" ? "The saved Home Assistant link cannot be read. Link it again."
             : !voiceTopic ? "The TV's helper does not answer for voice search." : ""
    }
    readonly property var theme: Lu.theme || ({})
    readonly property bool pointerOn: String(Sio.get("general.remotePointer")) === "on"
    readonly property string speed: R.name({ slow: "Slow speed", normal: "Normal speed", fast: "Fast speed" }, Sio.get("option.pointerSpeed"), "")
    subtitle: "Magic Remote \u00b7 pointer " + (Sio.has("general.remotePointer") ? (pointerOn ? "on" : "off") : "unknown")
    summary: "Pointer " + (Sio.has("general.remotePointer") ? (pointerOn ? "on" : "off") : "unknown") + (pointerOn && speed ? " \u00b7 " + speed : "")

    function refresh() {
        voiceSettings.refresh()
        Sio.read("general", ["remotePointer"])
        Sio.read("option", ["pointerSpeed", "pointerSize"])
    }
    Component.onCompleted: refresh()
    onOpened: refresh()
    // One row per app key: what it does now, and OK opens the chooser (PageAppKey) for it.
    readonly property var keyCfg: AK.config(theme)
    readonly property var installed: AK.installed(Apps.launchPoints)
    function appKeyRow(k) {
        return { kind: "value", icon: k.icon, label: k.label + " key",
                 detail: function() {
                     var t = AK.target(page.keyCfg, k.id, page.installed)
                     return t.kind === "nothing" ? "The key does nothing."
                          : t.kind === "default" ? (t.installed ? "LG's choice." : "LG's app is not installed here, so the key does nothing.")
                          : t.installed ? "Opens the app chosen here." : "The app chosen here is no longer installed, so the key does nothing."
                 },
                 value: function() { var t = AK.target(page.keyCfg, k.id, page.installed); return t.installed ? t.title : "Nothing" },
                 act: function() { if (page.host) page.host.openPage("appkey", { key: k.id }) } }
    }

    rows: [
        R.head("Pointer"),
        R.toggle(Sio, { ds: "general.remotePointer", icon: "mouse-pointer-2", label: "Magic Remote pointer",
                        detail: "Use the pointer on screen. Turn off if you only use a button remote." }),
        R.seg(Sio, { ds: "option.pointerSpeed", icon: "gauge", label: "Pointer speed", detail: "How far the pointer moves for a flick of the wrist.",
                     options: ["Slow", "Normal", "Fast"], values: ["slow", "normal", "fast"] }),
        R.seg(Sio, { ds: "option.pointerSize", icon: "scan-eye", label: "Pointer size", detail: "Bigger is easier to find from the sofa.",
                     options: ["Small", "Medium", "Large"], values: ["small", "medium", "large"] }),
        R.head("Buttons"),
        { kind: "value", icon: "house", label: "Hold Home for LG's Home", info: true, chevron: false, value: "Always on",
          detail: "Press and hold Home for 650 ms to open LG's own Home if LumaTV misbehaves." },
        { kind: "toggle", icon: "hash", label: "Number keys open dock tiles", detail: "1 opens the first tile on the dock, 2 the second, and so on.",
          on: function() { return !page.theme.remote || page.theme.remote.numberKeys !== false },
          act: function(v) { if (page.host) page.host.patch({ remote: { numberKeys: v } }) } },
        { kind: "toggle", icon: "bell", label: "Hold the gear key for notifications", detail: "Hold Settings for a moment to open the notifications. Off gives the hold back to LG's Settings.",
          on: function() { return !page.theme.remote || page.theme.remote.gearHold !== false },
          act: function(v) { if (page.host) page.host.patch({ remote: { gearHold: v } }) } },
        R.head("App keys")
    ].concat(AK.keys.map(function(k) { return page.appKeyRow(k) }), [
        R.head("Voice"),
        R.toggle(voiceSettings, { ds: "enabled", on: "true", off: "false", icon: "mic", label: "Search by voice",
                                  detail: "Hold the mic key and speak; the words go into search. The mic is open only while held." }),
        R.note(function() { return page.voiceNeeds }, 0, function() { return page.voiceOn && page.voiceNeeds !== "" }),
        R.head("On screen"),
        { kind: "toggle", icon: "tv-minimal-play", label: "Banners over video", detail: "LumaTV draws its own input banner over an input or Live TV, besides LG's. For when LG's is hidden.",
          on: function() { return !!(page.theme.banners && page.theme.banners.own === true) },
          act: function(v) { if (page.host) page.host.patch({ banners: { own: v } }) } },
        R.lg(page, { target: "RemoteSlowInput", detail: "Universal remote control, extended press times, Settings help." })
    ])
}
