#!/usr/bin/env python3
"""Install the worktree's ipk on the set the documented way (dev/install, wait until the app directory holds the ipk's files, `lumatv apply`),
then read the state back."""
import hashlib, io, os, sys, tarfile, time
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


def ipk_tree_md5(ipk, rel):
    """md5 of the files under `rel` in the ipk's data archive, in the byte order `find | sort` gives on the set."""
    with open(ipk, "rb") as f:
        blob = f.read()
    at, data = 8, None                      # ar: "!<arch>\n", then 60-byte member headers
    while at < len(blob):
        name, size = blob[at:at + 16].decode().strip(), int(blob[at + 48:at + 58].decode())
        if name.rstrip("/").startswith("data.tar"):
            data = blob[at + 60:at + 60 + size]
        at += 60 + size + (size & 1)
    prefix = "./usr/palm/applications/org.nphil.lumatv/" + rel + "/"
    with tarfile.open(fileobj=io.BytesIO(data), mode="r:*") as t:
        files = sorted(("./" + m.name[len(prefix):], m) for m in t.getmembers() if m.isfile() and m.name.startswith(prefix))
        h = hashlib.md5()
        for _, m in files:
            h.update(t.extractfile(m).read())
    return h.hexdigest()


# The helper's sha is no sign the new files are in place: a build that leaves the helper unchanged matches at once, and `apply` then
# staged the old QML (round 16, a438a2e: "staged set unchanged"). The compositor's QML tree is what the panels' changes live in.
want = ipk_tree_md5(IPK, "payload/compositor")
got = ""
for i in range(40):
    got = tv("cd %s/payload/compositor && find . -type f | sort | while IFS= read -r f; do cat \"$f\"; done | md5sum | cut -d' ' -f1" % APP).strip()
    if got == want:
        break
    time.sleep(3)
log("app dir compositor QML matches ipk: %s after %d s" % (got == want, int(time.time() - t0)))
if got != want:
    log("end: the app directory never held the ipk's QML, apply not run")
    sys.exit(1)
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
