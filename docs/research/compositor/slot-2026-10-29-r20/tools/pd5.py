from pd import *
LT="/media/developer/apps/usr/palm/applications/org.nphil.lumatv/tools/lumatv"
def restore_theme():
    __import__('subprocess').run(["python3","/tmp/lt/tv.py","put","/tmp/r20/theme.json.bak","/var/lib/lumatv/theme/theme.json"],capture_output=True)
    tv.tv("chmod 644 /var/lib/lumatv/theme/theme.json",10)
def oc():
    o=ctl("status"); return json.loads(o[o.index('{'):])
say("bl %s"%bl())
tv.tv("%s theme set '{\"oledcare\":{\"enabled\":true,\"dimAfterMin\":1,\"dimOverMin\":2,\"stepSec\":10}}' 2>&1 | tail -1 | cut -c1-80"%LT,30)
ctl("wake reset")
try:
    for i in range(18):
        time.sleep(10); j=oc(); o=j['oledcare']; p=o['panelDim']; st=j.get('scanout')
        say("bl %s homeWant %s dim %s written %s writes %s reads %s scanout %s"%(bl(),o.get('homeWant'),o.get('dim'),p['written'],p['writes'],p['reads'],json.dumps(st)[:160]))
    w=ctl("wake"); say("wake "+w[w.index('{'):][:400])
finally:
    t0=time.time(); ctl("remote 106 tap")
    for i in range(20):
        b=bl()
        if b==80: break
    say("key: bl %s after %.2f s"%(b,time.time()-t0)); time.sleep(2); restore_theme(); say("md5 "+tv.tv("md5sum /var/lib/lumatv/theme/theme.json",10).strip()+" bl %s"%bl())
