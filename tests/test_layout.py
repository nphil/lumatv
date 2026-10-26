"""Run with: python3 -m unittest tests.test_layout   (needs Xvfb and PyQt5; skipped without them)

The layout audit (tests/qmlhost/layoutaudit.py): first that it sees each kind of fault on a small surface built for it,
and that a clean pill, row and card pass; then that LumaTV's own surfaces pass it, one scene per kind of screen
(`python3 tests/qmlhost/scenes.py --audit` runs every scene). A finding reads as `[layout] kind: item (why)`.
"""
import os
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(HERE, "qmlhost"))
from xdisplay import DISPLAY, HAVE_QT  # noqa: E402

FONTS = os.path.join(os.path.dirname(HERE), "payload", "fonts")

PILL = """
import QtQuick 2.4
Item {
    width: 800; height: 400
    // %(title)s
    Item {
        id: pill; objectName: "pill"
        x: 20; y: 20; width: %(pillW)s; height: 48
        Rectangle { anchors.fill: parent; radius: 24; color: "#30ffffff" }
        Text { objectName: "pillText"; anchors.centerIn: parent; text: "%(text)s"; color: "white"; font.family: "Manrope"; font.pixelSize: 24
               lineHeight: %(line)s; lineHeightMode: Text.ProportionalHeight; renderType: Text.NativeRendering; %(extra)s }
    }
    Item {
        id: row; objectName: "row"
        x: 20; y: 120; width: 600; height: 80; scale: %(scale)s
        Rectangle { anchors.fill: parent; radius: 22; color: "#20ffffff" }
        Item { objectName: "icon"; x: 24; y: %(iconY)s; width: 48; height: 48; Rectangle { anchors.fill: parent; radius: 14; color: "#40ffffff" } }
        Text { objectName: "rowText"; x: 92; y: 22; width: 300; elide: Text.ElideRight; text: "%(rowText)s"; color: "white"; font.family: "Manrope"; font.pixelSize: 26; renderType: Text.NativeRendering }
        Text { objectName: "other"; x: %(otherX)s; y: 22; text: "Other"; color: "white"; font.family: "Manrope"; font.pixelSize: 26; renderType: Text.NativeRendering }
    }
}
"""

CLEAN = {"title": "clean", "text": "Standard", "line": "1.0", "extra": "", "scale": "1", "iconY": "16", "rowText": "A short label", "otherX": "500", "pillW": "200"}


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class Rules(unittest.TestCase):
    """The audit on a surface built to show each fault."""

    @classmethod
    def setUpClass(cls):
        os.environ["DISPLAY"] = DISPLAY
        os.environ.setdefault("QT_QPA_PLATFORM", "xcb")
        os.environ.setdefault("LIBGL_ALWAYS_SOFTWARE", "1")
        from PyQt5 import QtCore, QtGui, QtQuick  # noqa: F401
        cls.app = QtGui.QGuiApplication.instance() or QtGui.QGuiApplication([])
        for n in sorted(os.listdir(FONTS)):
            if n.endswith(".ttf"):
                QtGui.QFontDatabase.addApplicationFont(os.path.join(FONTS, n))

    def findings(self, **over):
        from PyQt5 import QtCore, QtQuick
        import layoutaudit
        spec = dict(CLEAN, **over)
        d = tempfile.mkdtemp(prefix="layout-")
        path = os.path.join(d, "Surface.qml")
        with open(path, "w") as f:
            f.write(PILL % spec)
        view = QtQuick.QQuickView()
        view.setSource(QtCore.QUrl.fromLocalFile(path))
        self.assertEqual(view.status(), QtQuick.QQuickView.Ready, [e.toString() for e in view.errors()])
        view.show()
        deadline = QtCore.QDeadlineTimer(400)
        while not deadline.hasExpired():
            self.app.processEvents()
        found = layoutaudit.audit(view.rootObject(), 800, 400)
        view.hide()
        return {(f["kind"], f["item"].split("/")[-1].split("[")[0]) for f in found}, found      # (kind, the item's objectName or type)

    def test_a_clean_pill_row_and_icon_pass(self):
        kinds, found = self.findings()
        self.assertEqual(kinds, set(), found)

    def test_a_word_sitting_high_in_its_pill_is_off_centre(self):
        # the leading of lineHeight 1.3 goes under the glyphs: a centred Text item has its words above the middle
        kinds, found = self.findings(line="1.3")
        self.assertIn(("off-centre", "pill"), kinds, found)

    def test_a_word_wider_than_its_pill_is_outside_and_one_nearly_so_is_tight(self):
        # "Standard" at 24 px is about 106 px wide
        kinds, _ = self.findings(pillW="80")
        self.assertIn(("outside", "pillText"), kinds)
        kinds, _ = self.findings(pillW="114")
        self.assertIn(("tight", "pillText"), kinds)
        self.assertNotIn(("outside", "pillText"), kinds)

    def test_a_cut_word_is_elided(self):
        kinds, _ = self.findings(rowText="A label much longer than the three hundred pixels it is given")
        self.assertIn(("elided", "rowText"), kinds)

    def test_words_over_words_overlap(self):
        kinds, _ = self.findings(otherX="100")
        self.assertIn(("overlap", "rowText"), kinds)

    def test_words_inside_a_scaled_item_are_reported(self):
        kinds, _ = self.findings(scale="1.02")
        self.assertIn(("scaled", "rowText"), kinds)

    def test_an_icon_not_level_with_its_words_is_reported(self):
        kinds, _ = self.findings(iconY="4")
        self.assertIn(("icon-level", "row"), kinds)


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class Surfaces(unittest.TestCase):
    """LumaTV's own screens pass the audit: one scene per kind of screen, in the dark material (the light one differs in
    colour only). Every scene, both materials: python3 tests/qmlhost/scenes.py --audit"""

    SCENES = ["01-settings-picture", "02-settings-sound-mode", "30-settings-system", "03-settings-remote-focused", "10-quick-panel",
              "08-input-picker", "16-context-menu", "06-confirm-dialog", "13-overlay-drawer", "12-all-apps", "09-search-keyboard",
              "34-screen-remote", "25-input-banner", "05-ai-request-prompt"]

    def test_every_kind_of_screen_passes_the_audit(self):
        import scenes
        scenes.load_modules()
        self.assertEqual(scenes.LOAD_ERRORS, [])
        out = tempfile.mkdtemp(prefix="layout-scenes-")
        bad = {}
        for name in self.SCENES:
            rc, warn, text = scenes.run_scene(name, out, "dark", audit=True)
            layout = [w for w in warn if w.startswith("[layout")]
            if rc != 0 or layout:
                bad[name] = layout or warn or text[-800:]
        self.assertEqual(bad, {})


if __name__ == "__main__":
    unittest.main()
