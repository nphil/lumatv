"""Scenes of the notification view: toast, alert, input popup and PIN prompt, dark and light.

These are not Home's surfaces: the view is LG's `SystemUI/Notification` tree patched by tools/notification_patch.py with
LumaTV's own files laid over it, so each scene runs it through tests/qmlhost/notify.py (LG's real data layer behind
stand-ins for what is native) over a render of Home in the same material, and registers with `runner=True`.
"Light" stages the view with the light material as its palette: on the set the sheet is always the dark material's
(tools/tokens.py), so this shows the same files in a theme whose sheet is light.

    python3 tests/qmlhost/scenes.py --only 07-toast,06-alert,18-device-connected,19-pin-prompt --out /tmp/notify-shots
"""
import json
import os
import subprocess
import sys
import tempfile

from scenes import scene, png_path, ROOT, HERE
import xdisplay

_BACKDROPS = {}
_DIR = tempfile.mkdtemp(prefix="notify-scenes-")
APP = os.path.join(ROOT, "docs", "mocks", "img", "apps", "crunchyroll.png")
INPUT_ICON = os.path.join(ROOT, "assets", "icon160.png")


def backdrop(material):
    """Home in this material, rendered once per run: what the view is drawn over."""
    if material not in _BACKDROPS:
        path = os.path.join(_DIR, "home-%s.png" % material)
        user = {"perf": {"syncResidents": True}, "material": material}
        r = subprocess.run([sys.executable, os.path.join(HERE, "qmlhost.py"), "LumaHome.qml", "--fake-helper", "--prop", "hostActive=true",
                            "--theme", json.dumps(user), "--shot", path, "--wait", "8000"],
                           env=dict(os.environ, DISPLAY=xdisplay.DISPLAY), capture_output=True, text=True, timeout=120)
        if not os.path.exists(path):
            raise RuntimeError("Home backdrop not rendered: " + (r.stdout + r.stderr)[-400:])
        _BACKDROPS[material] = path
    return _BACKDROPS[material]


def render(name, out_dir, material, docs, press=(), wait=1800):
    """`docs` is [(kind, ms, json)]; `press` is [(ms, "KEY,KEY")]. One grab at `wait` ms."""
    path = png_path(out_dir, name, material)
    if os.path.exists(path):
        os.remove(path)
    args = [sys.executable, os.path.join(HERE, "notify.py"), "--patched", "--backdrop", backdrop(material), "--wait", str(wait),
            "--shot", "%d:%s" % (wait, path)]
    if material == "light":
        args.append("--light")
    for kind, ms, doc in docs:
        args += ["--" + kind, "%d:%s" % (ms, json.dumps(doc))]
    for ms, keys in press:
        args += ["--press", "%d:%s" % (ms, keys)]
    r = subprocess.run(args, env=dict(os.environ, DISPLAY=xdisplay.DISPLAY), capture_output=True, text=True, timeout=240)
    text = r.stdout + r.stderr
    warn = [ln for ln in text.splitlines() if ln.startswith("[warning]") and "Implicitly defined onFoo" not in ln]
    if not os.path.exists(path):
        warn.append("[snap] not written: %s" % path)
    return (r.returncode or (1 if not os.path.exists(path) else 0)), warn, text


TOAST = {"sourceId": "com.webos.app.home", "type": "advanced", "timeout": 30, "iconPath": "", "readable": "", "userData": {"id": "scene"},
         "message": "Volume 24 \u00b7 set by Cody\nAllowed by your AI access settings", "actionLabel": "Undo",
         "action": {"serviceURI": "luna://com.webos.audio", "serviceMethod": "/setVolume", "launchParams": {"volume": 18}}}

ALERT = {"alertInfo": {"sourceId": "com.webos.app.home", "title": "Uninstall Crunchyroll?", "iconUrl": "file://" + APP,
                       "message": "The app and its sign-in on this TV are removed. Downloads in the app are lost. It can be installed again from the LG Content Store.",
                       "readable": "", "showTimeout": False, "modal": True, "timeout": 0, "timestamp": 11,
                       "buttons": [{"label": "Keep it", "focus": True, "params": {}}, {"label": "Uninstall", "danger": True, "params": {}}]}}

# What LG sends for a warning (the protected kind: emergency alerts, thermal and battery warnings): drawn full screen, never filtered.
WARNING = {"alertInfo": {"sourceId": "com.webos.app.home", "type": "warning", "title": "The TV is too hot",
                         "message": "The TV will turn off to cool down. Move it away from heat and leave the vents clear.",
                         "readable": "", "showTimeout": False, "modal": True, "timeout": 0, "timestamp": 12,
                         "buttons": [{"label": "OK", "focus": True, "params": {}}]}}

INPUT = {"alertAction": "open", "alertInfo": {"sourceId": "com.webos.app.hdmi3", "timeout": 30, "timestamp": 13, "portName": "PlayStation 5",
                                               "portIcon": "file://" + INPUT_ICON,
                                               "action": {"serviceURI": "luna://com.webos.applicationManager", "serviceMethod": "/launch",
                                                          "launchParams": {"id": "com.webos.app.hdmi3"}}}}

PIN = {"pincodePromptAction": "open", "pincodePromptInfo": {"appId": "com.webos.app.hdmi3", "title": "Enter the TV PIN", "promptType": "parental",
                                                          "message": "HDMI 3 \u00b7 PlayStation 5 is locked for the kids.", "retry": 0, "timestamp": 14}}


@scene("07-toast", runner=True)
def _(out_dir, material):
    return render("07-toast", out_dir, material, [("toast", 300, TOAST)])


@scene("06-alert", runner=True)
def _(out_dir, material):
    return render("06-alert", out_dir, material, [("alert", 300, ALERT)])


@scene("06-alert-warning", runner=True)
def _(out_dir, material):
    return render("06-alert-warning", out_dir, material, [("alert", 300, WARNING)])


@scene("18-device-connected", runner=True)
def _(out_dir, material):
    return render("18-device-connected", out_dir, material, [("input", 300, INPUT)])


@scene("19-pin-prompt", runner=True)
def _(out_dir, material):
    # two digits in, the way the mock shows it
    return render("19-pin-prompt", out_dir, material, [("pin", 300, PIN)], press=[(1000, "5,5")])


@scene("19-pin-retry", runner=True)
def _(out_dir, material):
    retry = json.loads(json.dumps(PIN))
    retry["pincodePromptInfo"].update({"retry": 1, "message": "", "timestamp": 15})
    return render("19-pin-retry", out_dir, material, [("pin", 300, retry)], press=[(1000, "1,2,3")])
