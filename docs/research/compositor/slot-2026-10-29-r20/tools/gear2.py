import sys,json,time,re,subprocess
sys.path.insert(0,'/tmp/r20-build')
import rustlib as R, tv
LT="/media/developer/apps/usr/palm/applications/org.nphil.lumatv/tools/lumatv"
def say(s): print(time.strftime("%T"),s,flush=True)
def st():
    r=R.ctl("shell").get("rust",{}); u=r.get("ui") or {}
    return {"quick":(u.get("panels") or {}).get("quick"),"settings":(u.get("settings") or {}).get("open"),"drawer":(u.get("panels") or {}).get("drawer"),"fg":(r.get("topics") or {}).get("foreground",{}).get("app")}
def wins(): return [a for w in R.ctl("windows").get("windows",[]) for a in [w["appId"]]]
def nl(): return int(tv.tv("wc -l < /var/lib/lumatv/compositor/compositor.log",20).strip() or 0)
def jl(n0): return " || ".join(l[:90] for l in tv.tv("tail -n +%d /var/lib/lumatv/compositor/compositor.log | grep -a -E 'gear|AV reset'|cut -c1-160"%(n0+1),20).splitlines())[:300]
def hold(): R.ctl("remote 139 press"); time.sleep(1.4); R.ctl("remote 139 release"); time.sleep(1.5)
def back(n=4):
    for _ in range(n): R.ctl("remote 158 tap"); time.sleep(0.8)
def restore():
    subprocess.run(["python3","/tmp/lt/tv.py","put","/tmp/r20/theme.json.bak","/var/lib/lumatv/theme/theme.json"],capture_output=True); tv.tv("chmod 644 /var/lib/lumatv/theme/theme.json",10)
try:
    # hold on Home opens, second hold closes
    n=nl(); hold(); say("hold on Home: %s | %s"%(st(),jl(n)))
    n=nl(); hold(); say("second hold: %s | %s"%(st(),jl(n)))
    back(3)
    # over an app
    tv.tv("luna-send -n 1 luna://com.webos.applicationManager/launch '{\"id\":\"com.webos.app.settings\"}'" if False else "luna-send -n 1 luna://com.webos.applicationManager/launch '{\"id\":\"youtube.leanback.v4\"}'",20); time.sleep(10)
    say("YouTube up: %s windows %s"%(st(),wins()))
    n=nl(); hold(); say("hold over app: %s | %s | tvhotkey %s lgsettings %s"%(st(),jl(n),'com.webos.app.tvhotkey' in wins(),'com.palm.app.settings' in wins()))
    n=nl(); hold(); say("second hold over app: %s | %s"%(st(),jl(n)))
    back(3)
    tv.tv("luna-send -n 1 luna://com.webos.applicationManager/closeByAppId '{\"id\":\"youtube.leanback.v4\"}'",20); time.sleep(3)
    R.ctl("remote 102 tap"); time.sleep(3); say("home: %s"%st())
    # Hold gear options
    for w in ("notifications","nothing"):
        tv.tv("%s theme set '{\"remote\":{\"gearHold\":\"%s\"}}' >/dev/null 2>&1"%(LT,w),30); time.sleep(3)
        n=nl(); hold(); say("gearHold=%s: %s windows %s | %s"%(w,st(),[a for a in wins() if 'lumatv' not in a],jl(n))); back(3)
finally:
    restore(); time.sleep(3); back(2)
    say("md5 "+tv.tv("md5sum /var/lib/lumatv/theme/theme.json",10).split()[0]+" final "+json.dumps(st()))
