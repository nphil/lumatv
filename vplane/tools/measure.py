#!/usr/bin/env python3
"""On the set: what a screensaver costs. For the TV steps of docs/research/videoplane-saver-result.md, python3 and nothing else (no forks).

    measure.py run LABEL SECONDS [--every S] [--warm S] [--proc NAME]... [--out FILE]
    measure.py table FILE...

`run` samples the counters every `--every` seconds (default 1) for SECONDS, leaves the first `--warm` seconds out (default 3: the start, the first
frames), prints one line of figures and writes the samples and the figures to FILE (default /tmp/vplane-measure/LABEL.json). `table` puts the
figures of several runs side by side: the as-found Screen Saver, LumaTV's saver on the GPU, the video saver.

The counters are the ones the census and the compositor's measurements read (docs/research/hardware-census.md, "Live counters"):
  Mali       /sys/kernel/debug/mali0/dvfs_utilization `busy_time` and `idle_time`, cumulative; the share is the change in busy over the change in both
  flips      /proc/lg/fbdev/fb0/flip_cnt, the OSD plane's page flips; per second
  vsync      /proc/interrupts DE2VSYNC and osd_irq, per second (the display engine's rate: 120 with Home up, 0 in the as-found Screen Saver)
  CPU        /proc/stat, the share of the online cores' time that was not idle
  memory     /proc/meminfo CmaFree and MemAvailable, the lowest of the run
  processes  `--proc NAME`: every process whose command name or command line contains NAME, summed: share of one core, largest VmRSS
MEASURE_ROOT puts every path under a directory (the tests).
"""
import argparse
import json
import os
import re
import sys
import time

ROOT = os.environ.get("MEASURE_ROOT", "")
CLK_TCK = 100.0
MALI = "/sys/kernel/debug/mali0/dvfs_utilization"
FLIPS = "/proc/lg/fbdev/fb0/flip_cnt"
TEMP = "/proc/lg/pm/temperature"


def read(path):
    try:
        with open(ROOT + path, errors="replace") as f:
            return f.read()
    except OSError:
        return ""


def parse_dvfs(text):
    """(busy, idle) from the Mali's utilisation file, or None."""
    m = {k: int(v) for k, v in re.findall(r"(busy_time|idle_time):\s*(\d+)", text)}
    return (m["busy_time"], m["idle_time"]) if len(m) == 2 else None


def parse_stat(text):
    """(non-idle, total) jiffies of all cores from the first line of /proc/stat, or None."""
    for line in text.splitlines():
        f = line.split()
        if f and f[0] == "cpu" and len(f) >= 8:
            v = [int(x) for x in f[1:9]]
            idle = v[3] + v[4]
            return sum(v) - idle, sum(v)
    return None


def parse_interrupts(text, names):
    """{name: count over all CPUs} for the lines whose last word contains the name."""
    out = {}
    for line in text.splitlines():
        f = line.split()
        if len(f) < 3 or not f[0].endswith(":"):
            continue
        counts = []                                   # the per-CPU counts: the run of numbers after the label, up to the controller's name
        for x in f[1:]:
            if not x.isdigit():
                break
            counts.append(int(x))
        for n in names:
            if n in f[-1]:
                out[n] = out.get(n, 0) + sum(counts)
    return out


def parse_meminfo(text):
    return {k: int(v) for k, v in re.findall(r"^(CmaFree|MemAvailable):\s*(\d+) kB", text, re.M)}


def parse_pid_stat(text):
    """utime + stime in ticks; the command name may hold spaces and brackets, so the fields are counted from the last `)`."""
    i = text.rfind(")")
    f = text[i + 2:].split() if i >= 0 else []
    return int(f[11]) + int(f[12]) if len(f) > 12 else None


def parse_rss_kb(text):
    m = re.search(r"^VmRSS:\s*(\d+) kB", text, re.M)
    return int(m.group(1)) if m else 0


def parse_temp(text):
    m = re.search(r"-?\d+(\.\d+)?", text)
    return float(m.group(0)) if m else None


def find_pids(names):
    """{name: [pid]} of the processes whose comm or command line contains the name."""
    found = {n: [] for n in names}
    try:
        entries = [e for e in os.listdir(ROOT + "/proc") if e.isdigit()]
    except OSError:
        return found
    for e in entries:
        comm = read("/proc/%s/comm" % e).strip()
        cmd = read("/proc/%s/cmdline" % e).replace("\0", " ")
        for n in names:
            if n in comm or n in cmd:
                found[n].append(int(e))
    return found


def sample(names, pids):
    s = {"t": time.monotonic()}
    d = parse_dvfs(read(MALI))
    if d:
        s["busy"], s["idle"] = d
    st = parse_stat(read("/proc/stat"))
    if st:
        s["cpu_busy"], s["cpu_total"] = st
    try:
        s["flips"] = int(read(FLIPS).strip())
    except ValueError:
        pass
    irq = parse_interrupts(read("/proc/interrupts"), ("DE2VSYNC", "osd_irq"))
    s["vsync"], s["osd"] = irq.get("DE2VSYNC"), irq.get("osd_irq")
    s.update({k: v for k, v in parse_meminfo(read("/proc/meminfo")).items()})
    s["temp"] = parse_temp(read(TEMP))
    procs = {}
    for n in names:
        ticks, rss = 0, 0
        for pid in pids.get(n, []):
            t = parse_pid_stat(read("/proc/%d/stat" % pid))
            if t is not None:
                ticks += t
                rss = max(rss, parse_rss_kb(read("/proc/%d/status" % pid)))
        procs[n] = {"ticks": ticks, "rss_kb": rss}
    s["procs"] = procs
    return s


def mali_share(a, b):
    busy, idle = b["busy"] - a["busy"], b["idle"] - a["idle"]
    return 100.0 * busy / (busy + idle) if busy + idle > 0 else 0.0


def summarize(samples, warm=3.0):
    """The figures of a run from its samples, leaving the first `warm` seconds out."""
    if len(samples) < 2:
        return {"error": "fewer than two samples"}
    rows = [s for s in samples if s["t"] - samples[0]["t"] >= warm]
    if len(rows) < 2:
        rows = samples
    a, b = rows[0], rows[-1]
    secs = b["t"] - a["t"]
    out = {"seconds": round(secs, 2), "samples": len(rows)}
    if "busy" in a and "busy" in b:
        out["mali_pct"] = round(mali_share(a, b), 1)
    if "flips" in a and "flips" in b and secs > 0:
        out["flips_per_s"] = round((b["flips"] - a["flips"]) / secs, 1)
    for key, name in (("vsync", "vsync_per_s"), ("osd", "osd_irq_per_s")):
        if a.get(key) is not None and b.get(key) is not None and secs > 0:
            out[name] = round((b[key] - a[key]) / secs, 1)
    if "cpu_total" in a and "cpu_total" in b and b["cpu_total"] > a["cpu_total"]:
        out["cpu_pct"] = round(100.0 * (b["cpu_busy"] - a["cpu_busy"]) / (b["cpu_total"] - a["cpu_total"]), 1)
    for key, name in (("CmaFree", "cma_free_min_mb"), ("MemAvailable", "mem_avail_min_mb")):
        v = [s[key] for s in rows if key in s]
        if v:
            out[name] = round(min(v) / 1024.0, 1)
    t = [s["temp"] for s in rows if s.get("temp") is not None]
    if t:
        out["temp_max"] = max(t)
    procs = {}
    for n in a.get("procs", {}):
        ticks = b["procs"][n]["ticks"] - a["procs"][n]["ticks"]
        procs[n] = {"cpu_pct": round(100.0 * ticks / CLK_TCK / secs, 1) if secs > 0 else 0.0,
                    "rss_mb_max": round(max(s["procs"][n]["rss_kb"] for s in rows) / 1024.0, 1)}
    if procs:
        out["procs"] = procs
    return out


def cmd_run(a):
    pids = find_pids(a.proc)
    samples = []
    end = time.monotonic() + a.seconds
    last_scan = time.monotonic()
    while True:
        samples.append(sample(a.proc, pids))
        if time.monotonic() >= end:
            break
        time.sleep(a.every)
        if a.proc and time.monotonic() - last_scan > 5:
            pids, last_scan = find_pids(a.proc), time.monotonic()
    figures = summarize(samples, a.warm)
    out = a.out or "/tmp/vplane-measure/%s.json" % re.sub(r"[^\w.-]+", "_", a.label)
    os.makedirs(os.path.dirname(out) or ".", exist_ok=True)
    with open(out, "w") as f:
        json.dump({"label": a.label, "figures": figures, "warm": a.warm, "samples": samples}, f)
    print(json.dumps({"label": a.label, **figures}))
    print("wrote %s" % out, file=sys.stderr)
    return 0


COLUMNS = (("seconds", "s", 7), ("mali_pct", "Mali %", 7), ("flips_per_s", "flips/s", 8), ("vsync_per_s", "vsync/s", 8), ("cpu_pct", "CPU %", 6),
           ("cma_free_min_mb", "CMA min", 8), ("mem_avail_min_mb", "Avail min", 10), ("temp_max", "C max", 6))


def table(docs):
    width = max([len(d["label"]) for d in docs] + [5])
    lines = ["%-*s " % (width, "label") + "".join("%*s" % (w, h) for _, h, w in COLUMNS) + "  processes (CPU % of a core, largest RSS MB)"]
    for d in docs:
        f = d["figures"]
        cells = "".join("%*s" % (w, f.get(k, "-")) for k, _, w in COLUMNS)
        procs = "  ".join("%s %s%%/%s" % (n, p["cpu_pct"], p["rss_mb_max"]) for n, p in sorted(f.get("procs", {}).items()))
        lines.append("%-*s " % (width, d["label"]) + cells + "  " + procs)
    return "\n".join(lines)


def cmd_table(a):
    docs = []
    for path in a.files:
        with open(path) as f:
            docs.append(json.load(f))
    print(table(docs))
    return 0


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run")
    r.add_argument("label")
    r.add_argument("seconds", type=float)
    r.add_argument("--every", type=float, default=1.0)
    r.add_argument("--warm", type=float, default=3.0)
    r.add_argument("--proc", action="append", default=[])
    r.add_argument("--out")
    t = sub.add_parser("table")
    t.add_argument("files", nargs="+")
    a = ap.parse_args(argv)
    return cmd_run(a) if a.cmd == "run" else cmd_table(a)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
