import sys, time, json, subprocess
sys.path.insert(0, "/tmp/r21-build"); sys.path.insert(0, "/tmp/lumatv-r21/docs/research/arch")
import tv, rustlib as R
IMG = "/tmp/lumatv-r21/docs/research/compositor/slot-2026-10-29-r21/img"
mode = sys.argv[1]; cam = sys.argv[2] if len(sys.argv) > 2 else "240"
def say(s): print(time.strftime("%T"), s, flush=True)
def ui(): return R.ctl("shell")["rust"]["ui"]
n0 = int(tv.tv("wc -l < /var/lib/lumatv/compositor/compositor.log", 10).strip())
R.ctl("vwin idle")
if mode == "yt": subprocess.run(["python3", "/tmp/r21-build/ytplay.py"])
else:
    tv.tv("""luna-send -n 1 luna://com.webos.applicationManager/launch '{"id":"com.webos.app.home"}'""", 20); time.sleep(3)
tv.tv("""curl -s -X POST http://127.0.0.1:18731/v1/cameras/event -H 'Content-Type: application/json' -H 'X-Luma-Client: r21' -d '{"camera":"%s","kind":"animal"}' >/dev/null""" % cam, 20)
time.sleep(0.9)
say("popup before OK: " + json.dumps((ui().get("popup") or {}).get("live"))[:300])
t0 = time.time(); R.ctl("remote 28 tap"); say("OK pressed")
seen = set()
for i in range(60):
    time.sleep(0.2)
    u = ui(); c = u.get("cameras") or {}
    k = json.dumps({x: c.get(x) for x in ("firstLiveMs", "homeMs", "askedMs", "switched", "live", "camera")}, sort_keys=True)
    if k not in seen: seen.add(k); say("+%.1fs fg=%s cameras=%s" % (time.time() - t0, (R.ctl("shell")["rust"]["topics"]["foreground"] or {}).get("app"), k[:400]))
    if c.get("firstLiveMs"): break
time.sleep(4)
subprocess.run("python3 /tmp/r21-build/shot.py full-%s --ctl" % mode, shell=True, env={"SHOTDIR": IMG, "PATH": "/usr/bin:/bin"})
say("cameras: " + json.dumps(ui().get("cameras"))[:900])
say("vwin: " + json.dumps([{k: c.get(k) for k in ("app","slot","placed","sink","state")} for c in R.ctl("vwin")["vwin"]["claims"]]))
say("helper: " + tv.tv("grep -a -E 'live for full view|first picture|bridge answered' /var/lib/lumatv/log-helper | tail -3 | cut -c1-330", 15).strip().replace("\n", " | "))
say("journal: " + tv.tv("tail -n +%d /var/lib/lumatv/compositor/compositor.log | grep -a -E 'cameras:|vwin:' | cut -c1-220 | tail -8" % (n0 + 1), 20).strip().replace("\n", " | "))
tb = time.time(); R.ctl("remote 158 tap"); time.sleep(2.5)
say("after Back (%.1fs): fg=%s cameras=%s claims=%s" % (time.time() - tb, json.dumps(R.ctl("shell")["rust"]["topics"]["foreground"]), json.dumps({x: (ui().get("cameras") or {}).get(x) for x in ("switched", "live")}), [c["id"] for c in R.ctl("vwin")["vwin"]["claims"]]))
say("journal after: " + tv.tv("tail -n +%d /var/lib/lumatv/compositor/compositor.log | grep -a -E 'cameras:' | cut -c1-200 | tail -4" % (n0 + 1), 20).strip().replace("\n", " | "))
say("helper ended: " + tv.tv("grep -a -E 'ended:' /var/lib/lumatv/log-helper | tail -1 | cut -c1-330", 15).strip())
