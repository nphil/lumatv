import sys, time, json
sys.path.insert(0, "/tmp/r21-build"); sys.path.insert(0, "/tmp/lumatv-r20/docs/research/arch")
import rustlib as R, tv
D = "/tmp/lumatv-r21/docs/research/compositor/slot-2026-10-29-r21/img"
def pk():
    u = R.ctl("shell")["rust"]["ui"]; return (u.get("panels") or {}).get("picker"), u.get("layer")
def say(s): print(time.strftime("%T"), s, flush=True)
R.ctl("remote 241 tap"); time.sleep(2.5)
p, l = pk(); say("open: layer %s picker %s" % (l, json.dumps(p)[:1500]))
R.capture_png(D + "/r21-picker-0-open.png")
R.ctl("remote 106 tap"); time.sleep(1.2); R.capture_png(D + "/r21-picker-1-right.png"); say("after right: %s" % json.dumps(pk()[0])[:600])
R.ctl("remote 105 tap"); time.sleep(1.0); R.ctl("remote 105 tap"); time.sleep(1.2); R.capture_png(D + "/r21-picker-2-left-left.png"); say("after left x2: %s" % json.dumps(pk()[0])[:600])
R.ctl("remote 158 tap"); time.sleep(1.5); say("closed: layer %s" % pk()[1])
R.ctl("remote 241 tap"); time.sleep(2.5); R.capture_png(D + "/r21-picker-3-reopen-a.png"); say("reopen a: %s" % json.dumps(pk()[0])[:600])
R.ctl("remote 106 tap"); time.sleep(1.2); R.ctl("remote 158 tap"); time.sleep(1.5)
R.ctl("remote 241 tap"); time.sleep(2.5); R.capture_png(D + "/r21-picker-4-reopen-b.png"); say("reopen b: %s" % json.dumps(pk()[0])[:600])
R.ctl("remote 158 tap"); time.sleep(1.5); say("closed: layer %s" % pk()[1])
