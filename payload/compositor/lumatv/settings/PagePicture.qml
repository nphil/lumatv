import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "."

// Picture (docs/mocks/01-settings-picture): the few settings a person changes from the sofa, written
// to the TV's own settings service (so LG's picture processing, panel care and stored values are
// unchanged; only the screen is new). The keys and categories are the ones LG's own Settings app uses
// (docs/mocks/settings-map.md). Expert calibration stays LG's behind "More in LG settings".
// A row the service does not answer is dimmed, not invented; the aspect ratio is a per-input setting, so
// it is only offered while an input is in front.
PageList {
    id: page
    title: "Picture"
    tabs: ["Mode", "Brightness", "Colour", "Advanced"]

    readonly property var modeValues: ["filmMaker", "cinema", "normal", "vivid", "game", "sports", "eco", "expert1", "expert2"]
    readonly property var modeNames: ({ filmMaker: "Filmmaker", cinema: "Cinema", normal: "Standard", vivid: "Vivid", game: "Game", sports: "Sports",
                                        eco: "Eco", expert1: "Expert \u00b7 day", expert2: "Expert \u00b7 night" })
    readonly property string mode: R.name(modeNames, Sio.get("picture.pictureMode"), "")
    readonly property string inputApp: Inputs.current ? Inputs.current.appId : ""
    subtitle: (mode || "Picture mode") + (Inputs.current ? " on " + Inputs.current.port : "")
    summary: (mode || "Picture") + (Sio.has("picture.backlight") ? " \u00b7 Brightness " + Sio.num("picture.backlight") : "")

    function refresh() {
        Inputs.refresh()
        Sio.read("picture", ["pictureMode", "backlight", "energySaving", "eyeComfortMode", "filmmakerAmbientLight", "contrast", "brightness", "color",
                             "tint", "sharpness", "truMotionMode"])
        Sio.read("aiPicture", ["ai_Picture", "ai_Genre", "ai_Brightness"])
        if (inputApp !== "") Sio.read("aspectRatio", ["arcPerApp"], inputApp)
    }
    Component.onCompleted: refresh()
    onOpened: refresh()
    onInputAppChanged: if (inputApp !== "") Sio.read("aspectRatio", ["arcPerApp"], inputApp)

    rows: [
        R.cards(Sio, { ds: "picture.pictureMode", tab: 0, strip: true, cols: 4, values: modeValues, options: [
            { title: "Filmmaker", sub: "Director's grade", icon: "clapperboard" }, { title: "Cinema", sub: "Warm, dark room", icon: "film" },
            { title: "Standard", sub: "Everyday", icon: "tv" }, { title: "Vivid", sub: "Bright, punchy", icon: "sun" },
            { title: "Game", sub: "Lowest lag", icon: "gamepad-2" }, { title: "Sports", sub: "Fast motion", icon: "activity" },
            { title: "Eco", sub: "Lower power", icon: "leaf" }, { title: "Expert \u00b7 day", sub: "Bright room", icon: "sun-medium" },
            { title: "Expert \u00b7 night", sub: "Dark room", icon: "moon-star" }] }),
        R.slider(Sio, { ds: "picture.backlight", tab: 0, min: 0, max: 100, step: 1, icon: "sun", label: "OLED pixel brightness", detail: "How bright the panel may go." }),
        R.toggle(Sio, { ds: "picture.filmmakerAmbientLight", tab: 0, icon: "lamp", label: "Filmmaker ambient light", detail: "Lift dark scenes a little when the room is bright." }),
        R.toggle(Sio, { ds: "aiPicture.ai_Picture", tab: 0, icon: "sparkles", label: "AI Picture Pro", detail: "LG's sharpening and upscaling. Off keeps Filmmaker faithful." }),
        R.over(R.seg(Sio, { ds: "aspectRatio.arcPerApp", tab: 0, icon: "rectangle-horizontal", label: "Aspect ratio", appId: function() { return page.inputApp },
                            detail: "Original keeps the film's own shape. Set for this input only.",
                            options: ["16:9", "Original", "4:3"], values: ["16x9", "original", "4x3"] }),
               { enabled: function() { return page.inputApp !== "" && Sio.usable("aspectRatio.arcPerApp") },
                 detail: function() { return page.inputApp === "" ? "Open an input to change its shape." : Sio.has("aspectRatio.arcPerApp")
                                      ? "Original keeps the film's own shape. Set for this input only." : "Not available on this TV" } }),
        R.seg(Sio, { ds: "picture.energySaving", tab: 0, icon: "leaf", label: "Energy saving", detail: "Lowers brightness to save power. Off while Filmmaker is on.",
                     options: ["Auto", "Off", "Low", "Medium", "High"], values: ["auto", "off", "min", "med", "max"] }),
        R.lg(page, { tab: 0, target: "AddOptions", detail: "Expert calibration: white balance, colour gamut, gamma, 4:4:4." }),

        R.slider(Sio, { ds: "picture.contrast", tab: 1, min: 0, max: 100, step: 1, icon: "contrast", label: "Contrast", detail: "The gap between the darkest and brightest parts." }),
        R.slider(Sio, { ds: "picture.brightness", tab: 1, min: 0, max: 100, step: 1, icon: "sun-dim", label: "Brightness", detail: "Lifts or deepens the dark parts." }),
        R.toggle(Sio, { ds: "picture.eyeComfortMode", tab: 1, icon: "eye", label: "Eye comfort", detail: "A warmer picture for the evening." }),
        R.lg(page, { tab: 1, target: "Brightness", detail: "Peak brightness, dynamic tone mapping, gamma, black level, motion eye care." }),

        R.slider(Sio, { ds: "picture.color", tab: 2, min: 0, max: 100, step: 1, icon: "palette", label: "Colour", detail: "How rich the colours are." }),
        R.slider(Sio, { ds: "picture.tint", tab: 2, min: -50, max: 50, step: 1, icon: "blend", label: "Tint", detail: "Shifts skin tones between green and red." }),
        R.lg(page, { tab: 2, target: "Color", detail: "Colour temperature, white balance, colour gamut and fine tuning." }),

        R.slider(Sio, { ds: "picture.sharpness", tab: 3, min: 0, max: 50, step: 1, icon: "aperture", label: "Sharpness", detail: "Filmmaker keeps the picture as made; more adds edges." }),
        R.seg(Sio, { ds: "picture.truMotionMode", tab: 3, icon: "fast-forward", label: "TruMotion", detail: "Smooths motion. Off keeps the film's own cadence.",
                     options: ["Off", "Natural", "Smooth", "Clear"], values: ["off", "natural", "smooth", "clear"] }),
        R.toggle(Sio, { ds: "aiPicture.ai_Genre", tab: 3, icon: "wand-sparkles", label: "AI genre selection", detail: "Picks a look for sport, film or news by what is playing." }),
        R.toggle(Sio, { ds: "aiPicture.ai_Brightness", tab: 3, icon: "sun-medium", label: "AI brightness", detail: "Follows the room light and the scene." }),
        R.lg(page, { tab: 3, target: "Clarity", detail: "HDR tone mapping, noise reduction, apply to all inputs, reset." })
    ]
}
