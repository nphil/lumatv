import sys, time, json, subprocess
sys.path.insert(0, "/tmp/r21-build"); sys.path.insert(0, "/tmp/lumatv-r20/docs/research/arch")
import tv, rustlib as R
T = "/media/developer/apps/usr/palm/applications/org.nphil.lumatv/tools/lumatv"
def say(s): print(time.strftime("%T"), s, flush=True)
def backlight():
    o = tv.tv("luna-send -n 1 luna://com.webos.service.settings/getSystemSettings '{\"category\":\"picture\",\"keys\":[\"backlight\"]}'", 20)
    try: return int(json.loads(o)["settings"]["backlight"])
    except Exception: return o[:80]
def pd(): 
    o = R.ctl("status")["oledcare"]; p = o["panelDim"]; return {k: p[k] for k in ("written","writes","reads","restores","dimmed","want")} | {"homeWant": o["homeWant"]}
start = backlight(); say("start backlight %s" % start)
try:
    tv.tv("%s theme set '{\"oledcare\":{\"enabled\":true,\"dimAfterMin\":1,\"dimOverMin\":2,\"stepSec\":10}}' 2>&1" % T, 30)
    R.ctl("wake reset"); t0 = time.time()
    while time.time() - t0 < 200:
        time.sleep(5)
        p = pd()
        if p["dimmed"] and p["written"] is not None and p["written"] <= 69: break
    say("dimmed: %s backlight %s after %d s" % (p, backlight(), time.time() - t0))
    say("release: " + str(R.ctl("paneldim release"))[:300]); time.sleep(1.5)
    say("after release: %s backlight %s" % (pd(), backlight()))
    R.ctl("wake reset"); time.sleep(60); w = R.ctl("wake")
    say("wake 60 s after release: " + w["line"]); say("state: %s backlight %s" % (pd(), backlight()))
    json.dump({"wake": w, "state": pd()}, open("/tmp/r21/step11-after-release.json", "w"), indent=1)
finally:
    subprocess.run(["python3", "/tmp/lt/tv.py", "put", "/tmp/r21/theme.json.bak", "/var/lib/lumatv/theme/theme.json"], capture_output=True)
    tv.tv("chmod 644 /var/lib/lumatv/theme/theme.json", 10); time.sleep(4); R.ctl("paneldim release"); time.sleep(2)
    say("end backlight %s (start %s) %s" % (backlight(), start, pd()))
