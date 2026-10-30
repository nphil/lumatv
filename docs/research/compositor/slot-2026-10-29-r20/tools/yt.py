import sys,json,time,re,collections
sys.path.insert(0,'/tmp/lumatv-r20/docs/research/arch')
import tv
CTL="/tmp/lumatv/lib/ld-linux-aarch64.so.1 --library-path /tmp/lumatv/lib /tmp/lumatv/bin/lumatv-compositor ctl --runtime-dir /tmp/xdg"
def ctl(a,t=30): return tv.tv("%s %s 2>/dev/null"%(CTL,a),t)
app=sys.argv[1] if len(sys.argv)>1 else 'youtube'
secs=int(sys.argv[2]) if len(sys.argv)>2 else 20
ids={'youtube':('youtube.leanback.v4','{"id":"youtube.leanback.v4","params":{"contentTarget":"https://www.youtube.com/tv?v=aqz-KE-bpKQ"}}')}
vid,launch=ids[app]
tv.tv("luna-send -n 1 luna://com.webos.applicationManager/launch '%s'"%launch,20)
for i in range(45):
    if '"video":[{' in ctl("windows").replace(' ',''): break
    time.sleep(1)
else: print("no video"); 
time.sleep(8)
ctl("refresh reset"); ctl("note probe-start")
time.sleep(secs)
ring=ctl("ring 8000",60); ref=ctl("refresh"); wk=ctl("wake")
open('/tmp/r20/probe-ring.json','w').write(ring); open('/tmp/r20/probe-refresh.json','w').write(ref)
print(ref[:600])
try: L=json.loads(ring[ring.index('{'):])['lines']
except Exception as e: print('ring parse',e,ring[:200]); L=[]
print(len(L), L[0][:30] if L else '', L[-1][:30] if L else '')
D=[l for l in L if ' D frame ' in l]
print('D frames',len(D), collections.Counter(re.search(r'\(([^)]*)\)$',l).group(1) for l in D if re.search(r'\(([^)]*)\)$',l)))
for l in D[:10]: print(l[:170])
for l in [l for l in L if ' S ticks' in l][-3:]: print(l[:300])
tv.tv("luna-send -n 1 luna://com.webos.applicationManager/closeByAppId '{\"id\":\"%s\"}'"%vid,20)
time.sleep(3)
tv.tv("luna-send -n 1 luna://com.webos.applicationManager/launch '{\"id\":\"com.webos.app.home\"}'",20)
