#!/usr/bin/env python3
"""Points a system-UI panel's QML entry at a different file, through configd.

The compositor loads Quick Settings, the input picker, the context menu and the
rest in-process, from whatever `main` the configd object
com.webos.surfacemanager.systemUIManager gives for each app id. The value is a
plain file:// URL and the manager subscribes to it, so redirecting one is a
supported, reversible, reboot-surviving change that writes nothing to /usr.

usage: systemui.py on <appId> <qml path> | off <appId> | status
Runs on the set (python 3.10). Called by tools/lumatv; see systemui_set.
"""
import json
import subprocess
import sys

NAME = "com.webos.surfacemanager.systemUIManager"
# Where each app id's stock entry points, so `off` restores the exact original
# without needing a saved copy that a firmware update could invalidate.
STOCK = {
    "com.webos.app.quicksettings": "file:///usr/lib/qt5/qml/QuickSettings/QuickSettings.qml",
    "com.webos.app.quickinputpicker": "file:///usr/lib/qt5/qml/InputPicker/InputPicker.qml",
    "com.webos.app.contextmenu": "file:///usr/lib/qt5/qml/ContextMenu/ContextMenu.qml",
}


def luna(uri, payload):
    return subprocess.run(["luna-send", "-n", "1", uri, json.dumps(payload)],
                          capture_output=True, text=True).stdout


def read():
    reply = json.loads(luna("luna://com.webos.service.config/getConfigs",
                            {"configNames": [NAME]}))
    return reply["configs"][NAME]


mode = sys.argv[1]
config = read()

if mode == "status":
    print(json.dumps({e["id"]: e.get("main") for e in config["appinfo"] if e.get("main")}))
    sys.exit(0)

app_id = sys.argv[2]
if mode == "on":
    target = "file://" + sys.argv[3]
elif mode == "off":
    target = STOCK.get(app_id)
    if target is None:
        sys.exit("no stock entry known for " + app_id)
else:
    sys.exit("usage: systemui.py on|off|status")

found = False
for entry in config["appinfo"]:
    if entry.get("id") == app_id:
        entry["main"] = target
        found = True
if not found:
    sys.exit(app_id + " has no systemUIManager entry")

reply = luna("luna://com.webos.service.config/setConfigs", {"configs": {NAME: config}})
sys.exit(0 if '"returnValue":true' in reply else 1)
