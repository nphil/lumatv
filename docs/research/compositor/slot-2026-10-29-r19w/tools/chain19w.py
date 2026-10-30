#!/usr/bin/env python3
"""chain19w.py STEP [STEP...]: round 19W's unattended steps, one after the other, each harvested into the slot and committed, LumaTV's compositor put
back live (persistent, the Rust kinds as the set had them) after every gate session. Stops at the first sign of use (/tmp/r19w-gate/FAMILY, written by
famwatch.py) and says so. Steps: video, apps-on, apps-off, x1-1, x1-2, x1-3, frames-60, frames-120, standby, shell, oled, hwplanes.
The receiver is the agent's (HA MCP); the gate's own receiver calls are no-ops (gate_r19w.py)."""
import glob, json, os, re, shutil, subprocess, sys, time
sys.path.insert(0, "/tmp/lumatv-r19w/docs/research/arch")
import tv

W = "/tmp/lumatv-r19w"
SLOT = W + "/docs/research/compositor/slot-2026-10-29-r19w"
GATE = "/tmp/r19w-gate"
KINDS = "home,osd,toast,alert,quick,picker,drawer,settings,entry,keyboard,menu,confirm,switcher,saver,displaylab"
CTL = "/tmp/lumatv/lib/ld-linux-aarch64.so.1 --library-path /tmp/lumatv/lib /tmp/lumatv/bin/lumatv-compositor ctl --runtime-dir /tmp/xdg"

def say(s):
    line = "%s %s" % (time.strftime("%T"), s)
    print(line, flush=True)
    open(GATE + "/chain.log", "a").write(line + "\n")

def family():
    return os.path.exists(GATE + "/FAMILY")

def pty(cmd, t=300):
    import paramiko
    c = paramiko.SSHClient(); c.set_missing_host_key_policy(paramiko.AutoAddPolicy())
    c.connect(tv.TV, username="root", password="alpine", timeout=10, look_for_keys=False, allow_agent=False)
    try:
        _, o, _ = c.exec_command(cmd, timeout=t, get_pty=True)
        return o.read().decode("utf-8", "replace")
    finally:
        c.close()

def ctl(a, t=30):
    out = tv.tv("%s %s 2>/dev/null" % (CTL, a), t).strip()
    try:
        return json.loads(out)
    except Exception:
        return {"raw": out[:300]}

def live():
    st = pty("sh /tmp/lumatv/cutover.sh status 2>&1 | head -5", 60)
    return ("flag: persistent" in st) and ("compositor: " in st and re.search(r"compositor: \d+", st) is not None)

def set_kinds(kinds):
    tv.tv("printf '%%s\\n' '%s' > /var/lib/lumatv/compositor/shell-rust.conf; cat /var/lib/lumatv/compositor/shell-rust.conf" % kinds, 20)

def ensure_live(kinds=KINDS):
    """After a gate session: the kinds as asked, LumaTV's compositor live and persistent, the Rust client connected when kinds are on."""
    set_kinds(kinds)
    if not live():
        out = pty("sh /tmp/lumatv/cutover.sh enable 2>&1 | tail -4", 300)
        say("enable: %s" % out.strip().replace("\n", " | ")[:300])
    for _ in range(60):
        time.sleep(2)
        r = ctl("shell")
        if kinds and r.get("rust", {}).get("connected"):
            break
        if not kinds and live():
            break
    fg = tv.tv("luna-send -n 1 -f luna://com.webos.service.applicationmanager/getForegroundAppInfo '{}' | grep appId", 20).strip()
    say("live %s, rust connected %s, %s" % (live(), (ctl("shell").get("rust") or {}).get("connected"), fg))

def harvest(name, sub):
    d = "%s/%s" % (SLOT, sub)
    os.makedirs(d, exist_ok=True)
    for src, dst in (("%s/%s.out" % (GATE, name), "%s/%s-stdout.txt" % (d, name)), ("/tmp/lumatv-tv-out/gate.tgz", "%s/%s-gate.tgz" % (d, name))):
        if os.path.exists(src):
            shutil.copy(src, dst)
    return d

def commit(msg):
    subprocess.run("cd %s && git add docs && git -c user.name=Round19W -c user.email=round19w@lumatv.local commit -q -m %r && timeout 60 git fetch -q forge && "
                   "git -c user.name=Round19W -c user.email=round19w@lumatv.local rebase forge/main > /dev/null 2>&1; timeout 60 git push -q forge HEAD:main > /dev/null 2>&1" % (W, msg), shell=True)

def gate(name, sub, args, kinds=KINDS):
    if family():
        say("STOP before %s: %s" % (name, open(GATE + "/FAMILY").read().strip()))
        return None
    say("start %s: %s" % (name, " ".join(args)))
    t0 = time.time()
    subprocess.run(["sh", "/tmp/r19w-build/run19w.sh", name] + args)
    out = open("%s/%s.out" % (GATE, name)).read()
    exit_line = [l for l in out.splitlines() if l.startswith("EXIT")][-1:] or ["EXIT ?"]
    res = [l[:260] for l in out.splitlines() if l.startswith("GATE") or l.startswith("EXIT")]
    say("%s done in %d s: %s" % (name, time.time() - t0, " || ".join(res)[:900]))
    harvest(name, sub)
    ensure_live(kinds)
    commit("Round 19W gate %s" % name)
    return out

def step_video():
    gate("video", "video", ["--only", "video", "--video-apps", "youtube,netflix"])

def step_apps_on():
    # one session per call: the gate's own count is --apps-repeat
    gate("apps-on", "apps-kinds-on", ["--only", "apps", "--apps-repeat", "15"])

def restore_kinds():
    """The kinds back on at runtime (ctl shell rust +kind writes the file and starts the client)."""
    r = ctl("shell rust " + " ".join("+" + k for k in KINDS.split(",")), 60)
    for _ in range(60):
        time.sleep(2)
        if (ctl("shell").get("rust") or {}).get("connected"):
            break
    say("kinds restored: %s" % json.dumps((ctl("shell").get("rust") or {}).get("kinds"))[:200])

def step_apps_off():
    set_kinds("")
    gate("apps-off", "apps-kinds-off", ["--only", "apps", "--apps-repeat", "15"], kinds="")
    restore_kinds()

def step_x1(n):
    gate("x1-%d" % n, "x1", ["--only", "x1"])

def step_frames(hz):
    gate("frames-%d" % hz, "frames-%d" % hz, ["--only", "frames"] + (["--refresh", "120"] if hz == 120 else []))

def step_standby():
    gate("standby", "standby", ["--only", "standby"])

def step_shell():
    gate("shell", "shell", ["--only", "shell"])

STEPS = {"video": step_video, "apps-on": step_apps_on, "apps-off": step_apps_off, "x1-1": lambda: step_x1(1), "x1-2": lambda: step_x1(2), "x1-3": lambda: step_x1(3),
         "frames-60": lambda: step_frames(60), "frames-120": lambda: step_frames(120), "standby": step_standby, "shell": step_shell}

if __name__ == "__main__":
    os.makedirs(GATE, exist_ok=True)
    for s in sys.argv[1:]:
        if s not in STEPS:
            say("unknown step %s" % s)
            continue
        if family():
            say("STOP before %s: %s" % (s, open(GATE + "/FAMILY").read().strip()))
            break
        STEPS[s]()
    say("chain over")
