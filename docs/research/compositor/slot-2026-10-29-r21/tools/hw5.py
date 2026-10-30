import sys, time, json, subprocess
sys.path.insert(0, "/tmp/r21-build"); sys.path.insert(0, "/tmp/lumatv-r20/docs/research/arch")
import tv, rustlib as R
T = "/media/developer/apps/usr/palm/applications/org.nphil.lumatv/tools/lumatv"
D = "/tmp/lumatv-r21/docs/research/compositor/slot-2026-10-29-r21/img"
def say(s): print(time.strftime("%T"), s, flush=True)
def respawn():
    pid = R.ctl("shell")["rust"]["pid"]
    tv.tv("kill %s" % pid, 10); t0 = time.time()
    while time.time() - t0 < 60:
        time.sleep(1)
        try:
            r = R.ctl("shell")["rust"]
            if r.get("connected") and r.get("pid") != pid and "home" in ((r.get("ui") or {}).get("kinds") or []):
                break
        except Exception: pass
    time.sleep(3); say("ui respawned in %.1f s (pid %s -> %s)" % (time.time()-t0, pid, r.get("pid")))
try:
    say("helper stop: " + tv.tv("%s helper stop 2>&1" % T, 30).strip())
    say("mv qml: " + tv.tv("mv /var/lib/lumatv/qml /var/lib/lumatv/qml.x; ls -d /var/lib/lumatv/qml* ", 10).strip().replace("\n"," "))
    respawn()
    time.sleep(4)
    R.capture_png(D + "/r21-nomodule-helperdown-full.png")
    say("env module dir of ui: " + tv.tv("pid=$(pidof lumatv-ui | cut -d' ' -f1); tr '\\0' '\\n' < /proc/$pid/environ | grep -i LUMATV_ | head -5", 10).strip().replace("\n"," | "))
    say("last-profile.json: " + tv.tv("ls -la /var/lib/lumatv/*/last-profile.json /var/lib/lumatv/last-profile.json 2>&1 | head -3", 10).strip().replace("\n"," | "))
finally:
    say("mv back: " + tv.tv("mv /var/lib/lumatv/qml.x /var/lib/lumatv/qml; ls -d /var/lib/lumatv/qml*", 10).strip().replace("\n"," "))
    say("helper start: " + tv.tv("%s helper start 2>&1" % T, 40).strip())
    time.sleep(6)
    respawn()
    R.capture_png(D + "/r21-restored-full.png")
