import paramiko, sys
TV = "192.168.1.242"
def tv(cmd, timeout=120):
    c = paramiko.SSHClient(); c.set_missing_host_key_policy(paramiko.AutoAddPolicy())
    c.connect(TV, username="root", password="alpine", timeout=10, banner_timeout=20, auth_timeout=20, look_for_keys=False, allow_agent=False)
    try:
        _, out, err = c.exec_command(cmd, timeout=timeout)
        o = out.read().decode("utf-8", "replace"); e = err.read().decode("utf-8", "replace")
        return o + (("\n[stderr]\n" + e) if e.strip() else "")
    finally:
        c.close()
def put(local, remote):
    t = paramiko.Transport((TV, 22)); t.connect(username="root", password="alpine")
    s = paramiko.SFTPClient.from_transport(t); s.put(local, remote); s.close(); t.close()
def get(remote, local):
    t = paramiko.Transport((TV, 22)); t.connect(username="root", password="alpine")
    s = paramiko.SFTPClient.from_transport(t); s.get(remote, local); s.close(); t.close()
if __name__ == "__main__":
    print(tv(" ".join(sys.argv[1:])))
