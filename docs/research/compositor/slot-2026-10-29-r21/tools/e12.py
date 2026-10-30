import sys, time, json, subprocess
sys.path.insert(0, "/tmp/r21-build"); sys.path.insert(0, "/tmp/lumatv-r21/docs/research/arch")
import tv, rustlib as R
from mv import pty
D = "/tmp/lumatv-r21/docs/research/multiview/tv-2026-10-30"
IMG = "/tmp/lumatv-r21/docs/research/compositor/slot-2026-10-29-r21/img"
which = sys.argv[1]   # e1 | e2
def say(s): print(time.strftime("%T"), s, flush=True)
def claims(): return [{k: c.get(k) for k in ("app","slot","placed","sink","state","text","window")} for c in R.ctl("vwin")["vwin"]["claims"]]
def vo():
    o = tv.tv("luna-send -n 1 -f luna://com.webos.service.videooutput/getStatus '{}'", 20)
    try: j = json.loads(o)
    except Exception: return o[:200]
    return [(v.get("sink"), v.get("appId"), v.get("connected"), v.get("width"), v.get("height"), v.get("frameRate"), v.get("context"), v.get("subScalerOccupied")) for v in j.get("video", [])]
try:
    subprocess.run(["python3", "/tmp/r21-build/ytplay.py"])
    R.ctl("vwin apps")
    if which == "e2": say("fg: " + str(R.ctl("vwin fg com.webos.app.home"))[:200])
    say("elements: " + str([(e["app"], e["counted"], e["playing"]) for e in R.ctl("vwin")["vwin"]["elements"]]))
    say("videooutput before: %s" % vo())
    n0 = int(tv.tv("wc -l < /var/lib/lumatv/compositor/compositor.log", 10).strip())
    say(pty("setsid sh /tmp/lumatv/hwplanes.sh feeder /tmp/gfn/clips/cam720p30.h264 1280x720 h264 30 40 org.nphil.lumatv.cameras com.webos.app.home > /tmp/hwf.out 2>&1 < /dev/null; sleep 1; tail -1 /tmp/hwf.out", 60).strip())
    time.sleep(2)
    say("claim: " + json.dumps(R.ctl("vwin claim e1 pip 1300 60 560 315 org.nphil.lumatv.cameras h264 1280x720@30")["claim"])[:400])
    for i in range(16):
        time.sleep(0.5)
        c = [x for x in claims() if x["app"] == "org.nphil.lumatv.cameras"]
        if c and c[0]["placed"]: say("placed after %.1f s" % ((i + 1) * 0.5)); break
    time.sleep(6)
    say("claims: %s" % claims())
    say("videooutput during: %s" % vo())
    out = pty("sh /tmp/lumatv/mvread.sh luna %s 2>&1 | tail -3" % which, 120)
    time.sleep(8)
    j = tv.tv("tail -c 600 /tmp/lumatv/out/hwplanes/feeder-org.nphil.lumatv.cameras.jsonl", 15).strip().replace("\n", " | ")
    say("feeder stats: " + j[-500:])
    say("yt: " + json.dumps(R.ctl("shell")["rust"]["topics"]["foreground"]) + " video windows: " + json.dumps(R.ctl("windows").get("video"))[:300])
    say("journal: " + tv.tv("tail -n +%d /var/lib/lumatv/compositor/compositor.log | grep -a -E 'vwin:|foreign:|policy|closeBy' | cut -c1-230 | tail -10" % (n0 + 1), 20).strip().replace("\n", " | "))
    say("messages: " + tv.tv("grep -a -i -E 'policy action|closeByAppId|multiview' /var/log/messages | tail -6 | cut -c1-260", 20).strip().replace("\n", " | "))
    say("mvc: " + tv.tv("luna-send -n 1 -f luna://com.webos.service.multiviewcontroller/getStatus '{}' | tr -d '\\n ' | head -c 500", 20))
    subprocess.run("python3 /tmp/r21-build/shot.py %s-shot --luna" % which, shell=True, env={"SHOTDIR": IMG, "PATH": "/usr/bin:/bin"})
finally:
    say("stop fg: " + str(R.ctl("vwin fg off"))[:80])
    say(pty("sh /tmp/lumatv/hwplanes.sh stop 2>&1 | tail -1", 60).strip()[:200])
    R.ctl("vwin idle")
    say("yt end: " + json.dumps(R.ctl("shell")["rust"]["topics"]["foreground"]))
