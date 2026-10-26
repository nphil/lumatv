import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "Choice.js" as C
import "."

// Emergency Information (LG: General › System › Additional Settings): the NEXTGEN TV (ATSC 3.0) emergency alert and the language of its voice
// guidance, `option.emergencyInformationAtsc30` and `option.emergencyInformationLanguageAtsc30`. LG shows both only where the TV has an ATSC 3.0
// tuner (the config `system.supportATSC3tuner`; System's row for this page waits for it) and the TV's description calls the key visible.
PageList {
    id: page
    title: "Emergency Information"
    back: "System"

    function refresh() { Sio.read("option", ["emergencyInformationAtsc30", "emergencyInformationLanguageAtsc30"]) }
    onOpened: refresh()

    rows: [
        C.lgShown(Sio, "option.emergencyInformationAtsc30", R.toggle(Sio, { ds: "option.emergencyInformationAtsc30", tab: 0, icon: "octagon-alert", label: "Enable the function",
                                                                          help: "When an emergency occurs, the TV shows the alert and speaks it." })),
        C.choice(Sio, { ds: "option.emergencyInformationLanguageAtsc30", tab: 0, icon: "languages", label: "Audio Language", names: "digitalPlusCaption", page: page,
                        help: "The language of the spoken alert." })
    ]
}
