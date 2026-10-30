"""Helpers for the round's Rust-shell steps: ctl on the set, shell state, screenshots, waiting for a kind."""
import json, subprocess, sys, time
sys.path.insert(0, "/tmp/lumatv-r19w/docs/research/arch")
import tv
CTL = "/tmp/lumatv/lib/ld-linux-aarch64.so.1 --library-path /tmp/lumatv/lib /tmp/lumatv/bin/lumatv-compositor ctl --runtime-dir /tmp/xdg"
def ctl(a, t=30):
    out = tv.tv("%s %s 2>/dev/null" % (CTL, a), t).strip()
    try:
        return json.loads(out)
    except Exception:
        return out
def shot(name):
    r = subprocess.run(["python3", "/tmp/r19w-build/shot.py", name], capture_output=True, text=True)
    return (r.stdout.strip() or r.stderr.strip()[-200:]).split(" (")[0]
def rust():
    return ctl("shell").get("rust", {})
def ui():
    return rust().get("ui") or {}
def summary(r=None):
    r = r or rust()
    u = r.get("ui") or {}
    return {"pid": r.get("pid"), "connected": r.get("connected"), "engine": r.get("engine"), "kinds": r.get("kinds"), "ui.kinds": u.get("kinds"), "wants": r.get("wants"),
            "can": r.get("can"), "error": r.get("error"), "systemLive": r.get("systemLive"), "foreground": (r.get("topics") or {}).get("foreground")}
def wait_kinds(want, secs=30):
    t0 = time.time()
    while time.time() - t0 < secs:
        r = rust()
        u = r.get("ui") or {}
        if r.get("connected") and set(want) <= set(u.get("kinds") or []):
            return time.time() - t0, r
        time.sleep(0.3)
    return None, rust()
def journal(n0=0, pat="rust"):
    j = ctl("journal")["lines"][n0:]
    return [x for x in j if pat in x.lower()]
def status():
    return ctl("status").get("summary", "")

def capture_png(local):
    """ctl capture at full resolution to a local PNG; returns the path (the file is written after ctl answers: polled)."""
    tv.tv("rm -f /tmp/r19wfull.png; %s capture /tmp/r19wfull.png >/dev/null 2>&1" % CTL, 30)
    for _ in range(40):
        if tv.tv("test -s /tmp/r19wfull.png && echo y").strip().startswith("y"):
            break
        time.sleep(0.15)
    time.sleep(0.2)
    tv.get("/tmp/r19wfull.png", local)
    return local
