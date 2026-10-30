#!/usr/bin/env python3
"""shot.py NAME [--luna|--ctl]: what the panel shows now, as a 1280-wide JPEG in the slot's img/ directory; prints the absolute path.
Under LumaTV's compositor (a lumatv-compositor process) `ctl capture` writes the composed picture; otherwise LG's DISPLAY capture
(luna capture/executeOneShot) does. Protected video shows black in both."""
import os, subprocess, sys, tempfile, time
sys.path.insert(0, "/tmp/lumatv-r19w/docs/research/arch")
import tv
from PIL import Image
IMG = "/tmp/lumatv-r19w/docs/research/compositor/slot-2026-10-29-r19w/img"
CTL = "/tmp/lumatv/lib/ld-linux-aarch64.so.1 --library-path /tmp/lumatv/lib /tmp/lumatv/bin/lumatv-compositor ctl --runtime-dir /tmp/xdg"
name = sys.argv[1]
mode = sys.argv[2] if len(sys.argv) > 2 else ""
live = tv.tv("test -S /tmp/xdg/lumatv-control && echo yes").strip().startswith("yes")
use_ctl = (mode == "--ctl") or (live and mode != "--luna")
remote = "/tmp/r19wshot.png"
tv.tv("rm -f %s" % remote)
how = None
if use_ctl:
    out = tv.tv("%s capture %s 2>&1 | tail -3" % (CTL, remote), 30)
    how = "ctl"
    if not tv.tv("test -s %s && echo y" % remote).strip().startswith("y"):
        print("ctl capture gave no file (%s); trying LG's capture" % out.strip()[:200], file=sys.stderr)
        use_ctl = False
if not use_ctl:
    how = "luna"
    for attempt in range(3):
        out = tv.tv("luna-send -n 1 luna://com.webos.service.capture/executeOneShot '{\"path\":\"%s\",\"method\":\"DISPLAY\",\"format\":\"PNG\",\"width\":1280,\"height\":720}'" % remote, 30)
        if '"returnValue":true' in out.replace(" ", ""):
            break
        time.sleep(1.5)
os.makedirs(IMG, exist_ok=True)
local = os.path.join(tempfile.gettempdir(), "r19wshot-%d.png" % os.getpid())
tv.get(remote, local)
im = Image.open(local).convert("RGB")
w, h = im.size
if w != 1280:
    im = im.resize((1280, round(h * 1280 / w)), Image.LANCZOS)
dest = os.path.join(IMG, name + ".jpg")
im.save(dest, "JPEG", quality=88)
os.remove(local)
print(dest, "(%s, source %dx%d)" % (how, w, h))
