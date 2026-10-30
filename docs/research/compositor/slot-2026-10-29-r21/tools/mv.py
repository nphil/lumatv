import sys, subprocess, paramiko, time
sys.path.insert(0, "/tmp/lumatv-r21/docs/research/arch")
import tv
def pty(cmd, t=300):
    c = paramiko.SSHClient(); c.set_missing_host_key_policy(paramiko.AutoAddPolicy())
    c.connect(tv.TV, username="root", password="alpine", timeout=10, look_for_keys=False, allow_agent=False)
    try:
        _, o, _ = c.exec_command(cmd, timeout=t, get_pty=True); return o.read().decode("utf-8","replace").replace("\r","")
    finally: c.close()
if __name__ == "__main__":
    print(pty(" ".join(sys.argv[1:]), 280)[-6000:])
