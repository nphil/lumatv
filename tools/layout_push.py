#!/usr/bin/env python3
"""Sends a Home layout from this workstation to the TV, or reads the one it has: the way the layout simulator (sim/, a page on the
workstation or on a phone through it) reaches the TV. Nothing listens on the TV for this: the helper's API is loopback only and the one
LAN listener the PLAN allows is AI access, so the layout travels over the SSH the workstation already has to the set, and the TV's own
Home layer picks the file up within 2 s.

usage: layout_push.py check FILE            the layout's problems, from the same rules as the editor on Home; nothing is sent
       layout_push.py put FILE              check it, then replace the TV's layout.json with it (written whole, renamed into place)
       layout_push.py get                   print the TV's layout.json
       layout_push.py reset                 the TV's layout.json becomes `{}`: Home follows the shipped layout again
env:   LUMATV_HOST (192.168.1.242), LUMATV_USER (root), LUMATV_PASS (the Homebrew Channel's default), LUMATV_DATA (/var/lib/lumatv)

No copy of the file it replaces is kept (the way back is `reset`, or `put` of the layout wanted). `FILE` is what the simulator exports
(layout.json), or `-` for stdin.
"""
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import layout as rules  # noqa: E402

SHIPPED = os.path.join(HERE, "..", "payload", "theme", "layout.json")


def connect():
    import paramiko
    c = paramiko.SSHClient()
    c.set_missing_host_key_policy(paramiko.AutoAddPolicy())
    c.connect(os.environ.get("LUMATV_HOST", "192.168.1.242"), username=os.environ.get("LUMATV_USER", "root"),
              password=os.environ.get("LUMATV_PASS", "alpine"), timeout=10, banner_timeout=20, auth_timeout=20, look_for_keys=False, allow_agent=False)
    return c


def remote_file():
    return os.environ.get("LUMATV_DATA", "/var/lib/lumatv") + "/theme/layout.json"


def load(arg):
    text = sys.stdin.read() if arg == "-" else open(arg).read()
    return json.loads(text)


def problems(data):
    return rules.check(data, rules.read(SHIPPED))


def push(data):
    """The whole file to a temporary name beside it, then renamed over it: Home never reads half a file."""
    text = json.dumps(data, indent=2) + "\n"
    c = connect()
    try:
        s = c.open_sftp()
        path = remote_file()
        tmp = path + ".tmp.%d" % os.getpid()
        with s.open(tmp, "w") as f:
            f.write(text)
        s.posix_rename(tmp, path)
        s.close()
    finally:
        c.close()


def fetch():
    c = connect()
    try:
        s = c.open_sftp()
        with s.open(remote_file()) as f:
            return f.read().decode("utf-8")
    finally:
        c.close()


def main(argv):
    if not argv or argv[0] not in ("check", "put", "get", "reset") or (argv[0] in ("check", "put") and len(argv) < 2):
        sys.exit(__doc__)
    mode = argv[0]
    if mode == "get":
        print(fetch().rstrip())
        return 0
    if mode == "reset":
        push({})
        print(json.dumps({"ok": True, "layout": {}}))
        return 0
    try:
        data = load(argv[1])
    except (OSError, ValueError) as e:
        print(json.dumps({"ok": False, "error": "cannot read the layout: %s" % e}))
        return 1
    errors = problems(data)
    if errors:
        print(json.dumps({"ok": False, "error": "; ".join(errors[:6])}))
        return 1
    if mode == "put":
        push(data)
    print(json.dumps({"ok": True, "cards": len(data.get("widgets", []))}))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
