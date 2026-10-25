#!/usr/bin/env python3
"""Uploads the helper and smoke.sh to the set's /tmp/lumatv-helper-smoke and runs the smoke test there.

    python3 helper/smoke-tv.py [port]

Needs helper/dist/lumatv-helper (helper/build.sh) and paramiko for docs/research/arch/tv.py. Everything goes to volatile
/tmp/lumatv-helper-smoke and is removed again; nothing on the set persists, the production data dir and port are not touched.
The helper reads the receiver's status page (read only) and fetches weather and news through the set's openssl while it runs,
and the run takes about a minute. The output is also written to helper/dist/smoke-<time>.txt. Exit status 0 = VERDICT PASS.
"""
import os
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "docs", "research", "arch"))
import tv  # noqa: E402

REMOTE = "/tmp/lumatv-helper-smoke"
port = sys.argv[1] if len(sys.argv) > 1 else "18741"

binary = os.path.join(HERE, "dist", "lumatv-helper")
script = os.path.join(HERE, "smoke.sh")
for path in (binary, script):
    if not os.path.exists(path):
        sys.exit("missing %s (run helper/build.sh)" % path)

tv.tv("mkdir -p %s" % REMOTE)
for local, name in ((binary, "lumatv-helper"), (script, "smoke.sh")):
    print("put", name, flush=True)
    tv.put(local, "%s/%s" % (REMOTE, name))
tv.tv("chmod +x %s/lumatv-helper %s/smoke.sh" % (REMOTE, REMOTE))

out = tv.tv("cd %s && sh smoke.sh %s 2>&1; echo \"exit=$?\"" % (REMOTE, port), timeout=240)
print(out)
tv.tv("rm -rf %s" % REMOTE)
path = os.path.join(HERE, "dist", "smoke-%s.txt" % time.strftime("%Y%m%d-%H%M%S"))
os.makedirs(os.path.dirname(path), exist_ok=True)
with open(path, "w") as f:
    f.write(out)
print("saved", path)
sys.exit(0 if "VERDICT PASS" in out else 1)
