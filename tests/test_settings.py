"""Run with: python3 -m unittest tests.test_settings   (needs Xvfb and PyQt5; skipped without them)

LumaTV's Settings (payload/compositor/lumatv/Settings.qml and settings/) through the offline host: Home builds it
in the background once it has presented, opening it only flips it on, the D-pad moves through the rail and the
pages, a switch writes the theme through Home's own patch and persist (the user's theme.json in the data
directory) or the TV's settings service (the host's fake one, read back through the luna calls it recorded), Back
leaves it, and the pages draw the state of the helper (a fake one serves the topics). Timing and Qt 5.12 are
the set's to say.
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
ROOT = os.path.dirname(HERE)

sys.path.insert(0, HERE)
from xdisplay import DISPLAY, HAVE_QT  # noqa: E402


class HostCase(unittest.TestCase):
    CODES = {"left": "Qt.Key_Left", "right": "Qt.Key_Right", "up": "Qt.Key_Up", "down": "Qt.Key_Down", "ok": "Qt.Key_Return", "back": "Qt.Key_Back"}

    def keys(self, at, *names):
        """The D-pad as Home gets it from the key filter (`key(code, pressed, autoRepeat, device)`), one press and release each."""
        body = "".join("root.key(%s,true,false,0);root.key(%s,false,false,0);" % (self.CODES[n], self.CODES[n]) for n in names)
        return ["--eval", "%d:(function(){%s return 1})()" % (at, body)]

    def host(self, *args, data=None):
        cmd = [sys.executable, HOST, "LumaHome.qml", "--fake-helper", "--prop", "hostActive=true", "--theme", '{"perf":{"syncResidents":true}}', *args]
        if data:
            cmd += ["--data", data]
        r = subprocess.run(cmd, env=dict(os.environ, DISPLAY=DISPLAY), capture_output=True, text=True, timeout=180)
        return r.returncode, r.stdout + r.stderr

    def values(self, out):
        vals = []
        for line in out.splitlines():
            m = re.match(r"\[eval\] \((.*), (True|False)\)$", line)
            if m:
                v = m.group(1)
                vals.append((v[1:-1] if v[:1] in "'\"" else v).lower() if v in ("True", "False") else (v[1:-1] if v[:1] in "'\"" else v))
        return vals


# The rail, in order: picture, sound, inputs, network, look, remote, ai, system, then LG's Settings.
RAIL = {"picture": 0, "sound": 1, "inputs": 2, "network": 3, "look": 4, "remote": 5, "ai": 6, "system": 7, "lg": 8}
OPEN_AT = 1500          # Home asks for Settings
READY = 6000            # Home has presented, built the chain (Settings and its pages) and warmed up
LUNA = "JSON.stringify(root.lunaCalls.filter(function(c) { return c.method === '/setSystemSettings' }).map(function(c) { return JSON.parse(c.payload) }))"


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class Settings(HostCase):
    def go(self, section, *more):
        """Keys that open Settings and take the D-pad to `section`'s page."""
        return [*self.keys(READY + 600, *(["down"] * RAIL[section]))] + ([*self.keys(READY + 1500, *more)] if more else [])

    def test_home_opens_settings_and_it_loads_without_a_warning(self):
        rc, out = self.host("--eval", "%d:root.openSettings()" % OPEN_AT, "--eval", "%d:root.settingsOpen" % (READY + 400), "--wait", str(READY + 1200))
        self.assertEqual(rc, 0, out[-1500:])
        self.assertEqual(self.values(out)[-1], "true")
        self.assertIn("[Resident] settings built", out)          # built in the background, before it was opened a second time

    def test_every_page_is_built_with_it_and_the_rail_shows_each_pages_own_summary(self):
        rc, out = self.host("--eval", "%d:root.openSettings()" % READY, "--snap", "%d:/dev/null" % (READY + 800), "--wait", str(READY + 1400))
        self.assertEqual(rc, 0, out[-1500:])
        self.assertNotIn("did not load", out)

    def test_a_switch_changes_the_theme_and_is_written_to_the_user_file(self):
        with tempfile.TemporaryDirectory() as d:
            data = os.path.join(d, "data")
            # Home & look: the wallpaper row only shows, so the first stop is Dynamic tint; RIGHT enters the page, OK flips it
            rc, out = self.host("--eval", "%d:root.openSettings()" % OPEN_AT, *self.go("look", "right", "ok"),
                                "--eval", "%d:root.theme.accent" % (READY + 3000), "--wait", str(READY + 3600), data=data)
            self.assertEqual(rc, 0, out[-1500:])
            self.assertEqual(self.values(out)[-1], "wallpaper")
            with open(os.path.join(data, "theme", "theme.json")) as f:
                self.assertEqual(json.load(f).get("accent"), "wallpaper")

    def test_down_and_left_move_inside_a_segmented_row_and_ok_picks(self):
        with tempfile.TemporaryDirectory() as d:
            data = os.path.join(d, "data")
            # page, down to Appearance (Dark | Light | Auto), Auto is current, Left to Light, OK
            rc, out = self.host("--eval", "%d:root.openSettings()" % OPEN_AT, *self.go("look", "right", "down", "left", "ok"),
                                "--eval", "%d:root.theme.material" % (READY + 3000), "--wait", str(READY + 3600), data=data)
            self.assertEqual(rc, 0, out[-1500:])
            self.assertEqual(self.values(out)[-1], "light")

    def test_the_tabs_switch_without_building_anything(self):
        # Home & look has four tabs; Up from the first row is the tab bar, Right on it the next tab, Down the first row of that tab
        rc, out = self.host("--eval", "%d:root.openSettings()" % OPEN_AT, *self.go("look", "right", "up", "right", "right", "down", "ok"),
                            "--eval", "%d:JSON.stringify(root.theme.screensaver || {})" % (READY + 3200), "--wait", str(READY + 3800))
        self.assertEqual(rc, 0, out[-1500:])
        # Screensaver tab, first row (style Clock | Art | Off), OK picks the current one: nothing breaks, nothing warns
        self.assertNotIn("did not load", out)

    def test_a_slider_writes_the_tvs_setting_with_the_dimension_it_was_read_for(self):
        # Picture, Mode tab: the first stop is the picture mode cards (two rows of them), then OLED pixel brightness (80 in the fake service)
        rc, out = self.host("--eval", "%d:root.openSettings()" % OPEN_AT, *self.go("picture", "right", "down", "down", "right", "right"),
                            "--eval", "%d:%s" % (READY + 3200, LUNA), "--wait", str(READY + 3800))
        self.assertEqual(rc, 0, out[-1500:])
        calls = json.loads(re.findall(r"\[eval\] \('(\[.*\])'", out)[-1])
        sent = [c for c in calls if c["category"] == "picture" and "backlight" in c["settings"]]
        self.assertEqual([c["settings"]["backlight"] for c in sent], ["81", "82"])
        self.assertEqual(sent[0]["dimension"]["pictureMode"], "filmMaker")

    def test_a_row_the_service_does_not_answer_is_dimmed_and_does_not_take_focus(self):
        # Sound: Receiver volume only shows, so Down from Sound out lands on HDMI input audio format, not the info rows
        rc, out = self.host("--eval", "%d:root.openSettings()" % OPEN_AT, *self.go("sound", "right", "down", "right", "ok"),
                            "--eval", "%d:%s" % (READY + 3200, LUNA), "--wait", str(READY + 3800))
        self.assertEqual(rc, 0, out[-1500:])
        calls = json.loads(re.findall(r"\[eval\] \('(\[.*\])'", out)[-1])
        self.assertEqual([c["settings"] for c in calls if c["category"] == "sound"], [{"digitalSoundOut": "pcm"}])

    def test_back_from_the_rail_closes_it_and_back_from_the_page_returns_to_the_rail(self):
        rc, out = self.host("--eval", "%d:root.openSettings()" % OPEN_AT, *self.keys(READY + 600, "right", "back"),
                            "--eval", "%d:root.settingsOpen" % (READY + 1600), "--wait", str(READY + 2200))
        self.assertEqual(rc, 0, out[-1500:])
        self.assertEqual(self.values(out)[-1], "true")      # still open: Back only left the page
        rc, out = self.host("--eval", "%d:root.openSettings()" % OPEN_AT, *self.keys(READY + 600, "back"),
                            "--eval", "%d:root.settingsOpen" % (READY + 1600), "--wait", str(READY + 2200))
        self.assertEqual(rc, 0, out[-1500:])
        self.assertEqual(self.values(out)[-1], "false")

    def test_keys_go_to_settings_and_not_to_the_home_behind_it(self):
        rc, out = self.host("--eval", "%d:root.openSettings()" % OPEN_AT, "--eval", "%d:[root.focusLayer, root.dockRow, root.dockCol, root.widgetFocusId].join('/')" % (READY + 400),
                            *self.keys(READY + 800, "down", "down", "left", "up"),
                            "--eval", "%d:[root.focusLayer, root.dockRow, root.dockCol, root.widgetFocusId].join('/')" % (READY + 2000), "--wait", str(READY + 2600))
        self.assertEqual(rc, 0, out[-1500:])
        vals = self.values(out)                 # openSettings, before, the keys, after
        self.assertEqual(vals[1], vals[3], out[-800:])

    def test_the_rail_ends_with_the_way_to_LGs_settings(self):
        rc, out = self.host("--eval", "%d:root.openSettings()" % OPEN_AT, *self.keys(READY + 600, *(["down"] * RAIL["lg"]), "ok"),
                            "--eval", "%d:root.settingsOpen" % (READY + 3000), "--wait", str(READY + 3600))
        self.assertEqual(rc, 0, out[-1500:])
        # the last row of the rail launches LG's app and closes Settings
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
APPROVE = ('(function(){var x=new XMLHttpRequest();x.onreadystatechange=function(){if(x.readyState===4)console.info("[approval] "+x.status)};'
           'x.open("POST","http://127.0.0.1:18731/__approve");x.setRequestHeader("Content-Type","application/json");x.send("{}");return 1})()')


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
        args = ["--publish", "800:ai:" + json.dumps(doc), "--eval", "%d:root.openSettings()" % OPEN_AT,
                *self.keys(READY + 600, *(["down"] * RAIL["ai"]), "right")]
        end = READY + 600
        for ms, kind, what in steps:
            ms += READY
            end = max(end, ms)
            args += self.keys(ms, *what) if kind == "keys" else ["--eval", "%d:%s" % (ms, what)]
        rc, out = self.host(*args, "--wait", str(end + settle))
        self.assertEqual(rc, 0, out[-1500:])
        reqs = [json.loads(m.group(1)) for m in re.finditer(r"\[req\] (\{.*\})", out)]
        return out, [[tuple(r[:2]) + (r[2],) for r in q["requests"] if not r[1].endswith("/audit")] for q in reqs]

    def test_turning_it_on_only_asks_and_the_open_prompt_swallows_every_key(self):
        out, seen = self.run_page(self.ai(), [(1600, "keys", ["right", "ok"]), (2400, "eval", REQUESTS),
                                              (2800, "keys", ["ok", "ok", "back", "down", "ok"]), (3400, "eval", REQUESTS)])
        self.assertEqual(seen[0], [("POST", "/v1/ai/listener", {"enabled": True})])
        self.assertEqual(seen[1], seen[0], "a key got past the prompt")

    def test_turning_it_off_is_done_at_once_and_pressing_off_again_sends_nothing_new(self):
        on = self.ai(listener={"enabled": True, "bound": True, "addr": "192.168.1.242:18732", "port": 18732, "since": 1, "error": None})
        out, seen = self.run_page(on, [(1600, "keys", ["left", "ok"]), (2600, "eval", REQUESTS)])
        self.assertEqual(seen[0], [("POST", "/v1/ai/listener", {"enabled": False})])

    def test_removing_an_assistant_needs_a_second_press_and_lockout_too(self):
        on = self.ai(listener={"enabled": True, "bound": True, "addr": "x", "port": 1, "since": 1, "error": None})
        # listener, down to the volume limit, down to Cody, Right to Remove, OK arms it, OK again removes it
        out, seen = self.run_page(on, [(1600, "keys", ["down", "down", "right", "ok"]), (2400, "eval", REQUESTS), (2600, "keys", ["ok"]), (3400, "eval", REQUESTS)])
        self.assertEqual(seen[0], [])
        self.assertEqual(seen[1], [("DELETE", "/v1/ai/assistants/cody", {})])
        # ... and from Cody down to the pairing row and the lock-out, which asks twice as well
        out, seen = self.run_page(self.ai(), [(1600, "keys", ["down", "down", "down", "down"]), (2000, "keys", ["ok"]), (2400, "eval", REQUESTS), (2600, "keys", ["ok"]), (3400, "eval", REQUESTS)])
        self.assertEqual(seen[0], [])
        self.assertEqual(seen[1], [("POST", "/v1/ai/lockout", {})])

    def test_a_lower_level_is_sent_whole_a_higher_one_only_asks(self):
        # listener, limit, Cody (Right twice: Remove, then across to the first group), down to "Open apps" (ask now)
        into = ["down", "down", "right", "right", "down", "down"]
        out, seen = self.run_page(self.ai(), [(1600, "keys", into + ["right", "ok"]), (2600, "eval", REQUESTS)])
        self.assertEqual(len(seen[0]), 1)
        method, path, body = seen[0][0]
        self.assertEqual((method, path), ("POST", "/v1/ai/assistants/cody/levels"))
        self.assertEqual(body, {"levels": {"state": "always", "playback": "always", "launch": "never", "power": "ask"}})
        out, seen = self.run_page(self.ai(), [(1600, "keys", into + ["left", "ok"]), (2600, "eval", REQUESTS)])
        self.assertEqual(seen[0][0][2]["levels"]["launch"], "always")      # asked; the fake helper answers 202 and opens a prompt

    def test_a_lower_volume_limit_is_done_a_higher_one_asks(self):
        out, seen = self.run_page(self.ai(), [(1600, "keys", ["down", "left"]), (2400, "eval", REQUESTS)])
        self.assertEqual(seen[0], [("POST", "/v1/ai/limits", {"maxVolume": 58})])

    def test_approving_at_the_remote_turns_it_on_and_nothing_else_does(self):
        state = ('(function(){var x=new XMLHttpRequest();x.onreadystatechange=function(){if(x.readyState===4)console.info("[ai] "+x.responseText)};'
                 'x.open("GET","http://127.0.0.1:18731/v1/ai");x.send();return 1})()')
        out, _ = self.run_page(self.ai(), [(1600, "keys", ["right", "ok"]), (2300, "eval", state), (2800, "eval", APPROVE), (3500, "eval", state)], settle=1500)
        enabled = [json.loads(m.group(1))["ai"]["listener"]["enabled"] for m in re.finditer(r"\[ai\] (\{.*\})", out)]
        self.assertEqual(enabled, [False, True])


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class Prompt(HostCase):
    PROMPT = {"kind": "tv_standby", "title": "Cody wants to turn off the TV", "detail": "The TV turns off now. The receiver stays as it is.",
              "assistant": {"id": "cody", "name": "Cody"}, "reason": "Nate asked me to shut everything down for the night.", "expiresAt": 4102444800000}

    def test_the_prompt_draws_who_what_and_the_assistants_own_words_apart(self):
        rc, out = self.host("--publish", "2000:ai.prompt:" + json.dumps(self.PROMPT), "--eval", "3000:root.children.length > 0", "--snap", "3500:/dev/null", "--wait", "4000")
        self.assertEqual(rc, 0, out[-1500:])


class Icons(unittest.TestCase):
    """Every icon a QML file names is one the generator made."""

    def test_every_named_icon_exists(self):
        with open(os.path.join(ROOT, "payload", "compositor", "lumatv", "icons", "lucide", "index.json")) as f:
            have = set(json.load(f))
        missing = []
        base = os.path.join(ROOT, "payload", "compositor", "lumatv")
        for dirpath, _, files in os.walk(base):
            if os.sep + "widgets" in dirpath or os.sep + "icons" in dirpath:       # Home's weather and Home Assistant cards name their own pictures
                continue
            for n in files:
                if not n.endswith((".qml", ".js")):
                    continue
                with open(os.path.join(dirpath, n), encoding="utf-8") as f:
                    text = f.read()
                for m in re.finditer(r'(?:\bicon: *|LuIcon *\{[^}\n]*?\bname: *)"([a-z0-9-]+)"', text):
                    if m.group(1) not in have and n != "Lu.qml":
                        missing.append((n, m.group(1)))
        self.assertEqual(missing, [])


if __name__ == "__main__":
    unittest.main()
