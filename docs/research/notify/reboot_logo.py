#!/usr/bin/env python3
"""The power-on logo check: one `reboot` of a set that is ON, frames of the panel from the TV's own boot capture.

usage: reboot_logo.py <dest dir> [capture seconds, default 60]     NO_REBOOT=1: only collect after a reboot already issued

Why the capture runs on the TV: the logo is up about 14-30 s into a boot and sshd answers on the new system only at
about 46 s (2026-10-25 16:34), so frames taken over SSH start after the logo. `echo <s> > /var/lib/lumatv/bootcap`
makes the boot hook (tools/autostart.sh) start tools/bootcap.sh at ~14 s; frames land in /tmp/lumatv-bootcap and
this script fetches them once the new system answers. The flag is consumed by that boot.

The set must be on (power state Active) before the reboot: from Active Standby a reboot comes back in standby with the
panel off and no logo (16:34). Needs paramiko and the tool and hook of this commit on the set. The old system
answers for a few seconds after `reboot`, so a connection counts only when its uptime is below the one before.
"""
import os, sys, time, paramiko

DEST = sys.argv[1]
SECS = int(sys.argv[2]) if len(sys.argv) > 2 else 60
TV = ("192.168.1.242", "root", "alpine")
TOOL = "/media/developer/apps/usr/palm/applications/org.nphil.lumatv/tools/lumatv"
FLAG = "/var/lib/lumatv/bootcap"
CAPDIR = "/tmp/lumatv-bootcap"
POWER = "luna-send -n 1 -f luna://com.webos.service.tvpower/power/getPowerState '{}' | tr -d '\\n '"


def connect(timeout=3):
    c = paramiko.SSHClient(); c.set_missing_host_key_policy(paramiko.AutoAddPolicy())
    c.connect(TV[0], username=TV[1], password=TV[2], timeout=timeout, banner_timeout=timeout, auth_timeout=timeout,
              look_for_keys=False, allow_agent=False)
    return c


def run(c, cmd, timeout=60):
    _, o, _ = c.exec_command(cmd, timeout=timeout, get_pty=True)
    return o.read().decode("utf-8", "replace").replace("\r", "")


os.makedirs(DEST, exist_ok=True)
log = open(os.path.join(DEST, "log.txt"), "a")


def say(s):
    print(s, flush=True); log.write(s + "\n"); log.flush()


t0 = time.time()
before = 1e9
if os.environ.get("NO_REBOOT"):
    say("collect only at %s" % time.strftime("%H:%M:%S"))
else:
    c = connect(8)
    state = run(c, POWER, 20).strip()
    before = float(run(c, "cut -d' ' -f1 /proc/uptime", 10).split()[0])
    say("before: power %s, uptime %.1f s, logo %s" % (state, before, run(c, "%s logo status 2>&1 | head -c 200" % TOOL, 20).strip()))
    if '"state":"Active"' not in state:
        say("the set is not on; a reboot from standby comes back in standby with no logo. Not rebooting."); sys.exit(2)
    run(c, "echo %d > %s" % (SECS, FLAG), 10)
    say("reboot at %s" % time.strftime("%H:%M:%S"))
    try:
        c.exec_command("reboot", timeout=20)
    except Exception as e:
        say("reboot call ended: %r" % (e,))
    try:
        c.close()
    except Exception:
        pass
    time.sleep(3)

cli = None
up = 0.0
while time.time() - t0 < 300:      # retried until the NEW system answers
    try:
        cli = connect(3)
        up = float(run(cli, "cut -d' ' -f1 /proc/uptime", 10).split()[0])
        if up < min(before, 600):
            break
        cli.close(); cli = None     # still the old system
    except Exception:
        cli = None
    time.sleep(1)
if cli is None:
    say("SSH did not come back within 300 s"); sys.exit(1)
say("ssh answers at %s, uptime %.1f s" % (time.strftime("%H:%M:%S"), up))

end = time.time() + SECS + 30
while time.time() < end and "done" not in run(cli, "ls %s 2>&1" % CAPDIR, 10).split():
    time.sleep(2)
# The worker's last act is the harvest of the log (boot-events); .boot-pending goes when its apply is done.
end = time.time() + 90
while time.time() < end and ".boot-pending" in run(cli, "ls -a /var/lib/lumatv 2>&1", 10):
    time.sleep(3)
time.sleep(3)
listing = run(cli, "cat %s/frames.txt 2>&1" % CAPDIR, 20)
sftp = cli.open_sftp()
frames = []
for line in listing.splitlines():
    parts = line.split()
    if len(parts) < 3 or not parts[0].isdigit():
        continue
    try:
        sftp.get("%s/%s.png" % (CAPDIR, parts[0]), os.path.join(DEST, "boot-%03d-up%s.png" % (int(parts[0]), parts[1])))
        frames.append(parts)
    except IOError:
        pass
sftp.close()
say("frames: %d (uptime %s .. %s); bytes %s" % (len(frames), frames[0][1] if frames else "-", frames[-1][1] if frames else "-",
    " ".join(f[2] for f in frames)))
say("hook: " + run(cli, "grep -a 'boot capture' /var/lib/lumatv/log | tail -2", 20).strip())
say("logo lines: " + run(cli, "for f in /var/log/messages*; do case $f in *.gz) zcat $f;; *) cat $f;; esac; done 2>/dev/null | grep -a '\\[Logo\\]' | head -6", 40).strip())
# What was in front, once a second (the capture's state.txt), and the two durable records of the logo path: the windows' own
# (logo-events, written by BootLogo.qml / MasterBrand.qml) and the boot hook's harvest of the log (boot-events, with its
# .prev of the boot before). All kept in DEST as they are, so that the timeline can be rebuilt from files.
for remote, name in (("%s/state.txt" % CAPDIR, "state.txt"), ("/var/lib/lumatv/logo-events", "logo-events.txt"),
                     ("/var/lib/lumatv/boot-events", "boot-events.txt"), ("/var/lib/lumatv/boot-events.prev", "boot-events-prev.txt")):
    text = run(cli, "cat %s 2>&1" % remote, 30)
    if "No such file" in text[:60]:
        say("%s: not on the set" % name)
        continue
    with open(os.path.join(DEST, name), "w") as fh:
        fh.write(text)
    say("%s: %d lines" % (name, len(text.splitlines())))
say("boot log lines: " + run(cli, "grep -a 'boot: ' /var/lib/lumatv/log | tail -14", 20).strip())
say("state after boot:\n" + run(cli, "cat /proc/uptime; cat /sys/power/mode; %s; echo; "
    "luna-send -n 1 -f luna://com.webos.applicationManager/getForegroundAppInfo '{}' | tr -d '\\n ' | head -c 300; echo; "
    "%s homestub status 2>&1 | head -c 300; echo; %s preload status 2>&1 | head -c 300; echo; "
    "grep -a 'homestub\\|preload\\|apply:' /var/lib/lumatv/log | tail -12; ls /tmp/var/log/reports/librdx 2>&1 | head -3" % (POWER, TOOL, TOOL), 120))
cli.close()
say("done")
