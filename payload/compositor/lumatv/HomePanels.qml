import QtQuick 2.4
import WebOSCompositorBase 1.0
import WebOSServices 1.0
import "LumaBus.js" as Bus

// LumaHome's side of the gear's tap and the Input key (the key filter's second entry, payload/keyfilter/lumatv.js, asks through LumaBus.js):
// while Home is in front, the quick panel and the input picker are Layers' residents, shown here. The gear's tap shows the quick panel or, when
// it is up, hides it; the Input key shows the picker or, when it is up, goes on to the next input. Anything else that is up (Settings, the
// library, a menu, the other panel) gives way, as it does for the drawer. Over an app the overlay does the same (LumaOverlay.qml).
//
// What this answers false to is LG's: the panel not built yet, the layout editor up, the owner's switch off (theme `remote.quickPanel`,
// `remote.inputPicker`), and the key filter then lets LG's own Quick Settings or input picker open as it always did. The AI prompt, the
// screensaver, a question waiting for its answer and a microphone that is listening own the remote: the key is taken and does nothing.
//
// The panel's rows that open a Settings page go through LumaHome's own signal; the button for LG's own Quick Settings is here, and so is
// the Settings page that a panel over an app asked for, which waits in LumaBus.js until Home is in front.
Item {
    id: panels
    visible: false
    property var home                       // LumaHome
    property var layers                     // its Layers
    property var aiPrompt                   // its AI prompt

    readonly property bool quickOn: !home.theme.remote || home.theme.remote.quickPanel !== false
    readonly property bool pickerOn: !home.theme.remote || home.theme.remote.inputPicker !== false

    function ready(kind) {
        if (!home.hostActive || home.editing) return false
        if (!(kind === "quick" ? quickOn : kind === "picker" ? pickerOn : false)) return false
        var r = layers.find(kind)
        return !!r && !!r.item
    }
    function open(kind) {
        if (!ready(kind)) return false
        if (aiPrompt.up || layers.saverUp) return true
        var top = layers.open
        if (top === "confirm" || top === "voice") return true
        if (top === kind) {
            if (kind === "picker") layers.pickerNext()
            else home.closeLayer()
            return true
        }
        home.closeAllLayers()
        if (home.settingsOpen) home.closeSettings()
        home.openLayer(kind)
        return true
    }

    // LG's own Quick Settings, for what the panel does not have: through the key filter's luna client, the one LG's key filters open it with
    function lgQuick() {
        home.closeLayerById("quick")
        var through = Bus.systemLaunch("com.webos.app.quicksettings") ? "the key filter's client" : "LS.adhoc"
        if (through === "LS.adhoc") LS.adhoc.call("luna://com.webos.service.systemUIManager", "/launch", JSON.stringify({ appId: "com.webos.app.quicksettings" }))
        console.info("[LumaHome] LG's Quick Settings asked for through " + through)
    }
    readonly property var quickRes: layers.find("quick")
    Connections {
        target: panels.quickRes ? panels.quickRes.item : null
        ignoreUnknownSignals: true
        onOpenLgQuick: panels.lgQuick()
    }

    // A panel over an app asked for a Settings page and took the TV to Home: open it once Home is in front (and has finished its own return)
    Connections {
        target: panels.home
        onHostActiveChanged: if (panels.home.hostActive) takeTimer.restart()
    }
    Timer {
        id: takeTimer
        interval: 80
        onTriggered: {
            var page = Bus.takeSettings()
            if (page === null || !panels.home.hostActive) return
            panels.home.closeAllLayers()
            panels.home.openSettings()
            if (page !== "") panels.layers.settingsGoTo(page)
        }
    }

    Component.onCompleted: Bus.setPanels({ ready: panels.ready, open: panels.open })
    Component.onDestruction: Bus.setPanels(null)
}
