#!/usr/bin/env python3
"""One sweep of every thread of PID (state, wchan, syscall nr/args/sp/pc, kernel stack) at 0.25, 0.5 and 0.8 s after the file OUT.flag appears, to OUT; the process's
maps and open files at the first sweep. Between the sweeps it only sleeps (a clean close of Plex is over in 0.13 s, a stalled one is still there). usage: stall-snap.py PID OUT"""
import os, sys, time
pid, out = sys.argv[1], sys.argv[2]
def rd(p, flat=True):
    try:
        with open(p) as f:
            s = f.read().strip()
        return s.replace("\n", " | ") if flat else s
    except Exception as e:
        return "ERR(%s)" % type(e).__name__
def up():
    return float(open("/proc/uptime").read().split()[0])
t_wait = time.time() + 20
while not os.path.exists(out + ".flag") and time.time() < t_wait:
    time.sleep(0.01)
t_flag = time.time()
f = open(out, "w")
f.write("%.2f FLAG seen\n" % up())
first = True
for off in (0.25, 0.5, 0.8):
    d = t_flag + off - time.time()
    if d > 0:
        time.sleep(d)
    u = up()
    try:
        tids = sorted(os.listdir("/proc/%s/task" % pid), key=int)
    except Exception:
        f.write("%.2f +%.2f GONE (the process has exited)\n" % (u, off))
        break
    f.write("%.2f +%.2f SWEEP %d threads, stat: %s\n" % (u, off, len(tids), rd("/proc/%s/stat" % pid)[:200]))
    if first:
        open(out + ".maps", "w").write(rd("/proc/%s/maps" % pid, False) + "\n")
        fds = []
        for fd in sorted(os.listdir("/proc/%s/fd" % pid), key=lambda x: int(x)):
            try:
                fds.append("%s -> %s" % (fd, os.readlink("/proc/%s/fd/%s" % (pid, fd))))
            except Exception:
                pass
        open(out + ".fds", "w").write("\n".join(fds) + "\n")
        first = False
    for t in tids:
        b = "/proc/%s/task/%s/" % (pid, t)
        st = rd(b + "stat")
        r = st.rfind(")")
        state = st[r + 2:r + 3] if r > 0 else "?"
        f.write("%.2f %s %s %s wchan=%s syscall=%s | stack=%s\n" % (u, t, rd(b + "comm"), state, rd(b + "wchan"), rd(b + "syscall"), rd(b + "stack")[:200]))
    f.flush()
f.close()
