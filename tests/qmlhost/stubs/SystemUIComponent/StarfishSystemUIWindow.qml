import QtQuick 2.4

// Offline stand-in for LG's system-UI window: the life cycle the compositor drives (`launch` and
// `close` with a payload), the window calls the logos make, and a log of them in the console.
Item {
    property QtObject lifeCycleStatus: QtObject {
        signal launch(var payload)
        signal close(var payload)
    }
    property int answers: 0
    function fire(name, payload) {
        if (name === "launch") lifeCycleStatus.launch(payload)
        else lifeCycleStatus.close(payload)
    }
    function show() { visible = true; console.info("[host] show") }
    function hide() { visible = false; console.info("[host] hide") }
    function exit() { visible = false; console.info("[host] exit") }
    function responseOffLogoRequest(ok) { answers++; console.info("[host] responseOffLogoRequest " + ok) }
}
