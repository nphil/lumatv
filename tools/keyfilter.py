#!/usr/bin/env python3
"""Adds or removes our entries in configd's com.webos.surfacemanager.keyFilters.

usage: keyfilter.py add|remove <handler> <script path>
Runs on the set (python 3.10). Called by tools/lumatv; see keyfilter_set there.

Two entries of ours, both functions of the one script (the stock loader makes one object per file, so they share its state):
<handler> (handleLumaHome) goes first, because LG's handleSystemKeys answers Back with NotAccepted while Home is up and a later entry
would never see it; handleLumaQuick goes just before LG's handleHotkeys, the handler that opens Quick Settings and the input picker, so
the replay of a gear tap has been through LG's long-press handler and the AV-reset counter by then. A chain without handleHotkeys
(another firmware) gets the first entry only: LG's windows then open as they always did.
"""
import json
import subprocess
import sys

NAME = "com.webos.surfacemanager.keyFilters"
LATE = "handleLumaQuick"
BEFORE = "handleHotkeys"


def luna(uri, payload):
    return subprocess.run(["luna-send", "-n", "1", uri, json.dumps(payload)],
                          capture_output=True, text=True).stdout


def build(chain, mode, handler, path):
    """The chain with our entries taken out and, for add, put back in their places."""
    entries = [e for e in chain if e.get("handler") not in (handler, LATE)]
    if mode == "add":
        entries.insert(0, {"handler": handler, "file": path})
        for i, e in enumerate(entries):
            if e.get("handler") == BEFORE:
                entries.insert(i, {"handler": LATE, "file": path})
                break
    return entries


if __name__ == "__main__":
    mode, handler, path = sys.argv[1:4]
    current = json.loads(luna("luna://com.webos.service.config/getConfigs", {"configNames": [NAME]}))
    chain = current["configs"][NAME]
    entries = build(chain, mode, handler, path)
    # Every write makes the compositor reload the whole chain and rerun each LG
    # filter's init(), which in Game mode opens the Game Dashboard. Write only on
    # an actual change.
    if entries == chain:
        sys.exit(0)
    reply = luna("luna://com.webos.service.config/setConfigs", {"configs": {NAME: entries}})
    sys.exit(0 if '"returnValue":true' in reply else 1)
