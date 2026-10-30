import sys,json,time,re
sys.path.insert(0,'/tmp/lumatv-r20/docs/research/arch')
import tv
CTL="/tmp/lumatv/lib/ld-linux-aarch64.so.1 --library-path /tmp/lumatv/lib /tmp/lumatv/bin/lumatv-compositor ctl --runtime-dir /tmp/xdg"
def ctl(a,t=30): return tv.tv("%s %s 2>/dev/null"%(CTL,a),t)
def bl():
    o=tv.tv('luna-send -n 1 luna://com.webos.service.settings/getSystemSettings \'{"category":"picture","keys":["backlight"]}\'',10)
    m=re.search(r'"backlight":"(\d+)"',o); return int(m.group(1)) if m else None
def pdj():
    o=ctl("paneldim"); return json.loads(o[o.index('{'):])['panelDim']
def say(s): print(time.strftime("%T"),s,flush=True)
