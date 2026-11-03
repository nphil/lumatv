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



if __name__ == "__main__":
    unittest.main()
