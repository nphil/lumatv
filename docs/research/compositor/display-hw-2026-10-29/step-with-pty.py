# usage: step.py STEP [ENV=VAL...]; runs layerdeep-tv.sh STEP on the TV with a pty; waits at "Type yes" for /tmp/hw/yes-STEP to appear.
import sys,time,os,paramiko
step=sys.argv[1]; env=' '.join(sys.argv[2:])
flag='/tmp/hw/yes-'+step
if os.path.exists(flag): os.remove(flag)
log=open('/tmp/hw/%s.out'%step,'w')
c=paramiko.SSHClient();c.set_missing_host_key_policy(paramiko.AutoAddPolicy())
c.connect('192.168.1.242',username='root',password='alpine',timeout=8)
ch=c.get_transport().open_session();ch.get_pty();ch.set_combine_stderr(True)
ch.exec_command('cd /tmp/lumatv && %s sh ./layerdeep-tv.sh %s'%(env,step))
sent=False
while True:
    if ch.recv_ready():
        d=ch.recv(4096).decode(errors='replace');log.write(d);log.flush()
    elif ch.exit_status_ready(): break
    else: time.sleep(0.05)
    if not sent and os.path.exists(flag):
        ch.send('yes\n');sent=True;log.write('\n[sent yes]\n');log.flush()
log.write('\n[exit %d]\n'%ch.recv_exit_status());log.close()
