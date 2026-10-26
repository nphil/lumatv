import QtQuick 2.4

// Whether the TV has an ATSC 3.0 tuner: the config `system.supportATSC3tuner` of `com.webos.service.config`, read once when the page is built.
// LG's NEXTGEN TV rows are on show only where it is true: NEXTGEN TV Description, Amplify Dialogue and its gain, NEXTGEN TV Mode (captions), the
// two Interactive Service switches and Emergency Information. The owner's TV answers false (read 2026-10-26), so none of them is there.
LunaCall {
    property bool present: false

    Component.onCompleted: call("atsc3", "luna://com.webos.service.config", "getConfigs", { configNames: ["system.supportATSC3tuner"] })
    onAnswered: {
        if (tag !== "atsc3" || !doc || doc.returnValue === false || !doc.configs) return
        var v = doc.configs["system.supportATSC3tuner"]
        present = !(v === "false" || v === false || v === 0 || v === null || v === undefined)
    }
}
