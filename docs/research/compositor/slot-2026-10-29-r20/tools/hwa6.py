import sys,time
sys.argv=['x']
import hwchain as H
tv=H.tv
D="D=/tmp/lumatv; $D/lib/ld-linux-aarch64.so.1 --library-path $D/lib $D/bin/lumatv-compositor ctl --runtime-dir /tmp/xdg"
out=open('/tmp/r20/camprobe.txt','w')
def w(s): out.write(s+"\n"); out.flush()
t0=tv.tv("wc -l < /var/lib/lumatv/compositor/compositor.log",10).strip()
try:
    H.ctl("vwin","apps"); H.feeder("com.webos.app.home",60)
    H.ctl("vwin","claim","t1","pip","1280","720","480","270","com.webos.app.home","h264","1280x720@30")
    time.sleep(4)
    w("== VIDEO capture"); w(tv.tv("rm -f /tmp/v.png; luna-send -n 1 luna://com.webos.service.capture/executeOneShot '{\"path\":\"/tmp/v.png\",\"method\":\"VIDEO\",\"format\":\"PNG\",\"width\":960,\"height\":540}' 2>&1; ls -la /tmp/v.png 2>&1",30))
    w("== DISPLAY capture"); w(tv.tv("rm -f /tmp/d.png; luna-send -n 1 luna://com.webos.service.capture/executeOneShot '{\"path\":\"/tmp/d.png\",\"method\":\"DISPLAY\",\"format\":\"PNG\",\"width\":960,\"height\":540}' 2>&1; ls -la /tmp/d.png 2>&1",30))
    for n in ('v','d'):
        try: tv.get('/tmp/%s.png'%n,'/tmp/r20/img/cam-%s.png'%n)
        except Exception as e: w("get %s: %s"%(n,e))
    w("== videooutput getStatus"); w(tv.tv("luna-send -n 1 -f luna://com.webos.service.videooutput/getStatus '{}' 2>&1",30))
    w("== fb0"); w(tv.tv("cat /proc/lg/fb0/status 2>/dev/null | grep -i -E 'alpha|visible' | head; ls /sys/kernel/debug/ 2>/dev/null | head -0",20))
    w("== status pageFormat"); w(tv.tv("%s status 2>/dev/null | python3 -c \"import sys,json;j=json.load(sys.stdin);print(j['pageFormat'],j['videoHolesOpaque'])\""%D,20))
    w("== journal since claim"); w(tv.tv("tail -n +%s /var/lib/lumatv/compositor/compositor.log | grep -a -E 'setDisplayWindow|setAvMute|foreign:|vwin:' | cut -c1-400"%t0,20))
    w("== avmute"); w(tv.tv("luna-send -n 1 -f luna://com.webos.service.avmutemanager/getStatus '{}' 2>&1 | head -30",20))
    w("== feeder log"); w(tv.tv("grep -v '^\\[SDL2\\|^$\\|g_object_get' /tmp/lumatv/out/hwplanes/feeder-com.webos.app.home.log | cut -c1-300 | tail -12; tail -2 /tmp/lumatv/out/hwplanes/feeder-com.webos.app.home.jsonl | cut -c1-300",20))
finally:
    H.hw("stop"); out.close()
