"""Run with: python3 -m unittest tests.test_notification_patch

tools/notification_patch.py against a copy of LG's notification tree (the census copy of the set's
/usr/lib/qt5/qml/SystemUI/Notification; skipped when it is not on this workstation): what it
changes, that it changes nothing else, and that a missing anchor is fatal.
"""
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
TOOL = os.path.join(ROOT, "tools", "notification_patch.py")
TOKENS = os.path.join(ROOT, "tools", "tokens.py")
SHIPPED = os.path.join(ROOT, "payload", "theme", "theme.json")
CENSUS = "/data/home/lgtv/uiinv/qml/qml/SystemUI/Notification"


def palette():
    out = subprocess.run([sys.executable, TOKENS, SHIPPED, os.devnull, "qml"], capture_output=True, text=True, check=True).stdout
    return json.loads(out)


@unittest.skipUnless(os.path.isdir(CENSUS), "needs the census copy of LG's notification tree")
class NotificationPatch(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.dir = os.path.join(self.tmp.name, "Notification")
        shutil.copytree(CENSUS, self.dir)
        self.pal = palette()

    def tearDown(self):
        self.tmp.cleanup()

    def run_tool(self, pal=None):
        r = subprocess.run([sys.executable, TOOL, self.dir, json.dumps(pal or self.pal)], capture_output=True, text=True)
        return r.returncode, r.stderr

    def read(self, name):
        with open(os.path.join(self.dir, name)) as f:
            return f.read()

    def test_patches_the_real_tree(self):
        rc, err = self.run_tool()
        self.assertEqual(rc, 0, err)
        style = self.read("NotificationStyle.qml")
        self.assertTrue(style.startswith("// Luma: patched"))
        self.assertNotRegex(style, r"root\.languageStyle\.(normal|cond)\d+\.fontFamily\[0\]")
        self.assertGreaterEqual(style.count('"Manrope"'), 6)
        toast = style[style.index("property QtObject toast: QtObject {"):]
        toast = toast[:toast.index("property real   activateScale")]
        self.assertIn(self.pal["sheetBottom"], toast)
        self.assertNotIn("#787F86", toast)
        self.assertIn("getRValue(%d)" % self.pal["rowRadius"], toast)
        for name in ("Toast.qml", "SimpleButton.qml", "PincodePromptNumButton.qml"):
            text = self.read(name)
            self.assertNotRegex(text, r"layer\.enabled:\s*(true|root\.activeFocus)\s*layer\.effect: DropShadow")
            self.assertRegex(text, r"layer\.enabled: false\s*layer\.effect: DropShadow")

    def test_changes_nothing_but_what_it_names(self):
        self.run_tool()
        for name in sorted(os.listdir(CENSUS)):
            p = os.path.join(CENSUS, name)
            if not os.path.isfile(p) or name in ("NotificationStyle.qml", "Toast.qml", "SimpleButton.qml", "PincodePromptNumButton.qml"):
                continue
            with open(p, "rb") as a, open(os.path.join(self.dir, name), "rb") as b:
                self.assertEqual(a.read(), b.read(), name)
        # the other objects of the style keep LG's colours: "#ffffff" and "#e6e6e6" are used outside the toast
        style = self.read("NotificationStyle.qml")
        before = open(os.path.join(CENSUS, "NotificationStyle.qml")).read()
        self.assertEqual(style.count("#e6e6e6") + 1, before.count("#e6e6e6"))
        self.assertEqual(style.count("#ffffff") + 1, before.count("#ffffff"))

    def test_the_shadow_blocks_stay_and_only_the_switch_changes(self):
        self.run_tool()
        before = open(os.path.join(CENSUS, "Toast.qml")).read()
        after = self.read("Toast.qml")
        self.assertEqual(len(after.splitlines()), len(before.splitlines()))
        self.assertEqual(after.replace("layer.enabled: false", "layer.enabled: true"), before)

    def test_a_missing_anchor_is_fatal_and_says_which(self):
        with open(os.path.join(self.dir, "NotificationStyle.qml")) as f:
            text = f.read()
        with open(os.path.join(self.dir, "NotificationStyle.qml"), "w") as f:
            f.write(text.replace('"#787F86"', '"#111111"'))
        rc, err = self.run_tool()
        self.assertEqual(rc, 1)
        self.assertIn("backgroundColor", err)

    def test_a_tree_with_no_shadow_to_switch_off_is_fatal(self):
        with open(os.path.join(self.dir, "Toast.qml")) as f:
            text = f.read()
        with open(os.path.join(self.dir, "Toast.qml"), "w") as f:
            f.write(re.sub(r"layer\.enabled:\s*true", "layer.enabled: false", text))
        rc, err = self.run_tool()
        self.assertEqual(rc, 1)
        self.assertIn("Toast.qml", err)

    def test_a_tree_whose_fonts_were_renamed_is_fatal(self):
        with open(os.path.join(self.dir, "NotificationStyle.qml")) as f:
            text = f.read()
        with open(os.path.join(self.dir, "NotificationStyle.qml"), "w") as f:
            f.write(text.replace("languageStyle", "langStyle"))
        rc, err = self.run_tool()
        self.assertEqual(rc, 1)
        self.assertIn("font sites", err)

    def test_patching_twice_is_refused(self):
        self.assertEqual(self.run_tool()[0], 0)
        rc, err = self.run_tool()
        self.assertEqual(rc, 1)
        self.assertIn("already patched", err)

    def test_a_palette_without_what_it_needs_is_refused(self):
        rc, err = self.run_tool({"ink": "#ffffff"})
        self.assertEqual(rc, 1)
        self.assertIn("sheetBottom", err)

    def test_the_patched_tree_still_has_the_files_the_view_imports(self):
        self.run_tool()
        for name in ("NotificationView.qml", "Services.qml", "NLocaleService/qmldir", "Utils.js"):
            self.assertTrue(os.path.exists(os.path.join(self.dir, name)), name)


if __name__ == "__main__":
    unittest.main()
