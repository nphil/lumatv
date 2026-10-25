import QtQuick 2.4
import "LumaBus.js" as Bus

// Instantiated by the key filter JS (Qt.createComponent on this file's absolute URL) so the
// filter can reach the LumaBus.js library instance LumaHome registered with.
QtObject {
    function dispatch(key, pressed, autoRepeat, deviceId) { return Bus.dispatch(key, pressed, autoRepeat, deviceId) }
}
