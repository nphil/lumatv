import QtQuick 2.12

// A soft arc around a point: arc.png is a 400 px ring (radius 165, 7 px core, a faint 10 px glow)
// whose brightness trails off over 230 degrees behind its head, drawn foam to iris to love. The
// head sits at 12 o'clock; `sweep` is the head's angle in degrees, clockwise. `reverse` mirrors the
// ring so the tail follows a head that travels anticlockwise. One textured quad, rotated only.
// The PNG is a local file the engine reads on the first frame; without it only the arc is missing.
Image {
    property real sweep: 0
    property bool reverse: false

    width: 400
    height: 400
    source: "arc.png"
    smooth: true
    mirror: reverse
    rotation: sweep
    visible: opacity > 0
}
