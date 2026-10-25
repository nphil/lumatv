#!/usr/bin/env python3
"""Renders LumaTV's surfaces through the offline host into docs/mocks/host/, one PNG per mock they answer.

    python3 tests/qmlhost/scenes.py [--out DIR] [--only NAME[,NAME]] [--list]

Each scene is Home (LumaHome.qml) with a fake helper, driven by the host's own clock: topics published, keys
sent, a surface opened by the way Home opens it, and a window grab at the end. The file names follow the mocks
(`docs/mocks/shots/NN-name.png`), so a host render sits next to the design it answers. Qt 5.15 on software GL
draws what the set's 5.12 draws up to the look of a pixel: this shows layout, colour, text and states, and measures
nothing (the gate on the set is the only timing).

A scene lists steps: ("eval", ms, js), ("keys", ms, "down right ok"), ("publish", ms, topic, doc), ("snap", ms, name).
`READY` is when Home has built its resident surfaces (the build chain starts a second after Home presents).
"""
import argparse
import json
import os
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
sys.path.insert(0, os.path.join(ROOT, "tests"))
import xdisplay  # noqa: E402

READY = 7000
CODES = {"left": "Qt.Key_Left", "right": "Qt.Key_Right", "up": "Qt.Key_Up", "down": "Qt.Key_Down", "ok": "Qt.Key_Return", "back": "Qt.Key_Back",
         "menu": "Qt.Key_Menu", "volup": "Qt.Key_VolumeUp", "voldown": "Qt.Key_VolumeDown"}
NOW = int(time.time() * 1000)

AI = {"listener": {"enabled": True, "bound": True, "addr": "192.168.1.242", "port": 18732, "since": NOW - 3600000, "error": None}, "pairing": None,
      "assistants": [
          {"id": "cody", "name": "Cody", "createdAt": 1, "lastSeenAt": NOW - 120000, "paused": False, "calls": 12, "levels": {"state": "always", "playback": "always", "launch": "ask", "power": "ask"}},
          {"id": "ha", "name": "Home Assistant", "createdAt": 2, "lastSeenAt": NOW - 86400000, "paused": True, "calls": 40, "levels": {"state": "always", "playback": "always", "launch": "ask", "power": "never"}}],
      "limits": {"maxVolume": 60},
      "groups": [{"id": "state", "title": "See what is playing", "default": "always"}, {"id": "playback", "title": "Play, pause, volume", "default": "always"},
                 {"id": "launch", "title": "Open apps, switch inputs", "default": "ask"}, {"id": "power", "title": "Turn the TV off", "default": "ask"}], "tools": []}
AUDIT = [{"assistant": {"id": "ha", "name": "Home Assistant"}, "tool": "tv_media_key", "message": "pause Plex", "outcome": "ok", "at": NOW - 86400000},
         {"assistant": {"id": "cody", "name": "Cody"}, "tool": "tv_get_now_playing", "message": "what is playing", "outcome": "ok", "at": NOW - 420000},
         {"assistant": {"id": "cody", "name": "Cody"}, "tool": "tv_standby", "message": "turn the TV off", "outcome": "denied", "at": NOW - 300000},
         {"assistant": {"id": "cody", "name": "Cody"}, "tool": "tv_launch_app", "message": "volume 32 \u2192 24", "outcome": "ok", "at": NOW - 120000}]
PROMPT = {"kind": "tv_standby", "title": "Cody wants to turn off the TV", "detail": "The TV turns off now. The receiver stays as it is.",
          "assistant": {"id": "cody", "name": "Cody"}, "reason": "Nate asked me to shut everything down for the night.", "expiresAt": NOW + 24000 + 600000}


def receiver(volume_db=-34.5, mute=False, controls=False, **extra):
    doc = {"enabled": True, "host": "192.168.1.93", "polling": True, "reachable": True, "fresh": True, "updatedAt": NOW, "power": "on", "input": "TV Audio",
           "volumeDb": volume_db, "volumeAbs": round(volume_db + 80, 1), "volumeDisplay": "relative", "volumeText": "%.1f dB" % volume_db, "mute": mute,
           "model": "Denon AVR-S760H", "soundMode": "Dolby Surround", "soundModeId": "dolby"}
    if controls:
        doc.update({"controls": True, "inputs": ["TV Audio", "Apple TV", "Bluetooth", "HEOS Music", "PS4", "Blu-ray"],
                    "modes": [{"id": "movie", "title": "Movie", "sub": "Dolby Surround now", "icon": "clapperboard"}, {"id": "music", "title": "Music", "sub": "Wider, lighter centre", "icon": "music"},
                              {"id": "game", "title": "Game", "sub": "Effects forward", "icon": "gamepad-2"}, {"id": "direct", "title": "Direct", "sub": "Source as is, EQ off", "icon": "audio-lines"},
                              {"id": "pure", "title": "Pure Direct", "sub": "Also dims the display", "icon": "audio-waveform"}, {"id": "stereo", "title": "Stereo", "sub": "Front pair only", "icon": "speaker"},
                              {"id": "dolby", "title": "Dolby Digital", "sub": "Dolby decoders", "icon": "boom-box"}, {"id": "dts", "title": "DTS Surround", "sub": "DTS decoders", "icon": "boom-box"},
                              {"id": "auro", "title": "Auro-3D", "sub": "Height from any source", "icon": "mountain"}, {"id": "mch", "title": "Multi-ch Stereo", "sub": "Stereo on every speaker", "icon": "layout-grid"},
                              {"id": "more", "title": "More", "sub": "Rock Arena, Jazz Club", "icon": "ellipsis"}],
                    "quick": [{"n": 1, "label": "TV"}, {"n": 2, "label": "Apple TV"}, {"n": 3, "label": "Music"}, {"n": 4, "label": "PS4"}], "quickSelect": 1,
                    "audyssey": {"multEq": "reference", "dynEq": True, "refOffset": 0, "dynVol": "off"},
                    "levels": {"dialog": 0, "subwoofer": 2, "bass": 0, "treble": 0}, "cinemaEq": False, "eco": "auto", "zone2": {"power": False, "volumeAbs": 30, "input": "TV Audio"}})
    doc.update(extra)
    return doc


def k(*names):
    return " ".join(names)


# name -> steps. Names follow docs/mocks/shots.
SCENES = {}


def scene(name):
    def deco(fn):
        SCENES[name] = fn
        return fn
    return deco


def settings_to(section, *keys):
    """Open Settings and take the D-pad to a section (the rail's order), then the keys."""
    rail = ["picture", "sound", "inputs", "network", "look", "remote", "ai", "system"]
    steps = [("eval", 1500, "root.openSettings()")]
    if rail.index(section):
        steps.append(("keys", READY, k(*["down"] * rail.index(section))))
    if keys:
        steps.append(("keys", READY + 900, k(*keys)))
    return steps


@scene("01-settings-picture")
def _():
    return settings_to("picture") + [("snap", READY + 1500, "01-settings-picture")]


@scene("02-settings-sound")
def _():
    return settings_to("sound") + [("snap", READY + 1800, "02-settings-sound")]


@scene("30-settings-inputs")
def _():
    return settings_to("inputs") + [("snap", READY + 2200, "30-settings-inputs")]


@scene("30-settings-network")
def _():
    return settings_to("network") + [("snap", READY + 2200, "30-settings-network")]


@scene("30-settings-home")
def _():
    return settings_to("look") + [("snap", READY + 2200, "30-settings-home")]


@scene("03-settings-remote")
def _():
    return settings_to("remote") + [("snap", READY + 2200, "03-settings-remote")]


@scene("30-settings-system")
def _():
    return settings_to("system") + [("snap", READY + 2200, "30-settings-system")]


@scene("04-settings-ai-access")
def _():
    return [("publish", 300, "ai", AI), ("publish", 300, "__audit", AUDIT)] + settings_to("ai", "right") + [("snap", READY + 3000, "04-settings-ai-access")]


@scene("05-ai-request-prompt")
def _():
    return [("publish", 300, "ai", AI), ("publish", 300, "ai.prompt", PROMPT), ("snap", READY, "05-ai-request-prompt")]


@scene("24-settings-receiver")
def _():
    # from Sound, OK on "Sound out" opens the receiver's page
    return [("publish", 300, "receiver", receiver(controls=True))] + settings_to("sound", "right", "ok") + [("snap", READY + 3000, "24-settings-receiver"),
            ("keys", READY + 3200, "up right"), ("snap", READY + 4800, "24-settings-receiver-audyssey")]


@scene("11-volume-osd")
def _():
    return [("publish", 300, "receiver", receiver()), ("publish", READY - 800, "receiver", receiver(volume_db=-34.0)), ("snap", READY, "11-volume-osd")]


@scene("11-volume-osd-muted")
def _():
    return [("publish", 300, "receiver", receiver()), ("publish", READY - 1500, "receiver", receiver(volume_db=-34.5, mute=False, volumeText="-34.5 dB")),
            ("publish", READY - 800, "receiver", receiver(volume_db=-34.5, mute=True)), ("snap", READY, "11-volume-osd-muted")]


@scene("11-volume-osd-offline")
def _():
    return [("publish", 300, "receiver", {"enabled": True, "reachable": False, "fresh": False}),
            ("eval", READY - 800, "root.key(Qt.Key_VolumeUp, true, false, 0)"), ("snap", READY, "11-volume-osd-offline")]


@scene("08-input-picker")
def _():
    return [("eval", READY - 500, "root.openLayer('picker')"), ("keys", READY + 300, "right"), ("snap", READY + 1500, "08-input-picker")]


@scene("10-quick-panel")
def _():
    return [("publish", 300, "receiver", receiver(controls=True)), ("publish", 300, "ai", AI), ("eval", READY - 500, "root.openLayer('quick')"),
            ("keys", READY + 300, "down"), ("snap", READY + 1500, "10-quick-panel")]


APPS = "file://" + os.path.join(ROOT, "docs", "mocks", "img", "apps") + "/"


def js(v):
    return json.dumps(v)


@scene("15-launch-splash")
def _():
    return [("eval", READY, "root.showSplash('Netflix', %s)" % js(APPS + "netflix.png")), ("snap", READY + 1200, "15-launch-splash")]


@scene("15-spinner")
def _():
    return [("eval", READY, "root.showSpinner('Loading')"), ("snap", READY + 1000, "15-spinner")]


@scene("15-long-press-ring")
def _():
    return [("eval", READY, "root.showHold({x: 78, y: 856, width: 96, height: 96}, 'Apps', 0.72)"), ("snap", READY + 1000, "15-long-press-ring")]


@scene("16-context-menu")
def _():
    spec = {"title": "Netflix", "sub": "Dock tile 2 \u00b7 press 2 to open", "iconUrl": APPS + "netflix.png", "x": 280, "y": 700, "width": 520,
            "items": [{"id": "open", "icon": "play", "label": "Open"}, {"id": "move", "icon": "layout-grid", "label": "Move", "hint": "then \u2190 \u2192"},
                      {"id": "hide", "icon": "eye-off", "label": "Hide from the dock"}, {"id": "rename", "icon": "type", "label": "Rename"},
                      {"id": "uninstall", "icon": "trash-2", "label": "Uninstall", "danger": True}]}
    return [("eval", READY, "root.showMenu(%s)" % js(spec)), ("snap", READY + 1200, "16-context-menu")]


@scene("17-power-menu")
def _():
    spec = {"title": "Power", "x": 560, "y": 250, "width": 880, "hint": "Hold the power key for one second to turn off at once",
            "items": [{"id": "off", "icon": "power", "label": "Turn off", "detail": "The receiver turns off with it over HDMI-CEC."},
                      {"id": "screen", "icon": "monitor", "label": "Screen off, sound on", "detail": "Keeps playing through the receiver. Any key wakes the screen."},
                      {"id": "sleep", "icon": "moon", "label": "Sleep timer", "detail": "Off after this long.", "options": ["Off", "15", "30", "60", "90 min"], "index": 0},
                      {"id": "restart", "icon": "rotate-ccw", "label": "Restart", "detail": "About 40 seconds. Fixes a stuck app or no sound."}]}
    return [("eval", READY, "root.showMenu(%s)" % js(spec)), ("snap", READY + 1200, "17-power-menu")]


@scene("33-pointer-menu")
def _():
    spec = {"x": 1500, "y": 250, "width": 340, "scrim": False,
            "items": [{"id": "back", "icon": "arrow-left", "label": "Back"}, {"id": "home", "icon": "house", "label": "Home"},
                      {"id": "menu", "icon": "layout-grid", "label": "LumaTV menu"}, {"id": "inputs", "icon": "cable", "label": "Inputs"},
                      {"id": "quick", "icon": "settings", "label": "Quick settings"}]}
    return [("eval", READY, "root.showMenu(%s)" % js(spec)), ("snap", READY + 1200, "33-pointer-menu")]


@scene("06-confirm-dialog")
def _():
    spec = {"icon": "layout-grid", "title": "Uninstall Crunchyroll?", "body": "The app and its sign-in on this TV are removed. Downloads in the app are lost. It can be installed again from the LG Content Store.",
            "answers": [{"id": "keep", "label": "Keep it"}, {"id": "uninstall", "label": "Uninstall", "danger": True}], "focus": 0}
    return [("eval", READY, "root.showConfirm(%s)" % js(spec)), ("snap", READY + 1200, "06-confirm-dialog")]


@scene("34-screen-remote")
def _():
    return [("eval", READY, "root.showRemote()"), ("keys", READY + 500, "right right"), ("snap", READY + 1500, "34-screen-remote")]


def run_scene(name, out_dir, theme=None):
    steps = SCENES[name]()
    args = ["LumaHome.qml", "--fake-helper", "--prop", "hostActive=true", "--theme", json.dumps(dict({"perf": {"syncResidents": True}}, **(theme or {})))]
    end = 0
    for st in steps:
        kind, ms = st[0], st[1]
        end = max(end, ms)
        if kind == "eval":
            args += ["--eval", "%d:%s" % (ms, st[2])]
        elif kind == "keys":
            body = "".join("root.key(%s,true,false,0);root.key(%s,false,false,0);" % (CODES[n], CODES[n]) for n in st[2].split())
            args += ["--eval", "%d:(function(){%s return 1})()" % (ms, body)]
        elif kind == "publish":
            doc = dict(st[3]) if st[2] == "receiver" else st[3]
            if st[2] == "receiver" and doc.get("fresh"):
                doc["updatedAt"] = int(time.time() * 1000) + ms
            args += ["--publish", "%d:%s:%s" % (ms, st[2], json.dumps(doc))]
    args += ["--wait", str(end + 800)]
    env = dict(os.environ, DISPLAY=xdisplay.DISPLAY)
    r = subprocess.run([sys.executable, os.path.join(HERE, "qmlhost.py")] + args, env=env, capture_output=True, text=True, timeout=300)
    text = r.stdout + r.stderr
    warn = [ln for ln in text.splitlines() if ln.startswith("[warning]") and "Implicitly defined onFoo" not in ln]
    return r.returncode, warn, text


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--out", default=os.path.join(ROOT, "docs", "mocks", "host"))
    ap.add_argument("--only")
    ap.add_argument("--list", action="store_true")
    a = ap.parse_args()
    if a.list:
        print("\n".join(SCENES))
        return 0
    os.makedirs(a.out, exist_ok=True)
    names = a.only.split(",") if a.only else list(SCENES)
    bad = 0
    for n in names:
        rc, warn, text = run_scene(n, a.out)
        print("%-34s %s" % (n, "ok" if rc == 0 else "FAILED (%d warnings)" % len(warn)))
        for w in warn[:6]:
            print("   " + w[:240])
        bad += rc != 0
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
