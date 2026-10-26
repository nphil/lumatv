import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "."

// Ambient light (docs/mocks/30-settings-ambient): the settings of LumaTV's own lights-around-the-TV feature, kept in the
// helper's settings layer as the owner `base.ambient` (helper/app/src/schemas/base.ambient.json) and drawn through the same
// row builders as LG's settings (HelperSettings has Sio's interface). This page only sets; the capture, the mapping and the
// light output run in the helper (`GET /v1/ambient` says what they are doing). The Sound row is the helper's sound mode: the
// picture as it is, with the TV's own sound on it (brightness and beats), or the sound alone; the helper reads the sound only
// then. The strip's and Hue's addresses are not settings here: they live in the helper's own file. LG has no equivalent on
// the C4; its "Lighting" settings are for other models and are not shown.
PageList {
    id: page
    title: "Ambient light"
    tabs: ["Light", "Layout", "Colour"]

    HelperSettings { id: hs; owner: "base.ambient" }
    readonly property var settings: hs
    property var amb: null                                 // GET /v1/ambient: {state, sections, strip, hue, problem, ...}
    property string testProblem: ""                        // the helper's words when it could not run the test pattern (409)
    readonly property bool testing: !!amb && amb.state === "testing"
    readonly property bool stripKnown: !!amb && amb.strip === true
    readonly property int sections: hs.num("top") + hs.num("right") + hs.num("bottom") + hs.num("left")
    readonly property int edges: (hs.num("top") > 0 ? 1 : 0) + (hs.num("right") > 0 ? 1 : 0) + (hs.num("bottom") > 0 ? 1 : 0) + (hs.num("left") > 0 ? 1 : 0)
    readonly property string modeLine: !hs.has("enabled") ? "" : !hs.get("enabled") ? "Off"
        : hs.get("sound") === "sound" ? "Follows the sound"
        : R.name({ picture: "Follows the picture", colour: "Still colour", "picture-off-home": "Follows the picture, off on Home" }, hs.get("mode"), "")
          + (hs.get("sound") === "picture-sound" ? " with sound" : "")
    subtitle: modeLine === "" ? "" : modeLine + (hs.has("top") ? " \u00b7 " + sections + " sections on " + edges + (edges === 1 ? " edge" : " edges") : "")
    summary: modeLine === "" ? "Not available" : modeLine

    function refresh() {
        hs.refresh()
        Helper.request("GET", "/v1/ambient", undefined, function(status, doc) { amb = doc && doc.ok ? doc : null })
    }
    Component.onCompleted: refresh()
    onOpened: refresh()
    Timer { interval: 1500; repeat: true; running: page.visible && page.active && page.testing; onTriggered: page.refresh() }

    rows: [
        R.toggle(hs, { ds: "enabled", tab: 0, on: "true", off: "false", icon: "lamp", label: "Ambient light", detail: "Lights around the TV follow what is on the screen." }),
        R.slider(hs, { ds: "brightness", tab: 0, min: 0, max: 100, step: 5, unit: "%", icon: "sun", label: "Brightness", detail: "How bright the strip goes. Dims with the room at night." }),
        R.seg(hs, { ds: "mode", tab: 0, icon: "sliders-horizontal", label: "Mode", detail: "Follows the picture, a still colour, or off on Home and menus.",
                    options: ["Picture", "Colour", "Off on Home"], values: ["picture", "colour", "picture-off-home"], segWidth: 150 }),
        R.seg(hs, { ds: "sound", tab: 0, icon: "audio-waveform", label: "Sound", detail: "Add the TV's own sound: its loudness and its beats. Read before the volume and never saved.",
                    options: ["Picture", "Picture + sound", "Sound"], values: ["picture", "picture-sound", "sound"], segWidth: 150 }),
        R.note(function() { return page.testProblem !== "" ? page.testProblem : (page.amb && page.amb.problem ? String(page.amb.problem) : "") }, 0,
               function() { return page.testProblem !== "" || !!(page.amb && page.amb.problem) }),
        R.head("Strip", 0),
        { kind: "figure", tab: 0, zones: function() { return { top: hs.num("top"), right: hs.num("right"), bottom: hs.num("bottom"), left: hs.num("left") } },
          text: function() { return page.sections + " sections. Top " + hs.num("top") + " \u00b7 right " + hs.num("right") + " \u00b7 bottom " + hs.num("bottom") + " \u00b7 left " + hs.num("left")
                                    + ", clockwise from top-left. Delay " + hs.num("delay_ms") + " ms. More on the Layout tab." } },
        { kind: "action", tab: 0, icon: "cable", label: "Test pattern", button: function() { return page.testing ? "Stop" : "Run" },
          detail: function() { return !page.amb ? "The helper does not answer for the lights." : page.stripKnown ? "Lights each section in turn so the layout can be checked." : "No strip is set up in the helper yet." },
          enabled: function() { return page.stripKnown },
          act: function() {
              Helper.post("/v1/ambient/test", { run: !page.testing }, function(status, doc) {
                  page.testProblem = doc && doc.error ? String(doc.error.message || "") : ""
                  page.refresh()
              })
          } },

        R.slider(hs, { ds: "top", tab: 1, min: 0, max: 120, step: 1, icon: "arrow-big-up", label: "Top", detail: "Sections along the top edge, left to right." }),
        R.slider(hs, { ds: "right", tab: 1, min: 0, max: 120, step: 1, icon: "chevron-right", label: "Right", detail: "Sections down the right edge." }),
        R.slider(hs, { ds: "bottom", tab: 1, min: 0, max: 120, step: 1, icon: "chevron-down", label: "Bottom", detail: "Sections along the bottom edge, right to left." }),
        R.slider(hs, { ds: "left", tab: 1, min: 0, max: 120, step: 1, icon: "arrow-left", label: "Left", detail: "Sections up the left edge." }),
        R.slider(hs, { ds: "delay_ms", tab: 1, min: 0, max: 2000, step: 10, unit: " ms", icon: "timer", label: "Delay", detail: "Lights follow the picture this late, to line up with the sound." }),

        R.seg(hs, { ds: "still_colour", tab: 2, icon: "palette", label: "Still colour", detail: "The colour in Colour mode.",
                    options: ["Warm", "Cool", "Rose", "Iris", "Foam"], values: ["warm", "cool", "rose", "iris", "foam"] }),
        R.slider(hs, { ds: "saturation", tab: 2, min: 50, max: 150, step: 5, unit: "%", icon: "blend", label: "Saturation", detail: "How strong the colours are next to the picture." }),
        R.toggle(hs, { ds: "dark_hold", tab: 2, on: "true", off: "false", icon: "moon", label: "Keep a glow in dark scenes", detail: "A very dim glow instead of off, which a lamp can flicker at." }),
        R.toggle(hs, { ds: "skip_bars", tab: 2, on: "true", off: "false", icon: "film", label: "Skip black bars", detail: "Ignore the bars of letterboxed and pillarboxed films." }),
        R.toggle(hs, { ds: "game_edges", tab: 2, on: "true", off: "false", icon: "gamepad-2", label: "Weight the edges for games", detail: "Uses the edge-weighted capture on a game input." })
    ]
}
