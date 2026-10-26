import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "."

// Picture (docs/mocks/01-settings-picture): the few settings a person changes from the sofa, written
// to the TV's own settings service (so LG's picture processing, panel care and stored values are
// unchanged; only the screen is new). The keys and categories are the ones LG's own Settings app uses
// (docs/mocks/settings-map.md). Expert calibration stays LG's behind "More in LG settings".
// A row the service has no key for is left out, one it has not answered is dimmed; the aspect ratio and
// TruMotion are per-input settings (the service answers them only for an app), so they are offered while
// an input is in front, and the head's chip names that input. The rail shows the mode in force.
// Text: a row is its name and its value; what is not obvious is `help`, behind the row's info mark.
PageList {
    id: page
    title: "Picture"
    tabs: ["Mode", "Brightness", "Colour", "Advanced"]

    readonly property var modeValues: ["filmMaker", "cinema", "normal", "vivid", "game", "sports", "eco", "expert1", "expert2"]
    readonly property var modeNames: ({ filmMaker: "Filmmaker", cinema: "Cinema", normal: "Standard", vivid: "Vivid", game: "Game", sports: "Sports",
                                        eco: "Eco", expert1: "Expert \u00b7 day", expert2: "Expert \u00b7 night" })
    readonly property string mode: R.name(modeNames, Sio.get("picture.pictureMode"), "")
    readonly property string inputApp: Inputs.current ? Inputs.current.appId : ""
    chip: Inputs.current ? Inputs.current.port : ""
    chipTone: "info"
    summary: mode

    function refresh() {
        Inputs.refresh()
        Sio.read("picture", ["pictureMode", "backlight", "energySaving", "eyeComfortMode", "filmmakerAmbientLight", "contrast", "brightness", "color",
                             "tint", "sharpness"])
        Sio.read("aiPicture", ["ai_Picture", "ai_Genre", "ai_Brightness"])
        readPerApp()
    }
    function readPerApp() {
        if (inputApp === "") return
        Sio.read("aspectRatio", ["arcPerApp"], inputApp)
        Sio.read("picture", ["truMotionMode"], inputApp)
    }
    Component.onCompleted: refresh()
    onOpened: refresh()
    onInputAppChanged: readPerApp()
    // A per-input row: offered while an input is in front, else it says why not; the read with that app's id answers it.
    function perApp(row) {
        return R.over(row, { enabled: function() { return page.inputApp !== "" && Sio.usable(row.id) },
                             detail: function() { return page.inputApp === "" ? "No input open" : row.detail() } })
    }

    rows: [
        R.cards(Sio, { ds: "picture.pictureMode", tab: 0, strip: true, cols: 4, values: modeValues, options: [
            { title: "Filmmaker", icon: "clapperboard" }, { title: "Cinema", icon: "film" }, { title: "Standard", icon: "tv" }, { title: "Vivid", icon: "sun" },
            { title: "Game", icon: "gamepad-2" }, { title: "Sports", icon: "activity" }, { title: "Eco", icon: "leaf" },
            { title: "Expert \u00b7 day", icon: "sun-medium" }, { title: "Expert \u00b7 night", icon: "moon-star" }] }),
        R.slider(Sio, { ds: "picture.backlight", tab: 0, min: 0, max: 100, step: 1, icon: "sun", label: "OLED pixel brightness" }),
        R.toggle(Sio, { ds: "picture.filmmakerAmbientLight", tab: 0, icon: "lamp", label: "Filmmaker ambient light",
                        help: "Lifts dark scenes a little when the room is bright." }),
        R.toggle(Sio, { ds: "aiPicture.ai_Picture", tab: 0, icon: "sparkles", label: "AI Picture Pro",
                        help: "LG's sharpening and upscaling. Off keeps the picture as the film was made." }),
        perApp(R.seg(Sio, { ds: "aspectRatio.arcPerApp", tab: 0, icon: "rectangle-horizontal", label: "Aspect ratio", appId: function() { return page.inputApp },
                            help: "Original keeps the film's own shape. It is set for the input in front.",
                            options: ["16:9", "Original", "4:3"], values: ["16x9", "original", "4x3"] })),
        R.seg(Sio, { ds: "picture.energySaving", tab: 0, icon: "leaf", label: "Energy saving", help: "Dims the picture to save power.",
                     options: ["Auto", "Off", "Low", "Medium", "High"], values: ["auto", "off", "min", "med", "max"] }),
        R.lg(page, { tab: 0, target: "AddOptions" }),

        R.slider(Sio, { ds: "picture.contrast", tab: 1, min: 0, max: 100, step: 1, icon: "contrast", label: "Contrast" }),
        R.slider(Sio, { ds: "picture.brightness", tab: 1, min: 0, max: 100, step: 1, icon: "sun-dim", label: "Brightness",
                        help: "Lifts or deepens the dark parts. The panel's own light is OLED pixel brightness." }),
        R.toggle(Sio, { ds: "picture.eyeComfortMode", tab: 1, icon: "eye", label: "Eye comfort", help: "A warmer picture for the evening." }),
        R.lg(page, { tab: 1, target: "Brightness" }),

        R.slider(Sio, { ds: "picture.color", tab: 2, min: 0, max: 100, step: 1, icon: "palette", label: "Colour" }),
        R.slider(Sio, { ds: "picture.tint", tab: 2, min: -50, max: 50, step: 1, icon: "blend", label: "Tint", help: "Shifts skin tones between green and red." }),
        R.lg(page, { tab: 2, target: "Color" }),

        R.slider(Sio, { ds: "picture.sharpness", tab: 3, min: 0, max: 50, step: 1, icon: "aperture", label: "Sharpness",
                        help: "Filmmaker keeps the picture as made. More adds edges." }),
        perApp(R.seg(Sio, { ds: "picture.truMotionMode", tab: 3, icon: "fast-forward", label: "TruMotion", appId: function() { return page.inputApp },
                            help: "Smooths motion. Off keeps the film's own look.",
                            options: ["Off", "Natural", "Smooth", "Clear"], values: ["off", "natural", "smooth", "clear"] })),
        R.toggle(Sio, { ds: "aiPicture.ai_Genre", tab: 3, icon: "wand-sparkles", label: "AI genre selection", help: "Picks a look for sport, film or news by what is playing." }),
        R.toggle(Sio, { ds: "aiPicture.ai_Brightness", tab: 3, icon: "sun-medium", label: "AI brightness", help: "Follows the room light and the scene." }),
        R.lg(page, { tab: 3, target: "Clarity" })
    ]
}
