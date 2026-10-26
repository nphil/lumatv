import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "Choice.js" as C
import "."

// Picture (docs/mocks/01-settings-picture, docs/research/settings-coverage.md): LG's Picture menu, written to the TV's own settings
// service (so LG's picture processing, panel care and stored values are unchanged; only the screen is new). The keys, categories and
// value lists are LG's own (docs/mocks/settings-map.md): a short list is the TV's own list for the key, in LG's names, and a row the TV
// describes as not visible (HDR-only rows while an SDR picture plays, the ones this set never shows) is left out as LG leaves it out.
// A row the service has no key for is left out, one it has not answered is dimmed; the aspect ratio and TruMotion are per-input settings
// (the service answers them only for an app), so they are offered while an input is in front, and the head's chip names that input.
// The rail shows the mode in force. Fine Tune and White Balance (Colour tab) are pages of their own.
// Text: a row is its name and its value; what is not obvious is `help`, behind the row's info mark.
PageList {
    id: page
    title: "Picture"
    tabs: ["Mode", "Brightness", "Colour", "Clarity", "Motion", "Expert"]

    readonly property var modeValues: ["filmMaker", "cinema", "normal", "vivid", "game", "sports", "eco", "expert1", "expert2"]
    readonly property var modeNames: ({ filmMaker: "Filmmaker", cinema: "Cinema", normal: "Standard", vivid: "Vivid", game: "Game", sports: "Sports",
                                        eco: "Eco", expert1: "Expert \u00b7 day", expert2: "Expert \u00b7 night" })
    readonly property string mode: R.name(modeNames, Sio.get("picture.pictureMode"), "")
    readonly property string inputApp: Inputs.current ? Inputs.current.appId : ""
    readonly property bool userMotion: String(Sio.get("picture.truMotionMode")) === "user"
    chip: Inputs.current ? Inputs.current.port : ""
    chipTone: "info"
    summary: mode

    LunaCall { id: luna }

    function refresh() {
        Inputs.refresh()
        Sio.read("picture", ["pictureMode", "backlight", "energySaving", "eyeComfortMode", "filmmakerAmbientLight", "contrast", "brightness", "color",
                             "tint", "sharpness"])
        Sio.read("picture", ["dynamicContrast", "peakBrightness", "gamma", "nearBlackDetail", "motionEyeCare", "colorTemperature", "colorGamut", "dynamicColor", "blackLevel", "energySavingAutoMin"])
        Sio.read("picture", ["superResolution", "noiseReduction", "mpegNoiseReduction", "smoothGradation", "realCinema", "dolbyPrecisionDetail"])
        Sio.read("picture", ["truMotionJudder", "truMotionBlur", "motionPro", "filmMakerMode", "hdrDynamicToneMapping", "expressionEnhancer", "applyToAllInput"])
        Sio.read("aiPicture", ["ai_Picture", "ai_Genre", "ai_Brightness"])
        Sio.read("aspectRatio", ["justScan"])
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
    // What a picture mode's reset names: the input and the mode in force, as LG's own Reset sends them.
    function resetPicture() {
        var d = Sio.dimensions.picture || {}
        luna.act(Sio.service, "resetSystemSettings", { category: "picture", dimension: { input: d.input, pictureMode: String(Sio.get("picture.pictureMode")) } })
        refresh()
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
        R.seg(Sio, { ds: "picture.energySaving", tab: 0, icon: "leaf", label: "Energy saving",
                     options: ["Auto", "Off", "Minimum", "Medium", "Maximum"], values: ["auto", "off", "min", "med", "max"] }),
        R.over(C.range(Sio, { ds: "picture.energySavingAutoMin", tab: 0, icon: "leaf", label: "Minimum Brightness", min: 0, max: 10, step: 1,
                              help: "How dark Auto energy saving may make the picture." }),
               { enabled: function() { return String(Sio.get("picture.energySaving")) === "auto" && Sio.usable("picture.energySavingAutoMin") },
                 detail: function() { return !Sio.has("picture.energySavingAutoMin") ? "TV not answering" : String(Sio.get("picture.energySaving")) === "auto" ? "" : "Energy saving is not Auto" } }),
        R.toggle(Sio, { ds: "aspectRatio.justScan", tab: 0, icon: "monitor", label: "Just Scan",
                        help: "Shows the picture whole, without cropping its edges. Turn it off if a line shows at an edge." }),

        R.slider(Sio, { ds: "picture.contrast", tab: 1, min: 0, max: 100, step: 1, icon: "contrast", label: "Contrast" }),
        R.slider(Sio, { ds: "picture.brightness", tab: 1, min: 0, max: 100, step: 1, icon: "sun-dim", label: "Brightness",
                        help: "Lifts or deepens the dark parts. The panel's own light is OLED pixel brightness." }),
        R.toggle(Sio, { ds: "picture.eyeComfortMode", tab: 1, icon: "eye", label: "Eye comfort", help: "A warmer picture for the evening." }),
        C.seg(Sio, { ds: "picture.dynamicContrast", tab: 1, icon: "contrast", label: "Auto Dynamic Contrast", names: "lmha",
                     help: "Adjusts the contrast to how bright the picture is." }),
        C.seg(Sio, { ds: "picture.peakBrightness", tab: 1, icon: "sun", label: "Peak Brightness", names: "lmha",
                     help: "How bright the brightest parts of the picture may get." }),
        C.seg(Sio, { ds: "picture.gamma", tab: 1, icon: "gauge", label: "Gamma", names: "gamma",
                     help: "The lower the number, the brighter the middle tones." }),
        C.lgShown(Sio, "picture.nearBlackDetail", R.slider(Sio, { ds: "picture.nearBlackDetail", tab: 1, min: -3, max: 3, step: 1, icon: "moon", label: "Near Black Detail" })),
        C.seg(Sio, { ds: "picture.blackLevel", tab: 1, icon: "contrast", label: "Video Range", names: "blackLevel",
                     help: "Limited for video from a player, Full for a PC or console. Auto lets the signal decide." }),
        C.seg(Sio, { ds: "picture.motionEyeCare", tab: 1, icon: "eye", label: "Motion Eye Care", names: "lmha",
                     help: "Dims the picture a little in fast scenes to ease glare." }),

        R.slider(Sio, { ds: "picture.color", tab: 2, min: 0, max: 100, step: 1, icon: "palette", label: "Colour" }),
        R.slider(Sio, { ds: "picture.tint", tab: 2, min: -50, max: 50, step: 1, icon: "blend", label: "Tint", help: "Shifts skin tones between green and red." }),
        R.slider(Sio, { ds: "picture.colorTemperature", tab: 2, min: -50, max: 50, step: 1, icon: "sun-medium", label: "Color Temperature",
                        help: "Below zero is cooler (bluer), above zero is warmer (redder)." }),
        C.seg(Sio, { ds: "picture.colorGamut", tab: 2, icon: "palette", label: "Color Gamut", names: "colorGamut" }),
        C.seg(Sio, { ds: "picture.dynamicColor", tab: 2, icon: "sliders-horizontal", label: "Color Adjustment", names: "lmha" }),
        R.link({ tab: 2, id: "finetune", icon: "sliders-horizontal", label: "Fine Tune", help: "Saturation, tint and luminance of each of the six colours.", go: function() { page.host.openPage("finetune") } }),
        R.link({ tab: 2, id: "whitebalance", icon: "sun-medium", label: "White Balance", help: "The grey scale's red, green and blue, at 2, 10 or 22 points.", go: function() { page.host.openPage("whitebalance") } }),

        R.slider(Sio, { ds: "picture.sharpness", tab: 3, min: 0, max: 50, step: 1, icon: "aperture", label: "Sharpness",
                        help: "Filmmaker keeps the picture as made. More adds edges." }),
        C.seg(Sio, { ds: "picture.superResolution", tab: 3, icon: "image", label: "Super Resolution", names: "lmha",
                     help: "Sharpens a soft picture, such as a low-resolution broadcast." }),
        C.seg(Sio, { ds: "picture.noiseReduction", tab: 3, icon: "zap", label: "Noise Reduction", names: "lmha" }),
        C.seg(Sio, { ds: "picture.mpegNoiseReduction", tab: 3, icon: "cpu", label: "MPEG Noise Reduction", names: "lmha",
                     help: "Cleans up blocks in compressed video." }),
        C.seg(Sio, { ds: "picture.smoothGradation", tab: 3, icon: "blend", label: "Smooth Gradation", names: "lmha",
                     help: "Smooths the bands in a sky or a dark gradient." }),
        R.toggle(Sio, { ds: "picture.realCinema", tab: 3, icon: "film", label: "Real Cinema", help: "Plays film as 24 frames a second, the way it was shot." }),
        C.lgShown(Sio, "picture.dolbyPrecisionDetail", R.toggle(Sio, { ds: "picture.dolbyPrecisionDetail", tab: 3, icon: "image", label: "Precision Detail",
                                                                     help: "More detail in Dolby Vision pictures." })),

        perApp(C.choice(Sio, { ds: "picture.truMotionMode", tab: 4, icon: "fast-forward", label: "TruMotion", names: "truMotion", page: page,
                                appId: function() { return page.inputApp }, help: "Smooths motion. Off keeps the film's own look." })),
        R.over(R.slider(Sio, { ds: "picture.truMotionJudder", tab: 4, min: 0, max: 10, step: 1, icon: "timer", label: "De-Judder",
                               help: "Smooths the stutter of film on a 60 Hz screen." }), { visible: function() { return page.userMotion && !Sio.missing("picture.truMotionJudder") } }),
        R.over(R.slider(Sio, { ds: "picture.truMotionBlur", tab: 4, min: 0, max: 10, step: 1, icon: "activity", label: "De-Blur",
                               help: "Sharpens moving objects." }), { visible: function() { return page.userMotion && !Sio.missing("picture.truMotionBlur") } }),
        R.toggle(Sio, { ds: "picture.motionPro", tab: 4, icon: "fast-forward", label: "OLED Motion Pro",
                        help: "Reduces blur in fast motion by dimming the screen a little." }),

        R.toggle(Sio, { ds: "picture.filmMakerMode", tab: 5, icon: "clapperboard", label: "Filmmaker Auto Start",
                        help: "Switches to Filmmaker mode by itself when a film asks for it." }),
        C.seg(Sio, { ds: "picture.hdrDynamicToneMapping", tab: 5, icon: "sun-medium", label: "Dynamic Tone Mapping", names: "toneMapping",
                     help: "Adjusts the contrast of HDR pictures scene by scene." }),
        C.seg(Sio, { ds: "picture.expressionEnhancer", tab: 5, icon: "sparkles", label: "Expression Enhancer", names: "expression" }),
        R.toggle(Sio, { ds: "aiPicture.ai_Genre", tab: 5, icon: "wand-sparkles", label: "AI genre selection", help: "Picks a look for sport, film or news by what is playing." }),
        R.toggle(Sio, { ds: "aiPicture.ai_Brightness", tab: 5, icon: "sun-medium", label: "AI brightness", help: "Follows the room light and the scene." }),
        C.action({ id: "applyall", tab: 5, icon: "list-checks", label: "Apply to All Inputs", button: "Apply",
                   enabled: function() { return Sio.usable("picture.applyToAllInput") },
                   help: "Gives every input this mode's settings.",
                   act: C.ask(page, { icon: "list-checks", title: "Apply to all inputs?", body: "Every input gets this mode's settings.", yes: "Apply", no: "Cancel" },
                              function() { Sio.write("picture.applyToAllInput", "picture") }) }),
        C.action({ id: "reset", tab: 5, icon: "rotate-ccw", label: "Reset", button: "Reset", danger: true,
                   enabled: function() { return Sio.has("picture.pictureMode") },
                   act: C.ask(page, { icon: "rotate-ccw", title: "Reset this picture mode?", body: "Its settings go back to LG's.", yes: "Reset", no: "Cancel", danger: true },
                              function() { page.resetPicture() }) })
    ]
}
