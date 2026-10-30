#!/bin/sh
# What LG's capture service gives for the input pictures (docs/research/input-preview-result.md), asked of it directly as the helper asks, and
# the helper's own routes. Read-only: it writes pictures to /tmp/pv/ and changes nothing else. Run it on the set, as root, with the picture to
# test playing full screen on an HDMI input (or Live TV) and LumaTV's own sheets away. `sh tv-previews.sh help` lists the steps.
#
#   tv-previews.sh capture [NAME]   VIDEO and DISPLAY, as PNG, JPG, BMP and RGB, at 480x270: the reply, the time, the file's size, and for a
#                                   PNG what pngstat.py reads of it (black=yes is a picture with nothing in it). NAME tags the files.
#   tv-previews.sh sizes            VIDEO PNG at 320x180, 480x270, 640x360 and 960x540: time and bytes of each (3 runs).
#   tv-previews.sh cost [N] [MS]    N captures (default 40) of VIDEO PNG 480x270, MS apart (default 500): the call's time (median, p90, max)
#                                   and the capture service's CPU over the run, as a share of one core and per capture.
#   tv-previews.sh helper           the helper's routes (needs a helper with previews.rs): three captures of the input in front (open,
#                                   live, periodic), the index with the stats the helper keeps, the picture saved to
#                                   /tmp/pv/helper-<key>.jpg and what is on the flash. The owner's switch (Settings) is not touched.
#   tv-previews.sh all [NAME]       capture, sizes, cost, helper.
#
# `PNGSTAT` is where pngstat.py is on the set (`/tmp/lumatv/pngstat.py` once the gate has put it there; `compositor/tools/tv-pngstat.py`).
PNGSTAT=${PNGSTAT:-/tmp/lumatv/pngstat.py}
OUT=/tmp/pv
API=${API:-http://127.0.0.1:18731}
mkdir -p "$OUT"

say() { printf '%s\n' "$*"; }

# One luna-send call: prints "ms=<wall> reply=<json on one line>". Args: method format width height path
shoot() {
    python3 - "$@" <<'PY'
import json, subprocess, sys, time
method, fmt, w, h, path = sys.argv[1:6]
payload = json.dumps({"path": path, "method": method, "format": fmt, "width": int(w), "height": int(h)})
t0 = time.perf_counter()
r = subprocess.run(["luna-send", "-n", "1", "-w", "5000", "luna://com.webos.service.capture/executeOneShot", payload], capture_output=True, text=True, timeout=15, stdin=subprocess.DEVNULL)
ms = (time.perf_counter() - t0) * 1000
print("ms=%.0f reply=%s" % (ms, " ".join(r.stdout.split())[:300] or "(nothing) " + r.stderr.strip()[:200]))
PY
}

size_of() { [ -f "$1" ] && wc -c < "$1" | tr -d ' ' || echo 0; }

capture() {
    tag=${1:-now}
    for m in VIDEO DISPLAY; do
        for f in PNG JPG BMP RGB; do
            ext=$(echo "$f" | tr 'A-Z' 'a-z')
            p="$OUT/$tag-$m.$ext"
            rm -f "$p"
            line=$(shoot "$m" "$f" 480 270 "$p")
            say "capture $m $f: $line file=$(size_of "$p") bytes"
            if [ "$f" = PNG ] && [ -f "$p" ] && [ -f "$PNGSTAT" ]; then say "  pngstat $(python3 "$PNGSTAT" "$p")"; fi
        done
    done
    say "pictures are in $OUT (scp them off to look)"
}

sizes() {
    for wh in "320 180" "480 270" "640 360" "960 540"; do
        set -- $wh
        for n in 1 2 3; do
            p="$OUT/size-$1.png"
            rm -f "$p"
            say "size $1x$2 run $n: $(shoot VIDEO PNG "$1" "$2" "$p") file=$(size_of "$p") bytes"
        done
    done
}

cost() {
    n=${1:-40}
    gap=${2:-500}
    python3 - "$n" "$gap" <<'PY'
import json, os, subprocess, sys, time
n, gap = int(sys.argv[1]), int(sys.argv[2])
hz = os.sysconf("SC_CLK_TCK")
def pid_of(name):
    for p in os.listdir("/proc"):
        if p.isdigit():
            try:
                if open("/proc/%s/comm" % p).read().strip() == name:
                    return p
            except OSError:
                pass
def ticks(pid):
    if not pid:
        return 0
    f = open("/proc/%s/stat" % pid).read().rsplit(")", 1)[1].split()
    return int(f[11]) + int(f[12])
svc = pid_of("captureservice")
payload = json.dumps({"path": "/tmp/pv/cost.png", "method": "VIDEO", "format": "PNG", "width": 480, "height": 270})
c0, t0 = ticks(svc), time.time()
times, bad, child = [], 0, 0.0
for i in range(n):
    s = time.perf_counter()
    r = subprocess.run(["luna-send", "-n", "1", "-w", "5000", "luna://com.webos.service.capture/executeOneShot", payload], capture_output=True, text=True, stdin=subprocess.DEVNULL)
    times.append((time.perf_counter() - s) * 1000)
    if '"returnValue":true' not in r.stdout.replace(" ", ""):
        bad += 1
    time.sleep(max(0.0, gap / 1000.0 - (time.perf_counter() - s)))
wall = time.time() - t0
c1 = ticks(svc)
times.sort()
cpu_s = (c1 - c0) / hz
print("cost: %d captures, %d without returnValue true, %.1f s wall" % (n, bad, wall))
print("cost: call ms median %.0f p90 %.0f max %.0f" % (times[len(times) // 2], times[int(len(times) * 0.9) - 1], times[-1]))
print("cost: captureservice (pid %s) %.2f s of CPU over the run = %.1f %% of one core, %.0f ms a capture" % (svc, cpu_s, 100 * cpu_s / wall, 1000 * cpu_s / n))
PY
}

helper() {
    for why in open live periodic; do
        say "helper capture $why: $(curl -s -m 8 -X POST -H 'X-Luma-Client: probe' -H 'Content-Type: application/json' -d "{\"key\":\"$KEY\",\"why\":\"$why\"}" "$API/v1/previews/capture")"
        sleep 1
    done
    say "helper index: $(curl -s -m 5 -H 'X-Luma-Client: probe' "$API/v1/previews")"
    curl -s -m 5 -H 'X-Luma-Client: probe' -o "$OUT/helper-$KEY.jpg" -D "$OUT/helper-$KEY.headers" "$API/v1/previews/$KEY"
    say "helper picture: $(size_of "$OUT/helper-$KEY.jpg") bytes, $(tr -d '\r' < "$OUT/helper-$KEY.headers" | tr '\n' ' ')"
    say "on the flash: $(ls -l /var/lib/lumatv/previews 2>&1 | tr '\n' ';')"
}

KEY=${KEY:-}
if [ -z "$KEY" ]; then
    app=$(luna-send -n 1 -w 3000 luna://com.webos.applicationManager/getForegroundAppInfo '{}' 2>/dev/null | tr -d ' \n' | sed -n 's/.*"appId":"\([^"]*\)".*/\1/p')
    case "$app" in
        com.webos.app.hdmi[1-9]) KEY=hdmi${app#com.webos.app.hdmi} ;;
        com.webos.app.livetv) KEY=livetv ;;
        *) KEY=hdmi1 ;;
    esac
    say "in front: ${app:-unknown} (key $KEY; KEY=... overrides)"
fi

case "${1:-help}" in
    capture) capture "$2" ;;
    sizes) sizes ;;
    cost) cost "$2" "$3" ;;
    helper) helper ;;
    all) capture "$2"; sizes; cost; helper ;;
    *) sed -n 2,22p "$0" ;;
esac
