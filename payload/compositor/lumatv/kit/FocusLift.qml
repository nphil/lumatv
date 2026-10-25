import QtQuick 2.4

// The lift half of focus, shared by cards, tiles and status buttons. The highlight half is
// the plain `focused` flag, which follows every key press at once. `value` stays 0 until the
// item is focused and the focus has rested (`rested`, LumaHome's focus-rest timer), then
// animates to 1 over `duration`. It returns to 0 immediately when the focus leaves or moves
// on, so sweeping over a row animates nothing and the first lift starts when the user
// stops. `reduced` skips the animation.
QtObject {
    id: lift

    property bool focused: false
    property bool rested: true
    property bool reduced: false
    property int duration: 150
    property int easing: Easing.OutCubic
    property real value: 0
    readonly property bool up: focused && rested

    // `up` is one derived value, so focus arriving before its rest flag clears in the same
    // turn starts and cancels the animation without a frame in between.
    onUpChanged: {
        anim.stop()
        if (!up) value = 0
        else if (reduced) value = 1
        else anim.restart()
    }

    property NumberAnimation anim: NumberAnimation {
        target: lift
        property: "value"
        to: 1
        duration: lift.duration
        easing.type: lift.easing
    }
}
