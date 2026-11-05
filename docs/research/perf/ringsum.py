#!/usr/bin/env python3
"""The frames of a compositor trace ring dump (`ctl ring N`, saved by the gate as /tmp/perf/<name>.ring.json) in numbers: what each frame
the compositor drew cost its loop thread (`D frame N render X us owed P px in R rects, ui D draws L layers (why)`), what it cost from the
draw to the GPU's finish and what the pacer made of it (`L flip T +I ms: frame G ms (gpu done), late N lost M`), and how the flips lay.

    python3 ringsum.py RING.json [NOTE [SECONDS]]

With NOTE (a `ctl note` text the gate writes, such as `settings-scroll-open`) only the SECONDS (default 8) after its last occurrence are read.
`summarize` is what perf/gate.py prints for every measured window under LumaTV's compositor."""
import json
import re
import sys

D_LINE = re.compile(r"^(\S+) D frame (\d+) render (\d+) us owed (\d+) px in (\d+) rects, ui (\d+) draws (\d+) layers \(([^)]*)\)")
L_LINE = re.compile(r"^(\S+) L flip (\S+) \+([\d.]+) ms: frame ([\d.]+) ms \(([a-z ]+)\), late (-?\d+) lost (\d+)( missed)?( anim)?")


def pct(values, q):
    vals = sorted(values)
    return vals[int(q * (len(vals) - 1))] if vals else 0


def window(lines, note=None, seconds=8.0):
    """The lines of the ring from the last `note:` line on, for `seconds`; all of them without a note, [] when the note is not there."""
    if not note:
        return lines
    starts = [i for i, line in enumerate(lines) if "note: " + note in line]
    if not starts:
        return []
    start = starts[-1]
    t0 = float(lines[start].split()[0])
    out = []
    for line in lines[start:]:
        try:
            if float(line.split()[0]) - t0 > seconds:
                break
        except (ValueError, IndexError):
            continue
        out.append(line)
    return out


def summarize(lines):
    """A dict of the frames' numbers, or None for a window with no frame."""
    drawn = [m for m in map(D_LINE.match, lines) if m]
    flips = [m for m in map(L_LINE.match, lines) if m]
    if not drawn and not flips:
        return None
    render = [int(m.group(3)) for m in drawn]
    owed = [int(m.group(4)) for m in drawn]
    layers = [int(m.group(7)) for m in drawn]
    draws = [int(m.group(6)) for m in drawn]
    gpu = [float(m.group(4)) for m in flips]
    gaps = [float(m.group(3)) for m in flips]
    return {
        "drawn": len(drawn), "render_us": [pct(render, 0.5), pct(render, 0.9), max(render, default=0)],
        "owed_px": [pct(owed, 0.5), pct(owed, 0.9), max(owed, default=0)], "rects_max": max((int(m.group(5)) for m in drawn), default=0),
        "ui_draws": [pct(draws, 0.5), max(draws, default=0)], "ui_layers": [pct(layers, 0.5), max(layers, default=0)],
        "flips": len(flips), "frame_ms": [pct(gpu, 0.5), pct(gpu, 0.9), max(gpu, default=0.0)], "gap_ms_max": max(gaps, default=0.0),
        "late": sum(1 for m in flips if int(m.group(6)) > 0), "lost": sum(int(m.group(7)) for m in flips),
        "missed": sum(1 for m in flips if m.group(8)), "over": sum(1 for g in gaps if g > 10.0),
    }


def line(name, s):
    """One line for the gate's output."""
    if s is None:
        return "%s: no frame in the ring" % name
    return ("%s: %d frames drawn, render p50/p90/max %d/%d/%d us, owed p50/p90/max %d/%d/%d px (%d rects at most), ui draws p50/max %d/%d, layers p50/max %d/%d; "
            "%d flips, draw to GPU done p50/p90/max %.1f/%.1f/%.1f ms, late %d lost %d missed %d, %d flips over 10 ms after the one before"
            % (name, s["drawn"], *s["render_us"], *s["owed_px"], s["rects_max"], *s["ui_draws"], *s["ui_layers"], s["flips"], *s["frame_ms"], s["late"], s["lost"],
               s["missed"], s["over"]))


def flips(lines):
    """The flips of a ring in order: (presented time in s, ms since the flip before, a frame with an animation still running after it)."""
    return [(float(m.group(2)), float(m.group(3)), bool(m.group(9))) for m in map(L_LINE.match, lines) if m]


def motion_drops(flips_, start, end, drop_ms):
    """(flips judged, flips dropped) for the window [start, end] (the ring's clock, CLOCK_MONOTONIC seconds): the compositor's own rule
    (`refresh::Measure::motion`), a flip later than `drop_ms` after the flip of a frame with an animation still running after it is a refresh the
    motion missed, and a flip after a frame with none (a still screen, the end of a motion) is no frame missed. None when the ring does not reach
    back to the window's start, so that a window is not judged by what the ring lost."""
    if not flips_ or flips_[0][0] > start + 0.2:
        return None
    judged = dropped = 0
    for before, now in zip(flips_, flips_[1:]):
        if start <= now[0] <= end and before[2]:
            judged += 1
            dropped += now[1] > drop_ms
    return judged, dropped


def burst_drops(flips_, start, drop_ms, gap_ms=50.0):
    """The intervals over `drop_ms` inside the burst a mark at `start` begins, read as analyse.py's `burst` chains it (flips within `gap_ms` of the
    one before, the first from the mark), counting only those after the flip of a frame with an animation still running after it. One after a
    still frame is the screen at rest until the next key's first frame: the compositor flips no frame that changes nothing (`kms::frames`), so
    such an interval is no refresh missed (session 11's home-widgets: 41.7 ms from a motion's end to the next key's frame, 9 ms after the key).
    None when the ring does not reach back to `start`."""
    if not flips_ or flips_[0][0] > start + 0.2:
        return None
    prev, moving, chained, over = start, False, 0, 0
    for t, _, anim in flips_:
        if t < start:
            continue
        gap = (t - prev) * 1000.0
        if gap > gap_ms:
            break
        over += chained > 0 and moving and gap > drop_ms
        prev, moving, chained = t, anim, chained + 1
    return over


def load(path):
    with open(path) as f:
        return json.load(f)["lines"]


def main(argv):
    if len(argv) < 2:
        print(__doc__)
        return 2
    lines = window(load(argv[1]), argv[2] if len(argv) > 2 else None, float(argv[3]) if len(argv) > 3 else 8.0)
    print(line(argv[1], summarize(lines)))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
