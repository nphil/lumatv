"""Run with: python3 -m unittest tests.test_keyboard_patch

tools/keyboard_patch.py against LG's `StarfishKeyboardView.qml` (the census copy of the set's compositor module; skipped when it is
not on this workstation): what it changes, that it changes the animation and nothing the compositor relies on, that a missing or
doubled anchor is fatal, and (with Xvfb and PyQt5) that the patched view, on LG's own BaseView, opens and closes the keyboard's
window, reports the panel the same way, and no longer overshoots.
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
TOOL = os.path.join(ROOT, "tools", "keyboard_patch.py")
SHIPPED = os.path.join(ROOT, "payload", "theme", "theme.json")
HOST = os.path.join(HERE, "qmlhost", "keyboard.py")
VIEW = "/data/home/lgtv/uiinv/qml/qml/WebOSCompositor/views/StarfishKeyboardView.qml"

sys.path.insert(0, HERE)
from xdisplay import DISPLAY, HAVE_QT  # noqa: E402

# What the compositor and BaseView rely on, as opposed to how the panel moves: every such line stays as it was.
CONTRACT = re.compile(r"function |Connections|onSurface|onPanel|onOpened|onClosed|onOrientation|onMoveArea|compositor\.|openView|closeView|contentChanged|"
                      r"openAnimation|closeAnimation|windowProperties|\bdraggable\b|panelRect|currentItem = |root\.access")


def read(path):
    with open(path) as f:
        return f.read()


@unittest.skipUnless(os.path.isfile(VIEW), "needs the census copy of LG's compositor module")
class KeyboardPatch(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.path = os.path.join(self.tmp.name, "StarfishKeyboardView.qml")
        self.theme = SHIPPED
        self.reset()

    def tearDown(self):
        self.tmp.cleanup()

    def reset(self, text=None):
        with open(self.path, "w") as f:
            f.write(read(VIEW) if text is None else text)

    def run_tool(self, theme=None):
        r = subprocess.run([sys.executable, TOOL, self.path, theme or self.theme], capture_output=True, text=True)
        return r.returncode, r.stderr

    def theme_with(self, **motion):
        t = json.loads(read(SHIPPED))
        t["motion"].update(motion)
        path = os.path.join(self.tmp.name, "theme.json")
        with open(path, "w") as f:
            json.dump(t, f)
        return path

    def test_the_jelly_is_replaced_by_the_layer_motion(self):
        rc, err = self.run_tool()
        self.assertEqual(rc, 0, err)
        text = read(self.path)
        self.assertTrue(text.startswith("// Luma: patched"))
        for gone in ("pudding", "Beziergon", "Easing.OutBack", "ShaderEffectSource", "duration: 600", "import Eos"):
            self.assertNotIn(gone, text)
        motion = json.loads(read(SHIPPED))["motion"]
        self.assertEqual(text.count("duration: %d" % motion["layerMs"]), 2)
        self.assertIn("easing.type: Easing.OutCubic", text)
        self.assertIn("easing.type: Easing.InCubic", text)

    def test_the_motion_comes_from_the_theme(self):
        self.assertEqual(self.run_tool(self.theme_with(layerMs=300, cardEasing="OutQuart"))[0], 0)
        text = read(self.path)
        self.assertEqual(text.count("duration: 300"), 2)
        self.assertIn("Easing.OutQuart", text)
        self.assertIn("Easing.InQuart", text)

    def test_a_theme_without_a_usable_motion_is_refused(self):
        for motion in ({"layerMs": 0}, {"layerMs": 5000}, {"layerMs": "fast"}, {"cardEasing": "OutBack"}):
            rc, err = self.run_tool(self.theme_with(**motion))
            self.assertEqual(rc, 1, motion)
            self.assertIn("motion", err)
            self.assertEqual(read(self.path), read(VIEW))

    def test_only_the_animation_changes_and_everything_the_compositor_relies_on_stays(self):
        self.run_tool()
        old, new = read(VIEW).splitlines(), read(self.path).splitlines()
        self.assertEqual(sorted(l for l in old if CONTRACT.search(l)), sorted(l for l in new if CONTRACT.search(l)))
        removed = [l[1:] for l in difflib.unified_diff(old, new, lineterm="", n=0) if l.startswith("-") and not l.startswith("---")]
        for l in removed:
            self.assertIsNone(CONTRACT.search(l), "removes `%s`" % l.strip())

    def test_a_changed_animation_or_mesh_is_fatal_and_says_so(self):
        for anchor in ("easing.type: Easing.OutBack", "Eos.Beziergon {", "PropertyAction { target: pudding; property: \"visible\"; value: true }",
                       "import Eos.Items 0.1 as Eos"):
            self.reset(read(VIEW).replace(anchor, anchor + " "))
            rc, err = self.run_tool()
            self.assertEqual(rc, 1, anchor)
            self.assertIn("StarfishKeyboardView.qml: anchor found 0 times", err, anchor)
            self.assertEqual(read(self.path), read(VIEW).replace(anchor, anchor + " "))

    def test_a_doubled_anchor_is_fatal_too(self):
        self.reset(read(VIEW) + "\nimport Eos.Items 0.1 as Eos\n")
        rc, err = self.run_tool()
        self.assertEqual(rc, 1)
        self.assertIn("anchor found 2 times, expected 1", err)

    def test_patching_twice_is_refused(self):
        self.assertEqual(self.run_tool()[0], 0)
        rc, err = self.run_tool()
        self.assertEqual(rc, 1)
        self.assertIn("already patched", err)

    def test_usage(self):
        self.assertEqual(subprocess.run([sys.executable, TOOL, self.path], capture_output=True, text=True).returncode, 2)


@unittest.skipUnless(os.path.isfile(VIEW) and DISPLAY and HAVE_QT, "needs the census copy of LG's module, Xvfb and PyQt5")
class PatchedViewInTheHost(unittest.TestCase):
    """LG's own BaseView behind stand-ins for the compositor's native types, in Qt 5.15 on software GL. The animation clock there
    does not match the wall clock, so durations are compared with each other's, never with a stopwatch."""

    def run_host(self, *args):
        r = subprocess.run([sys.executable, HOST, *args], env=dict(os.environ, DISPLAY=DISPLAY), capture_output=True, text=True, timeout=120)
        out = r.stdout + r.stderr
        events = {m.group(1): int(m.group(2)) for m in re.finditer(r"\[event\] (opened|closed) (\d+)", out)}
        reports = {m.group(1): json.loads(m.group(2)) for m in re.finditer(r"\[report (\w+)\] (\{.*\})", out)}
        panel = re.findall(r"\[panelRect\] (\S+ \S+)", out)
        return r.returncode, out, events, reports, panel

    def test_it_loads_clean_and_opens_and_closes_the_window_as_LGs_does(self):
        stock = self.run_host()
        patched = self.run_host("--patched")
        self.assertEqual(patched[0], 0, patched[1][-1500:])
        # LG's own view warns once: `visible: draggableMask.isTransitioning` binds the mesh to a property no item has
        self.assertEqual(stock[0], 1, stock[1][-1500:])
        self.assertIn("StarfishKeyboardView.qml:230", stock[1])
        # the panel is reported where the window ended up, and cleared on close, the same both ways
        self.assertEqual(patched[4], stock[4])
        self.assertEqual(patched[4], ["0,660 1920x420", "0,0 0x0"])
        for tag, (shown, y) in {"open": (True, 660), "closed": (False, 1080)}.items():
            for run in (stock, patched):
                r = run[3][tag]
                self.assertEqual((r["visible"], r["itemVisible"]), (shown, shown), (tag, r))
                self.assertAlmostEqual(r["itemY"], y, delta=1, msg=tag)
        self.assertEqual(sorted(patched[2]), ["closed", "opened"])

    def test_it_no_longer_overshoots_and_is_quicker(self):
        stock = self.run_host()
        patched = self.run_host("--patched")
        # LG's OutBack carries the window past its place (a smaller y); an ease-out stops on it
        self.assertLess(stock[3]["open"]["minY"], 660 - 20)
        self.assertGreaterEqual(patched[3]["open"]["minY"], 660 - 1)
        self.assertLess(patched[2]["opened"] * 2, stock[2]["opened"])
        self.assertLess(patched[2]["closed"] * 2, stock[2]["closed"])

    def test_the_duration_is_the_themes(self):
        slow = self.run_host("--patched", "--layer-ms", "600")
        fast = self.run_host("--patched", "--layer-ms", "150")
        self.assertLess(fast[2]["opened"] * 2, slow[2]["opened"])
        self.assertLess(fast[2]["closed"] * 2, slow[2]["closed"])


if __name__ == "__main__":
    unittest.main()
