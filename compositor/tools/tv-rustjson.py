#!/usr/bin/env python3
"""Reads JSON on stdin and prints the value at a dotted PATH, for the gate (tools/tv-gate.sh runs it on the set as /tmp/lumatv/rustjson.py):
`ctl shell | python3 rustjson.py rust.ui.kinds`.

A string prints as it is, true and false as those words, a number as itself, a list as its items joined with commas, an object as compact JSON;
null, a missing key or text that is not JSON print nothing (the caller tells them from a value by its emptiness). A list index is a number in
the path (`rust.kinds.0`). The exit status is 0 in every case: a reading that is not there is an empty answer, not a failure of the gate.

`--without KEY[,KEY]` first takes those top-level keys out of the document, so that `ctl shell | python3 rustjson.py --without rust` is the
QML client's report alone, as compact JSON with its keys sorted (no PATH). The Rust client's report is the `rust` object inside it, whose
keys (`wants`, `stats`, `ui`...) have the names of the QML client's own and sort before some of them: a grep for the first `"wants":` finds the
Rust client's."""
import json
import sys


def pick(doc, path):
    for part in path.split("."):
        if isinstance(doc, dict):
            doc = doc.get(part)
        elif isinstance(doc, list) and part.isdigit() and int(part) < len(doc):
            doc = doc[int(part)]
        else:
            return None
        if doc is None:
            return None
    return doc


def show(v):
    if v is None:
        return ""
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, list):
        return ",".join(show(i) for i in v)
    if isinstance(v, dict):
        return json.dumps(v, sort_keys=True, separators=(",", ":"))
    return str(v)


def main():
    args = sys.argv[1:]
    drop = []
    if args[:1] == ["--without"]:
        drop = args[1].split(",") if len(args) > 1 else []
        args = args[2:]
    path = args[0] if args else ""
    try:
        doc = json.loads(sys.stdin.read())
    except ValueError:
        return 0
    if isinstance(doc, dict):
        for key in drop:
            doc.pop(key, None)
    print(show(pick(doc, path) if path else doc))
    return 0


if __name__ == "__main__":
    sys.exit(main())
