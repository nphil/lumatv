"""Scenes of the overlay system (docs/mocks/overlays): the cards over a film still with its subtitles, dark and light, at 100 % and at 200 % text (`-200`),
as the surface draws them (payload/compositor/lumatv/panels/Overlays.qml and OverlayCard.qml) against the fake helper that publishes the plugin broker's records.

    python3 tests/qmlhost/scenes.py --only overlays-stack,overlays-alert --out /tmp/overlay-shots

Each is `LumaOverlay.qml` beside Home as the fullscreen container loads it (qmlhost.py `--overlay`), an app in front: the film still stands in for it. The
layout audit (layoutaudit.py) runs on the grab at the normal type size and again with the type doubled (overlap only), and the grab is measured:
every card ends above 75 % of the height, inside the 5 % safe area, and no card is where the subtitles are.
"""
import json
import os
import subprocess
import sys

from scenes import HERE, ROOT, audit_path, doubled_type, merge, png_path, scene
import xdisplay

STILL = os.path.join(ROOT, "docs", "mocks", "img", "still.jpg")
READY = 1500            # the fake helper is up and has synced about a second after the host starts
NEWS = 3300             # the hub marks what is there when the helper comes up (1.5 s after it did): a notification from here on is news
ROSTER = {"plugins": [{"id": "ha", "name": "Home Assistant", "version": "0.2.0", "class": "L", "state": "connected", "registered": True},
                      {"id": "scores", "name": "Scores", "version": "1", "class": "S", "state": "connected", "registered": True},
                      {"id": "clock", "name": "Timers", "version": "1", "class": "S", "state": "connected", "registered": True}],
          "firstParty": [], "problem": None}
CAMERAS = {"state": "ready", "reason": "", "now_skew_ms": 0, "rev": 1,
           "cameras": [{"id": "273", "name": "Front Door", "room": "Entry", "type": "doorbell", "online": True, "live": True, "recordings": True, "events": True, "clips": False,
                        "since": 0, "native": {"codec": "h264", "width": 1920, "height": 1080, "fps": 15}, "prebuffered": True}]}
SCORE = {"league": "NBA", "status": "Q4 2:31", "live": True, "home": {"name": "Celtics", "abbr": "BOS", "score": 99}, "away": {"name": "Lakers", "abbr": "LAL", "score": 102}}


def rec(ident, title, body="", channel="ha", icon="base.icons/home", ctitle="Home Assistant", actions=None, **post):
    p = {"id": ident, "channel": channel, "title": title, "body": body, "privacy": "public", "ongoing": False}
    if actions:
        p["actions"] = actions
    p.update(post)
    return {"id": ident, "channel": channel, "channelTitle": ctitle, "channelIcon": icon, "revision": 1, "receivedAt": 0, "expiresAt": None, "post": p}


def publish(topic, doc):
    return (READY, "(function(){var d=%s; var x=new XMLHttpRequest(); x.open('POST','http://127.0.0.1:18731/__publish/%s');"
                   "x.setRequestHeader('Content-Type','application/json'); x.send(JSON.stringify(d)); return 1})()" % (json.dumps(doc), topic))


def notes(at, plugin, *items):
    return (at, "(function(){var d=%s; var x=new XMLHttpRequest(); x.open('POST','http://127.0.0.1:18731/__publish/plugin.%s.notifications');"
                "x.setRequestHeader('Content-Type','application/json'); x.send(JSON.stringify(d)); return 1})()" % (json.dumps({"items": list(items)}), plugin))


def popup_at(at):
    doc = {"id": 1, "event": "e1", "camera": "273", "name": "Front Door", "kind": "doorbell", "label": None, "at": 0, "untilMs": 0, "seq": 1}
    return (at, "(function(){var d=%s; d.at = Date.now() - 5000; d.untilMs = Date.now() + 30000; var x=new XMLHttpRequest(); x.open('POST','http://127.0.0.1:18731/__publish/cameras.popup');"
                "x.setRequestHeader('Content-Type','application/json'); x.send(JSON.stringify(d)); return 1})()" % json.dumps(doc))


def render(name, out_dir, material, steps, snap_at, big=False, audit=True):
    """One scene: `steps` [(ms, js)] run in the host; a grab at `snap_at`; the audit at the normal size and (when `audit`) doubled. `big` doubles the type
    from the start (the picture at 200 %). Returns (exit code, warnings, the host's output)."""
    path = png_path(out_dir, name, material)
    if os.path.exists(path):
        os.remove(path)
    user = {"perf": {"syncResidents": True}, "material": material, "overlays": {"enabled": True, "armMs": 300}}
    if big:
        user = merge(user, doubled_type())
    args = [sys.executable, os.path.join(HERE, "qmlhost.py"), "LumaHome.qml", "--fake-helper", "--overlay", "--theme", json.dumps(user),
            "--eval", "100:overlay.warm = true", "--bgimage", STILL, "--prop", "hostActive=false", "--eval", "800:overlay.appFront = true"]
    for ms, code in steps:
        args += ["--eval", "%d:%s" % (ms, code)]
    # the geometry of what is shown, printed for the check below
    args += ["--eval", "%d:(function(){ var ov = overlay.overlaysItem; var cam = overlay.cameraItem; return JSON.stringify({cards: ov ? ov.geometry() : [], popup: overlay.popupUp ? {y: 134, bottom: cam.popupBottom, ph: cam.previewH} : null}) })()" % (snap_at - 50)]
    audits = []
    last = snap_at
    if audit:
        small = audit_path(out_dir, name, material)
        for p in (small, audit_path(out_dir, name, material, 200)):
            if os.path.exists(p):
                os.remove(p)
        args += ["--audit", "%d:%s" % (snap_at, small)]
        audits = [small]
        if not big:
            bigp = audit_path(out_dir, name, material, 200)
            args += ["--eval", "%d:(function(){root.theme = root.mergeDeep(root.theme, %s); return 1})()" % (snap_at + 100, json.dumps(doubled_type())),
                     "--audit", "%d:%s:overlap" % (snap_at + 700, bigp)]
            audits.append(bigp)
            last = snap_at + 700
    args += ["--snap", "%d:%s" % (snap_at, path), "--wait", str(last + 600)]
    r = subprocess.run(args, env=dict(os.environ, DISPLAY=xdisplay.DISPLAY), capture_output=True, text=True, timeout=240)
    text = r.stdout + r.stderr
    warn = [ln for ln in text.splitlines() if ln.startswith("[warning]") and "Implicitly defined onFoo" not in ln]
    if not os.path.exists(path):
        warn.append("[snap] not written: %s" % path)
    if audits:
        import layoutaudit      # needs PyQt5, like the host itself
    for jpath in audits:
        if not os.path.exists(jpath):
            warn.append("[audit] not written: %s" % jpath)
            continue
        with open(jpath) as f:
            warn += layoutaudit.lines(json.load(f), "layout 200%" if jpath.endswith("-200.json") else "layout")
    geometry = None
    for ln in text.splitlines():
        if ln.startswith("[eval] (") and '"cards"' in ln:
            geometry = json.loads(ln[len("[eval] ("):].rsplit(", ", 1)[0].strip("'\""))
    if geometry is None:
        warn.append("[geometry] not read")
    else:
        warn += check(geometry)
    return (r.returncode or (1 if not os.path.exists(path) else 0)), warn, text


def check(geometry):
    """The card rectangles against the rules: inside the safe area, below the status bar's row, above 75 % of the height, and the camera's card under none."""
    out = []
    cards = geometry["cards"]
    if geometry.get("popup"):
        out.append("[zone] the camera's card ends at %d" % geometry["popup"]["bottom"] if geometry["popup"]["bottom"] > 810 else "")
    for i, g in enumerate(cards):
        if g["y"] + g["h"] > 810:
            out.append("[zone] card %d ends at %d, below 75 %% of the height" % (i, g["y"] + g["h"]))
        if g["x"] + g["w"] != 1920 - 96:
            out.append("[zone] card %d ends at x %d, not the safe area's edge" % (i, g["x"] + g["w"]))
        if g["y"] < 134:
            out.append("[zone] card %d starts at %d, above the status bar's row" % (i, g["y"]))
    return [o for o in out if o]


# the cards over the film still, at 100 % text and at 200 % (`-200`)
def both(name, steps, at, doc):
    def run100(out_dir, material):
        return render(name, out_dir, material, steps, at)

    def run200(out_dir, material):
        return render(name + "-200", out_dir, material, steps, at, big=True, audit=True)
    run100.__doc__ = doc
    run200.__doc__ = doc + " At 200 % text."
    scene(name, runner=True)(run100)
    scene(name + "-200", runner=True)(run200)


ALERT = notes(NEWS, "ha", rec("low", "Low battery", "Back door sensor is at 5 %"))
both("overlays-alert", [publish("plugins", ROSTER), notes(READY + 100, "ha"), ALERT], NEWS + 1400, "A Home Assistant alert over a film: one card, the first of the column.")
both("overlays-timer", [publish("plugins", ROSTER), notes(READY + 100, "clock"),
                        notes(NEWS, "clock", rec("pasta", "Pasta", "Time's up", channel="timers", icon="base.icons/timer", ctitle="Kitchen Show", actions=[{"action": "stop", "label": "Stop", "target": "pasta"}]))],
     NEWS + 1400, "A timer that finished, with its own button: OK says Stop.")
both("overlays-alarm", [publish("plugins", ROSTER), notes(READY + 100, "clock"),
                        notes(NEWS, "clock", rec("wk", "Weekday", "Alarm 7:00", channel="alarms", icon="base.icons/alarm-clock", ctitle="Bedroom Show"))],
     NEWS + 1400, "An alarm: the warning tone, no button of its own, so OK opens the drawer.")
both("overlays-score", [publish("plugins", ROSTER), notes(READY + 100, "scores"),
                        notes(NEWS, "scores", rec("g1", "Lakers 102, Celtics 99", "Q4 2:31", channel="scores", icon="base.icons/trophy", ctitle="Scores", score=SCORE))],
     NEWS + 1400, "A live score: the two sides, the clock, the one ahead in the full ink.")
both("overlays-score-text", [publish("plugins", ROSTER), notes(READY + 100, "scores"),
                             notes(NEWS, "scores", rec("g1", "Lakers 102, Celtics 99", "Q4 2:31", channel="scores", icon="base.icons/trophy", ctitle="Scores"))],
     NEWS + 1400, "A score as plain words: no structured score in the record, so the title and the line.")
both("overlays-alerts", [publish("plugins", ROSTER), notes(READY + 100, "ha"),
                         notes(NEWS, "ha", rec("low", "Low battery", "Back door sensor is at 5 %"), rec("leak", "Water leak", "Basement, near the heater", level="critical"))],
     NEWS + 1400, "Two alerts of one class: stacked, the second under the first.")
both("overlays-stack", [publish("plugins", ROSTER), publish("cameras", CAMERAS), notes(READY + 100, "clock"), popup_at(NEWS - 200),
                        notes(NEWS + 600, "clock", rec("pasta", "Pasta", "Time's up", channel="timers", icon="base.icons/timer", ctitle="Kitchen Show"))],
     NEWS + 2200, "The doorbell's camera card over a timer that finished: the most two cards ever show together; at 200 % text the timer waits.")
