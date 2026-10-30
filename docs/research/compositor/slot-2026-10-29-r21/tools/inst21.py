#!/usr/bin/env python3
"""inst19w.py pre | post: around the install through HomeLabber's run_command (grant): `pre` ends LumaTV's compositor (LG's is live for the install), clears the
install log and points lumatv-next at this worktree; `post` reads back the installed files against the build, drops /tmp/lumatv/compositor.flags, enables
LumaTV's compositor (persistent) and reads pageFormat, fb0's alpha and the Rust client."""
import hashlib, json, os, subprocess, sys, time
sys.path.insert(0, "/tmp/lumatv-r21/docs/research/arch")
import tv

W = "/tmp/lumatv-r21"
CTL = "/tmp/lumatv/lib/ld-linux-aarch64.so.1 --library-path /tmp/lumatv/lib /tmp/lumatv/bin/lumatv-compositor ctl --runtime-dir /tmp/xdg"

def pty(cmd, t=300):
    import paramiko
    c = paramiko.SSHClient(); c.set_missing_host_key_policy(paramiko.AutoAddPolicy())
    c.connect(tv.TV, username="root", password="alpine", timeout=10, look_for_keys=False, allow_agent=False)
    try:
        _, o, _ = c.exec_command(cmd, timeout=t, get_pty=True)
        return o.read().decode("utf-8", "replace")
    finally:
        c.close()

def md5(p):
    return hashlib.md5(open(p, "rb").read()).hexdigest()

def say(s):
    print("%s %s" % (time.strftime("%T"), s), flush=True)

if sys.argv[1] == "pre":
    say("cutover disable: " + pty("sh /tmp/lumatv/cutover.sh disable 2>&1 | tail -3", 300).strip().replace("\n", " | ")[:300])
    subprocess.run("rm -f /tmp/lumatv-install-out/log.txt; ln -sfn %s /data/home/tmp/lumatv-next; ls -la /data/home/tmp/lumatv-next" % W, shell=True)
    say("head %s; ipk %s" % (subprocess.run("cd %s && git rev-parse --short HEAD" % W, shell=True, capture_output=True, text=True).stdout.strip(),
                              os.path.getsize(W + "/org.nphil.lumatv_0.4.0_all.ipk")))
    say("foreground: " + tv.tv("luna-send -n 1 -f luna://com.webos.service.applicationmanager/getForegroundAppInfo '{}' | grep appId", 20).strip())
elif sys.argv[1] == "post":
    want = {"compositor": md5(W + "/compositor/deploy/bin/lumatv-compositor"), "ui": md5(W + "/compositor/deploy/bin/lumatv-ui")}
    got = tv.tv("md5sum /tmp/lumatv/bin/lumatv-compositor /tmp/lumatv/bin/lumatv-ui /var/lib/lumatv/compositor/bin/lumatv-compositor 2>&1 | cut -c1-70", 30)
    say("build: %s" % json.dumps(want))
    say("on the set:\n" + got)
    say("helper: want %s, set %s" % (open(W + "/helper/dist/lumatv-helper.sha256").read().split()[0][:16],
                                      tv.tv("sha256sum /media/developer/apps/usr/palm/applications/org.nphil.lumatv/payload/helper/lumatv-helper | cut -c1-16", 30).strip()))
    tv.tv("rm -f /tmp/lumatv/compositor.flags; ls /tmp/lumatv/compositor.flags 2>&1 | head -1", 20)
    say("flags file removed; enable (persistent): " + pty("sh /tmp/lumatv/cutover.sh enable 2>&1 | tail -4", 300).strip().replace("\n", " | ")[:400])
    for _ in range(60):
        time.sleep(2)
        try:
            r = json.loads(tv.tv("%s shell 2>/dev/null" % CTL, 20))
            if (r.get("rust") or {}).get("connected"):
                break
        except Exception:
            pass
    st = json.loads(tv.tv("%s status 2>/dev/null" % CTL, 20))
    af = json.loads(tv.tv("%s afbc 2>/dev/null" % CTL, 20))["afbc"]["pages"]
    say("pageFormat %s videoHolesOpaque %s pages %s alpha %s" % (st.get("pageFormat"), st.get("videoHolesOpaque"), af.get("kind"), af.get("alpha")))
    say(tv.tv("sed -n 's/^ *\\(hdr.global_alpha[a-z_]*\\) *[:=] *\\(.*\\)/\\1 \\2/p' /proc/lg/fbdev/fb0/status | tr '\\n' ' '", 20).strip())
    r = json.loads(tv.tv("%s shell 2>/dev/null" % CTL, 20)).get("rust") or {}
    say("rust connected %s kinds %s foreground %s" % (r.get("connected"), (r.get("ui") or {}).get("kinds"), (r.get("topics") or {}).get("foreground")))
