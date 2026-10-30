#!/usr/bin/env python3
"""OverlayJank's two asks in one YouTube session (video/oj/ and video/oj2/): the 30 s idle read (status, ring 1, 30 s, status, ring 200), then the Mali power A/B:
cond base (as found), ao (power_policy always_on), ao-on (also gpu power/control on); each cond: idle window (refresh, ring 300), osd (115/114) and quick
(show/Back 158), refresh and ring per window. The two files' values are read first, a detached dead-man restores them after 420 s, and they are put back
and read back at the end. Runtime sysfs only."""
import os, sys, time
sys.path.insert(0, "/tmp/r19w-build")
import ojcheck as O
tv = O.tv
D2 = "/tmp/lumatv-r19w/docs/research/compositor/slot-2026-10-29-r19w/video/oj2"
os.makedirs(D2, exist_ok=True)
PP = "/sys/class/misc/mali0/device/power_policy"
PC = "/sys/devices/platform/soc/c4250000.gpu/power/control"

def save2(name, text):
    open("%s/%s" % (D2, name), "w").write(text)

def current(path):
    t = tv.tv("cat %s 2>&1" % path, 20).strip()
    import re
    m = re.search(r"\[([a-z_]+)\]", t)
    return m.group(1) if m else t.split()[0] if t.split() else ""

def cond(name):
    pre = "%s-" % name
    O.ctl("refresh reset"); O.ctl("note OJ2-%sidle" % pre)
    time.sleep(4)
    save2(pre + "idle-refresh.json", O.ctl("refresh"))
    save2(pre + "idle-ring.json", O.ctl("ring 300"))
    for kind, opener, closer in (("osd", O.osd_open, O.osd_close), ("quick", O.quick_open, O.quick_close)):
        O.ctl("refresh reset"); O.ctl("wake reset"); O.ctl("note OJ2-%s%s" % (pre, kind))
        opener()
        time.sleep(4)
        save2("%s%s-refresh.json" % (pre, kind), O.ctl("refresh"))
        save2("%s%s-ring.json" % (pre, kind), O.ctl("ring 1500"))
        closer()
        time.sleep(3)
    O.say("cond %s done: policy %s control %s" % (name, current(PP), tv.tv("cat %s 2>&1" % PC, 20).strip()))

if __name__ == "__main__":
    pp0, pc0 = current(PP), tv.tv("cat %s 2>&1" % PC, 20).strip()
    has_pc = pc0 in ("on", "auto")
    O.say("power_policy read %r, power/control read %r (%s)" % (pp0, pc0, "present" if has_pc else "missing: skipped"))
    open(D2 + "/restore-values.txt", "w").write("power_policy %s\npower/control %s\n" % (pp0, pc0))
    # the detached dead-man: puts both back after 420 s if this process is gone
    tv.tv("setsid nohup sh -c 'sleep 420; echo %s > %s; %s' > /dev/null 2>&1 < /dev/null & echo armed" % (pp0, PP, ("echo %s > %s" % (pc0, PC)) if has_pc else ":"), 20)
    O.say("YouTube full screen")
    O.luna("luna://com.webos.service.applicationmanager/launch", '{"id":"youtube.leanback.v4","params":{"contentTarget":"https://www.youtube.com/tv?v=aqz-KE-bpKQ"}}')
    time.sleep(30)
    O.save("idle30-s0.json", O.ctl("status"))
    O.ctl("ring 1")
    time.sleep(30)
    O.save("idle30-s1.json", O.ctl("status"))
    O.save("idle30-ring.json", O.ctl("ring 200"))
    O.say("idle30 done")
    try:
        cond("base")
        tv.tv("echo always_on > %s" % PP, 20)
        O.say("power_policy now %s" % tv.tv("cat %s" % PP, 20).strip())
        cond("ao")
        if has_pc:
            tv.tv("echo on > %s" % PC, 20)
            O.say("power/control now %s" % tv.tv("cat %s" % PC, 20).strip())
            cond("ao-on")
    finally:
        tv.tv("echo %s > %s; %s" % (pp0, PP, ("echo %s > %s" % (pc0, PC)) if has_pc else ":"), 20)
        tv.tv("for p in $(ps | grep 'sleep 420' | grep -v grep | awk '{print $1}'); do kill $p; done", 20)
        after = "power_policy %s, power/control %s" % (tv.tv("cat %s" % PP, 20).strip(), tv.tv("cat %s 2>&1" % PC, 20).strip())
        O.say("restored: " + after)
        open(D2 + "/restore-values.txt", "a").write("after: " + after + "\n")
        O.luna("luna://com.webos.service.applicationmanager/closeByAppId", '{"id":"youtube.leanback.v4"}')
        O.luna("luna://com.webos.service.applicationmanager/launch", '{"id":"com.webos.app.home"}')
        time.sleep(5)
        O.say("done")
