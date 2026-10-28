#!/usr/bin/env python3
"""What a job does to the UI's page-flip cadence on the set, read only.

    gpu-ui-impact.py [--before S] [--after S] [--period-ms N] [--flips FILE] [--mali FILE] -- COMMAND [ARGS...]

Samples the OSD plane's flip counter (/proc/lg/fbdev/fb0/flip_cnt) and the Mali's busy and idle counters
(/sys/kernel/debug/mali0/dvfs_utilization) for S seconds, runs COMMAND (output kept in the report's own file, see --log) while sampling,
then samples S more seconds. For each of the three windows: flips per second, the gaps between flips (p50, p90, max), how many gaps
were longer than 1.5 times the "before" median, and the Mali busy share. The last line, UIIMPACT, repeats the numbers as key=value.

The gap resolution is the sampling period (10 ms by default), so it says whether a long job delays the UI's frames, not by how many
microseconds. Home idles at about 12 flips a second; an animation at 60. Nothing is written but the --log file.
"""
import argparse
import subprocess
import sys
import time

FLIPS = "/proc/lg/fbdev/fb0/flip_cnt"
MALI = "/sys/kernel/debug/mali0/dvfs_utilization"


def read_flips(path):
    try:
        with open(path) as f:
            return int(f.read().split()[0])
    except (OSError, ValueError, IndexError):
        return None


def read_mali(path):
    try:
        with open(path) as f:
            text = f.read()
    except OSError:
        return None
    busy = idle = None
    for line in text.splitlines():
        if line.startswith("busy_time:"):
            busy = int(line.split(":")[1])
        elif line.startswith("idle_time:"):
            idle = int(line.split(":")[1])
    return (busy, idle) if busy is not None and idle is not None else None


def sample_until(done, period, flips_path, mali_path):
    """Samples (time, flips, busy, idle) every |period| seconds until done() is true."""
    rows = []
    while True:
        now = time.monotonic()
        m = read_mali(mali_path)
        rows.append((now, read_flips(flips_path), m[0] if m else None, m[1] if m else None))
        if done(now):
            return rows
        time.sleep(period)


def percentile(values, p):
    if not values:
        return None
    s = sorted(values)
    return s[min(len(s) - 1, int(round(p * (len(s) - 1))))]


def summarise(rows):
    out = {"seconds": rows[-1][0] - rows[0][0] if len(rows) > 1 else 0.0, "fps": None, "gaps": [], "mali": None}
    flips = [r for r in rows if r[1] is not None]
    if len(flips) > 1 and out["seconds"] > 0:
        out["fps"] = (flips[-1][1] - flips[0][1]) / (flips[-1][0] - flips[0][0])
        last_t, last_n = flips[0][0], flips[0][1]
        for t, n, _, _ in flips[1:]:
            if n != last_n:
                # k flips since the last change: spread the time over them
                k = n - last_n
                out["gaps"].extend([(t - last_t) * 1000.0 / k] * k)
                last_t, last_n = t, n
    mali = [r for r in rows if r[2] is not None]
    if len(mali) > 1:
        busy = mali[-1][2] - mali[0][2]
        idle = mali[-1][3] - mali[0][3]
        if busy + idle > 0:
            out["mali"] = 100.0 * busy / (busy + idle)
    return out


def fmt(v, spec="%.1f"):
    return "n/a" if v is None else spec % v


def describe(name, s, late_limit):
    gaps = s["gaps"]
    late = sum(1 for g in gaps if late_limit is not None and g > late_limit)
    line = "%-7s %5.1f s  flips %s/s  gaps p50 %s p90 %s max %s ms" % (
        name, s["seconds"], fmt(s["fps"]), fmt(percentile(gaps, 0.5)), fmt(percentile(gaps, 0.9)), fmt(max(gaps) if gaps else None))
    if late_limit is not None:
        line += "  late %d of %d" % (late, len(gaps))
    line += "  Mali busy %s %%" % fmt(s["mali"])
    return line, late


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--before", type=float, default=6.0)
    ap.add_argument("--after", type=float, default=4.0)
    ap.add_argument("--period-ms", type=float, default=10.0)
    ap.add_argument("--flips", default=FLIPS)
    ap.add_argument("--mali", default=MALI)
    ap.add_argument("--log", default=None, help="file for the command's output (default: discarded)")
    ap.add_argument("command", nargs=argparse.REMAINDER)
    args = ap.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if not command:
        ap.error("a command to run is needed after --")
    period = args.period_ms / 1000.0

    t_end = time.monotonic() + args.before
    before = summarise(sample_until(lambda now: now >= t_end, period, args.flips, args.mali))

    log = open(args.log, "w") if args.log else subprocess.DEVNULL
    proc = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT)
    during = summarise(sample_until(lambda now: proc.poll() is not None, period, args.flips, args.mali))
    code = proc.wait()
    if args.log:
        log.close()

    t_end = time.monotonic() + args.after
    after = summarise(sample_until(lambda now: now >= t_end, period, args.flips, args.mali))

    base = percentile(before["gaps"], 0.5)
    limit = 1.5 * base if base else None
    results = {}
    for name, s in (("before", before), ("during", during), ("after", after)):
        text, late = describe(name, s, limit)
        print(text)
        results[name] = (s, late)
    print("command exit %d" % code)
    keys = []
    for name, (s, late) in results.items():
        keys += ["%s_fps=%s" % (name, fmt(s["fps"])), "%s_gap_p90_ms=%s" % (name, fmt(percentile(s["gaps"], 0.9))),
                 "%s_gap_max_ms=%s" % (name, fmt(max(s["gaps"]) if s["gaps"] else None)), "%s_late=%d" % (name, late),
                 "%s_mali_pct=%s" % (name, fmt(s["mali"]))]
    print("UIIMPACT " + " ".join(keys) + " exit=%d" % code)
    return 0 if code == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
