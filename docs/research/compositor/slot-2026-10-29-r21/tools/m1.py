import sys, time, json, subprocess, re
sys.path.insert(0, "/tmp/r21-build"); sys.path.insert(0, "/tmp/lumatv-r21/docs/research/arch")
import tv, rustlib as R
from mv import pty
IMG = "/tmp/lumatv-r21/docs/research/compositor/slot-2026-10-29-r21/img"
def say(s): print(time.strftime("%T"), s, flush=True)
def vo():
    o = tv.tv("luna-send -n 1 -f luna://com.webos.service.videooutput/getStatus '{}'", 20)
    try: j = json.loads(o)
    except Exception: return o[:200]
    return [(v.get("sink"), v.get("appId"), v.get("connected"), v.get("width"), v.get("height"), v.get("frameRate")) for v in j.get("video", [])]
M = None
try:
    subprocess.run(["python3", "/tmp/r21-build/ytplay.py"])
    R.ctl("vwin apps")
    say("vwin: " + str(R.ctl("vwin fg com.webos.app.home"))[:60])
    say("serve: " + pty("rm -f /tmp/gfn/m1.sock; XDG_RUNTIME_DIR=/tmp/xdg WAYLAND_DISPLAY=wayland-0 setsid /var/lib/lumatv/bin/lumatv-gfn-feeder --serve /tmp/gfn/m1.sock --once -w video -a org.nphil.lumatv.cameras > /tmp/gfn/m1.out 2>&1 < /dev/null & sleep 3; cat /tmp/gfn/m1.out | cut -c1-300", 30).strip())
    m = re.search(r"_Window_Id_\d+", tv.tv("cat /tmp/gfn/m1.out", 10)); W = m.group(0) if m else None
    say("W = %s" % W)
    say("claim: " + json.dumps(R.ctl("vwin claim m1 pip 1300 60 560 315 org.nphil.lumatv.cameras h264 1280x720@30")["claim"])[:300])
    say("videooutput before load: %s" % vo())
    r = tv.tv("""luna-send -n 1 -f luna://com.webos.media/load '{"uri":"file:///tmp/gfn/clips/cam720p30.mp4","type":"media","payload":{"option":{"appId":"com.webos.app.home","windowId":"%s"}}}'""" % W, 30)
    say("load: " + r.replace("\n", " ")[:500])
    m = re.search(r'"mediaId"\s*:\s*"([^"]+)"', r); M = m.group(1) if m else None
    if M:
        say("play: " + tv.tv("""luna-send -n 1 -f luna://com.webos.media/play '{"mediaId":"%s"}'""" % M, 30).replace("\n", " ")[:300])
    time.sleep(8)
    say("videooutput after 8 s: %s" % vo())
    say("pipelines: " + tv.tv("luna-send -n 1 -f luna://com.webos.media/getActivePipelines '{}' | tr -d '\\n ' | head -c 2200", 20))
    say("mvc: " + tv.tv("luna-send -n 1 -f luna://com.webos.service.multiviewcontroller/getStatus '{}' | tr -d '\\n ' | head -c 400", 20))
    say("claims: " + json.dumps([{k: c.get(k) for k in ("app","slot","placed","sink","state")} for c in R.ctl("vwin")["vwin"]["claims"]]))
    subprocess.run("python3 /tmp/r21-build/shot.py m1 --luna", shell=True, env={"SHOTDIR": IMG, "PATH": "/usr/bin:/bin"})
    D = "/tmp/lumatv-r21/docs/research/multiview/tv-2026-10-30"
    out = tv.tv("(zcat /var/log/messages.*.gz 2>/dev/null; cat /var/log/messages) | grep -a -E 'starfish-media-pipeline.*(avc\\.|multiviewclient|ACQUIRE|_changeSink)|policy action|setSubType|doConnect\\]' | tail -60 | cut -c1-340", 60)
    open(D + "/m1-messages.txt", "w").write(out); say("messages lines: %d" % len(out.splitlines()))
finally:
    if M: tv.tv("""luna-send -n 1 -f luna://com.webos.media/unload '{"mediaId":"%s"}'""" % M, 20)
    say("fg off: " + str(R.ctl("vwin fg off"))[:60])
    say(pty("sh /tmp/lumatv/hwplanes.sh stop 2>&1 | tail -1; pkill -f 'lumatv-gfn-feeder --serve' ; true", 60).strip()[:150])
    R.ctl("vwin idle")
    say("yt end: " + json.dumps(R.ctl("shell")["rust"]["topics"]["foreground"]))
