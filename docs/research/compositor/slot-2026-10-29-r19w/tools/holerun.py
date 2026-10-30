#!/usr/bin/env python3
"""holerun.py VARIANT [HOLD] [--shots T1,T2,...]: one hole variant on the set (holetest-r19w.sh, dead-man armed there) with LG's DISPLAY capture
(video + UI blended, 1280x720) taken at the given seconds after the start, the log and the three regdumps fetched to the slot's hole/ directory.
Prints the log and the luma of the hole's rectangle and of a control rectangle in each capture."""
import os, sys, time, threading, subprocess
sys.path.insert(0, "/tmp/lumatv-r19w/docs/research/arch")
import tv
from PIL import Image, ImageChops, ImageStat

SLOT = "/tmp/lumatv-r19w/docs/research/compositor/slot-2026-10-29-r19w"
OUTD = "/tmp/lumatv/out/r19w-hole"
v = sys.argv[1]
hold = int(sys.argv[2]) if len(sys.argv) > 2 and sys.argv[2].isdigit() else 10
shots = [3.0, 6.5]
for i, a in enumerate(sys.argv):
    if a == "--shots":
        shots = [float(x) for x in sys.argv[i + 1].split(",")]
tag = os.environ.get("TAG", v)
os.makedirs(SLOT + "/img", exist_ok=True)
hd = SLOT + "/hole/" + tag
os.makedirs(hd, exist_ok=True)

def display_capture(path_local, remote):
    tv.tv("rm -f %s" % remote)
    out = tv.tv("luna-send -n 1 luna://com.webos.service.capture/executeOneShot '{\"path\":\"%s\",\"method\":\"DISPLAY\",\"format\":\"PNG\",\"width\":1280,\"height\":720}'" % remote, 30)
    ok = '"returnValue":true' in out.replace(" ", "")
    t = time.time()
    tv.get(remote, path_local)
    return ok, t

results = {}
def shot(name, at, t0):
    time.sleep(max(0.0, at - (time.time() - t0)))
    local = "/tmp/r19w-hole-%s-%s.png" % (tag, name)
    ts = time.time() - t0
    ok, _ = display_capture(local, "/tmp/r19w-hole-%s.png" % name)
    results[name] = (local, ts, ok)

tv.tv("rm -f %s/hole-%s*.txt %s/hole-%s.log %s/run-%s.out; mkdir -p %s; echo ok" % (OUTD, v, OUTD, v, OUTD, v, OUTD))
ok0, _ = display_capture("/tmp/r19w-hole-%s-pre.png" % tag, "/tmp/r19w-hole-pre.png")
results["pre"] = ("/tmp/r19w-hole-%s-pre.png" % tag, -1.0, ok0)
t0 = time.time()
print(tv.tv("OUT=%s %s setsid nohup sh /tmp/lumatv/holetest-r19w.sh %s %d > %s/run-%s.out 2>&1 < /dev/null & echo started %s" % (OUTD, os.environ.get("HOLEENV", ""), v, hold, OUTD, v, time.strftime("%T"))).strip())
threads = []
for i, at in enumerate(shots):
    th = threading.Thread(target=shot, args=("s%d" % (i + 1), at, t0))
    th.start(); threads.append(th)
for th in threads:
    th.join()
# wait for the run to finish
for _ in range(60):
    log = tv.tv("cat %s/hole-%s.log 2>/dev/null" % (OUTD, v))
    if ("variant %s done" % v) in log:
        break
    time.sleep(1.5)
print(log)
ok9, _ = display_capture("/tmp/r19w-hole-%s-post.png" % tag, "/tmp/r19w-hole-post.png")
results["post"] = ("/tmp/r19w-hole-%s-post.png" % tag, time.time() - t0, ok9)
for n in ("before", "mid", "after"):
    try:
        tv.get("%s/hole-%s-%s.txt" % (OUTD, v, n), "%s/regdump-%s.txt" % (hd, n))
    except Exception as e:
        print("no dump", n, e)
open(hd + "/hole-%s.log" % v, "w").write(log)

# the luma of the hole's rectangle (x 480..800, y 270..450 of 1280x720) and of a control rectangle, per capture
def stat(im, box):
    return ImageStat.Stat(im.convert("L").crop(box)).mean[0]
HOLE = (480, 270, 800, 450)
CTRL = (60, 560, 300, 680)
for name in sorted(results):
    local, ts, ok = results[name]
    if not os.path.exists(local):
        print(name, "no file"); continue
    im = Image.open(local).convert("RGB")
    dest = "%s/img/r19w-hole-%s-%s.jpg" % (SLOT, tag, name)
    im.save(dest, "JPEG", quality=85)
    print("%s at +%.1fs (%s): hole luma %.1f control luma %.1f  %s" % (name, ts, "ok" if ok else "luna refused", stat(im, HOLE), stat(im, CTRL), dest))
    os.remove(local)
