import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "Choice.js" as C
import "."

// HDMI Signalling Override (LG's hidden panel, reached there by a key sequence on Select Mode): what the TV is told an incoming HDMI picture is,
// whatever the source says. `other.colorimetry`, `eotf`, `masteringPeak`, `masteringColor`, `maxCLL`, `maxFALL` and `dolbyVSVDBVer` answer for the input in
// front (they are dimmed until an HDMI picture plays), each on the TV's own list and in LG's names for it; AUTO is the source's own. Reset sends the seven
// keys to LG's `resetSystemSettings` after a question, as LG's does. A wrong value here can blank the picture, so the page is behind the Expert tab's row.
PageList {
    id: page
    title: "HDMI Signalling"
    back: "Picture"
    chip: Inputs.current ? Inputs.current.port : ""
    chipTone: "info"

    readonly property var keys: ["colorimetry", "eotf", "masteringPeak", "masteringColor", "maxCLL", "maxFALL", "dolbyVSVDBVer"]
    readonly property var autoOnly: ({ auto: "AUTO" })
    LunaCall { id: luna }

    function refresh() {
        Inputs.refresh()
        Sio.read("other", keys)
    }
    onOpened: refresh()

    function choose(key, label, icon, names, help) {
        return C.choice(Sio, { ds: "other." + key, tab: 0, icon: icon, label: label, names: names, page: page, help: help })
    }

    rows: [
        choose("colorimetry", "Colorimetry", "palette", { auto: "AUTO", bt709: "BT709", bt601: "BT601", hdrGamma: "HDR Gamma", bt2020: "BT2020", p3D65: "P3 D65" },
               "The colour space the picture is read in."),
        choose("eotf", "EOTF", "gauge", { auto: "AUTO", sdrGamma: "SDR Gamma", hdrGamma: "HDR Gamma", st2084: "ST2084", hlg: "HLG" },
               "The brightness curve the picture is read with."),
        choose("masteringPeak", "Mastering Peak", "sun", autoOnly, "The peak brightness, in nits, the picture was mastered at."),
        choose("masteringColor", "Mastering Color", "palette", { auto: "AUTO", p3D65: "P3 D65", bt2020D65: "BT2020 D65", bt709D65: "BT709 D65" },
               "The colour space the picture was mastered in."),
        choose("maxCLL", "MaxCLL", "sun-dim", autoOnly, "The brightest single point of the picture, in nits."),
        choose("maxFALL", "MaxFALL", "sun-dim", autoOnly, "The brightest average frame of the picture, in nits."),
        choose("dolbyVSVDBVer", "Dolby VSVDB", "film", { v1: "V1", v2: "V2" }, "The version of the Dolby Vision capability the TV reports to the source."),
        C.action({ id: "reset", tab: 0, icon: "rotate-ccw", label: "Reset", button: "Reset", danger: true, help: "Puts the seven values back to LG's.",
                   act: C.ask(page, { icon: "rotate-ccw", title: "Reset the HDMI signalling?", body: "The seven values go back to LG's.", yes: "Reset", no: "Cancel", danger: true },
                              function() { luna.act(Sio.service, "resetSystemSettings", { category: "other", keys: page.keys }); page.refresh() }) })
    ]
}
