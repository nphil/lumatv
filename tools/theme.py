#!/usr/bin/env python3
"""Reads, updates or minimises a theme file in the data dir (theme.json, layout.json).

usage: theme.py <file> get
       theme.py <file> set '<json object>'
       theme.py <file> minimize <shipped file>

`set` deep-merges the object into the file (objects merge, everything else replaces) and writes it
atomically; the Home layer reloads it within 2 s. Prints the resulting file.

The Home layer merges the user's file over the shipped one key by key (LumaHome.mergeDeep), so a
user's file only needs what it changes. `minimize` rewrites it to exactly that: every value equal
to the shipped one is dropped, and so are the keys an earlier release used under another name, so
what the user never set follows the shipped file from then on. Run by `lumatv apply`.
"""
import json
import os
import sys

# Keys earlier releases wrote that nothing reads now: the file `version`, `radius` and `tileRadius`
# (now radii.card and radii.tile), and a material's `scrim` list (now wallScrim; scrim is a colour role).
RETIRED = {("version",), ("radius",), ("tileRadius",), ("dark", "scrim"), ("light", "scrim")}


def merge(base, over):
    if not isinstance(base, dict) or not isinstance(over, dict):
        return over
    out = dict(base)
    for k, v in over.items():
        out[k] = merge(base.get(k), v) if isinstance(v, dict) else v
    return out


def minimize(user, shipped, path=()):
    """The part of `user` that differs from `shipped`; objects recurse, empty ones vanish."""
    out = {}
    for k, v in user.items():
        here = path + (k,)
        if here in RETIRED:
            continue
        if isinstance(v, dict) and isinstance(shipped.get(k), dict):
            sub = minimize(v, shipped[k], here)
            if sub:
                out[k] = sub
        elif k not in shipped or shipped[k] != v:
            out[k] = v
    return out


def write(path, data):
    tmp = path + ".tmp.%d" % os.getpid()
    with open(tmp, "w") as f:
        json.dump(data, f, indent=2)
        f.write("\n")
    os.replace(tmp, path)


def main():
    path, mode = sys.argv[1], sys.argv[2]
    try:
        with open(path) as f:
            data = json.load(f)
    except (OSError, ValueError):
        data = {}

    if mode == "set":
        data = merge(data, json.loads(sys.argv[3]))
        write(path, data)
    elif mode == "minimize":
        with open(sys.argv[3]) as f:
            shipped = json.load(f)
        slim = minimize(data, shipped)
        if slim != data or not os.path.exists(path):
            write(path, slim)
        data = slim
    elif mode == "read":                      # read KEY.PATH [DEFAULT]: one value as plain text (the owner's, else DEFAULT)
        node = data
        for part in sys.argv[3].split("."):
            node = node.get(part) if isinstance(node, dict) else None
        print(node if node is not None else (sys.argv[4] if len(sys.argv) > 4 else ""))
        return
    elif mode != "get":
        sys.exit("usage: theme.py <file> get|set|minimize|read ...")

    print(json.dumps({"ok": True, "theme": data}))


if __name__ == "__main__":
    main()
