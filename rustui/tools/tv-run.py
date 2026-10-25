#!/usr/bin/env python3
"""Uploads the Rust UI client and what it runs on to the set's /tmp/rustui and runs one measured pass there.

    python3 rustui/tools/tv-run.py NAME PHASES [client options...]      e.g.  tv-run.py idle60mix30 idle:60,mix:30
    python3 rustui/tools/tv-run.py --upload-only

Needs rustui/build/tv (tools/build-tv.sh) and, for the 64-bit path, gfx64/build/bundle and gfx64/build/out. The bundle goes to /tmp/gfx64
(volatile) unless it is there already. It pulls NAME.log, NAME.trace and NAME.sys back to rustui/build/runs/ and runs analyse.py on the trace.
Only run it with a TV slot: the client covers the screen for the length of the run.
"""
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.normpath(os.path.join(HERE, "..", ".."))
sys.path.insert(0, os.path.join(ROOT, "docs", "research", "arch"))
import tv  # noqa: E402

REMOTE = "/tmp/rustui"
GFX = "/tmp/gfx64"
BUILD = os.path.join(ROOT, "rustui", "build", "tv")
RUNS = os.path.join(ROOT, "rustui", "build", "runs")


def upload():
    state = tv.tv("luna-send -n 1 -f luna://com.webos.service.tvpower/power/getPowerState '{}'")
    if "Screen Saver" in state:  # LG's idle detector starts it 180 s after the last remote key; a key ends it
        tv.put(os.path.join(ROOT, "docs", "research", "perf", "kb.py"), "/tmp/kb.py")
        tv.tv("python3 /tmp/kb.py HOME s2")
        state = tv.tv("luna-send -n 1 -f luna://com.webos.service.tvpower/power/getPowerState '{}'")
    if not re.search(r'"state"\s*:\s*"Active"', state):
        sys.exit("refusing to run: power state is not Active (screen on): %s" % state.strip())
    gfx = os.path.join(ROOT, "gfx64", "build")
    files = [(os.path.join(gfx, "..", "launch.sh"), GFX + "/launch.sh"), (os.path.join(gfx, "out", "heapshim.so"), GFX + "/heapshim.so")]
    bundle = os.path.join(gfx, "bundle", "lib")
    files += [(os.path.join(bundle, f), GFX + "/bundle/lib/" + f) for f in sorted(os.listdir(bundle))]
    files += [
        (os.path.join(BUILD, "lumatv-rustui"), REMOTE + "/lumatv-rustui"),
        (os.path.join(HERE, "tv-run.sh"), REMOTE + "/tv-run.sh"),
        (os.path.join(ROOT, "docs", "research", "perf", "capture.sh"), REMOTE + "/capture.sh"),
        (os.path.join(ROOT, "payload", "wallpaper", "wall_1080.png"), REMOTE + "/assets/wallpaper/wall_1080.png"),
    ]
    for local, _ in files:
        if not os.path.exists(local):
            sys.exit("missing %s" % local)
    tv.tv("mkdir -p %s/bundle/lib %s/assets/wallpaper /tmp/perf /tmp/xdg" % (GFX, REMOTE))
    listing = tv.tv("stat -c '%%s %%n' %s/launch.sh %s/heapshim.so %s/bundle/lib/* %s/lumatv-rustui %s/tv-run.sh %s/capture.sh %s/assets/wallpaper/wall_1080.png 2>/dev/null" % (GFX, GFX, GFX, REMOTE, REMOTE, REMOTE, REMOTE))
    have = {}
    for line in listing.splitlines():
        parts = line.split(" ", 1)
        if len(parts) == 2 and parts[0].isdigit():
            have[parts[1].strip()] = int(parts[0])
    for local, remote in files:
        if have.get(remote) == os.path.getsize(local):
            continue
        print("put", remote, flush=True)
        tv.put(local, remote)
    tv.tv("chmod +x %s/launch.sh %s/lumatv-rustui %s/bundle/lib/ld-linux-aarch64.so.1" % (GFX, REMOTE, GFX))


def main():
    if len(sys.argv) > 1 and sys.argv[1] == "--upload-only":
        upload()
        return
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    name, phases, extra = sys.argv[1], sys.argv[2], sys.argv[3:]
    upload()
    shot = os.environ.get("SHOT")
    env = ("SHOT=%s " % shot) if shot else ""
    seconds = sum(float(p.split(":")[1]) for p in phases.split(","))
    cmd = "cd %s && %ssh tv-run.sh %s %s %s 2>&1" % (REMOTE, env, name, phases, " ".join(extra))
    out = tv.tv(cmd, timeout=int(seconds) + 120)
    print(out)
    os.makedirs(RUNS, exist_ok=True)
    for ext, remote_dir in (("log", REMOTE), ("out", REMOTE), ("csv", REMOTE), ("trace", "/tmp/perf"), ("sys", "/tmp/perf")) + ((("png", REMOTE),) if shot else ()):
        try:
            tv.get("%s/%s.%s" % (remote_dir, name, ext), os.path.join(RUNS, "%s.%s" % (name, ext)))
        except Exception as e:  # a run that failed early has no trace
            print("no %s.%s: %s" % (name, ext, e))
    trace = os.path.join(RUNS, name + ".trace")
    if os.path.exists(trace):
        a = subprocess.run([sys.executable, os.path.join(ROOT, "docs", "research", "perf", "analyse.py"), trace], capture_output=True, text=True)
        with open(os.path.join(RUNS, name + ".analysis.txt"), "w") as f:
            f.write(a.stdout + a.stderr)
        print(a.stdout[-6000:])


if __name__ == "__main__":
    main()
