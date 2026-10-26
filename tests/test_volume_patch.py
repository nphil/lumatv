"""Run with: python3 -m unittest tests.test_volume_patch

tools/volume_patch.py against LG's `StarfishVolumeView.qml` (the census copy of the set's compositor module; skipped when it is not on
this workstation): what it changes, that it changes nothing the compositor relies on, that a missing or doubled anchor is fatal, and
(with Xvfb and PyQt5) that LG's own view, stock, opens its popup when the audio service announces a change and, patched, never does.
"""
import difflib
import json
import os
import re
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
TOOL = os.path.join(ROOT, "tools", "volume_patch.py")
HOST = os.path.join(HERE, "qmlhost", "volumeview.py")
VIEW = "/data/home/lgtv/uiinv/qml/qml/WebOSCompositor/views/StarfishVolumeView.qml"

sys.path.insert(0, HERE)
from xdisplay import DISPLAY, HAVE_QT  # noqa: E402

# What the compositor's other code reads from this view, as opposed to whether it opens: every such line stays as it was.
CONTRACT = re.compile(r"function |signal |Connections|onVolumeUpdated|onOpened|onClosed|onAvailableChanged|requestTts|muteStr|openView|closeView|"
                      r"handleVolumeBannerTimeout|setLocaleInfo|updateVisibility|BaseView|import ")


def read(path):
    with open(path) as f:
        return f.read()


@unittest.skipUnless(os.path.isfile(VIEW), "needs the census copy of LG's compositor module")
class VolumePatch(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.path = os.path.join(self.tmp.name, "StarfishVolumeView.qml")
        self.reset()

    def tearDown(self):
        self.tmp.cleanup()

    def reset(self, text=None):
        with open(self.path, "w") as f:
            f.write(read(VIEW) if text is None else text)

    def run_tool(self):
        r = subprocess.run([sys.executable, TOOL, self.path], capture_output=True, text=True)
        return r.returncode, r.stderr

    def test_the_view_is_never_available(self):
        rc, err = self.run_tool()
        self.assertEqual(rc, 0, err)
        text = read(self.path)
        self.assertTrue(text.startswith("// Luma: patched"))
        self.assertIn("property bool available: false && !((root.powerProcessing", text)
        # LG's expression is still there, after the `false &&`: only a guard is added to it
        self.assertIn('tvlinkOsdDisplay === "null" ? volumeOSD : tvlinkOsdDisplay === "on"', text)

    def test_what_is_added_is_a_header_a_guard_on_the_property_and_one_on_the_function(self):
        self.run_tool()
        old, new = read(VIEW).splitlines(), read(self.path).splitlines()
        changed = [(t, old[a1:a2], new[b1:b2]) for t, a1, a2, b1, b2 in difflib.SequenceMatcher(None, old, new, autojunk=False).get_opcodes() if t != "equal"]
        self.assertEqual([c[0] for c in changed], ["insert", "replace", "insert"], changed)
        self.assertEqual(len(changed[0][2]), 1)                                   # the header
        self.assertEqual((len(changed[1][1]), len(changed[1][2])), (1, 2))        # the property: a comment and `false &&`
        self.assertEqual(changed[1][2][1].replace("false && ", "", 1), changed[1][1][0])
        self.assertEqual(len(changed[2][2]), 1)                                   # `if (!available) return;` first in updateVisibility
        self.assertIn("if (!available) return;", changed[2][2][0])
        self.assertEqual(new[new.index(changed[2][2][0]) - 1].strip(), "function updateVisibility() {")

    def test_everything_the_compositor_relies_on_stays(self):
        self.run_tool()
        old, new = read(VIEW).splitlines(), read(self.path).splitlines()
        self.assertEqual(sorted(l for l in old if CONTRACT.search(l)), sorted(l for l in new if CONTRACT.search(l) and "Luma" not in l))

    def test_a_view_that_does_not_gate_the_signal_on_available_is_refused(self):
        # without the gate `available` would not stop anything: the patch would say it did
        for text in (read(VIEW).replace("            if (available)\n                updateVisibility();", "            updateVisibility();"),
                     read(VIEW).replace("onVolumeUpdated:", "onVolumeChanged:")):
            self.reset(text)
            rc, err = self.run_tool()
            self.assertEqual(rc, 1)
            self.assertIn("StarfishVolumeView.qml: anchor found 0 times", err)
            self.assertEqual(read(self.path), text)

    def test_a_changed_function_is_fatal_too(self):
        self.reset(read(VIEW).replace("function updateVisibility() {", "function updateVisibility(force) {"))
        rc, err = self.run_tool()
        self.assertEqual(rc, 1)
        self.assertIn("anchor found 0 times", err)

    def test_a_changed_or_doubled_property_is_fatal_and_says_so(self):
        self.reset(read(VIEW).replace("property bool available:", "property bool availableNow:"))
        rc, err = self.run_tool()
        self.assertEqual(rc, 1)
        self.assertIn("anchor found 0 times", err)
        self.reset(read(VIEW) + "\n    property bool available: !((root.powerProcessing === \"\"))\n")
        rc, err = self.run_tool()
        self.assertEqual(rc, 1)
        self.assertIn("anchor found 2 times, expected 1", err)

    def test_patching_twice_is_refused(self):
        self.assertEqual(self.run_tool()[0], 0)
        rc, err = self.run_tool()
        self.assertEqual(rc, 1)
        self.assertIn("already patched", err)

    def test_usage(self):
        self.assertEqual(subprocess.run([sys.executable, TOOL], capture_output=True, text=True).returncode, 2)


@unittest.skipUnless(os.path.isfile(VIEW) and DISPLAY and HAVE_QT, "needs the census copy of LG's module, Xvfb and PyQt5")
class LGsViewInTheHost(unittest.TestCase):
    """LG's own view and BaseView behind stand-ins for the audio service, the settings and the two popups the view hosts."""

    def run_host(self, *args):
        r = subprocess.run([sys.executable, HOST, *args], env=dict(os.environ, DISPLAY=DISPLAY), capture_output=True, text=True, timeout=120)
        out = r.stdout + r.stderr
        reports = {m.group(1): json.loads(m.group(2)) for m in re.finditer(r"\[report (\w+)\] (\{.*\})", out)}
        return r.returncode, out, reports

    def test_stock_opens_its_popup_on_a_change_and_patched_never_does(self):
        stock = self.run_host("--muted")
        patched = self.run_host("--muted", "--patched")
        self.assertEqual(stock[0], 0, stock[1][-1500:])
        self.assertEqual(patched[0], 0, patched[1][-1500:])
        # the mutation check: the stand-ins are enough for LG's view to open as it does on the set
        self.assertEqual(stock[2]["before"], {"available": True, "viewOpen": False, "visible": False, "popupOpens": 0, "muteOpens": 0})
        self.assertTrue(stock[2]["after"]["available"] and stock[2]["after"]["viewOpen"] and stock[2]["after"]["visible"], stock[2])
        self.assertEqual(stock[2]["after"]["popupOpens"], 1)
        self.assertEqual(patched[2]["before"], {"available": False, "viewOpen": False, "visible": False, "popupOpens": 0, "muteOpens": 0})
        self.assertEqual(patched[2]["after"], patched[2]["before"])
        # LG's own line, once for the change and once for the mute: the proof on the set that the signal arrived and was declined
        self.assertEqual(patched[1].count("volume view is not available"), 2)
        self.assertEqual(stock[1].count("volume view is not available"), 0)

    def test_the_country_path_that_asks_no_one_opens_nothing_either(self):
        stock = self.run_host("--ukraine")
        patched = self.run_host("--ukraine", "--patched")
        self.assertEqual(stock[0], 0, stock[1][-1500:])
        self.assertEqual(patched[0], 0, patched[1][-1500:])
        # without the guard on the function the mute bubble's own path would open the popup in Ukraine
        self.assertTrue(stock[2]["after"]["viewOpen"] and stock[2]["after"]["popupOpens"] == 1, stock[2])
        self.assertEqual(patched[2]["after"], patched[2]["before"])
        self.assertFalse(patched[2]["after"]["viewOpen"])

    def test_every_sound_output_is_declined_not_only_the_arc_one(self):
        for output in ("tv_speaker", "external_arc", "headphone", "external_optical", "bt_soundbar"):
            rc, out, reports = self.run_host("--patched", "--output", output)
            self.assertEqual(rc, 0, out[-1500:])
            self.assertFalse(reports["after"]["viewOpen"], output)
            self.assertEqual((reports["after"]["popupOpens"], reports["after"]["muteOpens"]), (0, 0), output)


if __name__ == "__main__":
    unittest.main()
