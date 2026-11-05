"""The launcher app (index.html, launch.js, app.js): opened from LG's launcher after an escape it starts LumaTV by the tool's own `compositor on`
(the Home stub first, LG's Flutter Home ended, LumaTV's Home up), with no key pressed, and where that fails it says why and offers LG's Home.
The page runs under node (tests/launcher_app_check.js) against stand-ins for its DOM and for the Homebrew Channel's root service."""
import json
from pathlib import Path
import re
import shutil
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[1]
CHECK = ROOT / "tests" / "launcher_app_check.js"


@unittest.skipUnless(shutil.which("node"), "node runs the page")
class LauncherApp(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        result = subprocess.run(["node", str(CHECK)], capture_output=True, text=True, timeout=60)
        assert result.returncode == 0, result.stderr
        cls.out = json.loads(result.stdout)

    def test_opened_after_an_escape_it_starts_lumatv_once_and_says_so_while_it_does(self):
        run = self.out["afterEscape"]
        self.assertEqual(run["on"], 1, "once, though the page's start runs twice (the deferred script and DOMContentLoaded)")
        self.assertLess(run["calls"].index("compositor status"), run["calls"].index("compositor on"))
        self.assertEqual(run["progressShownWhenStarted"], [True], "the screen says it is opening LumaTV when the compositor is asked for")
        self.assertEqual(run["home"], [], "the tool brings LumaTV's Home up itself: the page asks for no Home")
        self.assertFalse(run["failureShown"] or run["progressShownAtEnd"])

    def test_opened_while_lumatv_runs_it_starts_nothing_and_the_open_row_opens_home(self):
        self.assertEqual(self.out["whileLive"]["on"], 0)
        self.assertEqual(self.out["whileLive"]["home"], [])
        self.assertEqual(self.out["rowWhileLive"], {"on": 0, "homeKeys": 1})

    def test_a_start_that_fails_says_why_and_offers_lgs_home_and_a_second_try(self):
        failure = self.out["failure"]
        self.assertTrue(failure["shown"])
        self.assertEqual(failure["reason"], "the supervisor would not start: firmware: not a verified build", "the tool's own words")
        self.assertFalse(failure["progressShown"])
        self.assertEqual(failure["on"], 1, "a refusal is not retried on its own")
        self.assertEqual(failure["lgHome"], ["luna-send -n 1 luna://com.webos.applicationManager/launch '{\"id\":\"com.webos.app.home\"}'"])
        self.assertEqual(failure["afterRetry"], {"on": 2, "failureShown": False, "live": True})

    def test_a_recovery_that_is_still_running_is_waited_for_and_nothing_else_is(self):
        decision = self.out["decision"]
        recovering = decision["recovering"]
        self.assertEqual((recovering["outcome"], recovering["on"], recovering["sleeps"]), ({"outcome": "started"}, 3, [3000, 3000]))
        self.assertEqual(recovering["progress"][1].split(":")[0], "waiting")
        never = decision["never"]
        self.assertEqual(never["outcome"]["outcome"], "failed")
        self.assertEqual(never["waited"], 120000, "as long as the recovery's own budget of 116 s, and no longer")
        self.assertEqual(never["lastCall"], "compositor status", "and the status is read once more before it is called a failure")
        self.assertEqual(decision["refused"]["cli"], ["compositor status", "compositor on", "compositor status"])
        self.assertEqual(decision["refused"]["sleeps"], [])
        self.assertEqual(decision["mounted"], {"outcome": "failed", "reason": "the compositor is already mounted"})
        self.assertEqual(decision["cutOff"], {"outcome": "live"}, "a start whose answer the restart cut off, but that worked, is not a failure")

    def test_the_busy_texts_are_the_ones_the_tool_says(self):
        self.assertEqual(self.out["decision"]["busy"], [True, True, False])
        self.assertIn("a recovery of LG's Home is running", (ROOT / "tools" / "compositor.sh").read_text())
        self.assertIn("another mutation is still running", (ROOT / "tools" / "lumatv").read_text())

    def test_lgs_home_is_launched_by_id_and_a_refusal_is_an_error(self):
        self.assertEqual(self.out["openLgHome"]["commands"], ["luna-send -n 1 luna://com.webos.applicationManager/launch '{\"id\":\"com.webos.app.home\"}'"])
        self.assertEqual(self.out["openLgHome"]["refused"], "no way")

    def test_the_app_keeps_its_id_and_ships_the_new_file_with_the_package(self):
        self.assertIn('"id": "{{appId}}"', (ROOT / "appinfo.json").read_text())
        self.assertEqual(json.loads((ROOT / "brand.json").read_text())["appId"], "org.nphil.lumatv")
        page = (ROOT / "index.html").read_text()
        self.assertLess(page.index('src="launch.js"'), page.index('src="app.js"'), "the module is there before the page uses it")
        build = (ROOT / "tools" / "build-ipk.sh").read_text()
        self.assertTrue(re.search(r"^launch\.js$", build, re.M) and 'cp launch.js "$DATA_ROOT/"' in build)


if __name__ == "__main__":
    unittest.main()
