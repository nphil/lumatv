"""Run with: python3 -m unittest tests.test_store_dump

tools/store_dump.py, the read-only inventory of a page WAM is showing (the first thing the store theme's TV round does, so that the theme's
selectors are made against the store's real markup): how it chooses the page, and what it asks the page and answers.

`Choose` is the page selection. `Dump` starts the system Chromium with a remote debugging port on the stand-in home page of
docs/mocks/store (what WAM's own --remote-debugging-port offers on the set) and runs the tool against it; skipped without Chromium. What
the answer must hold: the markup's class names and the Sandstone components among them, the effects LG's rules pay for (box shadows,
will-change, transitions and their durations), the pictures and their sizes, the stylesheets, the fonts; and none of the page's words.
"""
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import unittest
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path.insert(0, os.path.join(ROOT, "tools"))
import store_dump  # noqa: E402

TOOL = os.path.join(ROOT, "tools", "store_dump.py")
CHROMIUM = "/usr/bin/chromium"
PAGE = "file://" + os.path.join(ROOT, "docs", "mocks", "store", "home.html")


def page(url, title="", kind="page"):
    return {"type": kind, "url": url, "title": title, "webSocketDebuggerUrl": "ws://127.0.0.1:9998/devtools/page/x"}


class Choose(unittest.TestCase):
    def test_the_store_is_the_page_whose_url_says_so(self):
        pages = [page("file:///usr/palm/applications/com.palm.app.settings/index.html"), page("https://us.app.lgwebostv.com/discovery_2024/discovery.html")]
        self.assertEqual(store_dump.choose(pages, None)[0], pages[1])
        pages = [page("file:///mnt/otncabi/usr/palm/applications/com.webos.app.discovery/index.html"), page("file:///a/b.html")]
        self.assertEqual(store_dump.choose(pages, None)[0], pages[0])

    def test_the_only_page_is_the_page(self):
        pages = [page("file:///x.html"), page("chrome://x", kind="browser_ui")]
        self.assertEqual(store_dump.choose(pages, None)[0], pages[0])

    def test_two_candidates_are_no_choice_and_are_listed(self):
        pages = [page("file:///a.html"), page("file:///b.html")]
        chosen, candidates = store_dump.choose(pages, None)
        self.assertIsNone(chosen)
        self.assertEqual(len(candidates), 2)

    def test_match_picks_by_url_or_title_and_must_be_one(self):
        pages = [page("file:///a.html", "Apps"), page("file:///b.html", "Other")]
        self.assertEqual(store_dump.choose(pages, "Other")[0], pages[1])
        self.assertEqual(store_dump.choose(pages, "b.html")[0], pages[1])
        self.assertIsNone(store_dump.choose(pages, "html")[0])
        self.assertIsNone(store_dump.choose(pages, "nothing")[0])

    def test_pages_that_are_not_pages_are_never_chosen(self):
        pages = [page("chrome://omnibox", kind="browser_ui"), page("chrome://other", kind="service_worker")]
        self.assertIsNone(store_dump.choose(pages, None)[0])


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


@unittest.skipUnless(os.path.exists(CHROMIUM), "needs the system Chromium")
class Dump(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        cls.port = free_port()
        cls.browser = subprocess.Popen([CHROMIUM, "--headless=new", "--no-sandbox", "--disable-gpu", "--remote-debugging-port=%d" % cls.port,
                                        "--user-data-dir=" + cls.tmp.name, PAGE], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        deadline = time.time() + 30
        while time.time() < deadline:
            try:
                pages = json.loads(urllib.request.urlopen("http://127.0.0.1:%d/json/list" % cls.port, timeout=2).read())
                if any(p.get("url", "").endswith("home.html") for p in pages):
                    break
            except OSError:
                pass
            time.sleep(0.5)
        else:
            cls.browser.kill()
            raise AssertionError("Chromium did not come up with its debugging port")
        time.sleep(1.5)       # the stand-in's pictures are made as the page loads
        r = subprocess.run([sys.executable, TOOL, "--port", str(cls.port), "--match", "home.html", "--out", os.path.join(cls.tmp.name, "dump.json")],
                           capture_output=True, text=True, timeout=60)
        cls.rc, cls.out = r.returncode, r.stdout
        cls.dump = json.loads(r.stdout)

    @classmethod
    def tearDownClass(cls):
        cls.browser.terminate()
        try:
            cls.browser.wait(timeout=10)
        except subprocess.TimeoutExpired:
            cls.browser.kill()
        cls.tmp.cleanup()

    def test_it_answers_and_writes_the_same_object_to_the_file(self):
        self.assertEqual((self.rc, self.dump["ok"]), (0, True))
        with open(os.path.join(self.tmp.name, "dump.json")) as f:
            self.assertEqual(json.loads(f.read()), self.dump)

    def test_the_markup_is_named_by_class_and_by_sandstone_component(self):
        dom = self.dump["page"]["dom"]
        self.assertGreater(dom["elements"], 100)
        sandstone = dict((k, v) for k, v in dom["sandstone"])
        self.assertEqual(sandstone["ImageItem_imageItem"], 5)
        self.assertIn("TabGroup_tab", sandstone)
        self.assertTrue(any(name.startswith("button.enact_sandstone_ImageItem_ImageItem_imageItem__") for name, _ in dom["topClasses"]))

    def test_the_effects_lgs_rules_pay_for_are_counted(self):
        e = self.dump["page"]["effects"]
        self.assertGreaterEqual(e["elementsWithBoxShadow"], 1)
        self.assertGreaterEqual(e["elementsWithWillChange"], 1)
        self.assertEqual(e["elementsWithBackdropFilter"], 0)
        self.assertIn("0.2s", [d for d, _ in e["transitionDurations"]])

    def test_the_pictures_and_their_sizes_are_there(self):
        images = self.dump["page"]["images"]
        self.assertGreaterEqual(images["count"], 10)
        self.assertGreater(images["decodedMB"], 1)
        self.assertTrue(images["oversized"] and all(i["ratio"] >= 1.5 for i in images["oversized"]))
        self.assertEqual(images["lazy"], 0)

    def test_the_styles_the_timing_and_whether_the_theme_ran_are_there(self):
        page = self.dump["page"]
        self.assertTrue(any(s["href"].endswith("stock.css") for s in page["styles"]["sheets"]))
        self.assertEqual(page["styles"]["htmlBackground"], "rgb(0, 0, 0)")
        self.assertIsNone(page["marker"], "this page was loaded without the theme's script")
        self.assertIn("first-contentful-paint", page["paint"])
        self.assertEqual(page["navigation"]["type"], "navigate")
        self.assertIn(page["protocol"], ("file:", "https:"))

    def test_the_answer_holds_none_of_the_pages_words(self):
        for word in ("Daily Radio", "Karaoke", "Entertainment", "Arcade", "Streamline", "Featured", "Most Popular"):
            self.assertNotIn(word, self.out)

    def test_the_list_prints_one_object_a_page_and_two_candidates_exit_2(self):
        r = subprocess.run([sys.executable, TOOL, "--port", str(self.port), "--list"], capture_output=True, text=True, timeout=30)
        self.assertEqual(r.returncode, 0)
        self.assertTrue(any("home.html" in json.loads(l)["url"] for l in r.stdout.splitlines()))
        r = subprocess.run([sys.executable, TOOL, "--port", str(self.port), "--match", "nothing-like-this"], capture_output=True, text=True, timeout=30)
        self.assertEqual(r.returncode, 2)
        self.assertEqual(json.loads(r.stdout)["error"], "no single page matched")

    def test_an_inspector_that_does_not_answer_is_exit_1(self):
        r = subprocess.run([sys.executable, TOOL, "--port", str(free_port())], capture_output=True, text=True, timeout=30)
        self.assertEqual(r.returncode, 1)
        self.assertFalse(json.loads(r.stdout)["ok"])

    def test_the_tool_changes_nothing_it_only_asks(self):
        with open(TOOL) as f:
            code = f.read()
        for word in ("Page.navigate", "Input.dispatch", "Page.reload", "DOM.setAttributeValue", "Emulation.", "Network.enable"):
            self.assertNotIn(word, code)


if __name__ == "__main__":
    unittest.main()
