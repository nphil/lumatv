import sys, time, json, re
sys.path.insert(0, "/tmp/r21-build"); sys.path.insert(0, "/tmp/lumatv-r21/docs/research/arch")
import tv, rustlib as R
from mv import pty
import subprocess
def subprocess_out(c): return subprocess.run(c,shell=True,capture_output=True,text=True).stdout.strip()
D = "/tmp/lumatv-r21/docs/research/multiview/tv-2026-10-30"
IMG = "/tmp/lumatv-r21/docs/research/compositor/slot-2026-10-29-r21/img"
def say(s): print(time.strftime("%T"), s, flush=True)
def hw(a, t=60): return pty("sh /tmp/lumatv/hwplanes.sh %s 2>&1" % a, t)
def vw(): return R.ctl("vwin")
try:
    say("vwin idle: " + str(R.ctl("vwin idle"))[:120])
    say("elements: " + str([(e["app"],e["counted"],e["playing"]) for e in vw()["vwin"]["elements"]]))
    say("yt up: " + str(R.ctl("shell")["rust"]["topics"]["foreground"]))
    n0 = tv.tv("wc -l < /var/lib/lumatv/compositor/compositor.log", 10).strip()
    # pop-up-like claim via the harness: feeder in texture mode (640x360) for the claim app id
    say(pty("HWPLANES_TEXTURE=640x360 setsid sh /tmp/lumatv/hwplanes.sh feeder /tmp/gfn/clips/cam720p30.h264 1280x720 h264 30 60 org.nphil.lumatv.cameras > /tmp/hwf.out 2>&1 < /dev/null; sleep 1; cat /tmp/hwf.out | tail -2", 60).strip())
    time.sleep(2)
    say("claim: " + str(R.ctl("vwin claim t1 pip 1300 60 560 315 org.nphil.lumatv.cameras h264 1280x720@30"))[:500])
    placed_t = None; t0 = time.time()
    for i in range(20):
        time.sleep(0.5)
        c = [x for x in vw()["vwin"]["claims"] if x["id"] == "t1"]
        if c and c[0].get("placed"): placed_t = time.time() - t0; break
    say("placed after %s s; claim %s" % (placed_t, json.dumps(c[0] if c else None)[:400]))
    time.sleep(8)
    R.ctl("refresh reset")
    for k in ("display", "video"):
        pass
    say("shot: " + subprocess_out("python3 /tmp/r21-build/shot.py tex-yt --ctl"))
    time.sleep(20)
    j = tv.tv("tail -c 1500 /tmp/lumatv/out/hwplanes/feeder-org.nphil.lumatv.cameras.jsonl; echo; grep -a -c tex /tmp/lumatv/out/hwplanes/feeder-org.nphil.lumatv.cameras.jsonl", 20)
    say("feeder tex lines: " + j.strip()[-900:])
    say("yt: " + json.dumps(R.ctl("shell")["rust"]["topics"]["foreground"]) + " windows video: " + str(R.ctl("windows"))[:200])
    say("policy in messages: " + tv.tv("grep -a -i 'policy action' /var/log/messages | tail -3 | cut -c1-200; echo n=$(grep -a -c -i 'policy action' /var/log/messages)", 20).strip())
    say("journal: " + tv.tv("tail -n +%s /var/lib/lumatv/compositor/compositor.log | grep -a -E 'vwin:|texture|foreign:' | cut -c1-220 | tail -8" % (int(n0) + 1), 20).strip().replace("\n", " | "))
    say("cpu: " + tv.tv("top -b -n 1 | head -12 | cut -c1-120", 20).strip().replace("\n", " | "))
finally:
    say(hw("stop", 60).strip()[-200:])
