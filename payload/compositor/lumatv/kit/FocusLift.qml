import QtQuick 2.4

// The lift half of focus, shared by cards, tiles, rows, buttons and status buttons. The highlight half is
// the plain `focused` flag, which follows every key press at once. `value` goes to 1 once the item is
// focused and the focus has rested (`rested`, LumaHome's focus-rest timer) and back to 0 when it is not,
// each way over `duration` on `easing`. One animation is retargeted from wherever `value` is, never
// chained: the item losing focus eases down while the one gaining it eases up, and a key repeat that moves
// on again reverses the lift without a jump. `reduced` skips the animation.
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
    // turn retargets once without a frame in between.
    onUpChanged: {
        anim.stop()
        if (reduced) value = up ? 1 : 0
        else { anim.to = up ? 1 : 0; anim.restart() }
    }

    property NumberAnimation anim: NumberAnimation {
        target: lift
        property: "value"
        to: 0
        duration: lift.duration
        easing.type: lift.easing
    }
}
