#!/usr/bin/env python3
"""Reads, checks and writes the owner's Home layout (layout.json in the data dir). Stdlib only: the TV runs it (`lumatv layout`), and the
workstation's push tool (tools/layout_push.py) checks a layout with the same rules before it sends it.

usage: layout.py <file> get
       layout.py <file> check '<json>' [--shipped FILE]
       layout.py <file> put '<json>' [--shipped FILE]     replace the file with this layout (it must pass `check`)
       layout.py <file> set '<json>'                      merge it into the file (objects merge, lists and numbers replace)
       layout.py <file> reset                             the file becomes `{}`: Home follows the shipped layout again

The Home layer merges the owner's file over the shipped one key by key (LumaHome.mergeDeep) and reads it again within 2 s, so a write is
live without a restart. The rules are the layout editor's (payload/compositor/lumatv/LayoutGrid.js): every card has an id (unique) and a
type, whole numbers for its column, row, span and rows, it sits inside the grid, and no two cards (a hidden one takes no room) cover the
same cell. A file is written whole and atomically (a temporary file renamed over it); no copy of the file it replaces is kept: the way
back is `reset`, or `put` of the layout wanted.
"""
import json
import os
import sys


def merge(base, over):
    if not isinstance(base, dict) or not isinstance(over, dict):
        return over
    out = dict(base)
    for k, v in over.items():
        out[k] = merge(base.get(k), v) if isinstance(v, dict) else v
    return out


def whole(v, low):
    return isinstance(v, int) and not isinstance(v, bool) and v >= low


def check(layout, shipped=None):
    """The reasons a layout cannot be Home's, as a list of short sentences; empty when it can. `shipped` (the shipped layout) gives the grid
    a file that only names cards leaves out."""
    if not isinstance(layout, dict):
        return ["the layout is not an object"]
    full = merge(shipped or {}, layout)
    errors = []
    grid = full.get("grid", {})
    cols, rows = 12, 8
    if not isinstance(grid, dict):
        errors.append("grid is not an object")
    else:
        if "cols" in grid:
            if whole(grid["cols"], 1) and grid["cols"] <= 24:
                cols = grid["cols"]
            else:
                errors.append("grid.cols is a whole number from 1 to 24")
        r = grid.get("rows", rows)
        if isinstance(r, list):
            if not r or not all(isinstance(x, (int, float)) and not isinstance(x, bool) and x > 0 for x in r):
                errors.append("grid.rows as a list holds heights greater than zero")
            else:
                rows = len(r)
        elif whole(r, 1) and r <= 32:
            rows = r
        else:
            errors.append("grid.rows is a whole number from 1 to 32, or a list of heights")
    widgets = layout.get("widgets")
    if widgets is None:
        return errors
    if not isinstance(widgets, list):
        return errors + ["widgets is not a list"]
    seen = {}
    cards = []
    for i, e in enumerate(widgets):
        where = "card %d" % (i + 1)
        if not isinstance(e, dict):
            errors.append(where + " is not an object")
            continue
        cid = e.get("id")
        if not isinstance(cid, str) or cid == "":
            errors.append(where + " has no id")
        elif cid in seen:
            errors.append("id %s is used twice" % cid)
        else:
            seen[cid] = True
        where = cid if isinstance(cid, str) and cid else where
        if not isinstance(e.get("type"), str) or e.get("type") == "":
            errors.append(where + " has no type")
        ok = True
        for key, low in (("col", 0), ("row", 0), ("span", 1), ("rows", 1)):
            if not whole(e.get(key), low):
                errors.append("%s: %s is a whole number from %d" % (where, key, low))
                ok = False
        if ok and (e["col"] + e["span"] > cols or e["row"] + e["rows"] > rows):
            errors.append("%s is outside the grid (%d columns, %d rows)" % (where, cols, rows))
            ok = False
        if ok and not e.get("hidden"):
            for o in cards:
                if e["col"] < o["col"] + o["span"] and o["col"] < e["col"] + e["span"] and e["row"] < o["row"] + o["rows"] and o["row"] < e["row"] + e["rows"]:
                    errors.append("%s covers %s" % (where, o["id"]))
            cards.append(e)
    return errors


def read(path):
    try:
        with open(path) as f:
            data = json.load(f)
        return data if isinstance(data, dict) else {}
    except (OSError, ValueError):
        return {}


def write(path, data):
    tmp = path + ".tmp.%d" % os.getpid()
    with open(tmp, "w") as f:
        json.dump(data, f, indent=2)
        f.write("\n")
    os.replace(tmp, path)


def main(argv):
    path, mode = argv[0], argv[1]
    rest = argv[2:]
    shipped = None
    if "--shipped" in rest:
        i = rest.index("--shipped")
        shipped = read(rest[i + 1])
        rest = rest[:i] + rest[i + 2:]
    if mode == "get":
        out = {"ok": True, "layout": read(path)}
    elif mode in ("check", "put", "set"):
        if not rest:
            sys.exit("%s needs a JSON object" % mode)
        try:
            new = json.loads(rest[0])
        except ValueError:
            print(json.dumps({"ok": False, "error": "that is not JSON"}))
            return 1
        data = merge(read(path), new) if mode == "set" else new
        errors = check(data, shipped)
        if errors:
            print(json.dumps({"ok": False, "error": "; ".join(errors[:6])}))
            return 1
        if mode != "check":
            write(path, data)
        out = {"ok": True, "layout": data}
    elif mode == "reset":
        write(path, {})
        out = {"ok": True, "layout": {}}
    else:
        sys.exit("usage: layout.py <file> get|check|put|set|reset ...")
    print(json.dumps(out))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
