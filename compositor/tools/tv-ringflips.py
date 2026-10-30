#!/usr/bin/env python3
"""The flips of a window of the compositor's trace ring (`ctl ring N`, saved by the video check as video-<app>-<kind>-ring.json).

usage: tv-ringflips.py RING.json NOTE [SECONDS]

NOTE is the `ctl note` text the window starts at (the last such line is taken), SECONDS how long it is read (default 4). Prints when the first
flip landed after the note and every interval between flips: under 20 ms as a bare number, longer ones as [offset:ms] (an idle gap of the video
app's own, or a stall inside a motion), then the frames' own cost where the ring has the `L flip` lines (compositor builds from round 20): the
draw-to-GPU-finish time of each frame and what the pacer made of it. The flip stamps and the ring's own time are the same clock (CLOCK_MONOTONIC).
"""
import json
import re
import sys


def main(argv):
    if len(argv) < 3:
        print(__doc__)
        return 2
    path, note = argv[1], argv[2]
    span = float(argv[3]) if len(argv) > 3 else 4.0
    with open(path) as f:
        lines = json.load(f)["lines"]
    starts = [i for i, line in enumerate(lines) if "note: " + note in line]
    if not starts:
        print("no note %r in %s" % (note, path))
        return 1
    start = starts[-1]
    t0 = float(lines[start].split()[0])
    f_flips, l_flips, costs = [], [], []
    for line in lines[start:]:
        if float(line.split()[0]) - t0 > span:
            break
        m = re.search(r" F flip ([0-9.]+)", line)
        if m:
            f_flips.append(float(m.group(1)))
        m = re.search(r" L flip ([0-9.]+) \+[0-9.]+ ms: frame ([0-9.]+) ms \(([a-z ]+)\), late (-?\d+) lost (\d+)( missed)?", line)
        if m:
            l_flips.append(float(m.group(1)))
            costs.append((float(m.group(2)), int(m.group(4)), int(m.group(5)), bool(m.group(6))))
    # builds with the `L` lines write one for every flip, the older ones only the `F` line of the last flip a pass reached
    flips = l_flips or f_flips
    if not flips:
        print("no flips within %.1f s of %s" % (span, note))
        return 0
    out = ["first flip %+.3f s after the note:" % (flips[0] - t0)]
    for a, b in zip(flips, flips[1:]):
        ms = (b - a) * 1000
        out.append("%.0f" % ms if ms < 20 else "[%+.3f:%.0f]" % (b - t0, ms))
    print(" ".join(out))
    if costs:
        worst = max(costs)
        print("frames %d: p50 %.1f ms, worst %.1f ms, late %d, lost %d, missed %d" % (
            len(costs), sorted(c[0] for c in costs)[len(costs) // 2], worst[0], sum(1 for c in costs if c[1] > 0), sum(c[2] for c in costs), sum(1 for c in costs if c[3])))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
