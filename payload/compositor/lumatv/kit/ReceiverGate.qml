import QtQuick 2.4
import "."

// Whether the receiver's reading may be shown, for every surface that shows it (the volume OSD, the quick panel's tile,
// Settings' receiver page). A value is shown only while it is in date, so a surface can never draw a number that is
// no longer true:
//   - the helper must be up (`Helper.up`: its long poll answers), and the topic must say `fresh` and `reachable`; the
//     helper hides a value itself 6 s after the last good read and publishes that as an event;
//   - the reading's own `updatedAt` must be no older than `staleMs`. The helper moves that stamp quietly (a poll that
//     changes nothing raises no event, so a long poll is not woken every 2 s), so the copy this surface holds is as old
//     as the last real change. When it ages out the gate shows nothing and, for a surface that is on screen
//     (`confirmAged`), asks the helper for the topic's newest copy (`GET /v1/state/receiver`): a receiver that is still
//     answering has a new stamp there and the value comes back; a helper that cannot be reached leaves it hidden.
//
// `doc` is the newest copy while it may be shown and null otherwise: a surface draws `gate.doc.volumeText` and nothing
// else. `reading(doc)` fires for each copy that is in date (a new event, or an answer to a confirmation). While `active`
// is false (the surface is hidden) nothing is tracked and no timer runs; on becoming active the gate looks again.
QtObject {
    id: gate
    property bool active: true
    property bool confirmAged: true                 // ask the helper when the copy ages out (a surface that is on screen)
    property int staleMs: 6000
    readonly property var topic: Helper.receiver
    property var answer: null                       // the helper's newest copy, from a confirmation
    property double topicAt: 0                      // when this surface last got the topic from an event or a sync
    property double answerAt: 0
    readonly property var rc: newest()
    // not `rc`: a change handler runs before the bindings that depend on the property have been re-evaluated
    function newest() { return answer && answerAt >= topicAt ? answer : topic }
    property bool stale: true
    readonly property bool fresh: active && Helper.up && !!(rc && rc.reachable && rc.fresh) && !stale
    readonly property var doc: fresh ? rc : null
    // The topic says the receiver is switched off in Settings (`enabled: false`): there is nothing to wait for.
    readonly property bool disabled: !!(rc && rc.enabled === false)
    signal reading(var doc)

    function evaluate() {
        expiry.stop()
        var r = newest()
        if (!active || !r) { stale = true; return }
        var at = Number(r.updatedAt)
        var remaining = staleMs - (Date.now() - at)
        // a reading dated more than a second in the future is a clock that cannot be trusted
        stale = !(r.fresh && r.reachable && isFinite(at) && at > 0 && remaining > 0 && remaining <= staleMs + 1000)
        if (stale) return
        expiry.interval = Math.max(1, Math.ceil(Math.min(remaining, staleMs)))
        expiry.start()
        reading(r)
    }

    // Ask the helper for the topic's newest copy; what it answers replaces the one the events gave if it is newer.
    property bool asking: false
    function confirm() {
        if (asking || !Helper.up) return
        asking = true
        Helper.request("GET", "/v1/state/receiver", undefined, function(status, doc) {
            asking = false
            if (status !== 200 || !doc || !doc.value) return
            answer = doc.value
            answerAt = Date.now()
            evaluate()
        })
    }

    onTopicChanged: { topicAt = Date.now(); evaluate() }
    onActiveChanged: { evaluate(); if (active && stale && confirmAged) confirm() }
    onStaleMsChanged: evaluate()
    readonly property bool up: Helper.up
    onUpChanged: evaluate()
    Component.onCompleted: evaluate()
    property Timer expiry: Timer {
        onTriggered: { gate.stale = true; if (gate.active && gate.confirmAged) gate.confirm() }
    }
}
