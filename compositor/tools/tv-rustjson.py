#!/usr/bin/env python3
"""Reads JSON on stdin and prints the value at a dotted PATH, for the gate (tools/tv-gate.sh runs it on the set as /tmp/lumatv/rustjson.py):
`ctl shell | python3 rustjson.py rust.ui.kinds`.

A string prints as it is, true and false as those words, a number as itself, a list as its items joined with commas, an object as compact JSON;
null, a missing key or text that is not JSON print nothing (the caller tells them from a value by its emptiness). A list index is a number in
the path (`rust.kinds.0`). The exit status is 0 in every case: a reading that is not there is an empty answer, not a failure of the gate.

`--without KEY[,KEY]` first takes those top-level keys out of the document, so that `ctl shell | python3 rustjson.py --without rust` is the
QML client's report alone, as compact JSON with its keys sorted (no PATH). The Rust client's report is the `rust` object inside it, whose
keys (`wants`, `stats`, `ui`...) have the names of the QML client's own and sort before some of them: a grep for the first `"wants":` finds the
Rust client's.

`--merge-rust` is that same report with what the Rust client shows folded in where the QML client's own field is empty or false, and no `rust`
object: a Rust panel that is opening or open (quick, picker, drawer) is the `layer` and the `overlay`, the Rust volume display is `osd`, its toasts
are `toasts`, its saver is `saver`, its `wants` is the `wants`, and what it can show is added to `can`. A kind that is on in the Rust client is the
screen the set shows, so the gate's checks that read the QML client's fields (`keys`, `shell`) read the one that is up; with no Rust kind on the
report is the QML client's alone, as with `--without rust`."""
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


def merge_rust(doc):
    """The QML client's report with the Rust client's screens folded in (see the module's text); the `rust` object is dropped."""
    rust = doc.pop("rust", None)
    if not isinstance(rust, dict) or not rust.get("kinds"):
        return doc
    ui = doc.get("ui")
    if not isinstance(ui, dict):
        ui = doc["ui"] = {}
    rui = rust.get("ui") if isinstance(rust.get("ui"), dict) else {}
    panels = rui.get("panels") if isinstance(rui.get("panels"), dict) else {}
    up = next((name for name in ("quick", "picker", "drawer") if panels.get(name) in ("opening", "open")), "")
    if up:
        ui["layer"] = ui.get("layer") or up
        ui["overlay"] = ui.get("overlay") or up
    if rui.get("osd") is True:
        ui["osd"] = True
    toasts = rui.get("toasts")
    if isinstance(toasts, int) and not isinstance(toasts, bool) and toasts > (ui.get("toasts") or 0):
        ui["toasts"] = toasts
    if rui.get("saver") is True:
        ui["saver"] = True
    if rust.get("wants") is True:
        doc["wants"] = True
    can = doc.get("can") if isinstance(doc.get("can"), list) else []
    rust_can = rust.get("can") if isinstance(rust.get("can"), list) else []
    doc["can"] = can + [k for k in rust_can if k not in can]
    return doc


def main():
    args = sys.argv[1:]
    drop = []
    merge = False
    if args[:1] == ["--without"]:
        drop = args[1].split(",") if len(args) > 1 else []
        args = args[2:]
    elif args[:1] == ["--merge-rust"]:
        merge = True
        args = args[1:]
    path = args[0] if args else ""
    try:
        doc = json.loads(sys.stdin.read())
    except ValueError:
        return 0
    if isinstance(doc, dict):
        for key in drop:
            doc.pop(key, None)
        if merge:
            doc = merge_rust(doc)
    print(show(pick(doc, path) if path else doc))
    return 0


if __name__ == "__main__":
    sys.exit(main())
