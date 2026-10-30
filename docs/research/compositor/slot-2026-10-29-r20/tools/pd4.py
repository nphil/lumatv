from pd import *
LT="/media/developer/apps/usr/palm/applications/org.nphil.lumatv/tools/lumatv"
def restore_theme():
    subprocess_put=__import__('subprocess').run(["python3","/tmp/lt/tv.py","put","/tmp/r20/theme.json.bak","/var/lib/lumatv/theme/theme.json"])
    tv.tv("chmod 644 /var/lib/lumatv/theme/theme.json; md5sum /var/lib/lumatv/theme/theme.json",10)
say("bl %s"%bl())
say(tv.tv("%s theme set '{\"screensaver\":{\"dimOverMin\":1}}' 2>&1 | tail -2"%LT,30).strip()[:200])
time.sleep(3)
try:
    tv.tv("luna-send -n 1 luna://com.webos.service.tvpower/power2/turnOnScreenSaver '{}'",10)
    for i in range(14):
        time.sleep(5); p=pdj(); 
        sh=json.loads(ctl("shell")[ctl("shell").index('{'):]).get('rust',{}); ss=((sh.get('ui') or {}).get('saverState') or {})
        say("bl %s written %s writes %s carry %s level %s saver %s"%(bl(),p['written'],p['writes'],ss.get('carry'),ss.get('level'),(sh.get('ui') or {}).get('saver')))
        if p['written'] is not None and i==8: say(tv.tv("ls -la /var/lib/lumatv/compositor/paneldim.json /var/lib/lumatv/paneldim.json 2>&1",10).strip())
finally:
    t0=time.time(); ctl("remote 106 tap")
    for i in range(20):
        b=bl()
        if b==80: break
    p=pdj(); say("key: bl %s after %.2f s restores %s"%(b,time.time()-t0,p['restores']))
    time.sleep(2); restore_theme(); say("theme md5 "+tv.tv("md5sum /var/lib/lumatv/theme/theme.json",10).strip()); say("bl %s"%bl())
