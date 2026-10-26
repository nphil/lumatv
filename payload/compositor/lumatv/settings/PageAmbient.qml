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
    summary: !hs.has("enabled") ? "Not available" : hs.get("enabled") ? "On" : "Off"

    function refresh() {
        hs.refresh()
        Helper.request("GET", "/v1/ambient", undefined, function(status, doc) { amb = doc && doc.ok ? doc : null })
    }
    Component.onCompleted: refresh()
    onOpened: refresh()
    Timer { interval: 1500; repeat: true; running: page.visible && page.active && page.testing; onTriggered: page.refresh() }

    rows: [
        R.toggle(hs, { ds: "enabled", tab: 0, on: "true", off: "false", icon: "lamp", label: "Ambient light", help: "Lights around the TV follow what is on the screen." }),
        R.slider(hs, { ds: "brightness", tab: 0, min: 0, max: 100, step: 5, unit: "%", icon: "sun", label: "Brightness", help: "How bright the strip goes. It dims with the room at night." }),
        R.seg(hs, { ds: "mode", tab: 0, icon: "sliders-horizontal", label: "Mode", help: "Follows the picture, shows a still colour, or goes off on Home and menus.",
                    options: ["Picture", "Colour", "Off on Home"], values: ["picture", "colour", "picture-off-home"] }),
        R.seg(hs, { ds: "sound", tab: 0, icon: "audio-waveform", label: "Sound", help: "Adds the TV's own sound: its loudness and its beats. Nothing is saved.",
                    options: ["Picture", "Picture + sound", "Sound"], values: ["picture", "picture-sound", "sound"] }),
        R.note(function() { return page.testProblem !== "" ? page.testProblem : (page.amb && page.amb.problem ? String(page.amb.problem) : "") }, 0,
               function() { return page.testProblem !== "" || !!(page.amb && page.amb.problem) }),
        R.head("Strip", 0),
        { kind: "figure", tab: 0, zones: function() { return { top: hs.num("top"), right: hs.num("right"), bottom: hs.num("bottom"), left: hs.num("left") } },
          text: function() { return page.sections + " sections" } },
        { kind: "action", tab: 0, icon: "cable", label: "Test pattern", button: function() { return page.testing ? "Stop" : "Run" },
          detail: function() { return !page.amb ? "Not answering" : page.stripKnown ? "" : "No strip set up" },
          help: "Lights each section in turn so the layout can be checked.",
          enabled: function() { return page.stripKnown },
          act: function() {
              Helper.post("/v1/ambient/test", { run: !page.testing }, function(status, doc) {
                  page.testProblem = doc && doc.error ? String(doc.error.message || "") : ""
                  page.refresh()
              })
          } },

        R.slider(hs, { ds: "top", tab: 1, min: 0, max: 120, step: 1, icon: "arrow-big-up", label: "Top", help: "Sections along the top edge, left to right. The edges run clockwise from the top left." }),
        R.slider(hs, { ds: "right", tab: 1, min: 0, max: 120, step: 1, icon: "chevron-right", label: "Right" }),
        R.slider(hs, { ds: "bottom", tab: 1, min: 0, max: 120, step: 1, icon: "chevron-down", label: "Bottom" }),
        R.slider(hs, { ds: "left", tab: 1, min: 0, max: 120, step: 1, icon: "arrow-left", label: "Left" }),
        R.slider(hs, { ds: "delay_ms", tab: 1, min: 0, max: 2000, step: 10, unit: " ms", icon: "timer", label: "Delay", help: "The lights follow the picture this late, to line up with the sound." }),

        R.seg(hs, { ds: "still_colour", tab: 2, icon: "palette", label: "Still colour", help: "The colour in Colour mode.",
                    options: ["Warm", "Cool", "Rose", "Iris", "Foam"], values: ["warm", "cool", "rose", "iris", "foam"] }),
        R.slider(hs, { ds: "saturation", tab: 2, min: 50, max: 150, step: 5, unit: "%", icon: "blend", label: "Saturation", help: "How strong the colours are next to the picture." }),
        R.toggle(hs, { ds: "dark_hold", tab: 2, on: "true", off: "false", icon: "moon", label: "Glow in dark scenes", help: "A very dim glow instead of off, which a lamp can flicker at." }),
        R.toggle(hs, { ds: "skip_bars", tab: 2, on: "true", off: "false", icon: "film", label: "Skip black bars", help: "Ignores the bars of letterboxed and pillarboxed films." }),
        R.toggle(hs, { ds: "game_edges", tab: 2, on: "true", off: "false", icon: "gamepad-2", label: "Edges for games", help: "Uses the edge-weighted capture on a game input." })
    ]
}
