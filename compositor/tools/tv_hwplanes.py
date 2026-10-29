#!/usr/bin/env python3
"""Drives the hardware-planes TV round (docs/research/hw-planes-result.md, "TV-round checklist") from the workstation.

    tv_hwplanes.py STEP [ARGS...]      puts tv-hwplanes.sh on the set as /tmp/lumatv/hwplanes.sh and runs `hwplanes.sh STEP ARGS` in a PTY
                                       (luna-send needs one): read, alpha, pointer, afbc, shot NAME, feeder CLIP SIZE CODEC FPS SECS, vwin, sizes, stop
    tv_hwplanes.py ctl WORDS...        one `lumatv-compositor ctl` line against the live compositor ("ctl vwin apps", "ctl afbc on")
    tv_hwplanes.py put-feeder FILE     uploads a feeder build (gfn/feeder/build/arm/lumatv-gfn-feeder) to /tmp/gfn/
    tv_hwplanes.py put-clip FILE       uploads a test clip to /tmp/gfn/clips/
    tv_hwplanes.py fetch               copies /tmp/lumatv/out/hwplanes to /tmp/lumatv-tv-out/hwplanes
    tv_hwplanes.py compare A.png B.png the share of pixels that differ between two fetched captures, and where (needs Pillow)

Reads and switches only what the script says; nothing is written outside /tmp/lumatv, /tmp/gfn and the out directory."""
import os
import shlex
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "..", "docs", "research", "arch"))

REMOTE = "/tmp/lumatv/hwplanes.sh"
OUT = "/tmp/lumatv/out/hwplanes"
LOCAL_OUT = "/tmp/lumatv-tv-out/hwplanes"
STEPS = ("read", "alpha", "pointer", "afbc", "shot", "feeder", "vwin", "sizes", "stop")
CTL = ("/tmp/lumatv/lib/ld-linux-aarch64.so.1 --library-path /tmp/lumatv/lib /tmp/lumatv/bin/lumatv-compositor ctl --runtime-dir %s %s")


def pty(cmd, timeout=120):
    import paramiko
    import tv
    c = paramiko.SSHClient()
    c.set_missing_host_key_policy(paramiko.AutoAddPolicy())
    c.connect(tv.TV, username="root", password="alpine", timeout=10, look_for_keys=False, allow_agent=False)
    try:
        _, out, _ = c.exec_command(cmd, timeout=timeout, get_pty=True)
        return out.read().decode("utf-8", "replace").replace("\r", "")
    finally:
        c.close()


def compare(a, b):
    from PIL import Image, ImageChops
    ia, ib = Image.open(a).convert("RGB"), Image.open(b).convert("RGB")
    if ia.size != ib.size:
        sys.exit("sizes differ: %s and %s" % (ia.size, ib.size))
    diff = ImageChops.difference(ia, ib).convert("L").point(lambda v: 255 if v > 2 else 0)
    box = diff.getbbox()
    n = sum(1 for v in diff.getdata() if v)
    print("differing pixels: %d of %d (%.3f %%), bounding box %s" % (n, ia.size[0] * ia.size[1], 100.0 * n / (ia.size[0] * ia.size[1]), box))


def main():
    args = sys.argv[1:]
    if not args:
        sys.exit(__doc__)
    if args[0] == "compare" and len(args) == 3:
        return compare(args[1], args[2])
    import tv
    what = args[0]
    if what == "fetch":
        tv.tv("cd /tmp/lumatv/out && tar czf /tmp/lumatv-hwplanes.tgz hwplanes")
        os.makedirs("/tmp/lumatv-tv-out", exist_ok=True)
        tv.get("/tmp/lumatv-hwplanes.tgz", "/tmp/lumatv-tv-out/hwplanes.tgz")
        os.system("tar -C /tmp/lumatv-tv-out -xzf /tmp/lumatv-tv-out/hwplanes.tgz && ls -la %s" % LOCAL_OUT)
        return
    if what == "put-feeder" and len(args) == 2:
        tv.tv("mkdir -p /tmp/gfn")
        tv.put(args[1], "/tmp/gfn/lumatv-gfn-feeder")
        tv.tv("chmod +x /tmp/gfn/lumatv-gfn-feeder")
        return
    if what == "put-clip" and len(args) == 2:
        tv.tv("mkdir -p /tmp/gfn/clips")
        tv.put(args[1], "/tmp/gfn/clips/" + os.path.basename(args[1]))
        return
    if what == "ctl" and len(args) >= 2:
        print(pty(CTL % (os.environ.get("HWPLANES_RT", "/tmp/xdg"), " ".join(shlex.quote(a) for a in args[1:])), 30))
        return
    if what not in STEPS:
        sys.exit(__doc__)
    tv.put(os.path.join(HERE, "tv-hwplanes.sh"), REMOTE)
    env = "HWPLANES_RT=%s " % shlex.quote(os.environ["HWPLANES_RT"]) if "HWPLANES_RT" in os.environ else ""
    print(pty("%ssh %s %s" % (env, REMOTE, " ".join(shlex.quote(a) for a in args)), 300))


if __name__ == "__main__":
    main()
