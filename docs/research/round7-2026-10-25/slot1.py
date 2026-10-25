#!/usr/bin/env python3
"""Slot 1: the current ipk on the set (dev/install), `lumatv apply`, then the state read back."""
import json, os, sys, time
W = "/tmp/lumatv-slot"
sys.path.insert(0, W + "/compositor/tools"); sys.path.insert(0, W + "/docs/research/arch")
import tv, tv_gate
IPK = W + "/org.nphil.lumatv_0.4.0_all.ipk"
APP = "/media/developer/apps/usr/palm/applications/org.nphil.lumatv"
TOOL = APP + "/tools/lumatv"
OUT = sys.argv[1]
os.makedirs(OUT, exist_ok=True)
log = open(os.path.join(OUT, "slot1.txt"), "w")
def say(s):
    print(s, flush=True); log.write(s + "\n"); log.flush()

t0 = time.time()
say("start %s" % time.strftime("%H:%M:%S"))
import urllib.request
def avr_state():
    try:
        req = urllib.request.Request(tv_gate.HA + "/api/states/" + tv_gate.AVR, headers={"Authorization": "Bearer " + tv_gate.ha_token()})
        j = json.loads(urllib.request.urlopen(req, timeout=10).read())
        return "%s volume_level %s muted %s" % (j.get("state"), j["attributes"].get("volume_level"), j["attributes"].get("is_volume_muted"))
    except Exception as e:
        return "unreadable: %s" % e
say("receiver before: " + avr_state())
if not tv_gate.wake():
    sys.exit("the set does not answer")
say("power: %s" % tv_gate.power_state())
say("before: " + tv.tv("%s version 2>&1 | head -2; md5sum %s/payload/helper/lumatv-helper %s/tools/lumatv 2>&1" % (TOOL, APP, APP)).strip())
tv.put(IPK, "/tmp/lumatv.ipk")
say("ipk put: " + tv.tv("ls -la /tmp/lumatv.ipk; sha256sum /tmp/lumatv.ipk").strip())
r = tv_gate.pty("luna-send -n 4 -f luna://com.webos.appInstallService/dev/install '{\"id\":\"org.nphil.lumatv\",\"ipkUrl\":\"/tmp/lumatv.ipk\",\"subscribe\":true}'", 180)
say("install: " + " | ".join(l.strip() for l in r.splitlines() if "status" in l or "statusValue" in l or "returnValue" in l or "error" in l.lower())[:600])
want = open(W + "/helper/dist/lumatv-helper.sha256").read().split()[0]
for i in range(40):
    got = tv.tv("sha256sum %s/payload/helper/lumatv-helper 2>/dev/null | cut -d' ' -f1" % APP).strip()
    if got == want:
        break
    time.sleep(3)
say("app dir helper sha matches ipk: %s after %d s" % (got == want, int(time.time() - t0)))
r = tv_gate.pty(TOOL + " apply", 300)
say("apply (%d s): %s" % (int(time.time() - t0), r.strip()[-700:].replace("\r", "")))
time.sleep(5)
for v in ("status", "homestub status", "helper status", "preload status", "compositor status", "inputcommon status", "surfaces"):
    r = tv_gate.pty("%s %s" % (TOOL, v), 120)
    say("%s: %s" % (v, r.strip().replace("\r", "")[:900]))
say("lean: " + tv_gate.pty("sh /var/lib/lumatv/dc-dev/tools/declutter.sh status", 60).strip().replace("\r", "")[:500])
say("helper log tail: " + tv.tv("tail -5 /var/lib/lumatv/log-helper 2>/dev/null; ls /var/lib/lumatv/ | tr '\\n' ' '").strip()[:800])
say("foreground: " + tv_gate.pty("luna-send -n 1 -f luna://com.webos.applicationManager/getForegroundAppInfo '{}' | tr -d '\\n '", 20).strip())
say("picture: " + tv_gate.pty("luna-send -n 1 -f luna://com.webos.settingsservice/getSystemSettings '{\"category\":\"picture\",\"keys\":[\"pictureMode\"]}' | grep pictureMode", 20).strip())
say("end %s (%d s)" % (time.strftime("%H:%M:%S"), int(time.time() - t0)))
