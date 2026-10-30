#!/usr/bin/env python3
"""famwatch.py [KILL...]: watches for someone picking up the Magic Remote (lginput2's reconnect or SLEEP -> ACTIVE line in /var/log/messages, a
newer one than at the start) and for real input the compositor reads (`ctl wake` input count: a gate injects its keys through `ctl remote`, which
does not count there). On the first sign it writes /tmp/r19w-gate/FAMILY with the time and the evidence, kills the host processes named on the
command line (pkill -f each), and exits. Checks every 15 s. WAKEONLY=1: the remote's wake lines only (a step that moves the pointer through `ctl`
counts in the input count). INPUTONLY=1: stops only on a rise of the input count between two looks (a bare Bluetooth lift of the remote is not a stop)."""
import json, os, re, subprocess, sys, time
sys.path.insert(0, "/tmp/lumatv-r19w/docs/research/arch")
import tv

CTL = "/tmp/lumatv/lib/ld-linux-aarch64.so.1 --library-path /tmp/lumatv/lib /tmp/lumatv/bin/lumatv-compositor ctl --runtime-dir /tmp/xdg"
PAT = r'STAGE_ADDING|==> \"\[ACTIVE\]\"'

def last_wake():
    out = tv.tv("tail -n 3000 /var/log/messages | grep -a lginput2 | grep -a -E '%s' | tail -n 1 | cut -c1-120" % PAT, 30).strip()
    return out

def input_n():
    try:
        return json.loads(tv.tv("%s wake 2>/dev/null" % CTL, 20).strip())["input"]["n"]
    except Exception:
        return None

if __name__ == "__main__":
    kill = sys.argv[1:]
    w0, n0 = last_wake(), input_n()
    print("%s armed: last wake %r input %s" % (time.strftime("%T"), w0, n0), flush=True)
    while True:
        time.sleep(15)
        try:
            w, n = last_wake(), input_n()
        except Exception as e:
            print("%s poll error %s" % (time.strftime("%T"), str(e)[:100]), flush=True)
            continue
        why = None
        if os.environ.get("INPUTONLY"):
            # real key or pointer events only: a rise of the compositor's input count since the last look (a `ctl wake reset` lowers it: re-based)
            if n is not None and n0 is not None and n > n0:
                why = "input events %s -> %s" % (n0, n)
            n0 = n
        elif w != w0:
            why = "remote woke: %s" % w
        elif not os.environ.get("WAKEONLY") and n is not None and n0 is not None and n != n0:
            why = "input events %s -> %s" % (n0, n)
        if why:
            msg = "%s FAMILY? %s" % (time.strftime("%T"), why)
            print(msg, flush=True)
            open("/tmp/r19w-gate/FAMILY", "a").write(msg + "\n")
            for k in kill:
                subprocess.run(["pkill", "-f", k])
            break
