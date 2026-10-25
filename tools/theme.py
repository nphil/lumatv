#!/usr/bin/env python3
"""Reads or updates the data dir's theme/theme.json for the companion app and the Home popovers.

usage: theme.py <theme.json path> get
       theme.py <theme.json path> set '<json object>'
`set` deep-merges the object into the file (objects merge, everything else replaces) and
writes it atomically; the Home layer reloads it within 2 s. Prints the resulting theme.
"""
import json
import os
import sys

path, mode = sys.argv[1], sys.argv[2]


def merge(base, over):
    if not isinstance(base, dict) or not isinstance(over, dict):
        return over
    out = dict(base)
    for k, v in over.items():
        out[k] = merge(base.get(k), v) if isinstance(v, dict) else v
    return out


try:
    with open(path) as f:
        theme = json.load(f)
except (OSError, ValueError):
    theme = {}

if mode == "set":
    theme = merge(theme, json.loads(sys.argv[3]))
    tmp = path + ".tmp.%d" % os.getpid()
    with open(tmp, "w") as f:
        json.dump(theme, f, indent=2)
        f.write("\n")
    os.replace(tmp, path)

print(json.dumps({"ok": True, "theme": theme}))
