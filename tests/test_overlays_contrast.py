"""Run with: python3 -m unittest tests.test_overlays_contrast   (arithmetic only, no display)

Lucent section 3: contrast is read on the composited picture. The overlay cards (panels/OverlayCard.qml) are glass at `readingAlpha` over whatever the app
shows, so every colour a card draws is held to its ratio against the worst frame there can be under each material: white under the dark glass, black
under the light one, blended in the display's own (non-linear) values as the plane's blender does:

- the words in the full ink (a title, a score) read at the TV's distance contrast (7:1), the words in the second ink (a line, the source, the keys) at 4.5:1;
- the tile's glyph in each tone (the accent, the warning, the danger), which is a mark that carries meaning, at 3:1, with the pushed inks the card uses
  (`Lu.warningInk`, `Lu.dangerInk`, the light accent darkened) computed as Qt computes `Qt.lighter` and `Qt.darker`.
"""
import colorsys
import json
import os
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
with open(os.path.join(HERE, "..", "payload", "theme", "theme.json")) as f:
    THEME = json.load(f)

DISTANCE = 7.0
BODY = 4.5
CONTROL = 3.0
WORST = {"dark": (255, 255, 255), "light": (0, 0, 0)}


def rgb(role):
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
    return tuple(top[i] * alpha + below[i] * (1 - alpha) for i in range(3))


def lighter(c, f):
    """Qt.lighter: the value times the factor, and where it passes the top the saturation gives up what it passed by."""
    h, s, v = colorsys.rgb_to_hsv(*(x / 255 for x in c))
    v *= f
    if v > 1:
        s = max(0.0, s - (v - 1))
        v = 1.0
    return tuple(round(x * 255) for x in colorsys.hsv_to_rgb(h, s, v))


def darker(c, f):
    h, s, v = colorsys.rgb_to_hsv(*(x / 255 for x in c))
    return tuple(round(x * 255) for x in colorsys.hsv_to_rgb(h, s, v / f))


def pane(material):
    m = THEME[material]
    base, _ = rgb(m["glass"])
    return over(base, m["readingAlpha"], WORST[material])


def ink(material, role):
    m = THEME[material]
    c, a = rgb(m[role])
    return over(c, a, pane(material))


def tone_inks(material):
    m = THEME[material]
    light = material == "light"
    accent, _ = rgb(m["accent"])
    warning, _ = rgb(m["warning"])
    danger, _ = rgb(m["danger"])
    return {"accent": darker(accent, 1.6) if light else accent,
            "warning": darker(warning, 2.4) if light else lighter(warning, 1.1),
            "danger": darker(danger, 3.2) if light else lighter(danger, 1.25)}


class OverlayContrast(unittest.TestCase):
    def test_the_full_ink_reads_at_the_distance_contrast_on_the_worst_frame(self):
        for material in ("dark", "light"):
            self.assertGreaterEqual(ratio(ink(material, "ink"), pane(material)), DISTANCE, material)

    def test_the_second_ink_reads_at_the_body_contrast_on_the_worst_frame(self):
        for material in ("dark", "light"):
            k = ratio(ink(material, "ink2"), pane(material))
            self.assertGreaterEqual(k, BODY, "%s: %.2f" % (material, k))

    def test_the_third_ink_is_not_used_for_words_because_it_does_not_read(self):
        # the camera pop-up's hint was in it: the reason the cards (and the pop-up's hint) use the second
        for material in ("dark", "light"):
            self.assertLess(ratio(ink(material, "ink3"), pane(material)), BODY, material)

    def test_each_tones_glyph_reads_as_a_mark_that_carries_meaning(self):
        for material in ("dark", "light"):
            for tone, c in tone_inks(material).items():
                k = ratio(c, pane(material))
                self.assertGreaterEqual(k, CONTROL, "%s %s: %.2f" % (material, tone, k))

    def test_the_light_accent_unpushed_would_not_have_read(self):
        # why the card darkens it: the iris of the light palette is 2.4:1 on the glass over a black frame
        accent, _ = rgb(THEME["light"]["accent"])
        self.assertLess(ratio(accent, pane("light")), CONTROL)


if __name__ == "__main__":
    unittest.main()
