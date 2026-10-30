from pd import *
ctl("paneldim want 0.8"); time.sleep(9); p=pdj(); say("dimmed bl %s written %s"%(bl(),p['written']))
pid=tv.tv("ps -ef | grep 'lumatv-compositor' | grep -v grep | grep -v qml | awk '$3=='$(pidof surface-manager)' {print $2}' | head -1",10).strip()
tv.tv("kill -9 %s"%pid,10); say("kill -9 %s sent"%pid)
import subprocess
t0=time.time()
r=subprocess.Popen("cd /tmp/lumatv-r20 && PATH=/data/home/.local/bin:/data/home/.cargo/bin:$PATH timeout 150 sh compositor/tools/tv-first-light.sh cutover enable",shell=True,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True)
while r.poll() is None:
    time.sleep(3)
    try: say("t+%.0f bl %s"%(time.time()-t0,bl()))
    except Exception as e: say("err %s"%str(e)[:50])
say("enable done: "+r.stdout.read().strip().splitlines()[-1][:100]); time.sleep(3); say("bl %s"%bl())
print(tv.tv("grep -a 'panel' /var/lib/lumatv/log-helper | tail -2 | cut -c1-200; grep -a 'paneldim' /var/lib/lumatv/compositor/compositor.log | tail -3 | cut -c1-200",10))
