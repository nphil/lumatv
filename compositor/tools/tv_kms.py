#!/usr/bin/env python3
"""Drives the C3 maintenance window from the workstation: runs kms-window.sh on the set, captures the panel through
LG's capture service while each case shows its colour (luna-send needs a PTY, which docs/research/arch/tv.py's
exec does not give), fetches the results. Usage: tv_kms.py [run|restore]."""
import sys, time, os
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', '..', 'docs', 'research', 'arch'))
import tv, paramiko

CAPTURE = ("luna-send -n 1 -f luna://com.webos.service.capture/executeOneShot "
           "'{\"path\":\"%s\",\"method\":\"DISPLAY\",\"format\":\"PNG\",\"width\":960,\"height\":540}'")

def pty(cmd, timeout=30):
    c = paramiko.SSHClient(); c.set_missing_host_key_policy(paramiko.AutoAddPolicy())
    c.connect(tv.TV, username="root", password="alpine", timeout=10, look_for_keys=False, allow_agent=False)
    try:
        _, out, _ = c.exec_command(cmd, timeout=timeout, get_pty=True)
        return out.read().decode("utf-8", "replace")
    finally:
        c.close()

def wait_for(path, seconds):
    t0 = time.time()
    while time.time() - t0 < seconds:
        if tv.tv("test -f %s && echo yes" % path).strip().startswith("yes"):
            return True
        time.sleep(1)
    return False

def main():
    what = sys.argv[1] if len(sys.argv) > 1 else "run"
    if what == "restore":
        print(tv.tv("sh /tmp/lumatv/kms-window.sh restore", timeout=120)); return
    here = os.path.dirname(os.path.abspath(__file__))
    tv.put(os.path.join(here, "tv-kms-window.sh"), "/tmp/lumatv/kms-window.sh")
    tv.tv("rm -rf /tmp/lumatv/out/kms; mkdir -p /tmp/lumatv/out/kms")
    print("power state:", pty("luna-send -n 1 -f luna://com.webos.service.tvpower/power/getPowerState '{}'", 20).strip()[:200])
    c = paramiko.SSHClient(); c.set_missing_host_key_policy(paramiko.AutoAddPolicy())
    c.connect(tv.TV, username="root", password="alpine", timeout=10, look_for_keys=False, allow_agent=False)
    chan = c.get_transport().open_session()
    chan.exec_command("sh /tmp/lumatv/kms-window.sh run > /tmp/lumatv/out/kms/run.log 2>&1")
    for case in ("a", "b"):
        if wait_for("/tmp/lumatv/out/kms/%s-ready" % case, 60):
            r = pty(CAPTURE % ("/tmp/lumatv/out/kms/panel-%s.png" % case))
            print("capture %s: %s" % (case, r.strip()[:200]))
        else:
            print("case %s never became ready (see run.log)" % case)
    t0 = time.time()
    while not chan.exit_status_ready() and time.time() - t0 < 180:
        time.sleep(2)
    print("run exit", chan.recv_exit_status() if chan.exit_status_ready() else "still running")
    c.close()
    print(tv.tv("cat /tmp/lumatv/out/kms/run.log"))
    tv.tv("cd /tmp/lumatv && tar czf /tmp/lumatv-kms.tgz out/kms")
    os.makedirs("/tmp/lumatv-tv-out", exist_ok=True)
    tv.get("/tmp/lumatv-kms.tgz", "/tmp/lumatv-tv-out/kms.tgz")
    os.system("tar -C /tmp/lumatv-tv-out -xzf /tmp/lumatv-tv-out/kms.tgz && ls -la /tmp/lumatv-tv-out/out/kms")
    print(tv.tv("systemctl is-active surface-manager-daemon; pgrep -l 'lumatv|x2cli' || echo 'none of ours running'"))

if __name__ == "__main__":
    main()
