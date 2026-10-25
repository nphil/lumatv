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


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class Settings(unittest.TestCase):
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

    def test_the_rail_lists_only_pages_that_exist_and_the_way_to_LGs(self):
        rc, out = self.host("--eval", "1500:root.openSettings()", *self.keys(3500, "down", "ok"),
                            "--eval", "5200:root.settingsOpen", "--wait", "6000")
        self.assertEqual(rc, 0, out[-1500:])
        # Down lands on "More in LG settings", OK on it launches LG's app and closes Settings
        self.assertEqual(self.values(out)[-1], "false")


if __name__ == "__main__":
    unittest.main()
