#!/usr/bin/env python3
"""Install the worktree's ipk on the set the documented way (dev/install, wait for the helper's sha, `lumatv apply`), then read the state back."""
import os, sys, time
sys.path.insert(0, "/tmp/r8")
import r8
from r8 import log, tv, pty, put, TOOL, APP

W = "/tmp/lumatv-slot"
IPK = W + "/org.nphil.lumatv_0.4.0_all.ipk"
t0 = time.time()
log("install: start, sha %s" % open(W + "/.git").read().strip()[:80] if os.path.isfile(W + "/.git") else "install: start")
log("before: " + tv("%s version 2>&1 | head -1; md5sum %s/payload/helper/lumatv-helper %s/tools/lumatv 2>&1" % (TOOL, APP, APP)).strip().replace("\n", " | "))
log("power %s fg %s picture %s" % (r8.power(), r8.fg(), r8.picture_mode()))
lines0 = r8.log_lines()
put(IPK, "/tmp/lumatv.ipk")
log("ipk put: " + tv("ls -la /tmp/lumatv.ipk; sha256sum /tmp/lumatv.ipk").strip().replace("\n", " | "))
r = pty("luna-send -n 4 -f luna://com.webos.appInstallService/dev/install '{\"id\":\"org.nphil.lumatv\",\"ipkUrl\":\"/tmp/lumatv.ipk\",\"subscribe\":true}'", 180)
log("install: " + " | ".join(l.strip() for l in r.splitlines() if "status" in l or "returnValue" in l or "error" in l.lower())[:600])
want = open(W + "/helper/dist/lumatv-helper.sha256").read().split()[0]
got = ""
for i in range(40):
    got = tv("sha256sum %s/payload/helper/lumatv-helper 2>/dev/null | cut -d' ' -f1" % APP).strip()
    if got == want:
        break
    time.sleep(3)
log("app dir helper sha matches ipk: %s after %d s" % (got == want, int(time.time() - t0)))
r = pty(TOOL + " apply", 300)
log("apply (%d s): %s" % (int(time.time() - t0), r.strip()[-700:]))
time.sleep(8)
for v in ("status", "homestub status", "helper status", "compositor status", "vplane status", "surfaces"):
    r = pty("%s %s" % (TOOL, v), 120)
    log("%s: %s" % (v, r.strip()[:900]))
log("lean: " + pty("sh /var/lib/lumatv/dc-dev/tools/declutter.sh status", 60).strip()[:400])
log("helper log tail: " + tv("tail -5 /var/lib/lumatv/log-helper 2>/dev/null").strip()[:800])
log("foreground: %s power %s picture %s" % (r8.fg(), r8.power(), r8.picture_mode()))
log("compositor log since install: " + r8.log_tail_from(lines0, r"\[Luma(Keys|Home|Overlay)\]|\[Resident\]|LumaOverlay failed|ReferenceError|is not defined|SyntaxError|TypeError|Unable to assign|Cannot assign|%7B%7B").strip()[:3000])
log("end (%d s)" % int(time.time() - t0))
