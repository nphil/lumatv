import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "."

// Sound (docs/mocks/02-settings-sound). The sound goes to the Denon over eARC, and webOS's own volume is
// a counter that drifts (PLAN, Receiver), so the volume row is the receiver's own level, read live from
// the helper, and the TV's sound modes are dimmed while the receiver plays. The receiver's own controls
// are one page deeper (PageReceiver), opened from "Sound out". Keys and values are the ones LG's own Settings
// app uses: its Cinema is `movie` and its Clear Voice Pro `news`.
PageList {
    id: page
    title: "Sound"
    tabs: ["Output", "Mode", "Advanced"]

    readonly property var outputNames: ({ external_arc: "Receiver (eARC)", external_optical: "Optical out", tv_speaker: "TV speakers",
                                          headphone: "Headphones", bt_soundbar: "Bluetooth speaker", tv_external_speaker: "TV and external speakers",
                                          tv_speaker_external_arc: "TV and receiver", lineout: "Line out", usb_speaker: "USB speaker",
                                          wow_cast: "Wireless speaker", wisa_speaker: "WiSA speakers" })
    readonly property var modeNames: ({ aiSoundPlus: "AI Sound Pro", aiSound: "AI Sound", standard: "Standard", movie: "Cinema", news: "Clear voice",
                                        sports: "Sports", music: "Music", game: "Game" })
    readonly property string output: String(Sio.get("sound.soundOutput"))
    readonly property bool toReceiver: output === "external_arc" || output === "tv_speaker_external_arc"
    readonly property var rc: Helper.receiver
    readonly property bool rcAnswers: !!(rc && rc.reachable && rc.fresh)
    readonly property string outName: R.name(outputNames, Sio.get("sound.soundOutput"), "")
    summary: !Sio.has("sound.soundOutput") ? "" : toReceiver ? "Receiver" : outName

    // LG's delay slider is one key per way out of the TV (its `fre` row in main.js), with its limits in the key's description
    // (`values.range`: min, max, interval). `sound.avSync` itself is a switch (the set answers "on").
    readonly property var delayKeys: ({ external_arc: "avSyncSpdif", external_optical: "avSyncSpdif", wow_cast: "avSyncSpdif", wisa_speaker: "avSyncWisa", tv_speaker_bluetooth: "avSyncBtSurround" })
    readonly property string delayDs: "sound." + (delayKeys[output] || "avSyncSpeaker")
    readonly property var delayRange: { var d = Sio.descs[delayDs]; return d && d.values && d.values.range && d.values.range.max !== undefined ? d.values.range : null }

    function refresh() { Sio.read("sound", ["soundOutput", "soundMode", "soundOutputDigital", "audioBalance", "avSyncSpdif", "avSyncSpeaker", "avSyncWisa", "avSyncBtSurround", "autoVolume"]) }
    Component.onCompleted: refresh()
    onOpened: refresh()

    rows: [
        { kind: "value", tab: 0, icon: "speaker", label: "Sound out", chevron: true,
          help: "Where the sound goes. OK opens the receiver's own controls, or LG's Sound Out screen without one.",
          value: function() { return page.outName },
          act: function() { if (page.toReceiver) page.host.openPage("receiver"); else { page.host.launchApp("com.palm.app.settings", { target: "SoundOut" }); page.host.closeRequested() } } },
        { kind: "slider", tab: 0, icon: "volume-2", label: "Receiver volume", min: 0, max: 98, step: 1,
          detail: function() { return page.rcAnswers ? "" : "Receiver not answering" },
          info: true, num: function() { return page.rcAnswers && rc.volumeAbs !== undefined ? rc.volumeAbs : 0 },
          valueText: function() { return page.rcAnswers && rc.volumeAbs !== undefined ? String(Math.round(rc.volumeAbs)) : "\u2013" } },
        { kind: "value", tab: 0, icon: "audio-waveform", label: "Sound mode", chevron: false, info: true,
          detail: function() { return page.toReceiver ? "Not used with the receiver" : "" },
          value: function() { return R.name(page.modeNames, Sio.get("sound.soundMode"), "") },
          enabled: function() { return !page.toReceiver } },
        { kind: "slider", tab: 0, icon: "timer", label: "Match screen and sound",
          detail: function() { return page.delayRange === null || !Sio.has(page.delayDs) ? "TV not answering" : Sio.usable(page.delayDs) ? "" : "Not available now" },
          help: "Delays the sound if lips move before the words.",
          visible: function() { return !Sio.missing(page.delayDs) },
          enabled: function() { return page.delayRange !== null && Sio.usable(page.delayDs) },
          min: function() { return page.delayRange ? page.delayRange.min : 0 }, max: function() { return page.delayRange ? page.delayRange.max : 0 },
          step: function() { return page.delayRange && page.delayRange.interval ? page.delayRange.interval : 1 },
          num: function() { return Sio.num(page.delayDs, 0) }, act: function(v) { Sio.write(page.delayDs, v) },
          valueText: function() { return Sio.has(page.delayDs) ? String(Math.round(Sio.num(page.delayDs, 0))) + " ms" : "\u2013" } },
        R.seg(Sio, { ds: "sound.soundOutputDigital", tab: 0, icon: "cable", label: "Digital sound output", help: "Pass through hands the original soundtrack to the receiver.",
                     options: ["Auto", "PCM", "Pass through"], values: ["auto", "pcm", "passThrough"] }),
        { kind: "toggle", tab: 0, icon: "tv", label: "TV speakers too", detail: function() { return page.toReceiver ? "" : "Needs the receiver" },
          help: "Also plays through the TV's own speakers while sound goes to the receiver.",
          enabled: function() { return Sio.usable("sound.soundOutput") && page.toReceiver },
          on: function() { return page.output === "tv_speaker_external_arc" },
          act: function(v) { Sio.write("sound.soundOutput", v ? "tv_speaker_external_arc" : "external_arc") } },
        R.link({ tab: 0, id: "btaudio", icon: "headphones", label: "Bluetooth Speakers", go: function() { page.host.openPage("btaudio") } }),
        R.lg(page, { tab: 0, target: "AdditionalSound" }),

        R.over(R.cards(Sio, { ds: "sound.soundMode", tab: 1, cols: 3, values: ["aiSoundPlus", "standard", "movie", "news", "sports", "music", "game"], options: [
            { title: "AI Sound Pro", icon: "sparkles" }, { title: "Standard", icon: "tv" }, { title: "Cinema", icon: "film" }, { title: "Clear voice", icon: "mic" },
            { title: "Sports", icon: "activity" }, { title: "Music", icon: "music" }, { title: "Game", icon: "gamepad-2" }] }),
               { enabled: function() { return Sio.usable("sound.soundMode") && !page.toReceiver } }),
        R.note("Sound is on the receiver.", 1, function() { return page.toReceiver }),

        R.slider(Sio, { ds: "sound.audioBalance", tab: 2, min: -10, max: 10, step: 1, icon: "sliders-horizontal", label: "Balance", help: "Left and right, for the TV's own speakers." }),
        R.toggle(Sio, { ds: "sound.autoVolume", tab: 2, icon: "volume-1", label: "Automatic volume", help: "Evens out loud adverts and quiet dialogue on the TV's speakers." }),
        R.lg(page, { tab: 2, target: "AdditionalSound" })
    ]
}
