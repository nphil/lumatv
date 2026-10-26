"""Run with: python3 -m unittest tests.test_osd_contrast   (the pixel half needs Xvfb and PyQt5; skipped without them)

Lucent section 3: contrast is read on the composited picture, and an overlay over video that is less opaque than the reading floor (.96) has to
show it. The volume display's glass is painted at `readingAlpha` (theme.json, 0.8) where it is over an app, so the picture shows through it; this
holds that number to the TV's distance contrast (7:1, `contrast.distance`) against the worst frame there can be:

- the arithmetic: the ink over the glass over a frame that is white under the dark glass and black under the light one, blended in the display's own
  (non-linear) values as the plane's blender does, and the bar against the track it fills;
- the pixels: the display drawn by the offline host over such a frame, and every run of words and every glyph of it measured on the picture, as
  the language measures (the background is the median of the box, the ink the pixel farthest from it): the level, the format's mark, the glyph of the
  muted pane and of the mark that stays;
- the layout: the host's layout audit on the display over a picture, at the normal type size and doubled.
"""
import json
import os
import re
import sys
import tempfile
import unittest

from xdisplay import DISPLAY, HAVE_QT

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path.insert(0, os.path.join(HERE, "qmlhost"))
with open(os.path.join(ROOT, "payload", "theme", "theme.json")) as f:
    THEME = json.load(f)

DISTANCE = 7.0          # contrast.distance: TV and glance text
CONTROL = 3.0           # contrast.control: a mark that carries meaning
# the worst frame under each material's glass: the one whose luminance is closest to the ink's
WORST = {"dark": (255, 255, 255), "light": (0, 0, 0)}


def rgb(role):
    """(r, g, b) 0..255 and alpha 0..1 of a theme colour, "#RRGGBB" or "#AARRGGBB"."""
    h = role.lstrip("#")
    a = int(h[:2], 16) / 255 if len(h) == 8 else 1.0
    h = h[-6:]
    return tuple(int(h[i:i + 2], 16) for i in (0, 2, 4)), a


def lin(c):
    c /= 255
    return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4


def luminance(c):
    return 0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2])


def ratio(a, b):
    ya, yb = luminance(a), luminance(b)
    return (max(ya, yb) + 0.05) / (min(ya, yb) + 0.05)


def over(top, alpha, below):
    """`top` at `alpha` over `below`, in the values the plane blends (not in linear light)."""
    return tuple(top[i] * alpha + below[i] * (1 - alpha) for i in range(3))


def glass_over_worst(material, alpha=None):
    m = THEME[material]
    base, _ = rgb(m["glass"])             # the flat colour the overlay's glass samples over video: the glass role's own colour, opaque
    return over(base, m["readingAlpha"] if alpha is None else alpha, WORST[material])


class Arithmetic(unittest.TestCase):
    def test_the_ink_reads_at_the_distance_contrast_on_the_worst_frame_in_both_materials(self):
        for material in ("dark", "light"):
            ink, _ = rgb(THEME[material]["ink"])
            self.assertGreaterEqual(ratio(ink, glass_over_worst(material)), DISTANCE, material)

    def test_it_is_less_opaque_than_the_overlay_floor_and_not_so_low_that_the_worst_frame_is_close_to_failing(self):
        for material in ("dark", "light"):
            alpha = THEME[material]["readingAlpha"]
            self.assertLess(alpha, rgb(THEME[material]["sheet"])[1], "less opaque than Lucent's overlay (.96): the ask")
            ink, _ = rgb(THEME[material]["ink"])
            lowest = min(a / 100 for a in range(1, 101) if ratio(ink, glass_over_worst(material, a / 100)) >= DISTANCE)
            self.assertGreater(alpha, lowest + 0.03, "%s: 7:1 holds down to %.2f; the token is %.2f, which leaves the display no margin for the plane's blend" % (material, lowest, alpha))

    def test_the_bar_reads_against_the_track_it_fills(self):
        for material in ("dark", "light"):
            m = THEME[material]
            bg = glass_over_worst(material)
            track_ink, track_alpha = rgb(m["trackOff"])
            track = over(track_ink, track_alpha, bg)
            fill, _ = rgb(m["ink"])
            self.assertGreaterEqual(ratio(fill, track), CONTROL, material + ": the level against the empty part")
            self.assertGreaterEqual(ratio(fill, bg), CONTROL, material + ": the level against the glass")


def measure(image, x, y, w, h):
    """(background, ink) of a box of the picture: the median of its pixels, and the pixel farthest from it in luminance."""
    px = []
    for j in range(max(0, int(y)), min(image.height(), int(y + h))):
        for i in range(max(0, int(x)), min(image.width(), int(x + w))):
            c = image.pixelColor(i, j)
            px.append((c.red(), c.green(), c.blue()))
    ys = sorted(luminance(c) for c in px)
    median = ys[len(ys) // 2]
    bg = min(px, key=lambda c: abs(luminance(c) - median))
    ink = max(px, key=lambda c: abs(luminance(c) - luminance(bg)))
    return bg, ink


GEOM = ("(function(){ var f=function(p){if(p.staleMs!==undefined)return p;for(var i=0;i<p.children.length;i++){var r=f(p.children[i]);if(r)return r}return null}; "
        "var o=f(overlay); var out=[]; var walk=function(it){ for (var i=0;i<it.children.length;i++){ var c=it.children[i]; var p=c.mapToItem(null,0,0); "
        "var rec={x:p.x,y:p.y,w:c.width,h:c.height,vis:c.visible}; "
        "if (c.text!==undefined) { rec.text=String(c.text) } else if (c.radius!==undefined && c.color!==undefined && c.height===8) { rec.bar=true } "
        "else if (c.aspect!==undefined) { rec.logo=String(c.label) } "
        "else if (c.name!==undefined && c.size!==undefined) { rec.icon=String(c.name) } else if (c.accent!==undefined && c.icon!==undefined) { rec.tile=String(c.icon) } "
        "out.push(rec); walk(c) } }; walk(o); console.info('[geom:%s] '+JSON.stringify(out)); return out.length })()")


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class OnThePicture(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        from PyQt5 import QtGui
        cls.QtGui = QtGui
        cls.tmp = tempfile.TemporaryDirectory()

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def frame(self, colour):
        path = os.path.join(self.tmp.name, "frame-%02x%02x%02x.png" % colour)
        image = self.QtGui.QImage(64, 36, self.QtGui.QImage.Format_RGB32)
        image.fill(self.QtGui.QColor(*colour))
        image.save(path)
        return path

    def run_material(self, material, receiver):
        import scenes_volume as sv
        name = "contrast"
        b = 1500                # the helper is up and has synced from here on (tests/test_volume_osd.py)
        steps = [sv.sound_out(b), sv.receiver_at(b + 200, receiver), sv.osd_at(b + 300, "o.holdMs = 1500"), sv.key_at(b + 2000, "volumeUp", 32),
                 (b + 2600, GEOM % "a"),
                 sv.receiver_at(b + 4000, dict(receiver, mute=True)), (b + 4900, GEOM % "b"),
                 (b + 7000, GEOM % "c")]
        shots = {s: os.path.join(self.tmp.name, "%s-%s.png" % (material, s)) for s in "bc"}
        rc, warn, text = sv.render(name, self.tmp.name, material, steps, b + 2700, bg=self.frame(WORST[material]),
                                   more_snaps=[(b + 5000, shots["b"]), (b + 7100, shots["c"])])
        ours = [w for w in warn if re.search(r"VolumeOsd|TvVolume|LumaOverlay|Glass\.qml|LuPane|kit/Lu\.qml|ReceiverGate|overlay volume", w)]
        self.assertEqual(ours, [], text[-1500:])           # a warning from another surface is work in progress elsewhere in the tree
        self.assertTrue(os.path.exists(sv.png_path(self.tmp.name, name, material)), text[-1500:])
        shots["a"] = sv.png_path(self.tmp.name, name, material)
        geom = {m.group(1): json.loads(m.group(2)) for m in re.finditer(r"\[geom:(\w)\] (.*)", text)}
        self.assertEqual(sorted(geom), ["a", "b", "c"], text[-1500:])
        return {s: (self.QtGui.QImage(shots[s]), [g for g in geom[s] if g["vis"]]) for s in "abc"}

    def ink_ratio(self, image, item):
        bg, ink = measure(image, item["x"], item["y"], item["w"], item["h"])
        return ratio(ink, bg)

    def words(self, state, text):
        image, items = state
        found = [g for g in items if g.get("text") == text and g["w"] > 0]
        self.assertEqual(len(found), 1, "%r on screen: %r" % (text, [g.get("text") for g in items if "text" in g]))
        return image, found[0]

    def check(self, material, receiver, mark):
        shown = self.run_material(material, receiver)
        image, item = self.words(shown["a"], "47")
        self.assertGreaterEqual(self.ink_ratio(image, item), DISTANCE, "%s, the level on the worst frame" % material)
        # the format's mark, in the bar's ink: its thinnest strokes included
        image, items = shown["a"]
        marks = [g for g in items if g.get("logo") == mark and g["w"] > 0]
        self.assertEqual(len(marks), 1, "%r on screen: %r" % (mark, [g.get("logo") for g in items if "logo" in g]))
        self.assertGreaterEqual(self.ink_ratio(image, marks[0]), DISTANCE, "%s, the %s mark on the worst frame" % (material, mark))
        # the slashed speaker: where the number was in the muted bar, and alone on the mark that stays
        for state, what in (("b", "the muted bar's"), ("c", "the mark's")):
            image, items = shown[state]
            glyphs = [g for g in items if "icon" in g and g["w"] > 0]
            self.assertTrue(glyphs, what + " glyph")
            self.assertGreaterEqual(self.ink_ratio(image, glyphs[0]), DISTANCE, "%s, %s glyph on the worst frame" % (material, what))
        # the level against the empty end of its bar, and against the glass above it, as drawn
        image, items = shown["a"]
        bars = sorted((g for g in items if g.get("bar")), key=lambda g: g["w"])
        self.assertEqual(len(bars), 2, bars)
        fill, well = bars
        px = lambda x, y: (lambda c: (c.red(), c.green(), c.blue()))(image.pixelColor(int(x), int(y)))
        mid, along = fill["y"] + fill["h"] / 2, fill["x"] + fill["w"] / 2
        self.assertGreaterEqual(ratio(px(along, mid), px(well["x"] + well["w"] - 8, mid)), CONTROL, material + ": the level against the empty end")
        self.assertGreaterEqual(ratio(px(along, mid), px(along, fill["y"] - 10)), CONTROL, material + ": the level against the glass")

    def test_the_dark_glass_over_a_white_frame(self):
        import scenes_volume as sv
        self.check("dark", sv.RX_FMT, "Dolby Digital+ \u00b7 Surround")

    def test_the_light_glass_over_a_black_frame(self):
        import scenes_volume as sv
        self.check("light", sv.RX_FMT, "Dolby Digital+ \u00b7 Surround")

    def test_the_atmos_mark_on_the_dark_glass_over_a_white_frame(self):
        import scenes_volume as sv
        self.check("dark", sv.RX_ATMOS, "Dolby Atmos")

    def test_the_dtsx_mark_on_the_light_glass_over_a_black_frame(self):
        import scenes_volume as sv
        self.check("light", sv.RX_DTSX, "DTS:X")


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class Layout(unittest.TestCase):
    """The layout audit (tests/qmlhost/layoutaudit.py) on the display over a picture: no words cut, crowded against the pane's edge or on top of each other
    at the normal type size, none overlapping when the type is doubled, and the ink reading on what is behind it."""
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def audit(self, name, material, steps, snap_at):
        import scenes_volume as sv
        rc, warn, text = sv.render(name, self.tmp.name, material, steps, snap_at, audit=True)
        found = [w for w in warn if w.startswith("[layout") or w.startswith("[audit]")]
        self.assertEqual(rc, 0, text[-1500:])
        self.assertEqual(found, [], text[-1500:])

    def test_the_widest_number_and_the_longest_format_in_the_dark_bar(self):
        import scenes_volume as sv
        b = sv.B
        wide = dict(sv.RX_DB, soundMode="DOLBY AUDIO-DD+DSUR", signal="DOLBY D+ 5.1")
        self.audit("audit-db", "dark", [sv.sound_out(b), sv.receiver_at(b + 200, wide), sv.key_at(b + 2000, "volumeUp", 32)], b + 2700)

    def test_the_muted_pane_in_the_light_material(self):
        import scenes_volume as sv
        b = sv.B
        self.audit("audit-muted", "light", [sv.sound_out(b), sv.receiver_at(b + 200, sv.RX), sv.receiver_at(b + 2000, dict(sv.RX, mute=True)),
                                            sv.key_at(b + 2020, "mute", None, True)], b + 3000)

    def test_the_mark_that_stays_while_muted(self):
        import scenes_volume as sv
        b = sv.B
        self.audit("audit-mark", "dark", [sv.sound_out(b), sv.receiver_at(b + 200, dict(sv.RX, mute=True)), sv.osd_at(b + 300, "o.holdMs = 300"),
                                          sv.key_at(b + 2000, "mute", None, True)], b + 4500)


if __name__ == "__main__":
    unittest.main()
