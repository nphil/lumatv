#!/usr/bin/env python3
"""What Qt's scene graph renderer says it did in each frame, from the offline host.

The batch renderer reports per frame how much of its own state it built again (`QSG_RENDERER_DEBUG=render`: "rebuild: none", `batches`,
`partial`, `renderlists` or `full`) and how many nodes it draws in how many batches. A node that joins the scene or leaves it (an item's
`visible` flipping) makes it build the render list and the batches of the whole scene again (`full`), and on the set that frame cost 1.3-2.2 ms
more than one that only moves a transform (docs/research/shell-host-result.md, "Round 15"). The host's software GL says nothing about time, but
what the renderer rebuilds is its own decision, the same in the host's Qt 5.15 and the set's 5.12 (run with `PYTHONPATH=<a PyQt5 5.12 install>`
to read it from 5.12).

    python3 tests/qmlhost/renderstats.py quick|picker [--cycles 3] [--hold 1000] [--gap 1100] [--start 13000] [--linger MS] [--theme JSON]

opens and closes the panel through the bus on Home, `--cycles` times, `--hold` ms open and `--gap` ms closed, from `--start` ms after the host
started (Home's own wallpaper and news animations are over by 12.8 s and come again every 10 s: 11-12.6 s, 21-22.6 s), and prints one line per
marker: a letter per frame (`.` none, `b` batches, `p` partial, `R` renderlists, `F` full) and the node and batch counts the frames drew.
As a library: `parse(text)` for the host's output, `kinds(frames)` and `scene(frames)`.
"""
import argparse
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
HOST = os.path.join(HERE, "qmlhost.py")

SYMBOLS = {"none": ".", "batches": "b", "partial": "p", "renderlists": "R", "full": "F"}
REBUILD = re.compile(r'Renderer::render\(\).*"rebuild: ([^"]*)"')
MARKER = re.compile(r"\[P:(\w+)\] (.*)")
SHAPE = re.compile(r"-> (Opaque|Alpha): (\d+) nodes in (\d+) batches")


def parse(text):
    """The host's output as a list of segments `{marker, value, frames}`: the frames the renderer drew from one `[P:marker]` line to the next
    (the first segment, `start`, is everything before the first). A frame is `{rebuild, nodes, batches}`."""
    segments = [{"marker": "start", "value": "", "frames": []}]
    frame = None
    for line in text.splitlines():
        m = MARKER.search(line)
        if m:
            segments.append({"marker": m.group(1), "value": m.group(2), "frames": []})
            frame = None
            continue
        m = REBUILD.search(line)
        if m:
            frame = {"rebuild": m.group(1).strip(), "nodes": 0, "batches": 0}
            segments[-1]["frames"].append(frame)
            continue
        m = SHAPE.search(line)
        if m and frame is not None:
            frame["nodes"] += int(m.group(2))
            frame["batches"] += int(m.group(3))
    return segments


def kinds(frames):
    """The frames as letters: `.` none, `b` batches, `p` partial, `R` renderlists, `F` full."""
    return "".join(SYMBOLS.get(f["rebuild"], "?") for f in frames)


def scene(frames):
    """The (nodes, batches) pairs the frames drew, without repeats."""
    return sorted({(f["nodes"], f["batches"]) for f in frames})


def whole(frames):
    """How many frames built the render list of the whole scene again."""
    return sum(1 for f in frames if f["rebuild"] in ("full", "renderlists"))


def steps(kind, cycles, hold, gap, start, linger=None):
    """The host's `--eval` arguments: open and close `kind` through the bus `cycles` times, a `[P:openN]` and a `[P:closeN]` line at each."""
    find = {"quick": "footFocus", "picker": "soundLine"}[kind]
    helpers = ("function find(p, name) { if (p[name] !== undefined && p.children !== undefined && p.visible !== undefined) return p; "
               "for (var i=0; i<p.children.length; i++) { var f=find(p.children[i], name); if (f) return f; } return null; } "
               "function bus() { return Qt.createQmlObject('import QtQuick 2.4\\nimport \"' + root.moduleDirUrl + 'LumaBus.js\" as Bus\\n"
               "QtObject { function open(k) { return Bus.panel(k) } }', root) } ")

    def ev(at, label, body):
        js = "(function(){%s var r=(function(){%s})(); console.info('[P:%s] '+JSON.stringify(r)); return 1;})()" % (helpers, body, label)
        return ["--eval", "%d:%s" % (at, js)]
    opening = "return bus().open('%s');" % kind
    closing = opening if kind == "quick" else "root.closeLayer(); return true;"         # the Input key again goes on to the next input; Back closes
    out = []
    if linger is not None:
        out += ev(start - 500, "linger", "find(root, '%s').lingerMs = %d; return %d;" % (find, linger, linger))
    at = start
    for n in range(cycles):
        out += ev(at, "open%d" % n, opening)
        out += ev(at + hold, "close%d" % n, closing)
        at += hold + gap
    return out, at


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("kind", choices=("quick", "picker"))
    ap.add_argument("--cycles", type=int, default=3)
    ap.add_argument("--hold", type=int, default=1000, help="ms the panel is open")
    ap.add_argument("--gap", type=int, default=1100, help="ms it is closed before it is opened again")
    ap.add_argument("--start", type=int, default=13000)
    ap.add_argument("--linger", type=int, help="ms the closed panel stays in the scene (PanelMotion.lingerMs), default the panel's own")
    ap.add_argument("--theme", default='{"perf":{"syncResidents":true}}')
    a = ap.parse_args(argv)
    sys.path.insert(0, os.path.join(HERE, ".."))
    from xdisplay import DISPLAY
    if not DISPLAY:
        raise SystemExit("no X display: Xvfb is needed")
    evals, end = steps(a.kind, a.cycles, a.hold, a.gap, a.start, a.linger)
    cmd = [sys.executable, HOST, "LumaHome.qml", "--fake-helper", "--prop", "hostActive=true", "--theme", a.theme, *evals, "--wait", str(end + 3000)]
    r = subprocess.run(cmd, env=dict(os.environ, DISPLAY=DISPLAY, QSG_RENDERER_DEBUG="render"), capture_output=True, text=True, timeout=900)
    for seg in parse(r.stdout + r.stderr):
        fr = seg["frames"]
        print("%8s %3d frames  %s   nodes/batches %s" % (seg["marker"], len(fr), kinds(fr), scene(fr)[:4]))
    return 0


if __name__ == "__main__":
    sys.exit(main())
