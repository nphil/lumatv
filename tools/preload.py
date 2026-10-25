#!/usr/bin/env python3
"""Keeps LG's background preloads down to what the owner wants warm.

usage: preload.py status <payload> <orig> <state>
       preload.py on     <payload> <orig> <state>
       preload.py off    <payload> <orig> <state>
       preload.py wake   <payload> <orig> <state>

Runs on the set (python 3.10). Called by `lumatv preload`; the on-the-set facts are in
docs/research/surfaces-p2-result.md.

The preload manager reads configd's com.webos.service.preloadmanager.supportAppList (one
entry per app with isEnabled) when it starts and at every switch-on, and loads as many enabled
apps as memory allows: YouTube at 58 MB, Prime Video 70 MB, the browser 39 MB, LG Channels 12 MB
(plus 21 MB swapped), and with room to spare LG's Live TV at 42 MB, the voice app at 39 MB and
the HDMI input app of a connected port at 32 MB (set, 2026-10-25). `payload` (payload/preload.json)
names the apps to switch off: everything but Netflix and the Home stub. `on` saves the list as LG
has it in `orig` (once; never overwritten, so a second run cannot save the changed list), writes
the list back with those entries off, tells the running preload manager with setPreloadPolicy,
and records the ids in `state` (`<dataDir>/preload.json`, which the helper reads at every
screen-on: webOS 10 forgets the list at each switch-on and the preload manager loads the apps
again). setPreloadPolicy is answered only while the screen is on; in Active Standby the preload
manager holds the call ("Pending-setPreloadPolicy") until the timeout, so `on` run then reports
the ids as failed and the helper's next screen-on sends them. `off` puts the entries back as
`orig` has them. `wake` sends setPreloadPolicy again. Nothing here closes an app: one that is
loaded stays loaded, and it is not loaded again. The voice app ignores closeByAppId; it ends
on SIGTERM and is not loaded again once the policy says so.
"""
import json
import os
import subprocess
import sys

KEY = "com.webos.service.preloadmanager.supportAppList"
LUNA_TIMEOUT = 10


def luna(uri, payload):
    try:
        r = subprocess.run(["luna-send", "-n", "1", uri, json.dumps(payload)], capture_output=True, text=True,
                           timeout=LUNA_TIMEOUT, stdin=subprocess.DEVNULL)
    except subprocess.TimeoutExpired:
        return None
    try:
        return json.loads(r.stdout)
    except ValueError:
        return None


def read_list():
    reply = luna("luna://com.webos.service.config/getConfigs", {"configNames": [KEY]})
    lst = reply and reply.get("configs", {}).get(KEY)
    return lst if isinstance(lst, list) else None


def write_list(lst):
    reply = luna("luna://com.webos.service.config/setConfigs", {"configs": {KEY: lst}})
    return bool(reply and reply.get("returnValue"))


def set_live(app_id, enabled):
    reply = luna("luna://com.webos.service.preloadmanager/setPreloadPolicy",
                 {"application": {"id": app_id, "isEnabled": enabled}})
    return bool(reply and reply.get("returnValue"))


def read_json(path, default):
    try:
        with open(path) as f:
            return json.load(f)
    except (OSError, ValueError):
        return default


def write_json(path, value):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = f"{path}.tmp.{os.getpid()}"
    with open(tmp, "w") as f:
        json.dump(value, f, indent=1)
        f.write("\n")
        f.flush()
        os.fsync(f.fileno())
    os.replace(tmp, path)


def wanted_off(payload):
    cfg = read_json(payload, {})
    ids = cfg.get("off")
    return [i for i in ids if isinstance(i, str)] if isinstance(ids, list) else []


def with_enabled(lst, ids, enabled_by_id):
    """lst with each of ids set to enabled_by_id[id] (False for all when it is a plain bool)."""
    out = []
    for e in lst:
        e = dict(e)
        if e.get("id") in ids:
            e["isEnabled"] = enabled_by_id if isinstance(enabled_by_id, bool) else enabled_by_id.get(e["id"], e.get("isEnabled"))
        out.append(e)
    return out


def status(payload, orig, state):
    lst = read_list()
    ids = wanted_off(payload)
    entries = {e.get("id"): e for e in (lst or [])}
    return {
        "ok": lst is not None,
        "on": os.path.exists(state),
        "backup": os.path.exists(orig),
        "off": [{"id": i, "listed": i in entries, "enabled": entries.get(i, {}).get("isEnabled")} for i in ids],
        "enabled": sorted(e["id"] for e in (lst or []) if e.get("isEnabled")),
    }


def turn_on(payload, orig, state):
    ids = wanted_off(payload)
    lst = read_list()
    if lst is None:
        return {"ok": False, "error": "the preload list could not be read"}
    if not os.path.exists(orig):
        write_json(orig, lst)
    target = with_enabled(lst, ids, False)
    if target != lst and not write_list(target):
        return {"ok": False, "error": "the preload list could not be written"}
    failed = [i for i in ids if not set_live(i, False)]
    write_json(state, {"off": ids})
    return {"ok": not failed, "off": ids, "changed": target != lst, "failed": failed}


def turn_off(payload, orig, state):
    saved = read_json(orig, None)
    if not isinstance(saved, list):
        return {"ok": False, "error": "no saved list to restore"}
    ids = wanted_off(payload) or (read_json(state, {}).get("off") or [])
    original = {e.get("id"): e.get("isEnabled") for e in saved}
    lst = read_list()
    if lst is None:
        return {"ok": False, "error": "the preload list could not be read"}
    target = with_enabled(lst, ids, original)
    if target != lst and not write_list(target):
        return {"ok": False, "error": "the preload list could not be written"}
    failed = [i for i in ids if i in original and not set_live(i, bool(original[i]))]
    try:
        os.remove(state)
    except OSError:
        pass
    return {"ok": not failed, "restored": ids, "failed": failed}


def wake(payload, orig, state):
    ids = read_json(state, {}).get("off") or []
    failed = [i for i in ids if not set_live(i, False)]
    return {"ok": not failed, "off": ids, "failed": failed}


def main(argv):
    if len(argv) != 5 or argv[1] not in ("status", "on", "off", "wake"):
        print(__doc__.split("\n\n")[0], file=sys.stderr)
        return 2
    out = {"status": status, "on": turn_on, "off": turn_off, "wake": wake}[argv[1]](*argv[2:])
    print(json.dumps(out))
    return 0 if out.get("ok") else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv))
