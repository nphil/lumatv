import sys, time, json
sys.path.insert(0, "/tmp/r21-build"); sys.path.insert(0, "/tmp/lumatv-r20/docs/research/arch")
import tv
T = "/media/developer/apps/usr/palm/applications/org.nphil.lumatv/tools/lumatv"
def hs(): 
    o = tv.tv("%s helper status 2>&1" % T, 20).strip(); 
    try: return json.loads(o)["helper"]
    except Exception: return o
def say(s): print(time.strftime("%T"), s, flush=True)
h = hs(); say("before %s" % h)
w = h["worker"]; sup = h["pid"]
say("kill -9 %s" % w); tv.tv("kill -9 %s" % w, 10); t0 = time.time()
for i in range(40):
    time.sleep(0.5); h2 = hs()
    if isinstance(h2, dict) and h2.get("worker") not in (None, w) and h2.get("running"):
        say("RESTART after %.1f s: %s" % (time.time()-t0, h2)); break
else: say("NO RESTART %s" % h2)
say("log: " + tv.tv("grep -a 'supervisor' /var/lib/lumatv/log-helper | tail -3 | cut -c1-260", 10).strip())
time.sleep(3); say("healthy after: %s" % hs())
# stop stays stopped
say("stop: " + tv.tv("%s helper stop 2>&1; echo rc=$?" % T, 40).strip().replace("\n"," | "))
time.sleep(8)
st = tv.tv("%s helper status 2>&1; echo rc=$?; ps | grep lumatv-helper | grep -v grep" % T, 20).strip().replace("\n"," | ")
say("after stop +8 s: " + st)
time.sleep(20)
st = tv.tv("%s helper status 2>&1; echo rc=$?; ps | grep lumatv-helper | grep -v grep" % T, 20).strip().replace("\n"," | ")
say("after stop +28 s: " + st)
say("start: " + tv.tv("%s helper start 2>&1; echo rc=$?" % T, 40).strip().replace("\n"," | "))
time.sleep(5); say("after start: %s" % hs())
