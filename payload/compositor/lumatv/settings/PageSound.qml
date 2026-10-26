import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "Choice.js" as C
import "Names.js" as N
import "SoundOut.js" as S
import "."

// Sound (docs/mocks/02-settings-sound, docs/research/settings-coverage.md): LG's Sound menu, written to the TV's own settings service. The
// sound goes to the Denon over eARC, and webOS's own volume is a counter that drifts (PLAN, Receiver), so the volume row is the receiver's
// own level, read live from the helper, and the TV's sound modes are dimmed while the receiver plays. The receiver's own controls are one
// page deeper (PageReceiver), opened from the Receiver row. Keys and values are the ones LG's own Settings app uses: its Cinema is `movie`
// and its Clear Voice Pro `news`. Sound out is LG's own chooser on the TV's own list; Bluetooth speakers have their page (the pairing).
// LG leaves a row out, or dims it, by the sound out in force (its own `i6`: the outputs that include the TV's speakers or the eARC
// receiver); a dimmed row says what it needs.
PageList {
    id: page
    title: "Sound"
    tabs: ["Output", "Mode", "Advanced", "Devices"]

    readonly property var outputNames: ({ external_arc: "Receiver (eARC)", external_optical: "Optical out", tv_speaker: "TV speakers",
                                          headphone: "Headphones", bt_soundbar: "Bluetooth speaker", tv_external_speaker: "TV and external speakers",
                                          tv_speaker_external_arc: "TV and receiver", lineout: "Line out", usb_speaker: "USB speaker",
                                          wow_cast: "Wireless speaker", wisa_speaker: "WiSA speakers" })
    readonly property var modeNames: ({ aiSoundPlus: "AI Sound Pro", aiSound: "AI Sound", standard: "Standard", movie: "Cinema", news: "Clear voice",
                                        sports: "Sports", music: "Music", game: "Game" })
    // the ways out the chooser offers (the TV's list still decides which exist); Bluetooth is paired on its own page and a USB or
    // wireless speaker needs the device list LG's own screens read
    readonly property var outputValues: ["tv_speaker", "external_arc", "external_optical", "lineout", "headphone", "tv_speaker_external_arc",
                                         "tv_external_speaker", "tv_speaker_headphone"]
    readonly property string output: String(Sio.get("sound.soundOutput"))
    readonly property bool toReceiver: S.toReceiver(output)
    // the outputs that play on the TV's own speakers, and those where LG lets the TV's sound settings act (SoundOut.js)
    readonly property bool tvSpeakers: S.tvSpeakers(output)
    readonly property bool tvOrArc: S.tvOrArc(output)
    readonly property string mode: String(Sio.get("sound.soundMode"))
    readonly property var rc: Helper.receiver
    readonly property bool rcAnswers: !!(rc && rc.reachable && rc.fresh)
    readonly property string outName: R.name(outputNames, Sio.get("sound.soundOutput"), "")
    summary: !Sio.has("sound.soundOutput") ? "" : toReceiver ? "Receiver" : outName

    // LG's delay slider is one key per way out of the TV (its `fre` row in main.js), with its limits in the key's description
    // (`values.range`: min, max, interval). `sound.avSync` itself is a switch (the set answers "on").
    readonly property var delayKeys: ({ external_arc: "avSyncSpdif", external_optical: "avSyncSpdif", wow_cast: "avSyncSpdif", wisa_speaker: "avSyncWisa", tv_speaker_bluetooth: "avSyncBtSurround" })
    readonly property string delayDs: "sound." + (delayKeys[output] || "avSyncSpeaker")
    readonly property var delayRange: { var d = Sio.descs[delayDs]; return d && d.values && d.values.range && d.values.range.max !== undefined ? d.values.range : null }
    readonly property bool bypass: String(Sio.get("sound.avSyncBypassInput")) === "on"

    // LG's Reset: the keys of the sound mode in force, offered where LG offers it (Standard, or any mode but Personalized when the TV has the wizard)
    readonly property var modeList: { var d = Sio.descs["sound.soundMode"]; return d && d.values && d.values.arrayExt ? d.values.arrayExt.filter(function(e) { return e.visible !== false }).map(function(e) { return String(e.value) }) : [] }
    readonly property bool wizard: modeList.indexOf("personalized") >= 0
    readonly property bool canReset: Sio.has("sound.soundMode") && Sio.usable("sound.soundMode") && (wizard ? mode !== "personalized" : mode === "standard")
    readonly property var resetKeys: mode === "game" ? ["audioBalance", "gameTVEqualizer", "gameTVEqualizer10band", "soundModeModified"]
                                   : mode === "standard" ? ["audioBalance", "userEqualizerValues", "soundModeModified"] : ["audioBalance", "soundModeModified"]

    LunaCall { id: luna }

    // the keys that do not depend on the sound out first, then the ones that do: the last answer's dimension (the sound out and the input the
    // mode belongs to) is what a write carries, so the mode and the output go last
    function refresh() {
        Sio.read("sound", ["avSyncSpdif", "avSyncSpeaker", "avSyncWisa", "avSyncBtSurround", "avSyncBypassInput", "autoVolume", "soundOutputDigital",
                           "supportATMOS", "userEqualizer", "userEqualizerValues", "tvSetupConfiguration", "magicSpaceSound", "applyToAllInputSSM"])
        Sio.read("sound", ["inputAudioFormatHDMI1", "inputAudioFormatHDMI2", "inputAudioFormatHDMI3", "inputAudioFormatHDMI4", "inputAudioFormatDP1",
                           "inputAudioFormatDP2", "inputAudioFormatUSBC1", "inputAudioFormatUSBC2", "lgSoundSync", "soundAlive", "eArcSupport", "digitalAudioPriority"])
        Sio.read("general", ["tvOnSound"])
        Sio.read("sound", ["audioBalance", "soundOutput", "soundMode"])
    }
    Component.onCompleted: refresh()
    onOpened: refresh()

    function resetNow() {
        luna.act(Sio.service, "resetSystemSettings", { category: "sound", keys: resetKeys })
        refresh()
    }
    // a row that LG dims while the sound out is not one that uses the TV's speakers (or the receiver, for `arc`), with the reason as its second line
    function needs(row, ok, why) {
        return R.over(row, { enabled: function() { return page.ev(row.enabled) !== false && ok() }, detail: function() { return ok() ? page.ev(row.detail) : why } })
    }
    readonly property string tvOnly: "Needs the TV speakers"

    rows: [
        C.choice(Sio, { ds: "sound.soundOutput", tab: 0, icon: "speaker", label: "Sound out", names: "soundOutput", values: outputValues, page: page,
                        help: "Where the TV's sound goes. Bluetooth speakers are paired on their own page." }),
        R.over(R.link({ tab: 0, id: "receiver", icon: "boom-box", label: "Receiver", help: "The receiver's own volume, input and sound modes.",
                        go: function() { page.host.openPage("receiver") } }), { visible: function() { return page.toReceiver } }),
        { kind: "slider", tab: 0, icon: "volume-2", label: "Receiver volume", min: 0, max: 98, step: 1,
          detail: function() { return page.rcAnswers ? "" : "Receiver not answering" },
          info: true, num: function() { return page.rcAnswers && rc.volumeAbs !== undefined ? rc.volumeAbs : 0 },
          valueText: function() { return page.rcAnswers && rc.volumeAbs !== undefined ? String(Math.round(rc.volumeAbs)) : "\u2013" } },
        { kind: "value", tab: 0, icon: "audio-waveform", label: "Sound mode", chevron: false, info: true,
          detail: function() { return page.toReceiver ? "Not used with the receiver" : "" },
          value: function() { return R.name(page.modeNames, Sio.get("sound.soundMode"), "") },
          enabled: function() { return !page.toReceiver } },
        { kind: "slider", tab: 0, icon: "timer", label: "Match screen and sound",
          detail: function() { return page.delayRange === null || !Sio.has(page.delayDs) ? "TV not answering" : !Sio.usable(page.delayDs) ? "Not available now" : page.bypass ? "Delay bypass is on" : "" },
          help: "Delays the sound if lips move before the words.",
          visible: function() { return !Sio.missing(page.delayDs) },
          enabled: function() { return page.delayRange !== null && Sio.usable(page.delayDs) && !page.bypass },
          min: function() { return page.delayRange ? page.delayRange.min : 0 }, max: function() { return page.delayRange ? page.delayRange.max : 0 },
          step: function() { return page.delayRange && page.delayRange.interval ? page.delayRange.interval : 1 },
          num: function() { return Sio.num(page.delayDs, 0) }, act: function(v) { Sio.write(page.delayDs, v) },
          valueText: function() { return Sio.has(page.delayDs) ? String(Math.round(Sio.num(page.delayDs, 0))) + " ms" : "\u2013" } },
        C.lgShown(Sio, "sound.avSyncBypassInput", R.toggle(Sio, { ds: "sound.avSyncBypassInput", tab: 0, icon: "timer", label: "Delay bypass",
                                                                 help: "Skips the delay above, for a source that keeps picture and sound in step itself." })),
        R.seg(Sio, { ds: "sound.soundOutputDigital", tab: 0, icon: "cable", label: "Digital sound output", help: "Pass through hands the original soundtrack to the receiver.",
                     options: ["Auto", "PCM", "Pass through"], values: ["auto", "pcm", "passThrough"] }),
        { kind: "toggle", tab: 0, icon: "tv", label: "TV speakers too", detail: function() { return page.toReceiver ? "" : "Needs the receiver" },
          help: "Also plays through the TV's own speakers while sound goes to the receiver.",
          enabled: function() { return Sio.usable("sound.soundOutput") && page.toReceiver },
          on: function() { return page.output === "tv_speaker_external_arc" },
          act: function(v) { Sio.write("sound.soundOutput", v ? "tv_speaker_external_arc" : "external_arc") } },
        R.link({ tab: 0, id: "btaudio", icon: "headphones", label: "Bluetooth Speakers", go: function() { page.host.openPage("btaudio") } }),

        R.over(R.cards(Sio, { ds: "sound.soundMode", tab: 1, cols: 3, values: ["aiSoundPlus", "standard", "movie", "news", "sports", "music", "game"], options: [
            { title: "AI Sound Pro", icon: "sparkles" }, { title: "Standard", icon: "tv" }, { title: "Cinema", icon: "film" }, { title: "Clear voice", icon: "mic" },
            { title: "Sports", icon: "activity" }, { title: "Music", icon: "music" }, { title: "Game", icon: "gamepad-2" }] }),
               { enabled: function() { return Sio.usable("sound.soundMode") && !page.toReceiver } }),
        R.note("Sound is on the receiver.", 1, function() { return page.toReceiver }),

        needs(C.lgShown(Sio, "sound.supportATMOS", R.toggle(Sio, { ds: "sound.supportATMOS", tab: 2, icon: "sparkles", label: "Dolby Atmos",
                                                                  help: "Dolby Atmos soundtracks play with height and surround effects." })),
              function() { return page.tvOrArc }, "Needs the TV or receiver"),
        needs(R.slider(Sio, { ds: "sound.audioBalance", tab: 2, min: -10, max: 10, step: 1, icon: "sliders-horizontal", label: "Balance", help: "Left and right, for the TV's own speakers." }),
              function() { return page.tvSpeakers }, tvOnly),
        needs(R.link({ tab: 2, id: "equalizer", icon: "sliders-vertical", label: "Equalizer", help: "Five bands for the TV's own speakers, in Standard mode.",
                       enabled: function() { return Sio.usable("sound.userEqualizer") }, go: function() { page.host.openPage("equalizer") } }),
              function() { return S.equalizer(page.output) && page.mode === "standard" }, "Standard mode and TV speakers"),
        needs(R.toggle(Sio, { ds: "sound.autoVolume", tab: 2, icon: "volume-1", label: "Automatic volume", help: "Evens out loud adverts and quiet dialogue on the TV's speakers." }),
              function() { return page.tvOrArc }, "Needs the TV or receiver"),
        needs(C.seg(Sio, { ds: "sound.tvSetupConfiguration", tab: 2, icon: "monitor-speaker", label: "Installation Type", names: "tvSetup",
                           help: "Tunes the TV's speakers for a stand or a wall mount." }),
              function() { return page.tvSpeakers && page.output !== "external_arc" }, tvOnly),
        needs(C.choice(Sio, { ds: "sound.magicSpaceSound", tab: 2, icon: "waves", label: "AI Acoustic Tuning Options", names: "magicSound", page: page,
                              help: "Shapes the TV's sound to the room, after a tuning with the Magic Remote." }),
              function() { return page.tvSpeakers }, tvOnly),
        C.choice(Sio, { ds: "general.tvOnSound", tab: 2, icon: "bell", label: "TV Power Sound Volume", names: "lowToHigh", page: page,
                        help: "The sound the TV makes when it turns on or off." }),
        C.action({ id: "applyall", tab: 2, icon: "list-checks", label: "Apply to All Inputs", button: "Apply",
                   enabled: function() { return Sio.usable("sound.applyToAllInputSSM") },
                   help: "Gives every input this sound mode's settings.",
                   act: C.ask(page, { icon: "list-checks", title: "Apply to all inputs?", body: "Every input gets this sound mode's settings.", yes: "Apply", no: "Cancel" },
                              function() { Sio.write("sound.applyToAllInputSSM", "sound") }) }),
        C.action({ id: "reset", tab: 2, icon: "rotate-ccw", label: "Reset", button: "Reset", danger: true,
                   enabled: function() { return page.canReset },
                   detail: function() { return page.canReset || !Sio.has("sound.soundMode") ? "" : page.wizard ? "Not for Personalized" : "Standard mode only" },
                   help: "Puts this sound mode's settings back to LG's.",
                   act: C.ask(page, { icon: "rotate-ccw", title: "Reset this sound mode?", body: "Its settings go back to LG's.", yes: "Reset", no: "Cancel", danger: true },
                              function() { page.resetNow() }) }),

        R.over(R.link({ tab: 3, id: "hdmiaudio", icon: "cable", label: "Select HDMI Input Audio Format", help: "Bitstream or PCM, for each HDMI input.",
                        go: function() { page.host.openPage("hdmiaudio") } }),
               { visible: function() { for (var i = 1; i <= 4; i++) if (C.shown(Sio, "sound.inputAudioFormatHDMI" + i)) return true; return false } }),
        needs(C.lgShown(Sio, "sound.lgSoundSync", R.toggle(Sio, { ds: "sound.lgSoundSync", tab: 3, icon: "cable", label: "LG Sound Sync",
                                                                 help: "Lets the remote set the volume of a sound bar on the optical out." })),
              function() { return page.output === "external_optical" }, "Needs the optical out"),
        needs(C.lgShown(Sio, "sound.soundAlive", R.toggle(Sio, { ds: "sound.soundAlive", tab: 3, icon: "boom-box", label: "TV Sound Mode Share",
                                                               help: "Gives a LG sound bar on HDMI the sound mode of the TV's speakers." })),
              function() { return page.output === "external_arc" || page.output === "tv_speaker_external_arc" }, "Needs HDMI(ARC)"),
        C.lgShown(Sio, "sound.eArcSupport", R.toggle(Sio, { ds: "sound.eArcSupport", tab: 3, icon: "cable", label: "eARC Support",
                                                          help: "For a receiver that supports eARC: a richer sound from the HDMI(ARC) connection." })),
        C.choice(Sio, { ds: "sound.digitalAudioPriority", tab: 3, icon: "audio-lines", label: "DTV Audio Setting", names: "dtvAudio", page: page,
                        help: "The digital audio format a broadcast is played in, when it offers more than one." })
    ]
}
