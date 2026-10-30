from pd import *
v=float(sys.argv[1]); n=int(sys.argv[2])
say("start backlight %s"%bl())
ctl("paneldim want %s"%v)
try:
    for i in range(n):
        time.sleep(1.5); p=pdj(); say("bl %s written %s writes %s dimmed %s err %r"%(bl(),p['written'],p['writes'],p['dimmed'],p['lastError']))
finally:
    t0=time.time(); ctl("paneldim release"); 
    for i in range(10):
        b=bl(); 
        if b==80: break
    say("released: bl %s after %.2f s  %s"%(b,time.time()-t0,tv.tv("ls /var/lib/lumatv/paneldim.json 2>&1; grep -a 'panel' /var/lib/lumatv/compositor/compositor.log | tail -4 | cut -c1-200",10)))
