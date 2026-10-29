"""Run with: python3 -m unittest tests.test_pointer_hover   (needs Xvfb and PyQt5; skipped without them)

The Magic Remote's pointer through the offline host. Hovering only shows a cue (the kit's `hovered`, a tile's partial highlight): it
never moves the focus, never opens a section or a tab, and the cue is gone at the next key press, when the D-pad's focus is what
shows. A click focuses and acts, as on LG's own Home.
"""
import json
import os
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from test_settings import HostCase, OPEN_AT, READY  # noqa: E402
from xdisplay import DISPLAY, HAVE_QT  # noqa: E402

JS = """
function walk(it, f) { var c = it.children; for (var i = 0; i < c.length; i++) { f(c[i]); walk(c[i], f) } }
function settings() { var s = null; walk(root, function(k) { if (k.railIndex !== undefined && k.sections !== undefined) s = k }); return s }
function railRows() {
    var o = [];
    walk(root, function(k) {
        if (k.tileSize !== undefined && k.pointerFocus !== undefined && k.icon !== undefined && k.visible && k.width > 300) {
            var p = k.mapToItem(null, 0, 0); if (p.x > 0 && p.x < 520) o.push({ k: k, y: p.y, x: p.x })
        }
    });
    o.sort(function(a, b) { return a.y - b.y });
    return o
}
function railAt(i, dx) { var r = railRows()[i]; return [r.x + r.k.width / 2 + (dx || 0), r.y + r.k.height / 2] }
function tabs() { var t = null; walk(root, function(k) { if (k.options !== undefined && k.fit > 1000 && k.pointerFocus !== undefined && k.visible) t = k }); return t }
function tabAt(i) { var t = tabs(), p = t.mapToItem(null, 0, 0); return [p.x + (i + 0.5) * t.segmentWidth, p.y + t.height / 2] }
function tiles() {
    var o = [];
    walk(root, function(k) { if (k.launchPoint !== undefined && k.hl !== undefined && k.visible) { var p = k.mapToItem(null, 0, 0); o.push({ k: k, x: p.x, y: p.y }) } });
    o.sort(function(a, b) { return a.y - b.y || a.x - b.x });
    return o
}
function tileAt(i) { var t = tiles()[i]; return [t.x + t.k.width / 2, t.y + t.k.height / 2] }
"""
DOWN = "root.key(Qt.Key_Down, true, false, 0); root.key(Qt.Key_Down, false, false, 0)"


def call(expr):
    return "(function() { %s; return %s })()" % (JS, expr)


def pointer(at, kind, expr):
    return ["--pointer", "%d:%s:%s" % (at, kind, call(expr))]


def probe(at, label, expr):
    return ["--eval", "%d:%s" % (at, call("JSON.stringify(Object.assign({ label: '%s' }, %s))" % (label, expr)))]


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class PointerHover(HostCase):
    def run_home(self, *args):
        rc, out = self.host("--eval", "%d:root.openSettings()" % OPEN_AT, *args, "--wait", str(READY + 4200))
        self.assertEqual(rc, 0, out[-1500:])
        return {d["label"]: d for d in self.docs(out) if "label" in d}

    RAIL = "{ rail: settings().railIndex, zone: settings().zone, hovered: railRows().map(function(r) { return r.k.hovered }) }"

    def test_hovering_the_rail_shows_a_cue_and_only_a_click_opens_a_section(self):
        t = READY + 600
        got = self.run_home(
            *pointer(t, "move", "railAt(3)"), *probe(t + 300, "hover", self.RAIL),
            *self.keys(t + 600, "down"), *probe(t + 900, "key", self.RAIL),
            *pointer(t + 1200, "move", "railAt(3)"), *probe(t + 1500, "same", self.RAIL),
            *pointer(t + 1800, "move", "railAt(3, 3)"), *probe(t + 2100, "moved", self.RAIL),
            *pointer(t + 2400, "click", "railAt(3)"), *probe(t + 2700, "click", self.RAIL))
        self.assertEqual((got["hover"]["rail"], got["hover"]["zone"]), (0, "rail"), "hovering a section does not open it")
        self.assertEqual([i for i, h in enumerate(got["hover"]["hovered"]) if h], [3], "the row under the pointer shows the cue")
        self.assertEqual(got["key"]["rail"], 1, "the D-pad moves from where it was, not from where the pointer is")
        self.assertFalse(any(got["key"]["hovered"]), "a key press hides the pointer's cue")
        self.assertFalse(any(got["same"]["hovered"]), "a pointer that has not moved does not bring the cue back")
        self.assertEqual([i for i, h in enumerate(got["moved"]["hovered"]) if h], [3], "moving it does")
        self.assertEqual(got["moved"]["rail"], 1)
        self.assertEqual((got["click"]["rail"], got["click"]["zone"]), (3, "page"), "a click opens the section")

    def test_hovering_a_tab_does_not_switch_it_and_a_click_does(self):
        t = READY + 600
        tab = "{ tab: tabs().current, hovered: tabs().pointerFocus !== undefined }"
        got = self.run_home(
            *pointer(t, "click", "railAt(5)"),                                       # Home & look, which has tabs
            *probe(t + 400, "before", tab),
            *pointer(t + 800, "move", "tabAt(2)"), *probe(t + 1100, "hover", tab),
            *pointer(t + 1400, "click", "tabAt(2)"), *probe(t + 1700, "click", tab))
        self.assertEqual(got["hover"]["tab"], got["before"]["tab"], "hovering a tab leaves the page where it is")
        self.assertEqual(got["click"]["tab"], 2, "a click on it opens it")

    def test_hovering_a_dock_tile_lights_it_faintly_and_moves_no_focus(self):
        t = READY + 600
        state = "{ layer: root.focusLayer, row: root.dockRow, col: root.dockCol, hl: tiles()[2].k.hl, own: tiles()[2].k.focused }"
        rc, out = self.host(*probe(t - 200, "before", state), *pointer(t, "move", "tileAt(2)"), *probe(t + 300, "hover", state),
                            *self.keys(t + 600, "right"), *probe(t + 900, "key", state), "--wait", str(READY + 2000))
        self.assertEqual(rc, 0, out[-1500:])
        got = {d["label"]: d for d in self.docs(out) if "label" in d}
        self.assertEqual((got["hover"]["layer"], got["hover"]["own"]), (got["before"]["layer"], False), "the layer's focus stays where it was")
        self.assertGreater(got["hover"]["hl"], 0, "the pointer shows a part of the highlight")
        self.assertLess(got["hover"]["hl"], 1, "not all of it: that is focus")
        self.assertEqual((got["hover"]["row"], got["hover"]["col"]), (got["before"]["row"], got["before"]["col"]), "the dock's focus did not follow the pointer")
        self.assertEqual(got["key"]["hl"], 0, "a key press hides the cue")


if __name__ == "__main__":
    unittest.main()
