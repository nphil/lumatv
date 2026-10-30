import sys,time
sys.argv=['x']
import hwchain as H
tv=H.tv
try:
    H.ctl("vwin","apps"); H.feeder(H.APP1,50)
    H.ctl("claim" and "vwin","claim","t1","pip","1280","720","480","270",H.APP1,"h264","1280x720@30")
    for i in range(3):
        time.sleep(3)
        print(tv.tv("luna-send -n 1 -f luna://com.webos.service.videooutput/getStatus '{}' | tr -s ' \\n' ' ' | cut -c1-1500; echo; D=/tmp/lumatv; $D/lib/ld-linux-aarch64.so.1 --library-path $D/lib $D/bin/lumatv-compositor ctl --runtime-dir /tmp/xdg vwin 2>/dev/null | tr -d '\\n' | cut -c1-1100; echo; grep -a 'vwin\\|videooutput' /var/lib/lumatv/compositor/compositor.log | tail -5 | cut -c1-230",30))
    H.hw("shot","v2c")
finally:
    H.hw("stop")
