#!/usr/bin/env python3
"""Compressed-RAM swap A/B on the set: LG's eMMC swap (/dev/f2io-0) alone against zram0 (lz4) at a higher priority in front of it.

    python3 docs/research/memory/swap_ab.py --side A|B [--zram-mb 384] [--out DIR] [--netflix-runs 2] [--hog-runs 2]

One side per call, same workload both sides, from the repository root with the set awake, LG's compositor live and the
receiver muted (the Netflix part plays nothing, but Netflix's browse page has sound). Side B puts zram0 up (lz4, --zram-mb,
`swapon -p 100`, f2io stays at -2 as overflow) before the workload and takes it down again at the end (`swapoff`, `reset`
to size 0) unless --keep says to leave it. Nothing is written outside /tmp, /sys/block/zram0 and the swap table.

The workload, in this order on both sides (docs/research/perf/run.py drives the first and last; the trace and the 1 s
sampler of capture.sh give the frame gaps and surface-manager's major faults):

1. `apphome` x3: LG Settings launched, HOME, closed. The `home` window's worst gap, dropped frames and the
   compositor's major faults in the 5 s after HOME (perf-tweaks.md, "Root causes of the Home-return gaps").
2. Netflix resume x N: Netflix to the front (it is preloaded), HOME, the browser plus a memory hog of HOG_MB for 6 s
   (the "other app" that makes swap matter), Netflix launched again: time to SAM's foreground and to the first panel
   change (DISPLAY captures at 320x180 every 0.25 s compared to the one taken before the launch), from a script run on
   the set so the clock is the set's.
3. `hog` x N: the browser in front, a hog of HOG_MB for 8 s, HOME: the pressured Home return (swap-out then swap-in).

HOG_MB is MemAvailable in MB plus 150 at the first call (written to DIR/hog_mb), so the hog pushes about 150 MB into
swap on either side; capped at 1100. Each side writes DIR/<side>-*.{trace,sys,json,txt}, <side>-netflix.json,
<side>-swap-{before,after}.txt (swaps, zram counters, vmstat) and <side>-summary.txt.
"""
import argparse
import json
import os
import re
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", "..", ".."))
PERF = os.path.join(REPO, "docs", "research", "perf")
sys.path.insert(0, PERF)
import run as perf  # noqa: E402

ZRAM = "/sys/block/zram0"
HOG_CAP = 1100

NETFLIX_SCRIPT = r'''
import json, os, subprocess, sys, time
other = sys.argv[1]; hog_mb = int(sys.argv[2]); out = sys.argv[3]
def luna(uri, payload="{}", timeout=15):
    try:
        r = subprocess.run(["luna-send", "-n", "1", uri, payload], capture_output=True, text=True, timeout=timeout, stdin=subprocess.DEVNULL)
        return r.stdout
    except subprocess.TimeoutExpired:
        return ""
def fg():
    o = luna("luna://com.webos.applicationManager/getForegroundAppInfo")
    try:
        return json.loads(o).get("appId", "")
    except ValueError:
        return ""
def wait_fg(app, limit=40):
    t0 = time.monotonic()
    while time.monotonic() - t0 < limit:
        if fg() == app:
            return time.monotonic() - t0
        time.sleep(0.2)
    return None
def capture(path):
    luna("luna://com.webos.service.capture/executeOneShot", json.dumps({"path": path, "method": "DISPLAY", "format": "PNG", "width": 320, "height": 180}), 10)
    try:
        return open(path, "rb").read()
    except OSError:
        return b""
def differs(a, b):
    # PNGs of the same size and content are byte-equal; a different picture changes the compressed stream. Size alone is
    # not enough (two different pictures can compress to the same length), so the bytes are compared.
    return a != b
def key(code):
    subprocess.run(["python3", "/tmp/kb.py", code], stdin=subprocess.DEVNULL, timeout=20)
rec = {"steps": []}
def step(name, **kw):
    rec["steps"].append(dict(step=name, at=time.time(), **kw))
luna("luna://com.webos.applicationManager/launch", '{"id":"netflix"}')
step("netflix-front", wait=wait_fg("netflix"))
time.sleep(6)
key("HOME")
step("home", wait=wait_fg("com.webos.app.home"))
time.sleep(3)
luna("luna://com.webos.applicationManager/launch", json.dumps({"id": other}))
step("other-front", wait=wait_fg(other))
time.sleep(2)
hog = subprocess.Popen(["python3", "-c", "import time,sys; n=int(sys.argv[1])<<20; b=bytearray(n); [b.__setitem__(i,1) for i in range(0,n,4096)]; time.sleep(float(sys.argv[2]))", str(hog_mb), "6"], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
hog.wait()
step("hog-done", mb=hog_mb)
time.sleep(1)
before = capture("/tmp/nfx-before.png")
vm0 = open("/proc/vmstat").read()
t0 = time.monotonic()
luna("luna://com.webos.applicationManager/launch", '{"id":"netflix"}')
t_launch = time.monotonic() - t0
t_fg = t_first = None
while time.monotonic() - t0 < 40:
    if t_fg is None and fg() == "netflix":
        t_fg = time.monotonic() - t0
    now = capture("/tmp/nfx-now.png")
    if t_first is None and now and differs(now, before):
        t_first = time.monotonic() - t0
        os.rename("/tmp/nfx-now.png", "/tmp/nfx-first.png")
    if t_fg is not None and t_first is not None:
        break
    time.sleep(0.25)
vm1 = open("/proc/vmstat").read()
def vm(text, k):
    for l in text.splitlines():
        if l.startswith(k + " "):
            return int(l.split()[1])
    return 0
rec.update(launch_call_s=round(t_launch, 3), to_foreground_s=None if t_fg is None else round(t_fg, 2),
           to_first_panel_change_s=None if t_first is None else round(t_first, 2),
           pswpin=vm(vm1, "pswpin") - vm(vm0, "pswpin"), pswpout=vm(vm1, "pswpout") - vm(vm0, "pswpout"),
           pgmajfault=vm(vm1, "pgmajfault") - vm(vm0, "pgmajfault"))
time.sleep(4)
key("HOME")
step("home-again", wait=wait_fg("com.webos.app.home"))
luna("luna://com.webos.applicationManager/closeByAppId", json.dumps({"id": other}))
with open(out, "w") as f:
    json.dump(rec, f)
print(json.dumps(rec))
'''


def sh(s, cmd, timeout=120):
    rc, out, err = s.sh(cmd, timeout=timeout)
    return out + (("\n[stderr]\n" + err) if err.strip() else "")


def swap_state(s):
    return sh(s, "date; cat /proc/swaps; echo; grep -E 'MemFree|MemAvailable|SwapTotal|SwapFree|AnonPages|Cached' /proc/meminfo; echo; "
                 "grep -E '^(pswpin|pswpout|pgmajfault) ' /proc/vmstat; echo; echo 'zram0 disksize comp_algorithm max_comp_streams'; "
                 "cat %s/disksize %s/comp_algorithm %s/max_comp_streams; echo 'zram0 mm_stat (orig_data_size compr_data_size mem_used_total "
                 "mem_limit mem_used_max same_pages pages_compacted huge_pages)'; cat %s/mm_stat; echo 'zram0 stat'; cat %s/stat; "
                 "head -1 /proc/stat" % (ZRAM, ZRAM, ZRAM, ZRAM, ZRAM))


def zram_up(s, mb):
    out = sh(s, "set -e; cd %s; test \"$(cat disksize)\" = 0 || echo reset > reset; echo lz4 > comp_algorithm; "
                "echo %dM > disksize; mkswap /dev/zram0 >/dev/null; swapon -p 100 /dev/zram0; cat /proc/swaps" % (ZRAM, mb))
    if "/dev/zram0" not in out:
        raise RuntimeError("zram0 did not come up: " + out)
    return out


def zram_down(s):
    return sh(s, "swapoff /dev/zram0 2>&1; echo 1 > %s/reset; cat /proc/swaps; cat %s/disksize" % (ZRAM, ZRAM), timeout=300)


def sys_rows(path):
    rows = []
    for line in open(path, errors="replace"):
        f = line.split("|")
        try:
            up = float(f[0])
        except ValueError:
            continue
        d = {"up": up}
        m = re.search(r"majflt=(\d+)", line)
        d["majflt"] = int(m.group(1)) if m else None
        for k in ("pswpin", "pswpout", "pgmajfault"):
            m = re.search(r"%s (\d+)" % k, line)
            d[k] = int(m.group(1)) if m else None
        # per cpu: user+nice+system+irq+softirq (busy) and idle+iowait; CPU3 is hot-plugged by LG and vanishes from
        # /proc/stat while offline, so the shares are taken per cpu over the cpus present in both samples
        d["cpu"] = {}
        for cm in re.finditer(r"(cpu\d) (\d+) (\d+) (\d+) (\d+) (\d+) (\d+) (\d+)", line):
            v = list(map(int, cm.groups()[1:]))
            d["cpu"][cm.group(1)] = (v[0] + v[1] + v[2] + v[5] + v[6], v[3] + v[4])
        rows.append(d)
    return rows


def trace_offset(trace):
    """`start NAME <uptime>` marker: the trace clock minus /proc/uptime at the start."""
    for line in open(trace, errors="replace"):
        m = re.search(r"\s(\d+\.\d+): tracing_mark_write: start \S+ (\d+\.\d+)", line)
        if m:
            return float(m.group(1)) - float(m.group(2))
    return 0.0


def marker_time(trace, text):
    for line in open(trace, errors="replace"):
        m = re.search(r"\s(\d+\.\d+): tracing_mark_write: %s$" % re.escape(text), line)
        if m:
            return float(m.group(1))
    return None


def around(rows, up0, before=1.0, after=5.0):
    """Deltas of the sampler's counters from the sample before up0 - before to the one at or after up0 + after."""
    lo = [r for r in rows if r["up"] <= up0 - before] or rows[:1]
    hi = [r for r in rows if r["up"] >= up0 + after] or rows[-1:]
    a, b = lo[-1], hi[0]
    out = {"span_s": round(b["up"] - a["up"], 1)}
    for k in ("majflt", "pswpin", "pswpout", "pgmajfault"):
        out[k] = None if a[k] is None or b[k] is None else b[k] - a[k]
    busy = idle = 0
    for c in set(a["cpu"]) & set(b["cpu"]):
        busy += b["cpu"][c][0] - a["cpu"][c][0]
        idle += b["cpu"][c][1] - a["cpu"][c][1]
    out["cpu_busy_pct"] = round(100.0 * busy / (busy + idle), 1) if busy + idle else None
    return out


def home_window(name):
    rep = json.load(open("%s/%s.json" % (perf.LOCAL, name)))
    w = next((w for w in rep["windows"] if w["name"] == "home"), None)
    if not w:
        return {}
    gaps = [g["ms"] for g in w.get("gaps", [])]
    return {"frames": w["frames"], "fps": w["fps"], "dropped": w["dropped"], "over33": w["over33"], "worst_ms": w["worst_ms"],
            "gaps_ms": gaps[:8], "burst_end_ms": w.get("burst_end_ms")}


def side_rows(side, names):
    rows = []
    for name in names:
        trace = "%s/%s.trace" % (perf.LOCAL, name)
        sysf = "%s/%s.sys" % (perf.LOCAL, name)
        hw = home_window(name)
        off = trace_offset(trace)
        t_home = marker_time(trace, "P:home")
        rs = sys_rows(sysf)
        counters = around(rs, t_home - off) if t_home is not None else {}
        t_hog = marker_time(trace, "P:hog")
        # the swap-out phase: the hog's 8 s hold plus the kernel's reclaim around it (the compressor's CPU on side B)
        hog = around(rs, t_hog - off, before=0.5, after=11) if t_hog is not None else {}
        rows.append({"run": name, "home": hw, "after_home_5s": counters, "hog_window": hog})
    return rows


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--side", required=True, choices=["A", "B"])
    ap.add_argument("--zram-mb", type=int, default=384)
    ap.add_argument("--out", default=os.path.join(HERE, "swap-ab-" + time.strftime("%Y-%m-%d")))
    ap.add_argument("--netflix-runs", type=int, default=2)
    ap.add_argument("--hog-runs", type=int, default=2)
    ap.add_argument("--apphome-runs", type=int, default=3)
    ap.add_argument("--keep", action="store_true", help="side B: leave zram0 up at the end")
    a = ap.parse_args()
    os.makedirs(a.out, exist_ok=True)
    os.makedirs(perf.LOCAL, exist_ok=True)
    s = perf.Set()
    # The sampler beside capture.sh: one process instead of 40 forks a second.
    for f in ("capture.sh", "sample.py", "kb.py"):
        data = open(os.path.join(PERF, f), "rb").read()
        dest = "/tmp/perf/" + f if f != "kb.py" else "/tmp/kb.py"
        stdin, out, _ = s.cli.exec_command("cat > %s && chmod 755 %s" % (dest, dest))
        stdin.write(data)
        stdin.channel.shutdown_write()
        out.channel.recv_exit_status()
    hog_path = os.path.join(a.out, "hog_mb")
    if os.path.exists(hog_path):
        hog_mb = int(open(hog_path).read())
    else:
        avail = int(re.search(r"MemAvailable:\s+(\d+)", sh(s, "grep MemAvailable /proc/meminfo")).group(1)) // 1024
        hog_mb = min(avail + 150, HOG_CAP)
        open(hog_path, "w").write("%d\n" % hog_mb)
    print("side %s, hog %d MB" % (a.side, hog_mb))
    tag = "swap" + a.side
    summary = ["side %s %s, hog %d MB, zram %s" % (a.side, time.strftime("%Y-%m-%d %H:%M"), hog_mb, "%d MB lz4 p100" % a.zram_mb if a.side == "B" else "off")]
    try:
        if a.side == "B":
            print(zram_up(s, a.zram_mb))
        open(os.path.join(a.out, "%s-swap-before.txt" % a.side), "w").write(swap_state(s))
        perf.WORKLOADS["hog"] = lambda st: perf.wl_hog(st, mb=hog_mb, hold=8)
        names = []
        if a.apphome_runs:
            perf.run(s, tag, ["apphome"], a.apphome_runs)
            names += ["%s-apphome-%d" % (tag, i) for i in range(a.apphome_runs)]
        nfx = []
        stdin, out, _ = s.cli.exec_command("cat > /tmp/nfx_resume.py")
        stdin.write(NETFLIX_SCRIPT.encode())
        stdin.channel.shutdown_write()
        out.channel.recv_exit_status()
        for i in range(a.netflix_runs):
            o = sh(s, "python3 /tmp/nfx_resume.py com.webos.app.browser %d /tmp/nfx-%d.json; md5sum /tmp/nfx-before.png /tmp/nfx-first.png" % (hog_mb, i), timeout=300)
            print(o)
            try:
                nfx.append(json.loads(o.splitlines()[0]))
            except (ValueError, IndexError):
                nfx.append({"error": o[:400]})
            for f in ("before", "first"):
                try:
                    s.get("/tmp/nfx-%s.png" % f, os.path.join(a.out, "%s-netflix-%d-%s.png" % (a.side, i, f)))
                except Exception:
                    pass
            time.sleep(3)
        json.dump(nfx, open(os.path.join(a.out, "%s-netflix.json" % a.side), "w"), indent=1)
        if a.hog_runs:
            perf.run(s, tag, ["hog"], a.hog_runs)
            names += ["%s-hog-%d" % (tag, i) for i in range(a.hog_runs)]
        open(os.path.join(a.out, "%s-swap-after.txt" % a.side), "w").write(swap_state(s))
        rows = side_rows(a.side, names)
        for r in rows:
            summary.append("%-18s home: %s  after HOME 5 s: %s%s" % (r["run"], json.dumps(r["home"]), json.dumps(r["after_home_5s"]),
                                                                  ("  hog window: " + json.dumps(r["hog_window"])) if r["hog_window"] else ""))
        for i, r in enumerate(nfx):
            summary.append("netflix resume %d: foreground %s s, first panel change %s s, launch call %s s, pswpin %s pswpout %s majfault %s" % (
                i, r.get("to_foreground_s"), r.get("to_first_panel_change_s"), r.get("launch_call_s"), r.get("pswpin"), r.get("pswpout"), r.get("pgmajfault")))
        for name in names:
            for ext in ("trace", "sys", "json", "txt"):
                src = "%s/%s.%s" % (perf.LOCAL, name, ext)
                if os.path.exists(src):
                    subprocess.run(["cp", src, os.path.join(a.out, "%s.%s" % (name, ext))])
    finally:
        if a.side == "B" and not a.keep:
            print(zram_down(s))
            summary.append("zram0 taken down: " + sh(s, "cat /proc/swaps | tail -n +2; cat %s/disksize" % ZRAM).replace("\n", " | "))
        with open(os.path.join(a.out, "%s-summary.txt" % a.side), "w") as f:
            f.write("\n".join(summary) + "\n")
        print("\n".join(summary))


if __name__ == "__main__":
    main()
