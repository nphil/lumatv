.pragma library

// The remote's keys as the compositor delivers them on wl_keyboard (evdev codes; the keymap it serves names the arrows,
// Return and the digits, nothing else) to the Qt key codes LumaTV's QML was written against under LG's compositor
// (`LumaHome.key`, `LumaOverlay.key`). A code not here keeps what Qt made of it.
var BY_SCANCODE = {
    103: Qt.Key_Up, 108: Qt.Key_Down, 105: Qt.Key_Left, 106: Qt.Key_Right,
    28: Qt.Key_Return, 352: Qt.Key_Return,
    412: Qt.Key_Back, 158: Qt.Key_Back, 174: Qt.Key_Back,     // Back (KEY_PREVIOUS on this set), Back, Exit
    139: Qt.Key_Menu,
    2: Qt.Key_1, 3: Qt.Key_2, 4: Qt.Key_3, 5: Qt.Key_4, 6: Qt.Key_5, 7: Qt.Key_6, 8: Qt.Key_7, 9: Qt.Key_8, 10: Qt.Key_9, 11: Qt.Key_0,
    113: Qt.Key_VolumeMute, 114: Qt.Key_VolumeDown, 115: Qt.Key_VolumeUp
}

// A Qt key event's evdev code: xkb keycodes are evdev codes plus 8.
function scancode(event) { return event.nativeScanCode > 8 ? event.nativeScanCode - 8 : 0 }

function qtKey(event) {
    var k = BY_SCANCODE[scancode(event)]
    return k !== undefined ? k : event.key
}
