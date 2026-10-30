from pd import *
import subprocess
ctl("paneldim want 0.8"); time.sleep(9); p=pdj(); say("dimmed bl %s written %s"%(bl(),p['written']))
pid=tv.tv("ps -ef | grep 'lumatv-compositor' | grep -v grep | grep -v qml | awk '$3=='$(pidof surface-manager)' {print $2}' | head -1",10).strip()
tv.tv("kill -9 %s"%pid,10); t0=time.time(); say("kill -9 %s sent"%pid)
while True:
    m=tv.tv("grep -c ' /usr/bin/surface-manager ' /proc/self/mountinfo",10).strip()
    if m=="0": break
    time.sleep(0.5)
say("unmounted at t+%.1f, bl %s"%(time.time()-t0,bl()))
r=subprocess.Popen("cd /tmp/lumatv-r20 && PATH=/data/home/.local/bin:/data/home/.cargo/bin:$PATH timeout 150 sh compositor/tools/tv-first-light.sh cutover enable",shell=True,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True)
while r.poll() is None:
    time.sleep(3); say("t+%.0f bl %s"%(time.time()-t0,bl()))
say("enable done: "+r.stdout.read().strip().splitlines()[-1][:80]); time.sleep(3); say("bl %s file %s"%(bl(),tv.tv("test -f /var/lib/lumatv/paneldim.json && echo yes || echo no",10).strip()))
print(tv.tv("grep -a 'paneldim' /var/lib/lumatv/compositor/compositor.log | tail -3 | cut -c1-200; grep -a 'panel dim' /var/lib/lumatv/log-helper | tail -1|cut -c1-160",10))
