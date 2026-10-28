"""Run with: python3 -m unittest discover -s tests -p 'test_transitions_home.py'   (needs Xvfb and PyQt5; skipped without them)

What Home does for LumaTV's compositor when it grows an app's card out of its tile (docs/research/transitions-result.md): `smoothLaunch`, set by
compositor/shell/Shell.qml from the compositor's `transitions` topic, makes a launch from the dock tell the compositor where the tile is
(`luna://com.webos.surfacemanager/setLaunchOrigin`) before it asks the app manager for the app, and makes the launch screen wait 400 ms (an app
that has a window by then never shows it; one that is slow shows it, fading in, and tells where its icon is). Without it, which is every launch under LG's
compositor, nothing changes: the launch screen is up at once and no origin is sent. Nothing here measures time against a gate: the set does.
"""
import json
import os
import re
import tempfile
import unittest

from test_settings import OPEN_AT, RAIL, HostCase
from test_settings import READY as SETTINGS_READY
from xdisplay import DISPLAY, HAVE_QT

READY = 7000
CALLS = "JSON.stringify(root.lunaCalls.map(function(c) { var p; try { p = JSON.parse(c.payload) } catch (e) { p = c.payload } return { m: c.method, p: p } }))"
FIND = ("function find(p, k) { if (p[k] !== undefined && p.children !== undefined) return p; for (var i = 0; i < p.children.length; i++) "
        "{ var f = find(p.children[i], k); if (f) return f } return null } ")
SPLASH = "(function(){ " + FIND + "var s = find(root, 'iconRect'); return s ? s.mode : 'none' })()"


def key(code, pressed="true"):
    return "root.key(%s, %s, false, 0)" % (code, pressed)


def do(js):
    return "(function(){ %s; return 1 })()" % js


def origins(got):
    return [c["p"] for c in json.loads(got) if c["m"] == "/setLaunchOrigin"]


def methods(got):
    return [c["m"] for c in json.loads(got) if c["m"] in ("/setLaunchOrigin", "/launch")]


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class LaunchOrigin(HostCase):
    def run_host(self, steps, wait):
        argv = []
        for ms, expr, label in steps:
            argv += ["--eval", "%d:(function(){ return %s + (%s) })()" % (READY + ms, json.dumps(label + "="), expr)]
        rc, out = self.host(*argv, "--wait", str(READY + wait))
        self.assertEqual(rc, 0, out[-1500:])
        got = {}
        for v in self.values(out):
            m = re.match(r"^([a-z0-9_]+)=(.*)$", v, re.S)
            if m:
                got[m.group(1)] = m.group(2)
        return got

    def test_a_launch_says_where_its_tile_is_and_the_launch_screen_waits_for_a_slow_app(self):
        got = self.run_host([
            (0, do("root.smoothLaunch = true"), "on"),
            (50, key("Qt.Key_2"), "press"), (70, key("Qt.Key_2", "false"), "release"),
            (250, CALLS, "calls_early"), (250, SPLASH, "splash_early"),
            (800, CALLS, "calls_late"), (800, SPLASH, "splash_late"),
        ], 1100)
        self.assertEqual(got["press"], "true")
        self.assertEqual(methods(got["calls_early"]), ["/setLaunchOrigin", "/launch"], "the compositor hears where the tile is before the app is asked for")
        first = origins(got["calls_early"])[0]
        self.assertEqual(first["appId"], "netflix")
        self.assertIsNone(first["from"], "a launch starts from its tile: a splash origin left from before is taken back")
        t = first["tile"]
        self.assertTrue(t["width"] > 40 and t["height"] > 40 and t["x"] >= 0 and t["y"] > 300, t)
        # the focused tile is lifted (theme focus.tileScale, 1.12), and what the card grows out of is what is on the screen
        self.assertAlmostEqual(t["radius"], 22 * 1.12, places=3)
        self.assertAlmostEqual(t["width"], t["height"], places=3)
        self.assertEqual(got["splash_early"], "", "no launch screen yet: the tile is what the card grows from")
        # the app is slow: the launch screen came (400 ms), and its icon is where the card grows from now
        self.assertEqual(got["splash_late"], "launch")
        late = origins(got["calls_late"])
        self.assertEqual(len(late), 2)
        icon = late[1]
        self.assertEqual(icon["appId"], "netflix")
        self.assertEqual((icon["from"]["width"], icon["from"]["height"], icon["from"]["radius"]), (180, 180, 38))
        self.assertNotIn("tile", icon)

    def test_an_app_that_has_a_window_in_time_never_shows_the_launch_screen(self):
        got = self.run_host([
            (0, do("root.smoothLaunch = true"), "on"),
            (50, key("Qt.Key_2"), "press"), (70, key("Qt.Key_2", "false"), "release"),
            (200, do("root.appFronted('netflix')"), "fronted"),
            (900, CALLS, "calls"), (900, SPLASH, "splash"),
        ], 1200)
        self.assertEqual(got["splash"], "")
        self.assertEqual(len(origins(got["calls"])), 1, "and says nothing of an icon")

    def test_without_the_compositors_word_a_launch_is_the_cut_it_was(self):
        got = self.run_host([
            (50, key("Qt.Key_2"), "press"), (70, key("Qt.Key_2", "false"), "release"),
            (250, CALLS, "calls"), (250, SPLASH, "splash"),
        ], 600)
        self.assertEqual(methods(got["calls"]), ["/launch"])
        self.assertEqual(got["splash"], "launch", "the launch screen is up at once")


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class SwitchRow(HostCase):
    def test_the_smooth_transitions_row_writes_the_switch_to_the_theme_file(self):
        # Home & look, Look tab: Dynamic tint, Appearance, Clock, Wallpaper motion, Glass depth, then Smooth transitions (off by default): OK turns it on
        with tempfile.TemporaryDirectory() as d:
            data = os.path.join(d, "data")
            rc, out = self.host("--eval", "%d:root.openSettings()" % OPEN_AT, *self.keys(SETTINGS_READY + 600, *(["down"] * RAIL["look"])),
                                *self.keys(SETTINGS_READY + 1500, "right", *["down"] * 5, "ok"),
                                "--eval", "%d:JSON.stringify(root.theme.motion)" % (SETTINGS_READY + 3200), "--wait", str(SETTINGS_READY + 3800), data=data)
            self.assertEqual(rc, 0, out[-1500:])
            self.assertIs(self.docs(out)[-1]["transitions"], True)
            with open(os.path.join(data, "theme", "theme.json")) as f:
                self.assertEqual(json.load(f)["motion"], {"transitions": True})


if __name__ == "__main__":
    unittest.main()
