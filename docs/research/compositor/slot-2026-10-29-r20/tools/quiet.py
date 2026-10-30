import sys,time,json
sys.path.insert(0,'/tmp/lumatv-r20/docs/research/arch')
import tv
CTL="/tmp/lumatv/lib/ld-linux-aarch64.so.1 --library-path /tmp/lumatv/lib /tmp/lumatv/bin/lumatv-compositor ctl --runtime-dir /tmp/xdg"
last=None; lastchange=time.time()
while True:
    try:
        o=tv.tv("%s wake 2>/dev/null"%CTL,20); n=json.loads(o[o.index('{'):])["input"]["n"]
    except Exception as e:
        n=None
    if n is not None and last is not None and n!=last: lastchange=time.time()
    if n is not None: last=n
    open('/tmp/r20/quiet.txt','w').write("%s n=%s quiet_for=%d s\n"%(time.strftime("%T"),n,time.time()-lastchange))
    time.sleep(30)
