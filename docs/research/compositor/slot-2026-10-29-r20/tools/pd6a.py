from pd import *
LT="/media/developer/apps/usr/palm/applications/org.nphil.lumatv/tools/lumatv"
say("bl %s"%bl())
tv.tv("luna-send -n 1 luna://com.webos.service.tvpower/power2/turnOnScreenSaver '{}'",10)
try:
    time.sleep(25); p=pdj(); say("saver dim: bl %s written %s block %s"%(bl(),p['written'],p['block']))
    tv.tv("luna-send -n 1 luna://com.webos.applicationManager/launch '{\"id\":\"youtube.leanback.v4\",\"params\":{\"contentTarget\":\"https://www.youtube.com/tv?v=aqz-KE-bpKQ\"}}'",20)
    for i in range(14):
        time.sleep(2); p=pdj(); w=ctl("windows"); say("app launch: bl %s written %s block %s dimmed %s video %s"%(bl(),p['written'],p['block'],p['dimmed'],'"video":[{' in w.replace(' ','')))
    time.sleep(20); p=pdj(); say("video 20 s on: bl %s block %s dimmed %s writes %s restores %s"%(bl(),p['block'],p['dimmed'],p['writes'],p['restores']))
finally:
    tv.tv("luna-send -n 1 luna://com.webos.applicationManager/closeByAppId '{\"id\":\"youtube.leanback.v4\"}'",20); time.sleep(3)
    tv.tv("luna-send -n 1 luna://com.webos.applicationManager/launch '{\"id\":\"com.webos.app.home\"}'",20); ctl("remote 106 tap"); time.sleep(2); say("end bl %s"%bl())
