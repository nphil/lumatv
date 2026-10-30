import sys, time, json, subprocess
sys.path.insert(0, "/tmp/r21-build"); sys.path.insert(0, "/tmp/lumatv-r21/docs/research/arch")
import tv, rustlib as R
from mv import pty
IMG = "/tmp/lumatv-r21/docs/research/compositor/slot-2026-10-29-r21/img"
mode = sys.argv[1]   # home | yt
def say(s): print(time.strftime("%T"), s, flush=True)
def post():
    return tv.tv("""curl -s -X POST http://127.0.0.1:18731/v1/cameras/event -H 'Content-Type: application/json' -H 'X-Luma-Client: r21' -d '{"camera":"240","kind":"animal"}' | head -c 300""", 20).strip()
def cams(): return (R.ctl("shell")["rust"]["ui"].get("cameras") or {}), (R.ctl("shell")["rust"]["ui"].get("popup") or {})
say("kinds+: " + str(R.ctl("shell rust +popup +cameras")).replace("\n", " ")[:300])
time.sleep(5)
R.ctl("vwin idle")
if mode == "yt":
    subprocess.run(["python3", "/tmp/r21-build/ytplay.py"])
else:
    tv.tv("""luna-send -n 1 luna://com.webos.applicationManager/launch '{"id":"com.webos.app.home"}'""", 20); time.sleep(3)
say("elements: " + str([(e["app"], e["counted"], e["playing"]) for e in R.ctl("vwin")["vwin"]["elements"]]))
t0 = time.time(); say("POST: " + post())
seen = {}
for i in range(60):
    time.sleep(0.25)
    u = R.ctl("shell")["rust"]["ui"]; p = u.get("popup") or {}
    live = p.get("live") if isinstance(p, dict) else None
    key = json.dumps(live, sort_keys=True)[:200]
    if key not in seen:
        seen[key] = time.time() - t0; say("+%.2fs popup.live %s" % (seen[key], key))
    if isinstance(live, dict) and live.get("video"): break
time.sleep(5)
subprocess.run("python3 /tmp/r21-build/shot.py popup-%s --ctl" % mode, shell=True, env={"SHOTDIR": IMG, "PATH": "/usr/bin:/bin"})
say("vwin: " + json.dumps([ {k: c.get(k) for k in ("app","slot","placed","sink","texture","state","text")} for c in R.ctl("vwin")["vwin"]["claims"]]))
time.sleep(1)
say("popup state: " + json.dumps((R.ctl("shell")["rust"]["ui"].get("popup")))[:700])
say("helper log: " + tv.tv("grep -a -E 'live for pop-up|ended:|first picture|cameras:' /var/lib/lumatv/log-helper | tail -6 | cut -c1-300", 15).strip().replace("\n", " | "))
# Back
tb = time.time(); R.ctl("remote 158 tap")
for i in range(20):
    time.sleep(0.05)
    p = (R.ctl("shell")["rust"]["ui"].get("popup") or {})
    if not p.get("up") and not p.get("live", {}).get("phase") in ("asked", "playing"): break
say("Back -> popup gone after %.2f s: %s ; claims %s" % (time.time() - tb, json.dumps(R.ctl("shell")["rust"]["ui"].get("popup"))[:200], [c["id"] for c in R.ctl("vwin")["vwin"]["claims"]]))
time.sleep(3)
say("helper log after: " + tv.tv("grep -a -E 'ended:' /var/lib/lumatv/log-helper | tail -2 | cut -c1-400", 15).strip().replace("\n", " | "))
