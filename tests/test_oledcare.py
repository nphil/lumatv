"""Run with: python3 -m unittest tests.test_oledcare   (from the repository root)

The OLED care's tokens (compositor/crates/lumatv-compositor/src/oledcare.rs, theme `oledcare`): the shipped theme matches the compositor's
own defaults, the care is off in what ships, the Settings row writes the switch, and the dim the compositor may apply keeps the palette's
text at 4.5 : 1 on its sheet, dark and light.
"""
import json
import os
import re
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
THEME = os.path.join(ROOT, "payload", "theme", "theme.json")
RUST = os.path.join(ROOT, "compositor", "crates", "lumatv-compositor", "src", "oledcare.rs")
LOOK = os.path.join(ROOT, "payload", "compositor", "lumatv", "settings", "PageLook.qml")


def read(path):
    with open(path) as f:
        return f.read()


def argb(h):
    h = h.lstrip("#")
    return (255, int(h[0:2], 16), int(h[2:4], 16), int(h[4:6], 16)) if len(h) == 6 else tuple(int(h[i:i + 2], 16) for i in (0, 2, 4, 6))


def over(top, bottom):
    a = top[0] / 255
    return tuple(round(top[i] * a + bottom[i] * (1 - a)) for i in (1, 2, 3))


def luminance(rgb):
    def lin(v):
        v /= 255
        return v / 12.92 if v <= 0.03928 else ((v + 0.055) / 1.055) ** 2.4
    r, g, b = map(lin, rgb)
    return 0.2126 * r + 0.7152 * g + 0.0722 * b


def contrast(a, b):
    la, lb = sorted((luminance(a), luminance(b)), reverse=True)
    return (la + 0.05) / (lb + 0.05)


class OledCare(unittest.TestCase):
    theme = json.loads(read(THEME))
    rust = read(RUST)

    def test_the_shipped_theme_is_off_and_says_what_the_compositor_defaults_to(self):
        care = self.theme["oledcare"]
        self.assertIs(care["enabled"], False, "off until the TV check has passed")
        m = re.search(r"Settings \{ enabled: false, radius: (\d+), step_ns: (\d+) \* SEC, dim_after_ns: (\d+) \* 60 \* SEC, "
                      r"dim_over_ns: (\d+) \* 60 \* SEC, dim_floor: ([0-9.]+) \}", self.rust)
        self.assertTrue(m, "the Default of Settings")
        radius, step, after, over_, floor = int(m[1]), int(m[2]), int(m[3]), int(m[4]), float(m[5])
        self.assertEqual((care["orbitPx"], care["stepSec"], care["dimAfterMin"], care["dimOverMin"], care["dimTo"]), (radius, step, after, over_, floor))

    def test_the_orbit_stays_within_a_few_pixels_and_a_step_is_slow(self):
        care = self.theme["oledcare"]
        self.assertTrue(2 <= care["orbitPx"] <= 4, "the ring's radius: 2 to 4 pixels")
        self.assertGreaterEqual(care["stepSec"], 10, "a pixel a beat, a beat every so many seconds: slow")

    def test_the_dim_floor_keeps_the_palettes_text_readable_dark_and_light(self):
        floor_min = float(re.search(r"MIN_DIM_TO: f32 = ([0-9.]+)", self.rust)[1])
        self.assertGreaterEqual(self.theme["oledcare"]["dimTo"], floor_min)
        for mode in ("dark", "light"):
            m = self.theme[mode]
            sheet = over(argb(m["sheet"]), argb(m["canvas"]))
            for role in ("ink", "ink2"):
                text = over(argb(m[role]), (255,) + sheet)
                for f in (floor_min, self.theme["oledcare"]["dimTo"]):
                    dim = lambda c: tuple(round(v * f) for v in c)
                    self.assertGreaterEqual(contrast(dim(text), dim(sheet)), 4.5, "%s %s at %.2f" % (mode, role, f))

    def test_the_settings_row_writes_the_switch_the_compositor_reads(self):
        text = read(LOOK)
        self.assertIn('label: "OLED care"', text)
        self.assertIn("page.patch({ oledcare: { enabled: v } })", text)
        self.assertIn("page.theme.oledcare.enabled === true", text)

    def test_the_compositor_reads_the_keys_the_theme_has(self):
        for key in self.theme["oledcare"]:
            self.assertIn('"%s"' % key, self.rust, "oledcare.%s is read by the compositor" % key)


if __name__ == "__main__":
    unittest.main()
