"""Run with: cd tests && python3 -m unittest test_shell_switcher   (needs Xvfb and PyQt5; skipped without them)

The app switcher through the shell client (compositor/shell/Shell.qml) against a stand-in of the compositor's UI channel
(tests/qmlhost/shellhost.py): the shell says it can show it only while the owner's switch is on, the compositor's `apps` and `switcher`
topics open it and fill it, it sends the places of the cards over `POST /v1/switcher`, takes the keys while it is up and sends what a key
chose, and a Home press the compositor offers (`homekey` with `switcher`) opens it only when Home has nothing open. The compositor's half is
`compositor/crates/lumatv-compositor/tests/switcher.rs`, the surface's own behaviour `tests/test_switcher.py`.
"""
import json
import os
import re
import subprocess
import sys
import unittest

from xdisplay import DISPLAY, HAVE_QT

HERE = os.path.dirname(os.path.abspath(__file__))
HOST = os.path.join(HERE, "qmlhost", "shellhost.py")
HOME = '{"app":"com.webos.app.home","home":true,"covered":false}'
APPS = json.dumps([{"id": "plex", "title": "Plex", "picture": "live", "w": 1920, "h": 1080, "state": ""},
                   {"id": "netflix", "title": "Netflix", "picture": "live", "w": 1920, "h": 1080, "state": ""},
                   {"id": "youtube.leanback.v4", "title": "YouTube", "picture": "live", "w": 1920, "h": 1080, "state": ""}])
CLOSED = '{"open":false,"from":"","n":0}'
OPEN = '{"open":true,"from":"","n":1}'
ON = '{"perf":{"syncResidents":true},"remote":{"appSwitcher":true}}'
OFF = '{"perf":{"syncResidents":true}}'
BUILT = 13500                  # the overlay builds its resident surfaces by then (LumaOverlay.warmSwitcher at 12.5 s)


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class ShellSwitcher(unittest.TestCase):
    def host(self, *args, wait, theme=ON):
        env = dict(os.environ, DISPLAY=DISPLAY, QT_QPA_PLATFORM="offscreen")
        cmd = [sys.executable, HOST, "--theme", theme, "--wait", str(wait), *args]
        r = subprocess.run(cmd, env=env, capture_output=True, text=True, timeout=int(os.environ.get("HOST_TIMEOUT", "240")))
        return r.returncode, r.stdout + r.stderr

    def lines(self, out, tag):
        return [json.loads(m.group(1)) for m in re.finditer(r"^\[%s\] (.*)$" % tag, out, re.M)]

    def fed(self):
        return ["--push", "500:foreground:" + HOME, "--push", "500:screen:{\"on\":true}", "--push", "600:apps:" + APPS, "--push", "600:switcher:" + CLOSED]

    def test_it_can_be_shown_only_while_the_switch_is_on_and_the_compositors_topics_open_it(self):
        rc, out = self.host(*self.fed(), "--push", "%d:switcher:%s" % (BUILT + 500, OPEN), wait=BUILT + 2500)
        self.assertEqual(rc, 0, out[-2500:])
        reports = self.lines(out, "report")
        self.assertTrue(any("switcher" in r["can"] for r in reports), "the client says it can show it: %s" % [r["can"] for r in reports])
        self.assertTrue(any(r["wants"] is True for r in reports), "and takes the keys once it is open: %s" % [r["wants"] for r in reports])
        posts = self.lines(out, "switcher")
        layouts = [p for p in posts if p["op"] == "layout"]
        self.assertTrue(layouts, "the places of the cards went to the compositor: %s" % posts)
        self.assertEqual([c["id"] for c in layouts[0]["cards"]], ["plex", "netflix", "youtube.leanback.v4"])
        self.assertEqual([c["x"] for c in layouts[0]["cards"]], [328, 760, 1192], "three cards, centred")
        rc, out = self.host(*self.fed(), wait=BUILT + 1500, theme=OFF)
        self.assertFalse(any("switcher" in r["can"] for r in self.lines(out, "report")), "with the switch off it never says so")

    def test_keys_in_the_system_layer_choose_and_the_choice_goes_to_the_compositor(self):
        rc, out = self.host(*self.fed(), "--push", "%d:switcher:%s" % (BUILT, OPEN),
                            "--key", "%d:sys:106" % (BUILT + 800), "--key", "%d:sys:28" % (BUILT + 1000), wait=BUILT + 2200)
        self.assertEqual(rc, 0, out[-2500:])
        posts = self.lines(out, "switcher")
        self.assertIn({"op": "activate", "id": "netflix"}, posts, "Right, then OK: the second card")

    def test_a_press_of_home_the_compositor_offers_opens_it_only_when_home_has_nothing_open(self):
        rc, out = self.host(*self.fed(), "--push", "%d:homekey:{\"n\":1,\"switcher\":true}" % (BUILT + 500), wait=BUILT + 1800)
        self.assertEqual(rc, 0, out[-2500:])
        self.assertIn({"op": "open"}, self.lines(out, "switcher"), "Home at rest: the switcher is asked for")
        rc, out = self.host(*self.fed(), "--push", "%d:homekey:{\"n\":1,\"switcher\":false}" % (BUILT + 500), wait=BUILT + 1800)
        self.assertEqual(self.lines(out, "switcher"), [], "not offered: Home is closed as ever and the switcher is not asked for")


if __name__ == "__main__":
    unittest.main()
