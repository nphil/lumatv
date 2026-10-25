"""Run with: python3 -m unittest tests.test_settings   (needs Xvfb and PyQt5; skipped without them)

LumaTV's Settings (payload/compositor/lumatv/Settings.qml and settings/) through the offline host: Home
opens it, the D-pad moves through the rail and the page, a switch writes the theme through Home's own
patch and persist (the user's theme.json in the data directory), Back leaves it, and the pages draw
the state of the helper (a fake one serves the topics). Timing and Qt 5.12 are the set's to say.
"""
import json
import os
import re
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
HOST = os.path.join(HERE, "qmlhost", "qmlhost.py")

sys.path.insert(0, HERE)
from xdisplay import DISPLAY, HAVE_QT  # noqa: E402


class HostCase(unittest.TestCase):
    CODES = {"left": "Qt.Key_Left", "right": "Qt.Key_Right", "up": "Qt.Key_Up", "down": "Qt.Key_Down", "ok": "Qt.Key_Return", "back": "Qt.Key_Back"}

    def keys(self, at, *names):
        """The D-pad as Home gets it from the key filter (`key(code, pressed, autoRepeat, device)`), one press and release each."""
        body = "".join("root.key(%s,true,false,0);root.key(%s,false,false,0);" % (self.CODES[n], self.CODES[n]) for n in names)
        return ["--eval", "%d:(function(){%s return 1})()" % (at, body)]

    def host(self, *args, data=None):
        cmd = [sys.executable, HOST, "LumaHome.qml", "--fake-helper", "--prop", "hostActive=true", *args]
        if data:
            cmd += ["--data", data]
        r = subprocess.run(cmd, env=dict(os.environ, DISPLAY=DISPLAY), capture_output=True, text=True, timeout=120)
        return r.returncode, r.stdout + r.stderr

    def values(self, out):
        vals = []
        for line in out.splitlines():
            m = re.match(r"\[eval\] \((.*), (True|False)\)$", line)
            if m:
                v = m.group(1)
                vals.append((v[1:-1] if v[:1] in "'\"" else v).lower() if v in ("True", "False") else (v[1:-1] if v[:1] in "'\"" else v))
        return vals



@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class Settings(HostCase):
    def test_home_opens_settings_and_it_loads_without_a_warning(self):
        rc, out = self.host("--eval", "1500:root.openSettings()", "--eval", "2800:[root.settingsOpen, root.settingsBuilt].join(',')", "--wait", "3400")
        self.assertEqual(rc, 0, out[-1500:])
        self.assertEqual(self.values(out)[-1], "true,true")

    def test_a_switch_changes_the_theme_and_is_written_to_the_user_file(self):
        with tempfile.TemporaryDirectory() as d:
            data = os.path.join(d, "data")
            # RIGHT enters the page (Dynamic tint), OK flips it
            rc, out = self.host("--eval", "1500:root.openSettings()", *self.keys(3500, "right", "ok"),
                                "--eval", "5200:root.theme.accent", "--wait", "6000", data=data)
            self.assertEqual(rc, 0, out[-1500:])
            self.assertEqual(self.values(out)[-1], "wallpaper")
            with open(os.path.join(data, "theme", "theme.json")) as f:
                self.assertEqual(json.load(f).get("accent"), "wallpaper")

    def test_down_and_right_move_inside_a_segmented_row_and_ok_picks(self):
        with tempfile.TemporaryDirectory() as d:
            data = os.path.join(d, "data")
            # page, down to Appearance (Dark | Light | Auto), Auto is current, Left to Light, OK
            rc, out = self.host("--eval", "1500:root.openSettings()", *self.keys(3500, "right", "down", "left", "ok"),
                                "--eval", "5200:root.theme.material", "--wait", "6000", data=data)
            self.assertEqual(rc, 0, out[-1500:])
            self.assertEqual(self.values(out)[-1], "light")

    def test_back_from_the_rail_closes_it_and_back_from_the_page_returns_to_the_rail(self):
        rc, out = self.host("--eval", "1500:root.openSettings()", *self.keys(3500, "right", "back"),
                            "--eval", "5200:root.settingsOpen", "--wait", "6000")
        self.assertEqual(rc, 0, out[-1500:])
        self.assertEqual(self.values(out)[-1], "true")      # still open: Back only left the page
        rc, out = self.host("--eval", "1500:root.openSettings()", *self.keys(3500, "back"),
                            "--eval", "5200:root.settingsOpen", "--wait", "6000")
        self.assertEqual(rc, 0, out[-1500:])
        self.assertEqual(self.values(out)[-1], "false")

    def test_keys_go_to_settings_and_not_to_the_home_behind_it(self):
        rc, out = self.host("--eval", "1500:root.openSettings()", "--eval", "3000:[root.focusLayer, root.dockRow, root.dockCol, root.widgetFocusId].join('/')",
                            *self.keys(3500, "down", "down", "left", "up"),
                            "--eval", "5200:[root.focusLayer, root.dockRow, root.dockCol, root.widgetFocusId].join('/')", "--wait", "6000")
        self.assertEqual(rc, 0, out[-1500:])
        vals = self.values(out)                 # openSettings, before, the keys, after
        self.assertEqual(vals[1], vals[3], out[-800:])

    def test_the_rail_ends_with_the_way_to_LGs_settings(self):
        rc, out = self.host("--eval", "1500:root.openSettings()", *self.keys(3500, "down", "down", "ok"),
                            "--eval", "5200:root.settingsOpen", "--wait", "6000")
        self.assertEqual(rc, 0, out[-1500:])
        # two Downs land on "More in LG settings", OK on it launches LG's app and closes Settings
        self.assertEqual(self.values(out)[-1], "false")


AI_BASE = {"listener": {"enabled": False, "bound": False, "addr": None, "port": 18732, "since": None, "error": None}, "pairing": None,
           "assistants": [{"id": "cody", "name": "Cody", "createdAt": 1, "lastSeenAt": 0, "paused": False, "calls": 12,
                           "levels": {"state": "always", "playback": "always", "launch": "ask", "power": "ask"}}],
           "limits": {"maxVolume": 60},
           "groups": [{"id": "state", "title": "See what is playing", "default": "always"}, {"id": "playback", "title": "Play, pause, volume, mute", "default": "always"},
                      {"id": "launch", "title": "Open apps, switch inputs", "default": "ask"}, {"id": "power", "title": "Turn the TV off", "default": "ask"}],
           "tools": []}

REQUESTS = ('(function(){var x=new XMLHttpRequest();x.onreadystatechange=function(){if(x.readyState===4)console.info("[req] "+x.responseText)};'
            'x.open("GET","http://127.0.0.1:18731/__requests");x.send();return 1})()')
APPROVE = '(function(){var x=new XMLHttpRequest();x.open("POST","http://127.0.0.1:18731/__approve");x.setRequestHeader("Content-Type","application/json");x.send("{}");return 1})()'


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class AiAccess(HostCase):
    """The AI access page and the prompt, against a fake helper that models the /v1/ai routes and
    answers a prompt only through its own test door (`/__approve`), as only the real remote does."""

    def ai(self, **over):
        doc = json.loads(json.dumps(AI_BASE))
        for k, v in over.items():
            doc[k] = v
        return doc

    def run_page(self, doc, steps, settle=1200):
        """Opens Settings on the AI page and runs `steps` (a list of (ms, kind, text): keys or an eval), returning the requests
        the helper saw as each `[req]` printed."""
        args = ["--publish", "800:ai:" + json.dumps(doc), "--eval", "1500:root.openSettings()",
                *self.keys(3000, "down", "right")]
        end = 3000
        for ms, kind, what in steps:
            end = max(end, ms)
            args += self.keys(ms, *what) if kind == "keys" else ["--eval", "%d:%s" % (ms, what)]
        rc, out = self.host(*args, "--wait", str(end + settle))
        self.assertEqual(rc, 0, out[-1500:])
        reqs = [json.loads(m.group(1)) for m in re.finditer(r"\[req\] (\{.*\})", out)]
        return out, [[tuple(r[:2]) + (r[2],) for r in q["requests"] if not r[1].endswith("/audit")] for q in reqs]

    def test_turning_it_on_only_asks_and_the_open_prompt_swallows_every_key(self):
        out, seen = self.run_page(self.ai(), [(3600, "keys", ["right", "ok"]), (4400, "eval", REQUESTS),
                                              (4800, "keys", ["ok", "ok", "back", "down", "ok"]), (5400, "eval", REQUESTS)])
        self.assertEqual(seen[0], [("POST", "/v1/ai/listener", {"enabled": True})])
        self.assertEqual(seen[1], seen[0], "a key got past the prompt")

    def test_turning_it_off_is_done_at_once_and_pressing_off_again_sends_nothing_new(self):
        on = self.ai(listener={"enabled": True, "bound": True, "addr": "192.168.1.242:18732", "port": 18732, "since": 1, "error": None})
        out, seen = self.run_page(on, [(3600, "keys", ["left", "ok"]), (4600, "eval", REQUESTS)])
        self.assertEqual(seen[0], [("POST", "/v1/ai/listener", {"enabled": False})])

    def test_removing_an_assistant_needs_a_second_press_and_lockout_too(self):
        out, seen = self.run_page(self.ai(listener={"enabled": True, "bound": True, "addr": "x", "port": 1, "since": 1, "error": None}),
                                  [(3600, "keys", ["down", "down", "right", "ok"]), (4400, "eval", REQUESTS), (4600, "keys", ["ok"]), (5400, "eval", REQUESTS)])
        self.assertEqual(seen[0], [])
        self.assertEqual(seen[1], [("DELETE", "/v1/ai/assistants/cody", {})])
        out, seen = self.run_page(self.ai(), [(3600, "keys", ["down"] * 8), (4000, "keys", ["ok"]), (4400, "eval", REQUESTS), (4600, "keys", ["ok"]), (5400, "eval", REQUESTS)])
        self.assertEqual(seen[0], [])
        self.assertEqual(seen[1], [("POST", "/v1/ai/lockout", {})])

    def test_a_lower_level_is_sent_whole_a_higher_one_only_asks(self):
        # stops: 0 listener, 1 pairing, 2 Cody, 3 chooser, 4 state, 5 playback, 6 launch (ask), 7 power
        out, seen = self.run_page(self.ai(), [(3600, "keys", ["down"] * 6 + ["right", "ok"]), (4400, "eval", REQUESTS)])
        self.assertEqual(len(seen[0]), 1)
        method, path, body = seen[0][0]
        self.assertEqual((method, path), ("POST", "/v1/ai/assistants/cody/levels"))
        self.assertEqual(body, {"levels": {"state": "always", "playback": "always", "launch": "never", "power": "ask"}})
        out, seen = self.run_page(self.ai(), [(3600, "keys", ["down"] * 6 + ["left", "ok"]), (4400, "eval", REQUESTS)])
        self.assertEqual(seen[0][0][2]["levels"]["launch"], "always")      # asked; the fake helper answers 202 and opens a prompt

    def test_approving_at_the_remote_turns_it_on_and_nothing_else_does(self):
        state = ('(function(){var x=new XMLHttpRequest();x.onreadystatechange=function(){if(x.readyState===4)console.info("[ai] "+x.responseText)};'
                 'x.open("GET","http://127.0.0.1:18731/v1/ai");x.send();return 1})()')
        out, _ = self.run_page(self.ai(), [(3600, "keys", ["right", "ok"]), (4300, "eval", state), (4800, "eval", APPROVE), (5500, "eval", state)], settle=1500)
        enabled = [json.loads(m.group(1))["ai"]["listener"]["enabled"] for m in re.finditer(r"\[ai\] (\{.*\})", out)]
        self.assertEqual(enabled, [False, True])


if __name__ == "__main__":
    unittest.main()
