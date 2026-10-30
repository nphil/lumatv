import sys,time
sys.argv=['x']
import hwchain as H
tv=H.tv
APP=sys.argv_app if hasattr(sys,'argv_app') else None
def probe(app, label):
    H.say("=== probe %s app=%s"%(label,app))
    t0=tv.tv("grep -a -c '' /var/lib/lumatv/compositor/compositor.log",10).strip()
    try:
        H.ctl("vwin","apps")
        H.feeder(app,60)
        H.ctl("vwin","claim","t1","pip","1280","720","480","270",app,"h264","1280x720@30")
        time.sleep(3.5)
        print(tv.tv("luna-send -n 1 -f luna://com.webos.service.videooutput/getStatus '{}' | python3 -c \"import sys,json;j=json.load(sys.stdin);[print({k:s.get(k) for k in ('sink','connected','appId','width','height','displayOutput','zOrder','context')}) for s in j['video']]\" 2>&1 | head -6; D=/tmp/lumatv; $D/lib/ld-linux-aarch64.so.1 --library-path $D/lib $D/bin/lumatv-compositor ctl --runtime-dir /tmp/xdg vwin 2>/dev/null | python3 -c \"import sys,json;j=json.load(sys.stdin)['vwin'];print(json.dumps({k:j[k] for k in ('claims','windows','sinks','world')})[:1800])\"; tail -n +%s /var/lib/lumatv/compositor/compositor.log | grep -a -E 'foreign:|vwin:|punch|videooutput' | cut -c1-260 | tail -14"%t0,40))
        H.hw("shot","probe-"+label)
    finally:
        H.hw("stop")
probe("com.webos.app.home","home")
H.say("youtube front")
tv.tv("luna-send -n 1 luna://com.webos.applicationManager/launch '{\"id\":\"youtube.leanback.v4\",\"params\":{\"contentTarget\":\"https://www.youtube.com/tv?v=aqz-KE-bpKQ\"}}'",20); time.sleep(20)
try: probe("youtube.leanback.v4","yt")
finally: H.home()
H.fetch(); H.say("probe over")
