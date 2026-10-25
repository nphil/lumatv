import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "."

// Sound (docs/mocks/02-settings-sound). The sound goes to the Denon over eARC, and webOS's own volume is
// a counter that drifts (PLAN, Receiver), so the volume row is the receiver's own level, read live from
// the helper, and the TV's sound modes are dimmed while the receiver plays. The receiver's own controls
// are one page deeper (PageReceiver), opened from "Sound out".
PageList {
    id: page
    title: "Sound"
    tabs: ["Output", "Mode", "Advanced"]

    readonly property var outputNames: ({ external_arc: "Receiver (eARC)", external_optical: "Optical out", tv_speaker: "TV speakers",
                                          headphone: "Headphones", bt_soundbar: "Bluetooth speaker", tv_external_speaker: "TV and external speakers" })
    readonly property bool toReceiver: String(Sio.get("sound.soundOutput")) === "external_arc"
    readonly property var rc: Helper.receiver
    readonly property bool rcAnswers: !!(rc && rc.reachable && rc.fresh)
    readonly property string outName: R.name(outputNames, Sio.get("sound.soundOutput"), "")
    subtitle: !Sio.has("sound.soundOutput") ? "" : toReceiver ? "Playing through the receiver" : "Playing through " + outName.toLowerCase()
    summary: !Sio.has("sound.soundOutput") ? "" : toReceiver ? "Receiver \u00b7 eARC" : outName

    Component.onCompleted: Sio.read("sound", ["soundOutput", "soundMode", "digitalSoundOut", "balance", "avSyncAdjustment"])

    rows: [
        { kind: "value", tab: 0, icon: "speaker", label: "Sound out", chevron: true,
          detail: function() { return page.toReceiver ? "eARC on HDMI 2. Opens the receiver's own controls." : "Where the sound goes. LG's Sound settings change it." },
          value: function() { return page.outName },
          act: function() { if (page.toReceiver) page.host.openPage("receiver"); else { page.host.launchApp("com.palm.app.settings"); page.host.closeRequested() } } },
        { kind: "slider", tab: 0, icon: "volume-2", label: "Receiver volume", min: 0, max: 98, step: 1,
          detail: function() { return page.rcAnswers ? "Read live from the receiver. The volume keys move it." : "The receiver is not answering." },
          info: true, num: function() { return page.rcAnswers && rc.volumeAbs !== undefined ? rc.volumeAbs : 0 } },
        { kind: "value", tab: 0, icon: "audio-waveform", label: "Sound mode", chevron: false,
          detail: function() { return page.toReceiver ? "Not used while sound goes to the receiver. Pick a mode on the receiver." : "The TV's own sound processing." },
          value: function() { return R.name({ standard: "Standard", cinema: "Cinema", clearVoice: "Clear voice", sports: "Sports", music: "Music", game: "Game" }, Sio.get("sound.soundMode"), "") },
          enabled: function() { return !page.toReceiver } },
        R.seg(Sio, { ds: "sound.digitalSoundOut", tab: 0, icon: "cable", label: "HDMI input audio format", detail: "Bitstream hands the original soundtrack to the receiver.",
                     options: ["Bitstream", "PCM"], values: ["bitstream", "pcm"] }),
        R.lg(page, { tab: 0, detail: "Equaliser, balance, LG Sound Sync, installation type." }),

        R.cards(Sio, { ds: "sound.soundMode", tab: 1, cols: 3, values: ["standard", "cinema", "clearVoice", "sports", "music", "game"], options: [
            { title: "Standard", sub: "Balanced", icon: "tv" }, { title: "Cinema", sub: "Wide and warm", icon: "film" },
            { title: "Clear voice", sub: "Dialogue forward", icon: "mic" }, { title: "Sports", sub: "Crowd and commentary", icon: "activity" },
            { title: "Music", sub: "Full range", icon: "music" }, { title: "Game", sub: "Positional", icon: "gamepad-2" }] }),
        R.note("The TV's sound modes are not used while sound goes to the receiver.", 1),

        R.slider(Sio, { ds: "sound.balance", tab: 2, min: -10, max: 10, step: 1, icon: "sliders-horizontal", label: "Balance", detail: "Left and right, for the TV's own speakers." }),
        R.slider(Sio, { ds: "sound.avSyncAdjustment", tab: 2, min: 0, max: 100, step: 10, unit: " ms", icon: "timer", label: "Match screen and sound", detail: "Delay the sound if lips move first." })
    ]
}
