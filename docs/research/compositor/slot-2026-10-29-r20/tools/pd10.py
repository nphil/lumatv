from pd import *
LT="/media/developer/apps/usr/palm/applications/org.nphil.lumatv/tools/lumatv"
def restore_theme():
    __import__('subprocess').run(["python3","/tmp/lt/tv.py","put","/tmp/r20/theme.json.bak","/var/lib/lumatv/theme/theme.json"],capture_output=True)
    tv.tv("chmod 644 /var/lib/lumatv/theme/theme.json",10)
ctl("paneldim want 0.8")
try:
    time.sleep(8); p=pdj(); say("dimmed bl %s written %s carries %s"%(bl(),p['written'],p['carries']))
    tv.tv("%s theme set '{\"oledcare\":{\"panelDim\":false}}' >/dev/null 2>&1"%LT,30)
    for i in range(5):
        time.sleep(1); p=pdj(); say("switch off: bl %s written %s carries %s enabled %s"%(bl(),p['written'],p['carries'],p['enabled']))
finally:
    ctl("paneldim release"); restore_theme(); time.sleep(3); p=pdj(); say("end bl %s carries %s enabled %s md5 %s"%(bl(),p['carries'],p['enabled'],tv.tv("md5sum /var/lib/lumatv/theme/theme.json",10).split()[0]))
