#!/usr/bin/env python3
"""The flips of a gate trace one by one, with what ran between them: the compositor's loop thread (its render bursts), the shell's process (its bursts)
and the GPU's fragment jobs started.

    python3 frametl.py TRACE MARKER FROM TO [--ui PID]

TRACE is a /tmp/perf/<name>.trace dump (full capture); MARKER a text of one of its markers (`settings-scroll-open`: the last that contains it); FROM and TO
seconds after it. The compositor is the `loop N` marker's pid; the shell is the process of the same loader (`ld-linux-aarch6`) that used most CPU, unless
--ui names it. Each line: the flip's time after the marker, the interval since the flip before, `comp [start..end ms]` bursts counted from that flip, `ui`
the same. A frame that took two refreshes shows two render bursts or one that crossed a refresh; a gap with none is nothing asked for."""
import collections
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import frametl_parse as parse  # noqa: E402


def bursts(trace, pid, a, b):
    """The runs of `pid` between a and b, adjacent ones merged: (start, end)."""
    out = []
    for runs in trace.runs.values():
        out += [(t0, t1) for t0, t1, p, _ in parse.overlap(runs, a, b) if p == pid]
    out.sort()
    merged = []
    for s in out:
        if merged and s[0] - merged[-1][1] < 0.0002:
            merged[-1] = (merged[-1][0], s[1])
        else:
            merged.append(s)
    return merged


def main(argv):
    if len(argv) < 5:
        print(__doc__)
        return 2
    trace = parse.Trace(argv[1])
    marks = [t for t, text in trace.marks if argv[2] in text]
    if not marks:
        print("no marker %r in %s: %s" % (argv[2], argv[1], [text for _, text in trace.marks][:12]))
        return 1
    zero = marks[-1]
    lo, hi = float(argv[3]), float(argv[4])
    comp = next((int(text.split()[1]) for _, text in trace.marks if text.startswith("loop ")), None)
    ui = int(argv[argv.index("--ui") + 1]) if "--ui" in argv else None
    if ui is None:
        cpu = collections.Counter()
        for runs in trace.runs.values():
            for t0, t1, p, comm in parse.overlap(runs, zero + lo, zero + hi):
                if comm.startswith("ld-linux") and p != comp:
                    cpu[p] += t1 - t0
        ui = cpu.most_common(1)[0][0] if cpu else None
    flips = [f for f in trace.flips if zero + lo <= f <= zero + hi]
    for before, flip in zip(flips, flips[1:]):
        cell = lambda pid: " ".join("[%+.1f..%+.1f %.1fms]" % ((s - before) * 1000, (e - before) * 1000, (e - s) * 1000) for s, e in bursts(trace, pid, before, flip))
        print("flip %+.4f %5.1f ms | comp %s | ui %s" % (flip - zero, (flip - before) * 1000, cell(comp), cell(ui)))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
