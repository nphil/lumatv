import sys,time,json
sys.argv=['x']
import hwchain as H
tv=H.tv
def feeder_bg(app,secs=60):
    tv.tv("cd /tmp/gfn; XDG_RUNTIME_DIR=/tmp/xdg WAYLAND_DISPLAY=wayland-0 setsid nohup /tmp/gfn/lumatv-gfn-feeder --play /tmp/gfn/clips/cam720p30.h264 --size 1280x720 --codec h264 --fps 30 --seconds %d --loop -w video -a %s > /tmp/r20feeder-%s.log 2>&1 < /dev/null &"%(secs,app,app),20)
H.ctl("vwin","apps"); feeder_bg(H.APP1); time.sleep(3)
print(tv.tv("ps -ef | grep -c '[l]umatv-gfn-feeder'",10))
H.ctl("vwin","claim","t1","pip","1280","720","480","270",H.APP1,"h264","1280x720@30")
time.sleep(4); H.hw("vwin"); H.hw("shot","v2b")
print(tv.tv("D=/tmp/lumatv; $D/lib/ld-linux-aarch64.so.1 --library-path $D/lib $D/bin/lumatv-compositor ctl --runtime-dir /tmp/xdg windows 2>/dev/null | python3 -c \"import sys,json;j=json.load(sys.stdin);print([(w['appId'],w['type']) for w in j['windows']], j['video'])\"; tail -3 /tmp/r20feeder-%s.log | cut -c1-200"%H.APP1,30))
