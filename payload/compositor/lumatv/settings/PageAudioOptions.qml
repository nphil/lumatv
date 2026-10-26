import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "Choice.js" as C
import "."

// Audio options (LG: General › Accessibility): what LG lists for hearing and for described video, in LG's names. Video Description,
// the audio type and the extra audio choice, the described-audio volume, Amplify Dialogue with its gain, the hearing-aid output, the
// sign-language zoom and the power sound. Each row is LG's own key; a row the TV does not answer or hides is left out, as in LG's screen.
// NEXTGEN TV Description and Amplify Dialogue (with its gain) wait for the ATSC 3.0 tuner (`system.supportATSC3tuner`), as LG's rows do.
PageList {
    id: page
    title: "Audio options"
    back: "System"
    readonly property bool describing: String(Sio.get("sound.audioDescription")) === "on"
    readonly property bool amplifying: String(Sio.get("sound.dialogEnhancementAtsc30")) === "on"

    // LG shows its NEXTGEN TV rows only on a TV with an ATSC 3.0 tuner
    Atsc3 { id: tuner }
    property alias atsc3: tuner.present
    function nextgen(ds, row) { return R.over(row, { visible: function() { return page.atsc3 && C.shown(Sio, ds) } }) }

    function refresh() {
        Sio.read("sound", ["audioDescription", "audioDescriptionVolume", "audioDescriptionAtsc30", "visualImpaired", "dialogEnhancementAtsc30",
                           "dialogEnhancementVolumeAtsc30", "hearingAid", "tvPowerSound"])
        Sio.read("option", ["supplementaryAudio", "additionalAudioSelection"])
        Sio.read("other", ["autoSLZoomWithObjectDetection"])
    }
    Component.onCompleted: refresh()
    onOpened: refresh()

    rows: [
        C.lgShown(Sio, "sound.visualImpaired", R.toggle(Sio, { ds: "sound.visualImpaired", icon: "eye", label: "Video Description" })),
        nextgen("sound.audioDescriptionAtsc30", R.toggle(Sio, { ds: "sound.audioDescriptionAtsc30", icon: "audio-lines", label: "NEXTGEN TV Description" })),
        C.choice(Sio, { ds: "option.supplementaryAudio", icon: "audio-lines", label: "Audio Type", names: "supplementaryAudio", page: page }),
        C.choice(Sio, { ds: "option.additionalAudioSelection", icon: "audio-waveform", label: "Additional Audio", names: "additionalAudioSelection", page: page }),
        R.over(C.lgShown(Sio, "sound.audioDescriptionVolume", R.slider(Sio, { ds: "sound.audioDescriptionVolume", min: -15, max: 15, step: 1, icon: "volume-2", label: "Description Volume" })),
               { enabled: function() { return page.describing && Sio.usable("sound.audioDescriptionVolume") }, detail: function() { return page.describing ? "" : "Turn Audio Description on" } }),
        nextgen("sound.dialogEnhancementAtsc30", R.toggle(Sio, { ds: "sound.dialogEnhancementAtsc30", icon: "mic", label: "Amplify Dialogue" })),
        R.over(nextgen("sound.dialogEnhancementVolumeAtsc30", R.slider(Sio, { ds: "sound.dialogEnhancementVolumeAtsc30", min: 0, max: 12, step: 1, unit: " dB", icon: "sliders-horizontal", label: "Gain Level" })),
               { enabled: function() { return page.amplifying && Sio.usable("sound.dialogEnhancementVolumeAtsc30") }, detail: function() { return page.amplifying ? "" : "Turn Amplify Dialogue on" } }),
        C.choice(Sio, { ds: "sound.hearingAid", icon: "headphones", label: "Hearing Aid", names: "hearingAid", page: page }),
        C.lgShown(Sio, "other.autoSLZoomWithObjectDetection", R.toggle(Sio, { ds: "other.autoSLZoomWithObjectDetection", icon: "scan-eye", label: "Sign Language Zoom" })),
        C.lgShown(Sio, "sound.tvPowerSound", R.toggle(Sio, { ds: "sound.tvPowerSound", icon: "bell", label: "Device Power Sound" }))
    ]
}
