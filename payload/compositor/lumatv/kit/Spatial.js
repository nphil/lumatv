.pragma library

// Focus moves to the nearest control in the direction pressed (docs/mocks/mock.js uses the same rule): the
// candidate's middle must lie past the focused one's, distance along the axis counts once and the sideways gap
// between the two boxes (0 when they overlap across the direction) three times; a candidate that overlaps the focused
// one across the direction (the same column for Up and Down) beats any that does not. Boxes are {x, y, w, h}.
// Returns the index of the box to move to, or -1 when nothing lies that way.
function nearest(boxes, from, dir) {
    var a = boxes[from]
    if (!a) return -1
    var ax = a.x + a.w / 2, ay = a.y + a.h / 2
    var best = -1, bestScore = 1e9, aligned = -1, alignedScore = 1e9
    for (var i = 0; i < boxes.length; i++) {
        if (i === from || !boxes[i]) continue
        var b = boxes[i]
        var dx = b.x + b.w / 2 - ax, dy = b.y + b.h / 2 - ay
        if (dir === "up" && dy >= -4) continue
        if (dir === "down" && dy <= 4) continue
        if (dir === "left" && dx >= -4) continue
        if (dir === "right" && dx <= 4) continue
        var vertical = dir === "up" || dir === "down"
        var gap = vertical ? Math.max(0, Math.abs(dx) - (a.w + b.w) / 2) : Math.max(0, Math.abs(dy) - (a.h + b.h) / 2)
        var primary = vertical ? Math.abs(dy) : Math.abs(dx)
        if (primary + 3 * gap < bestScore) { best = i; bestScore = primary + 3 * gap }
        if (gap === 0 && primary < alignedScore) { aligned = i; alignedScore = primary }
    }
    return aligned >= 0 ? aligned : best
}
