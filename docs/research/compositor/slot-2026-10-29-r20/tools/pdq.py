from pd import *
mode=sys.argv[1]
def setbl(v): pass
ctl("paneldim want 0.8"); time.sleep(9); p=pdj(); say("dimmed bl %s written %s"%(bl(),p['written']))
pid=tv.tv("ps -ef | grep 'lumatv-compositor' | grep -v grep | grep -v qml | awk '$3=='$(pidof surface-manager)' {print $2}' | head -1",10).strip(); say("pid %r"%pid)
t0=time.time()
if mode=="quit": say("quit: "+ctl("quit").strip()[:100])
else: tv.tv("kill -9 %s"%pid,10); say("kill -9 sent")
for i in range(30):
    b=bl(); say("t+%.1f bl %s file-present %s"%(time.time()-t0,b,tv.tv("test -f /var/lib/lumatv/paneldim.json && echo yes || echo no",10).strip()))
    if b==80 and i>1: break
    time.sleep(3)
