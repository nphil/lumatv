from pd import *
def setbl(v):
    return tv.tv('luna-send -n 1 luna://com.webos.service.settings/setSystemSettings \'{"category":"picture","settings":{"backlight":"%d"},"dimension":{"input":"default","pictureMode":"filmMaker"}}\''%v,10).strip()
say("bl %s"%bl())
ctl("paneldim want 0.8")
try:
    time.sleep(7); p=pdj(); say("dimmed: bl %s written %s"%(bl(),p['written']))
    say("their set 75: "+setbl(75))
    for i in range(14):
        time.sleep(2); p=pdj(); say("bl %s written %s dimmed %s want %s writes %s reads %s"%(bl(),p['written'],p['dimmed'],p['want'],p['writes'],p['reads']))
    say(tv.tv("grep -a 'paneldim' /var/lib/lumatv/compositor/compositor.log | tail -3 | cut -c1-200; ls /var/lib/lumatv/paneldim.json 2>&1",10).strip())
finally:
    ctl("paneldim release"); time.sleep(1); say("their set 80: "+setbl(80)); time.sleep(2); say("end bl %s %s"%(bl(), pdj()['dimmed']))
