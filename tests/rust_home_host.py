"""The Rust shell's Home on the host harness (`lumatv-ui --host`: the shell's logic against the engine in its own process on Mesa): the
make-believe feeds, layouts and launch points, and `RustHome.host`, which runs the binary with Home in front and returns its evals. Shared by
host_budget.py, the frame-gate replay (test_gate_surfaces.py) and the mock renderers; the behaviour itself is tested in rustui/ui."""
import json
import os
import re
import shutil
import subprocess
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, '..'))
BIN = os.path.join(ROOT, 'rustui', 'target', 'debug', 'lumatv-ui')
MESA = '/usr/lib/x86_64-linux-gnu/libEGL.so.1'
OUT = os.path.join(ROOT, 'docs', 'mocks', 'host-rust', 'home')
NOW = int(time.time() * 1000)
HOME = {"app": "com.webos.app.home", "home": True, "covered": False}
LP_TAG = 300


def launch_points():
    """The offline host's make-believe launch points (tests/qmlhost/stubs/WebOSServices/FakeApps.js), with the mocks' icons."""
    with open(os.path.join(HERE, 'qmlhost', 'stubs', 'WebOSServices', 'FakeApps.js')) as fh:
        src = fh.read()
    icons = os.path.join(ROOT, 'docs', 'mocks', 'img', 'apps')
    out = []
    for id, title, icon, opts in re.findall(r'\["([^"]+)", "([^"]+)", "([^"]+)", \{([^}]*)\}\]', src):
        o = dict((k, v.strip().strip('"')) for k, v in re.findall(r'(\w+): ([^,]+)', opts))
        path = os.path.join(ROOT, 'assets', 'icon160.png') if icon.startswith('@') else os.path.join(icons, icon + '.png')
        out.append({"id": id, "launchPointId": id + "_default", "title": title, "icon": path, "largeIcon": path, "iconColor": o.get('color', ''),
                    "hidden": o.get('hidden') == 'true', "params": {}, "unmovable": o.get('unmovable') == 'true'})
    return out


def weather():
    days = [("Today", "sun", 71, 51), ("Mon", "cloud", 73, 51), ("Tue", "rain", 62, 48), ("Wed", "partly", 66, 56), ("Thu", "drizzle", 59, 44), ("Fri", "sun", 68, 42), ("Sat", "storm", 74, 55)]
    return {"doc": {"schema": 1, "fetchedAt": NOW, "display": {"place": "Atlanta", "temp": "61\u00b0", "cond": "Clear", "stats": "Feels like 57\u00b0 \u00b7 53% humidity \u00b7 6 mph", "nowKind": "moon",
                                                          "days": [{"day": d, "kind": k, "hi": "%d\u00b0" % hi, "lo": "%d\u00b0" % lo} for d, k, hi, lo in days]}}, "state": "fresh"}


def news():
    return {"doc": {"schema": 1, "fetchedAt": NOW, "items": [
        {"tag": "WORLD", "src": "BBC News", "title": "Denmark says Russian warship fired flares at military helicopter",
         "summary": "The Danish prime minister has accused Russia of \"reckless\" behaviour, while the Kremlin is yet to comment. The helicopter was on a routine patrol over the Baltic when the flares were fired, the Danish defence ministry said.",
         "link": "https://example.invalid/a", "pubMs": NOW - 1800000},
        {"tag": "U.S.", "src": "BBC News", "title": "A second story", "summary": "", "pubMs": NOW - 7200000}]}, "state": "fresh"}


def card(id, title, facts, icon, role, actions=()):
    return {"id": id, "revision": 1, "card": {"id": id, "size": "half", "content": {"type": "facts", "title": title, "icon": icon, "role": role, "facts": [{"label": a, "value": b} for a, b in facts]}, "actions": list(actions)}}


HOME_CARDS = {"items": [card("living", "Living room", [("Lights", "3 on")], "base.icons/lightbulb", "warning", [{"action": "off", "label": "Turn off"}]),
                        card("climate", "72\u00b0 cooling", [("Set to", "70\u00b0")], "base.icons/sun", "info"), card("door", "Front door", [("State", "Locked")], "base.icons/lock", "positive")]}
CAMERAS = {"state": "ready", "cameras": [{"id": "front", "name": "Front door", "room": "Porch", "online": True}, {"id": "back", "name": "Back yard", "room": "Garden", "online": False, "since": NOW - 200000}]}


def profiles(active="nitin", can_admin=True, locked=("isabel",), at=1):
    people = [("nitin", "Nitin", "#A79BF5", True), ("isabel", "Isabel", "#EB6F92", False), ("mia", "Mia", "#F6C177", False)]
    return {"ready": True, "active": active, "owner": "nitin", "canAdmin": can_admin, "pickerOnWake": False, "startWith": "last", "max": 6, "switchedAt": at,
            "profiles": [{"id": i, "name": n, "initial": n[0], "color": c, "avatar": {"kind": "initial"}, "avatarFile": None, "owner": o, "active": i == active, "locked": i in locked, "firstRun": False} for i, n, c, o in people]}


class RustHome:
    def host(self, wait, steps=(), layout=None, feeds=None, mode=None, scale=None, data=None, theme=None, kinds='home', extra=()):
        """Runs the harness with Home in front, the launch points answered and the feeds published at 100 ms; `steps` are extra
        arguments (`k`, `snap`, `ev`). Returns (the output, the evals by label, the data directory)."""
        data = data or tempfile.mkdtemp(prefix='rusthome-')
        os.makedirs(os.path.join(data, 'theme'), exist_ok=True)
        shutil.copy(os.path.join(ROOT, 'payload', 'wallpaper', 'wall_1080.png'), os.path.join(data, 'theme', 'wall_1080.png'))
        if layout is not None:
            with open(os.path.join(data, 'theme', 'layout.json'), 'w') as f:
                json.dump(layout, f)
        if theme is not None:
            with open(os.path.join(data, 'theme', 'theme.json'), 'w') as f:
                json.dump(theme, f)
        feeds = {"weather": weather(), "news": news(), "plugins": {"plugins": [{"id": "home-assistant", "name": "Home Assistant", "state": "connected"}], "firstParty": []}, "plugin.home-assistant.cards": HOME_CARDS, "profiles": profiles(), "cameras": CAMERAS, **(feeds or {})}
        cmd = [BIN, '--host', '--kinds', kinds, '--data-dir', data, '--shm', os.path.join(data, 'shm'), '--wait', str(wait),
               '--push', '50:foreground:' + json.dumps(HOME), '--luna', '80:%d:%s' % (LP_TAG, json.dumps({"launchPoints": launch_points()}))]
        for topic, doc in feeds.items():
            cmd += ['--helper', '100:%s:%s' % (topic, json.dumps(doc))]
        if mode:
            # the material forced, so the wallpaper's luma does not choose (as scenes.py renders the QML)
            cmd += ['--mode', mode, '--theme', json.dumps({"material": mode})]
        if scale:
            cmd += ['--text-scale', str(scale)]
        cmd += list(extra)
        for s in steps:
            cmd += s
        env = dict(os.environ, LIBGL_ALWAYS_SOFTWARE='1')
        r = subprocess.run(cmd, env=env, capture_output=True, text=True, timeout=int(os.environ.get('HOST_TIMEOUT', '240')))
        out = r.stdout + r.stderr
        if r.returncode != 0:
            raise AssertionError(out[-2000:])
        evals = [json.loads(m.group(1)) for m in re.finditer(r"^\[eval\] (.*)$", out, re.M)]
        return out, evals, data

    @staticmethod
    def k(ms, *names):
        """Presses, 200 ms apart from `ms`, each released 40 ms later."""
        args = []
        for i, n in enumerate(names):
            at = ms + 200 * i
            args += ['--key', '%d:%s' % (at, n), '--key', '%d:%s:up' % (at + 40, n)]
        return args

    @staticmethod
    def hold(ms, name, for_ms):
        return ['--key', '%d:%s' % (ms, name), '--key', '%d:%s:repeat' % (ms + for_ms // 2, name), '--key', '%d:%s:up' % (ms + for_ms, name)]

    @staticmethod
    def ev(ms):
        return ['--eval', '%d:home' % ms]

    @staticmethod
    def snap(ms, name):
        os.makedirs(OUT, exist_ok=True)
        return ['--snap', '%d:%s' % (ms, os.path.join(OUT, name + '.png'))]
