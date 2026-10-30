import sys, time, json, paramiko
sys.path.insert(0, "/tmp/r21-build"); sys.path.insert(0, "/tmp/lumatv-r20/docs/research/arch")
import tv, rustlib as R
T = "/media/developer/apps/usr/palm/applications/org.nphil.lumatv/tools/lumatv"
def say(s): print(time.strftime("%T"), s, flush=True)
def pty(cmd, t=300):
    c = paramiko.SSHClient(); c.set_missing_host_key_policy(paramiko.AutoAddPolicy())
    c.connect(tv.TV, username="root", password="alpine", timeout=10, look_for_keys=False, allow_agent=False)
    try:
        _, o, _ = c.exec_command(cmd, timeout=t, get_pty=True); return o.read().decode("utf-8", "replace").replace("\r", "")
    finally: c.close()
R.ctl("remote 158 tap"); time.sleep(1.5); R.ctl("remote 158 tap"); time.sleep(1)
say("helper before: " + tv.tv("%s helper status" % T, 20).strip())
t0 = time.time()
say("disable: " + pty("sh /tmp/lumatv/cutover.sh disable 2>&1 | tail -3", 300).strip().replace("\n", " | ")[:300] + " (%d s)" % (time.time() - t0))
t1 = time.time()
r = pty("%s apply 2>&1" % T, 400)
say("apply right after disable (%d s after disable returned): %s" % (time.time() - t1, r.strip()[-600:].replace("\n", " | ")))
say("log: " + tv.tv("grep -a -i 'apply' /var/lib/lumatv/log 2>/dev/null | tail -5 | cut -c1-200", 15).strip().replace("\n", " | "))
say("module dir: " + tv.tv("ls -d /var/lib/lumatv/qml/WebOSCompositor 2>&1", 10).strip())
say("helper after: " + tv.tv("%s helper status" % T, 20).strip())
say("status: " + tv.tv("%s status | head -c 600" % T, 30).strip())
