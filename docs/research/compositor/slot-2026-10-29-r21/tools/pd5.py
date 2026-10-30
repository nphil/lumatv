import sys, time, json, subprocess
sys.path.insert(0, "/tmp/r21-build"); sys.path.insert(0, "/tmp/lumatv-r20/docs/research/arch")
import tv, rustlib as R
T = "/media/developer/apps/usr/palm/applications/org.nphil.lumatv/tools/lumatv"
OUT = "/tmp/r21/pd5"; import os; os.makedirs(OUT, exist_ok=True)
def say(s):
    print(time.strftime("%T"), s, flush=True)
def backlight():
    o = tv.tv("luna-send -n 1 luna://com.webos.service.settings/getSystemSettings '{\"category\":\"picture\",\"keys\":[\"backlight\"]}'", 20)
    try: return int(json.loads(o)["settings"]["backlight"])
    except Exception: return o[:80]
def oc():
    st = R.ctl("status"); o = st["oledcare"]; pd = o["panelDim"]; sc = R.ctl("scanout")["scanout"]["frames"]
    return {"beats": o["beats"], "homeWant": o["homeWant"], "dim": o["dim"], "written": pd["written"], "writes": pd["writes"], "reads": pd["reads"], "restores": pd["restores"], "carries": pd["carries"], "dimmed": pd["dimmed"], "block": pd["block"], "composed": sc["composed"], "direct": sc["direct"], "uiDrawn": st.get("stats", {}).get("uiDrawn")}
start = backlight(); say("backlight start %s picture %s" % (start, tv.tv("luna-send -n 1 luna://com.webos.service.settings/getSystemSettings '{\"category\":\"picture\",\"keys\":[\"pictureMode\"]}' | grep pictureMode | head -1", 20).strip()))
say("theme before oledcare: " + json.dumps(json.loads(tv.tv("cat /var/lib/lumatv/theme/theme.json", 10)).get("oledcare")))
rows = []
try:
    tv.tv("%s theme set '{\"oledcare\":{\"enabled\":true,\"dimAfterMin\":1,\"dimOverMin\":2,\"stepSec\":10}}' 2>&1" % T, 30)
    R.ctl("wake reset")
    time.sleep(3); t0 = time.time(); say("start " + json.dumps(oc()))
    engaged = None; last = None
    while time.time() - t0 < 330:
        time.sleep(10); r = oc(); r["t"] = int(time.time() - t0); r["backlight"] = backlight(); rows.append(r)
        say(json.dumps(r))
        if r["written"] is not None and r["written"] <= int(start * r["homeWant"]) + 1 and engaged is None and r["homeWant"] < 0.9:
            engaged = time.time()
        if engaged and time.time() - engaged > 70: break
    json.dump(rows, open(OUT + "/rows.json", "w"))
    # step 11 while dimmed: 60 s wake window
    R.ctl("wake reset"); a = oc(); time.sleep(60); b = oc(); w = R.ctl("wake")
    json.dump({"before": a, "after": b, "wake": w}, open(OUT + "/step11-dimmed.json", "w"), indent=1)
    say("step11 dimmed 60 s: writes %s -> %s reads %s -> %s | %s" % (a["writes"], b["writes"], a["reads"], b["reads"], w.get("line")))
    c = oc(); say("before key " + json.dumps(c)); bl = backlight()
    R.ctl("remote 158 tap"); t1 = time.time()
    for i in range(30):
        time.sleep(0.2); x = backlight()
        if x == start: break
    say("key restore: backlight %s after %.1f s (start %s) %s" % (x, time.time() - t1, start, json.dumps(oc())))
finally:
    R.ctl("paneldim release")
    subprocess.run(["python3", "/tmp/lt/tv.py", "put", "/tmp/r21/theme.json.bak", "/var/lib/lumatv/theme/theme.json"], capture_output=True)
    tv.tv("chmod 644 /var/lib/lumatv/theme/theme.json", 10); time.sleep(4)
    say("end backlight %s (start %s); theme md5 %s" % (backlight(), start, tv.tv("md5sum /var/lib/lumatv/theme/theme.json", 10).split()[0]))
