import sys,time,subprocess,json
sys.path.insert(0,'/tmp/r20-build')
import rustlib as R, tv
LT="/media/developer/apps/usr/palm/applications/org.nphil.lumatv/tools/lumatv"
def say(s): print(time.strftime("%T"),s,flush=True)
def st():
    u=R.ctl("shell").get("rust",{}).get("ui",{}); return (u.get("panels") or {}).get("quick")
say("keys.conf: "+tv.tv("cat /var/lib/lumatv/compositor/keys.conf /var/lib/lumatv/keys.conf 2>&1 | head -5",10).strip()[:200])
try:
    say("quick before %s"%st())
    tv.tv("%s theme set '{\"remote\":{\"buttons\":{\"red\":\"quick\"}}}' >/dev/null 2>&1"%LT,30); time.sleep(3)
    n=int(tv.tv("wc -l < /var/lib/lumatv/compositor/compositor.log",10).strip())
    R.ctl("remote 398 tap"); time.sleep(1.5)
    say("red tap -> quick %s | %s"%(st(), tv.tv("tail -n +%d /var/lib/lumatv/compositor/compositor.log | grep -a -i 'button\\|red\\|input:' | cut -c1-200 | tail -3"%(n+1),10).strip()))
    R.ctl("remote 158 tap"); time.sleep(1)
finally:
    subprocess.run(["python3","/tmp/lt/tv.py","put","/tmp/r20/theme.json.bak2","/var/lib/lumatv/theme/theme.json"],capture_output=True); tv.tv("chmod 644 /var/lib/lumatv/theme/theme.json",10)
    time.sleep(3); say("md5 "+tv.tv("md5sum /var/lib/lumatv/theme/theme.json",10).split()[0]+" quick %s"%st())
