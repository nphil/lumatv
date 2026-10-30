#!/usr/bin/env python3
"""gearcheck.py: GearHold's checks on the set: three gear taps (key 139) are the quick panel every time and never LG's hidden menu (tvhotkey), a gear hold is
LumaTV's Settings, and System > About shows the model and serial (not written to the repository: the serial is counted, not printed)."""
import json, re, sys, time
sys.path.insert(0, "/tmp/r19w-build")
import rustlib as R
import tv

def logtail(n0):
    out = tv.tv("tail -n +%d /var/lib/lumatv/compositor/compositor.log | grep -a -E 'gear|AV reset|tvhotkey|Settings|quick' | cut -c1-200" % (n0 + 1), 30)
    return [l for l in out.splitlines() if l.strip()]

def nlines():
    return int(tv.tv("wc -l < /var/lib/lumatv/compositor/compositor.log", 20).strip() or 0)

def state():
    r = R.ctl("shell").get("rust", {})
    u = r.get("ui") or {}
    return {"quick": (u.get("panels") or {}).get("quick"), "settings": (u.get("settings") or {}).get("open"), "page": (u.get("settings") or {}).get("page"),
            "fg": (r.get("topics") or {}).get("foreground")}

def windows():
    return [(w["appId"], w["type"]) for w in R.ctl("windows").get("windows", [])]

def say(s):
    print("%s %s" % (time.strftime("%T"), s), flush=True)

say("start state %s windows %s" % (state(), windows()))
for i in (1, 2, 3):
    n0 = nlines()
    R.ctl("remote 139 tap")
    time.sleep(0.5)
    say("tap %d: %s | %s" % (i, state(), " || ".join(logtail(n0))[:400]))
say("windows after three taps: %s (tvhotkey present: %s)" % (windows(), any(a == "com.webos.app.tvhotkey" for a, _ in windows())))
# close whatever is open, then the hold
for _ in range(3):
    R.ctl("remote 158 tap"); time.sleep(1.0)
say("after Back: %s" % state())
n0 = nlines()
R.ctl("remote 139 press"); time.sleep(1.4); R.ctl("remote 139 release"); time.sleep(1.5)
say("hold 1.4 s: %s | %s" % (state(), " || ".join(logtail(n0))[:400]))
for _ in range(4):
    R.ctl("remote 158 tap"); time.sleep(1.0)
say("after Back: %s" % state())
R.ctl("shell show settings about")
time.sleep(2.5)
s = R.ctl("shell").get("rust", {}).get("ui", {}).get("settings", {})
txt = json.dumps(s)
say("About: page %s; rows mention model %s, serial %s (value length %s)" % (s.get("page"), "model" in txt.lower(), "serial" in txt.lower(),
    len(re.findall(r"[A-Z0-9]{8,}", txt))))
for _ in range(4):
    R.ctl("remote 158 tap"); time.sleep(1.0)
say("final: %s" % state())
