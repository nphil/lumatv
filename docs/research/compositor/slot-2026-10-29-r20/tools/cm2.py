import sys,json,time
sys.path.insert(0,'/tmp/r20')
from pd import *
def snap():
    o=ctl("status"); j=json.loads(o[o.index('{'):]); return time.time(), j['stats']
t0,a=snap(); time.sleep(20); t1,b=snap()
print({k:round((b[k]-a[k])/(t1-t0),2) for k in ('commits','drawn','callbacks','ticks','releases','requests')})
