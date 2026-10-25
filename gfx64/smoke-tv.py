#!/usr/bin/env python3
"""Uploads the 64-bit bundle, shim, launcher and probe to the set's /tmp/gfx64 and runs smoke.sh there.

    python3 gfx64/smoke-tv.py [seconds] [auto|dumb|te]

Needs gfx64/build/bundle (fetch-blob.sh) and gfx64/build/out (build.sh), and paramiko for
docs/research/arch/tv.py. Everything goes to volatile /tmp/gfx64; nothing on the set persists. Only run it
when the set is idle and the main agent has released it: the probe covers the screen for the length of the run.
The output is also written to gfx64/build/smoke-<time>.txt.
"""
import os
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "docs", "research", "arch"))
import tv  # noqa: E402

REMOTE = "/tmp/gfx64"
secs = sys.argv[1] if len(sys.argv) > 1 else "20"
heap = sys.argv[2] if len(sys.argv) > 2 else "auto"

files = [(os.path.join(HERE, "launch.sh"), "launch.sh"), (os.path.join(HERE, "smoke.sh"), "smoke.sh"),
         (os.path.join(HERE, "build/out/heapshim.so"), "heapshim.so"), (os.path.join(HERE, "build/out/x1client"), "x1client")]
bundle = os.path.join(HERE, "build/bundle/lib")
files += [(os.path.join(bundle, f), "bundle/lib/" + f) for f in sorted(os.listdir(bundle))]
for local, _ in files:
    if not os.path.exists(local):
        sys.exit("missing %s (run gfx64/fetch-blob.sh and gfx64/build.sh)" % local)

# Skip files already on the set with the same size (the blob is 56 MB).
listing = tv.tv("mkdir -p %s/bundle/lib; cd %s && stat -c '%%s %%n' launch.sh smoke.sh heapshim.so x1client bundle/lib/* 2>/dev/null" % (REMOTE, REMOTE))
have = {}
for line in listing.splitlines():
    parts = line.split(" ", 1)
    if len(parts) == 2 and parts[0].isdigit():
        have[parts[1].strip()] = int(parts[0])
for local, rel in files:
    if have.get(rel) == os.path.getsize(local):
        continue
    print("put", rel, flush=True)
    tv.put(local, "%s/%s" % (REMOTE, rel))
tv.tv("chmod +x %s/launch.sh %s/smoke.sh %s/x1client %s/bundle/lib/ld-linux-aarch64.so.1" % ((REMOTE,) * 4))

out = tv.tv("cd %s && sh smoke.sh %s %s 2>&1" % (REMOTE, secs, heap), timeout=int(secs) + 120)
print(out)
path = os.path.join(HERE, "build", "smoke-%s.txt" % time.strftime("%Y%m%d-%H%M%S"))
with open(path, "w") as f:
    f.write(out)
print("saved", path)
