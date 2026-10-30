#!/usr/bin/env python3
"""hwchain.py [STEP...]: the hardware-planes checks of docs/research/hw-planes-result.md that need no one and no raw register write, in order, each switched back
after (`stop` at the end of every group): setup, read, p0 (alpha), p12 (pointer), a1 (afbc), a3 (layout), v1, v23, v45 (a YouTube video by id and a feeder claim
next to it), v5, v6 (sizes), v7, orbit (layerdeep-tv.sh orbit with `ctl scanout direct`). Output: the slot's hwplanes/ directory (log.txt, the set's out/hwplanes,
captures as 1280-wide JPEGs). The steps tolerate a failed command and say so."""
import glob, os, subprocess, sys, time
sys.path.insert(0, "/tmp/lumatv-r19w/docs/research/arch")
import tv

W = "/tmp/lumatv-r19w"
SLOT = W + "/docs/research/compositor/slot-2026-10-29-r19w/hwplanes"
T = ["python3", W + "/compositor/tools/tv_hwplanes.py"]
CLIP = "/tmp/r19w-build/cam720p30.h264"
FEEDER = "/tmp/lumatv-r19/gfn/feeder/build/arm/lumatv-gfn-feeder"
APP1, APP2 = "org.nphil.lumatv.cameras", "org.nphil.lumatv.cameras2"
os.makedirs(SLOT, exist_ok=True)
LOG = SLOT + "/log.txt"

def say(s):
    line = "%s %s" % (time.strftime("%T"), s)
    print(line, flush=True)
    open(LOG, "a").write(line + "\n")

def family():
    return os.path.exists("/tmp/r19w-gate/FAMILY")

def run(*args, t=300):
    r = subprocess.run(T + list(args), capture_output=True, text=True, timeout=t)
    out = (r.stdout + r.stderr).strip()
    for l in out.splitlines():
        if l.strip():
            open(LOG, "a").write("    " + l[:300] + "\n")
    return out

def hw(*args, t=300):
    out = run(*args, t=t)
    lines = [l for l in out.splitlines() if l.startswith("hw:") or l.startswith("P0") or "PASS" in l or "FAIL" in l]
    say("T %s -> %s" % (" ".join(args), " | ".join(l.strip() for l in lines)[:1200] if lines else out[:600].replace("\n", " | ")))
    return out

def ctl(*words):
    out = run("ctl", *words, t=60)
    say("ctl %s -> %s" % (" ".join(words), out.strip().replace("\n", " ")[:500]))
    return out

def cma():
    out = tv.tv("grep -E 'CmaFree|MemAvailable' /proc/meminfo | tr -s ' ' | tr '\\n' ';'", 20).strip()
    say("memory: " + out)
    return out

def luna(uri, payload):
    return tv.tv("luna-send -n 1 -f %s '%s' 2>&1 | tr -d '\\n' | cut -c1-200" % (uri, payload), 30)

def home():
    luna("luna://com.webos.service.applicationmanager/closeByAppId", '{"id":"youtube.leanback.v4"}')
    luna("luna://com.webos.service.applicationmanager/launch", '{"id":"com.webos.app.home"}')

def step_setup():
    run("put-feeder", FEEDER)
    run("put-clip", CLIP)
    say("setup: feeder and clip put; %s" % tv.tv("ls -la /tmp/gfn /tmp/gfn/clips | tail -4", 20).strip().replace("\n", " | ")[:300])
    cma()

def step_read():
    hw("read")

def step_p0():
    hw("alpha")

def step_p12():
    hw("pointer")
    ctl("pointer", "gpu")

def step_a1():
    hw("afbc")

def step_a3():
    cma()
    hw("layout", "alpha-on")
    hw("shot", "layout-alpha-on")
    hw("layout", "on")
    time.sleep(4)
    hw("shot", "layout-on")
    hw("afbc")
    hw("layout", "off")
    time.sleep(3)
    hw("shot", "layout-off")
    out = run("compare", "/tmp/lumatv-tv-out/hwplanes/layout-alpha-on-display.png", "/tmp/lumatv-tv-out/hwplanes/layout-on-display.png")
    cma()

def feeder(app, secs=60):
    hw("feeder", "/tmp/gfn/clips/cam720p30.h264", "1280x720", "h264", "30", str(secs), app)

def step_v1():
    hw("vwin")

def step_v23():
    ctl("vwin", "idle")
    feeder(APP1)
    ctl("vwin", "claim", "t1", "pip", "1280", "720", "480", "270", APP1, "h264", "1280x720@30")
    time.sleep(3)
    hw("vwin")
    hw("shot", "v2")
    hw("stop")

def step_v45():
    luna("luna://com.webos.service.applicationmanager/launch",
         '{"id":"youtube.leanback.v4","params":{"contentTarget":"https://www.youtube.com/tv?v=aqz-KE-bpKQ"}}')
    time.sleep(25)
    ctl("vwin", "apps")
    feeder(APP1)
    ctl("vwin", "claim", "t1", "pip", "1280", "720", "480", "270", APP1, "h264", "1280x720@30")
    time.sleep(3)
    hw("vwin")
    hw("shot", "v4")
    hw("stop")
    home()

def step_v5():
    ctl("vwin", "idle")
    ctl("vwin", "slots", "3")
    feeder(APP1)
    feeder(APP2)
    ctl("vwin", "claim", "t1", "pip", "1280", "720", "480", "270", APP1, "h264", "1280x720@30")
    ctl("vwin", "claim", "t2", "pip", "1280", "360", "480", "270", APP2, "h264", "1280x720@30")
    time.sleep(3)
    hw("vwin")
    hw("shot", "v5")
    ctl("vwin", "slots", "2")
    time.sleep(2)
    hw("vwin")
    hw("stop")

def step_v6():
    ctl("vwin", "idle")
    feeder(APP1)
    ctl("vwin", "claim", "t1", "pip", "1280", "720", "480", "270", APP1, "h264", "1280x720@30")
    time.sleep(3)
    hw("sizes", t=600)
    hw("stop")

def step_v7():
    ctl("vwin", "idle")
    feeder(APP1)
    feeder(APP2)
    ctl("vwin", "claim", "t1", "pip", "1280", "720", "0", "100", APP1, "h264", "1280x720@30")
    ctl("vwin", "claim", "t2", "pip", "1280", "720", "640", "100", APP2, "h264", "1280x720@30")
    time.sleep(3)
    hw("vwin")
    hw("shot", "v7")
    ctl("vwin", "release", "t2")
    time.sleep(2)
    hw("vwin")
    hw("stop")

def step_orbit():
    home()
    time.sleep(3)
    ctl("scanout", "direct")
    tv.put(W + "/tools/layerdeep-tv.sh", "/tmp/lumatv/layerdeep-tv.sh")
    out = tv.tv("sh /tmp/lumatv/layerdeep-tv.sh orbit 2>&1 | tail -30", 330)
    for l in out.splitlines():
        say("orbit: " + l[:300])
    ctl("scanout", "off")
    ctl("scanout", "offset", "off")

def fetch():
    run("fetch")
    src = "/tmp/lumatv-tv-out/hwplanes"
    dst = SLOT + "/set"
    os.makedirs(dst, exist_ok=True)
    try:
        from PIL import Image
        for p in glob.glob(src + "/*.png"):
            im = Image.open(p).convert("RGB")
            if im.size[0] != 1280:
                im = im.resize((1280, round(im.size[1] * 1280 / im.size[0])))
            im.save(dst + "/" + os.path.basename(p)[:-4] + ".jpg", "JPEG", quality=80)
    except Exception as e:
        say("images: %s" % e)
    for p in glob.glob(src + "/*"):
        if not p.endswith(".png") and os.path.isfile(p):
            subprocess.run(["cp", p, dst])

STEPS = [("setup", step_setup), ("read", step_read), ("p0", step_p0), ("p12", step_p12), ("a1", step_a1), ("a3", step_a3), ("v1", step_v1), ("v23", step_v23),
         ("v45", step_v45), ("v5", step_v5), ("v6", step_v6), ("v7", step_v7), ("orbit", step_orbit)]

if __name__ == "__main__":
    want = sys.argv[1:] or [n for n, _ in STEPS]
    for name, fn in STEPS:
        if name not in want:
            continue
        if family():
            say("STOP before %s: %s" % (name, open("/tmp/r19w-gate/FAMILY").read().strip()))
            break
        say("== %s" % name)
        try:
            fn()
        except Exception as e:
            say("step %s raised %s" % (name, str(e)[:300]))
            try:
                hw("stop")
            except Exception:
                pass
    try:
        hw("stop")
    except Exception:
        pass
    fetch()
    say("hwchain over")
