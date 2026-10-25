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
    subtitle: !Sio.has("sound.soundOutput") ? "" : toReceiver ? "Playing through the receiver" : "Playing through " + outName.toLowerCase()
    summary: !Sio.has("sound.soundOutput") ? "" : toReceiver ? "Receiver \u00b7 eARC" : outName

    function refresh() { Sio.read("sound", ["soundOutput", "soundMode", "soundOutputDigital", "audioBalance", "avSync", "autoVolume"]) }
    Component.onCompleted: refresh()
    onOpened: refresh()

    rows: [
        { kind: "value", tab: 0, icon: "speaker", label: "Sound out", chevron: true,
          detail: function() { return page.toReceiver ? "eARC on HDMI 2. Opens the receiver's own controls." : "Where the sound goes. LG's Sound settings change it." },
          value: function() { return page.outName },
          act: function() { if (page.toReceiver) page.host.openPage("receiver"); else { page.host.launchApp("com.palm.app.settings", { target: "SoundOut" }); page.host.closeRequested() } } },
        { kind: "slider", tab: 0, icon: "volume-2", label: "Receiver volume", min: 0, max: 98, step: 1,
          detail: function() { return page.rcAnswers ? "Read live from the receiver. The volume keys move it." : "The receiver is not answering." },
          info: true, num: function() { return page.rcAnswers && rc.volumeAbs !== undefined ? rc.volumeAbs : 0 } },
        { kind: "value", tab: 0, icon: "audio-waveform", label: "Sound mode", chevron: false, info: true,
          detail: function() { return page.toReceiver ? "Not used while sound goes to the receiver. Pick a mode on the Denon." : "The TV's own sound processing. Change it on the Mode tab." },
          value: function() { return R.name(page.modeNames, Sio.get("sound.soundMode"), "") },
          enabled: function() { return !page.toReceiver } },
        R.slider(Sio, { ds: "sound.avSync", tab: 0, min: 0, max: 100, step: 10, unit: " ms", icon: "timer", label: "Match screen and sound", detail: "Delay the sound if lips move first." }),
        R.seg(Sio, { ds: "sound.soundOutputDigital", tab: 0, icon: "cable", label: "Digital sound output", detail: "Pass through hands the original soundtrack to the receiver.",
                     options: ["Auto", "PCM", "Pass through"], values: ["auto", "pcm", "passThrough"], segWidth: 150 }),
        { kind: "toggle", tab: 0, icon: "tv", label: "TV speakers too", detail: function() { return page.toReceiver ? "Also play through the TV's own speakers." : "Only while sound goes to the receiver." },
          enabled: function() { return Sio.usable("sound.soundOutput") && page.toReceiver },
          on: function() { return page.output === "tv_speaker_external_arc" },
          act: function(v) { Sio.write("sound.soundOutput", v ? "tv_speaker_external_arc" : "external_arc") } },
        R.lg(page, { tab: 0, target: "AdditionalSound", detail: "Equaliser, LG Sound Sync, installation type, eARC, Dolby Atmos." }),

        R.over(R.cards(Sio, { ds: "sound.soundMode", tab: 1, cols: 3, values: ["aiSoundPlus", "standard", "movie", "news", "sports", "music", "game"], options: [
            { title: "AI Sound Pro", sub: "Adapts to the scene", icon: "sparkles" }, { title: "Standard", sub: "Balanced", icon: "tv" },
            { title: "Cinema", sub: "Wide and warm", icon: "film" }, { title: "Clear voice", sub: "Dialogue forward", icon: "mic" },
            { title: "Sports", sub: "Crowd and commentary", icon: "activity" }, { title: "Music", sub: "Full range", icon: "music" },
            { title: "Game", sub: "Positional", icon: "gamepad-2" }] }),
               { enabled: function() { return Sio.usable("sound.soundMode") && !page.toReceiver } }),
        R.note("The TV's sound modes are not used while sound goes to the receiver.", 1, function() { return page.toReceiver }),

        R.slider(Sio, { ds: "sound.audioBalance", tab: 2, min: -10, max: 10, step: 1, icon: "sliders-horizontal", label: "Balance", detail: "Left and right, for the TV's own speakers." }),
        R.toggle(Sio, { ds: "sound.autoVolume", tab: 2, icon: "volume-1", label: "Automatic volume", detail: "Evens out loud adverts and quiet dialogue on the TV's speakers." }),
        R.lg(page, { tab: 2, target: "AdditionalSound", detail: "Equaliser, installation type, LG Sound Sync, eARC, Dolby Atmos." })
    ]
}
