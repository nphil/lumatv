import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "."

// Picture (docs/mocks/01-settings-picture): the few settings a person changes from the sofa, written
// to the TV's own settings service (so LG's picture processing, panel care and stored values are
// unchanged; only the screen is new). Expert calibration stays LG's behind "More in LG settings".
// A row the service does not answer is dimmed, not invented.
PageList {
    id: page
    title: "Picture"
    tabs: ["Mode", "Brightness", "Colour", "Advanced"]

    readonly property var modeValues: ["filmMaker", "cinema", "normal", "vivid", "game", "sports", "eco", "expert1", "expert2"]
    readonly property var modeNames: ({ filmMaker: "Filmmaker", cinema: "Cinema", normal: "Standard", vivid: "Vivid", game: "Game", sports: "Sports",
                                        eco: "Eco", expert1: "Expert \u00b7 day", expert2: "Expert \u00b7 night" })
    readonly property string mode: R.name(modeNames, Sio.get("picture.pictureMode"), "")
    subtitle: (mode || "Picture mode") + (Inputs.current ? " on " + Inputs.current.port : "")
    summary: (mode || "Picture") + (Sio.has("picture.backlight") ? " \u00b7 Brightness " + Sio.num("picture.backlight") : "")

    Component.onCompleted: Sio.read("picture", ["pictureMode", "backlight", "energySaving", "eyeComfortMode", "screenShift", "contrast",
                                                "brightness", "color", "tint", "sharpness", "truMotionMode"])

    rows: [
        R.cards(Sio, { ds: "picture.pictureMode", tab: 0, cols: 5, values: modeValues, options: [
            { title: "Filmmaker", sub: "Director's grade", icon: "clapperboard" }, { title: "Cinema", sub: "Warm, dark room", icon: "film" },
            { title: "Standard", sub: "Everyday", icon: "tv" }, { title: "Vivid", sub: "Bright, punchy", icon: "sun" },
            { title: "Game", sub: "Lowest lag", icon: "gamepad-2" }, { title: "Sports", sub: "Fast motion", icon: "activity" },
            { title: "Eco", sub: "Lower power", icon: "leaf" }, { title: "Expert \u00b7 day", sub: "Bright room", icon: "sun-medium" },
            { title: "Expert \u00b7 night", sub: "Dark room", icon: "moon-star" }] }),
        R.slider(Sio, { ds: "picture.backlight", tab: 0, min: 0, max: 100, step: 1, icon: "sun", label: "OLED pixel brightness", detail: "How bright the panel may go." }),
        R.toggle(Sio, { ds: "picture.eyeComfortMode", tab: 0, icon: "eye", label: "Eye comfort", detail: "A warmer picture for the evening." }),
        R.seg(Sio, { ds: "picture.energySaving", tab: 0, icon: "leaf", label: "Energy saving", detail: "Lowers brightness to save power. Off while Filmmaker is on.",
                     options: ["Off", "Low", "Medium", "High"], values: ["off", "min", "med", "max"] }),
        R.lg(page, { tab: 0, detail: "Expert calibration: white balance, colour gamut, gamma, 4:4:4." }),

        R.slider(Sio, { ds: "picture.contrast", tab: 1, min: 0, max: 100, step: 1, icon: "contrast", label: "Contrast", detail: "The gap between the darkest and brightest parts." }),
        R.slider(Sio, { ds: "picture.brightness", tab: 1, min: 0, max: 100, step: 1, icon: "sun-dim", label: "Black level", detail: "How deep the dark parts go." }),
        R.toggle(Sio, { ds: "picture.screenShift", tab: 1, icon: "scan-eye", label: "Screen shift", detail: "Moves the picture a pixel now and then so the panel wears evenly." }),

        R.slider(Sio, { ds: "picture.color", tab: 2, min: 0, max: 100, step: 1, icon: "palette", label: "Colour", detail: "How rich the colours are." }),
        R.slider(Sio, { ds: "picture.tint", tab: 2, min: -50, max: 50, step: 1, icon: "blend", label: "Tint", detail: "Shifts skin tones between green and red." }),

        R.slider(Sio, { ds: "picture.sharpness", tab: 3, min: 0, max: 50, step: 1, icon: "aperture", label: "Sharpness", detail: "Filmmaker keeps the picture as made; more adds edges." }),
        R.seg(Sio, { ds: "picture.truMotionMode", tab: 3, icon: "fast-forward", label: "TruMotion", detail: "Smooths motion. Off keeps the film's own cadence.",
                     options: ["Off", "Natural", "Smooth", "Clear"], values: ["off", "natural", "smooth", "clear"] }),
        R.lg(page, { tab: 3, detail: "AI Picture Pro, HDR tone mapping, noise reduction, reset." })
    ]
}
