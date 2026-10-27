import QtQuick 2.4
import QtQuick.Window 2.2

// Offline stand-in for LG's `WebOSWindow` (Eos.Window, qml-webos-framework): a Window with the two properties the shell
// client sets; on the set the plugin sends them to the compositor as `_WEBOS_WINDOW_TYPE` and `appId`.
Window {
    property string windowType: ""
    property string appId: ""
}
