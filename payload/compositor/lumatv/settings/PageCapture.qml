import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "Capture.js" as Cap
import "."

// Screenshots & recording (docs/research/capture-result.md): a picture of what LumaTV composes, or a recording of it, saved on the TV. System >
// General opens it. The compositor does the work, the helper's /v1/capture routes are the way in and `capture.enabled` in the helper's config
// is the switch: while it is off the rows are dimmed and the page says so. Video is never in a capture (a window for it is black), and a
// recording shows a red dot in the corner of the screen for as long as it runs, on every screen (panels/CaptureMark.qml). The page asks
// the helper for the state when it opens and each second while a recording runs; nothing else polls.
PageList {
    id: page
    title: "Screenshots & recording"
    back: "System"
    property var cap: Cap.state(null)         // what the helper last said (settings/Capture.js)
    property string said: ""                  // the words for the last thing that happened
    property bool busy: false                 // a command is with the helper
    property bool waiting: false              // a screenshot is on its way after its delay
    readonly property bool usable: cap.enabled && cap.available
    summary: cap.recording ? "Recording " + Cap.clock(cap.seconds) : ""

    function refresh(done) {
        Helper.request("GET", "/v1/capture", undefined, function(status, doc) { page.cap = Cap.state(status === 200 ? doc : null); if (done) done(page.cap) })
    }
    onOpened: { said = ""; refresh() }
    Component.onCompleted: refresh()
    Timer { interval: 1000; repeat: true; running: page.visible && page.cap.recording; onTriggered: page.refresh() }

    function run(kind, path) {
        busy = true
        Helper.post(path, {}, function(status, doc) {
            page.busy = false
            page.refresh(function(st) { page.said = Cap.said(kind, status, doc, st) })
        })
    }
    // A screenshot a few seconds from now: time to go to the screen that is wanted, which is then what the picture shows.
    Timer { id: later; interval: 5000; onTriggered: { page.waiting = false; page.run("shot", "/v1/capture/screenshot") } }

    rows: [
        R.note("Screen capture is off on this TV.", 0, function() { return !page.cap.enabled }),
        { kind: "note", tab: 0, text: function() { return page.said }, visible: function() { return page.said !== "" } },
        { kind: "action", tab: 0, id: "shot", icon: "camera", label: "Take a screenshot", button: "Capture",
          help: "Saves a picture of the screen on the TV. Video stays black in it.",
          detail: function() { return page.cap.enabled && !page.cap.available ? "Needs LumaTV's own compositor" : "" },
          enabled: function() { return page.usable && !page.busy && !page.waiting },
          act: function() { page.run("shot", "/v1/capture/screenshot") } },
        { kind: "action", tab: 0, id: "later", icon: "camera", label: "Screenshot in 5 seconds", button: function() { return page.waiting ? "Waiting" : "Start" },
          help: "Waits five seconds first, so you can go to the screen you want.",
          enabled: function() { return page.usable && !page.busy && !page.waiting },
          act: function() { page.waiting = true; later.restart() } },
        { kind: "toggle", tab: 0, id: "record", icon: "video", label: "Screen recording",
          help: "Records what LumaTV shows into a file on the TV. A red dot is on the screen while it records. Video stays black in it.",
          detail: function() { return page.cap.recording ? Cap.clock(page.cap.seconds) : (page.usable && !page.cap.encoder ? "Not set up on this TV" : "") },
          enabled: function() { return (page.usable && page.cap.encoder && !page.busy) || page.cap.recording },
          on: function() { return page.cap.recording },
          act: function(v) { page.run(v ? "start" : "stop", v ? "/v1/capture/record/start" : "/v1/capture/record/stop") } },
        { kind: "value", tab: 0, id: "files", icon: "hard-drive", label: "Saved on the TV", info: true, value: function() { return Cap.files(page.cap) },
          help: "They are kept on the TV. When there is no more room for a new one, the oldest go first.",
          visible: function() { return page.cap.enabled } }
    ]
}
