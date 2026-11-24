#!/usr/bin/env python3
"""Reads the CSV of blank_probe.c and says what the kernel showed of a rate change: the panel's rate over time, the longest stop of the vsync, the
mute and black-out episodes, and how long the rate took to come back to the 120 Hz band after the mark (SIGUSR1) or the run's first change.

    python3 analyse_blank.py capture.csv [--json]

What this cannot show: a physical blink of the panel. The OSD's vsync, the video path's mute and the panel link's black-out are what the
driver reports; a panel that resyncs its own timing without any of them changing is not seen. A gap in the vsync is a scan that stopped, but its
length is only known to the sampler's own resolution (`resolution_ms` in the output).
"""
import json
import statistics
import sys

BAND = (115.0, 125.0)
STAY_S = 1.0
GAP_FRAMES = 1.5


def load(text):
    edges, states, marks = [], [], []
    for line in text.splitlines():
        if not line or line.startswith("#") or line.startswith("kind,"):
            continue
        f = line.split(",")
        if f[0] == "E":
            edges.append((int(f[1]), int(f[2]), int(f[3])))
        elif f[0] == "S":
            states.append((int(f[1]), f[2], f[3], f[4], f[5]))
        elif f[0] == "K":
            marks.append(int(f[1]))
    return edges, states, marks


def rates(edges, step_s=0.1, window_s=0.5):
    """(t_s, hz) every step_s over a window of window_s, from the cumulative increments at each edge's midpoint."""
    if not edges:
        return []
    t0 = edges[0][0]
    pts, total = [], 0
    for lo, hi, inc in edges:
        total += inc
        pts.append(((lo + hi) / 2 / 1e9, total))
    out, i, j = [], 0, 0
    t = pts[0][0] + window_s / 2
    while t + window_s / 2 <= pts[-1][0]:
        a = next((c for ts, c in reversed(pts) if ts <= t - window_s / 2), pts[0][1])
        b = next((c for ts, c in reversed(pts) if ts <= t + window_s / 2), pts[-1][1])
        out.append((t - t0 / 1e9, (b - a) / window_s))
        t += step_s
    return out


def gaps(edges):
    """The intervals between successive single-increment edges, with where they began, in ms; an edge that merged several increments ends a run."""
    out = []
    prev = None
    for lo, hi, inc in edges:
        mid = (lo + hi) / 2
        if inc != 1:
            prev = None
            continue
        if prev is not None:
            out.append((prev / 1e9, (mid - prev) / 1e6))
        prev = mid
    return out


def episodes(states, column, ok):
    """(start_s, end_s) while `column` of the S rows is not `ok`; an episode still open at the last row ends there."""
    out, begin = [], None
    for row in states:
        bad = not ok(row[column])
        if bad and begin is None:
            begin = row[0]
        elif not bad and begin is not None:
            out.append((begin / 1e9, row[0] / 1e9))
            begin = None
    if begin is not None:
        out.append((begin / 1e9, states[-1][0] / 1e9))
    return out


def analyse(text):
    edges, states, marks = load(text)
    if len(edges) < 10:
        return {"error": "fewer than 10 vsync edges"}
    start = edges[0][0] / 1e9
    series = rates(edges)
    gap_list = gaps(edges)
    median_ms = statistics.median(g for _, g in gap_list)
    limit_ms = max(GAP_FRAMES * max(g for _, g in gap_list[:50] or gap_list), 20.0)
    stops = [(round(t - start, 3), round(g, 2)) for t, g in gap_list if g > limit_ms]
    mark = marks[0] / 1e9 if marks else None
    t_ref = mark if mark is not None else start
    back = None
    for k, (t, hz) in enumerate(series):
        if t + start < t_ref:
            continue
        if BAND[0] <= hz <= BAND[1] and all(BAND[0] <= h <= BAND[1] for tt, h in series[k:] if tt <= t + STAY_S):
            back = t + start - t_ref
            break
    res = [e[2] for e in edges]
    return {
        "edges": len(edges),
        "merged_reads": sum(1 for r in res if r > 1),
        "resolution_ms": round(statistics.median((hi - lo) / 1e6 for lo, hi, _ in edges), 3),
        "start_hz": round(statistics.median(h for t, h in series if t < 1.0), 1) if series else None,
        "end_hz": round(statistics.median(h for t, h in series[-10:]), 1) if series else None,
        "median_interval_ms": round(median_ms, 3),
        "longest_interval_ms": round(max(g for _, g in gap_list), 2),
        "intervals_over_limit": stops,
        "limit_ms": round(limit_ms, 1),
        "rate_series_hz": [(round(t, 2), round(h, 1)) for t, h in series[::5]],
        "marks_s": [round(m / 1e9 - start, 3) for m in marks],
        "seconds_to_band_after_ref": None if back is None else round(back, 2),
        "ref": "mark" if mark is not None else "first edge",
        "mute_episodes_s": [(round(a - start, 3), round(b - start, 3)) for a, b in episodes(states, 1, lambda v: v in ("0", "-", "?"))],
        "black_out_episodes_s": [(round(a - start, 3), round(b - start, 3)) for a, b in episodes(states, 3, lambda v: v.startswith("orignal") or v in ("-", "?"))],
        "state_changes": [(round(t / 1e9 - start, 3), lm, vf, bo, src) for t, lm, vf, bo, src in states],
    }


def main(argv):
    if len(argv) < 2:
        sys.exit(__doc__)
    report = analyse(open(argv[1]).read())
    if "--json" in argv:
        print(json.dumps(report, indent=1))
        return
    for key, value in report.items():
        if key in ("rate_series_hz", "state_changes"):
            print(f"{key}:")
            for row in value:
                print("   ", row)
        else:
            print(f"{key}: {value}")


if __name__ == "__main__":
    main(sys.argv)
