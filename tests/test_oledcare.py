"""Run with: python3 -m unittest tests.test_oledcare   (from the repository root)

The OLED care's tokens (compositor/crates/lumatv-compositor/src/oledcare.rs, theme `oledcare`): the shipped theme matches the compositor's
own defaults, every care is off in what ships but the panel dim, the six Settings rows (System > OLED care > LumaTV panel care) write the switches, and the
dim the compositor may apply to LumaTV's own screens keeps the palette's text at 4.5 : 1 on its sheet, dark and light.
"""
import json
import os
import re
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
THEME = os.path.join(ROOT, "payload", "theme", "theme.json")
RUST = os.path.join(ROOT, "compositor", "crates", "lumatv-compositor", "src", "oledcare.rs")
BOXDIM = os.path.join(ROOT, "compositor", "crates", "lumatv-compositor", "src", "boxdim.rs")
PANELDIM = os.path.join(ROOT, "compositor", "crates", "lumatv-compositor", "src", "paneldim.rs")
SETTINGS = os.path.join(ROOT, "payload", "compositor", "lumatv", "settings")
LOOK = os.path.join(SETTINGS, "PageLook.qml")
SYSTEM = os.path.join(SETTINGS, "PageSystem.qml")
SRC = os.path.join(ROOT, "compositor", "crates", "lumatv-compositor", "src")
HELPER = os.path.join(ROOT, "helper", "app", "src")

# The switches of the care, in the order of the group, with what the Settings row is called.
SWITCHES = [("enabled", "Protect LumaTV screens"), ("appDim", "Dim still apps"), ("appShift", "Shift still apps"), ("boxDim", "Dim bright still areas"),
            ("panelDim", "Dim with the panel"), ("refreshGuard", "Protect panel refresh")]
# The switch that is on in what ships: it fails safe (the GPU dim and the saver's own dim take over when the panel cannot be dimmed).
ON_BY_DEFAULT = {"panelDim"}


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
        for key, _ in SWITCHES:
            self.assertIs(care[key], key in ON_BY_DEFAULT, "%s: off until its TV check has passed (the panel dim fails safe and is on)" % key)
        m = re.search(r"Settings \{\s*enabled: false,\s*radius: (\d+),\s*step_ns: (\d+) \* SEC,\s*dim_after_ns: (\d+) \* 60 \* SEC,\s*"
                      r"dim_over_ns: (\d+) \* 60 \* SEC,\s*dim_floor: ([0-9.]+),\s*app_dim: false,\s*app_shift: false,\s*app_after_ns: (\d+) \* 60 \* SEC,\s*"
                      r"app_floor: ([0-9.]+),", self.rust)
        self.assertTrue(m, "the Default of Settings")
        radius, step, after, over_, floor, app_after, app_floor = int(m[1]), int(m[2]), int(m[3]), int(m[4]), float(m[5]), int(m[6]), float(m[7])
        self.assertEqual((care["orbitPx"], care["stepSec"], care["dimAfterMin"], care["dimOverMin"], care["dimTo"], care["appDimAfterMin"], care["appDimTo"]),
                         (radius, step, after, over_, floor, app_after, app_floor))

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

    def test_the_apps_dim_floor_is_the_contracts_and_the_wait_is_a_few_minutes(self):
        floor_min = float(re.search(r"MIN_APP_DIM_TO: f32 = ([0-9.]+)", self.rust)[1])
        self.assertGreaterEqual(floor_min, 0.7, "an app is never dimmed below 70 %")
        self.assertGreaterEqual(self.theme["oledcare"]["appDimTo"], floor_min)
        self.assertTrue(1 <= self.theme["oledcare"]["appDimAfterMin"] <= 10)

    def test_the_box_dims_keys_say_what_its_module_defaults_to_and_its_floor_is_modest(self):
        care = self.theme["oledcare"]
        rust = read(BOXDIM)
        m = re.search(r"Settings \{ on: false, after_ns: (\d+) \* 60 \* SEC, ramp_ns: (\d+) \* SEC, floor: ([0-9.]+), min_luma: ([0-9.]+) \}", rust)
        self.assertTrue(m, "the Default of boxdim::Settings")
        self.assertEqual((care["boxDimAfterMin"], care["boxDimRampSec"], care["boxDimTo"], care["boxDimMinLuma"]),
                         (int(m[1]), int(m[2]), float(m[3]), float(m[4])))
        floor_min = float(re.search(r"MIN_FLOOR: f32 = ([0-9.]+)", rust)[1])
        self.assertGreaterEqual(floor_min, 0.75, "a bright box is never dimmed below 75 %")
        self.assertGreaterEqual(care["boxDimTo"], floor_min)
        self.assertTrue(1 <= care["boxDimAfterMin"] <= 10)
        self.assertGreaterEqual(care["boxDimRampSec"], 10, "the dim fades in over tens of seconds")
        for key in ("boxDim", "boxDimAfterMin", "boxDimRampSec", "boxDimTo", "boxDimMinLuma"):
            self.assertIn('"%s"' % key, rust, "oledcare.%s is read by boxdim.rs" % key)

    def test_the_six_settings_rows_sit_in_system_oled_care_in_order_and_write_the_switches_the_compositor_reads(self):
        text = read(SYSTEM)
        self.assertIn('R.head("LumaTV panel care", 2)', text)
        self.assertIn("page.patch({ oledcare: o })", text, "one object, one key")
        self.assertIn("typeof v === \"boolean\" ? v : !!dflt", text, "a key the theme does not have is the row's default")
        at = text.index('R.head("LumaTV panel care", 2)')
        rows = re.findall(r'page\.careRow\("(\w+)", "([\w-]+)", "([^"]+)", "([^"]+)"(, true)?\)', text[at:])
        self.assertEqual([(k, label) for k, _, label, _, _ in rows], SWITCHES, "the six toggles, in this order, under LG's rows")
        self.assertEqual({k for k, _, _, _, dflt in rows if dflt}, ON_BY_DEFAULT, "the rows that show on for a key the theme lacks are the compositor's defaults")
        for key, icon, label, help_, _ in rows:
            self.assertIn(key, self.theme["oledcare"], "%s has its default in the shipped theme" % key)
            self.assertTrue(help_.endswith("."), label + " has a one-line help")
            self.assertLess(len(help_), 130, label)
            self.assertTrue(os.path.exists(os.path.join(SETTINGS, "..", "icons", "lucide", icon + ".png")), icon)
        # LG's own rows (Screen shift, Logo Brightness, Pixel Cleaning) come first on the tab: the group is under them
        for lg in ("picture.screenShift", "picture.logoLuminanceAdjust", "picture.scheduledPixelCleaning"):
            self.assertLess(text.index(lg, text.index("rows: [")), at, lg)

    def test_the_panel_dims_keys_say_what_its_module_defaults_to_and_the_floor_leaves_a_clock_to_read(self):
        care = self.theme["oledcare"]
        rust = read(PANELDIM)
        m = re.search(r"Settings \{ enabled: (true|false), step_ns: (\d+) \* SEC \}", rust)
        self.assertTrue(m, "the Default of paneldim::Settings")
        self.assertEqual((care["panelDim"], care["panelDimStepSec"]), (m[1] == "true", int(m[2])))
        self.assertTrue(1 <= care["panelDimStepSec"] <= 60)
        for key in ("panelDim", "panelDimStepSec"):
            self.assertIn('"%s"' % key, rust, "oledcare.%s is read by paneldim.rs" % key)
        floor = int(re.search(r"pub const FLOOR: u32 = (\d+);", rust)[1])
        self.assertTrue(5 <= floor <= 30, "the panel is never taken to a dark screen, nor kept at full light")
        # the Rust Settings page and the QML one say the same about what is on without a key in the theme
        system_rs = read(os.path.join(ROOT, "rustui", "ui", "src", "settings", "pages", "system.rs"))
        self.assertIn('CARE_ON_BY_DEFAULT: &[&str] = &["panelDim"]', system_rs)

    def test_the_row_for_lumatvs_screens_is_not_left_behind_on_the_screensaver_tab(self):
        text = read(LOOK)
        self.assertNotIn("oledcare", text)
        self.assertNotIn('label: "OLED care"', text)

    def test_the_compositor_or_the_helper_reads_every_key_the_theme_has(self):
        sources = ""
        for d in (SRC, HELPER):
            for root, _, files in os.walk(d):
                for f in files:
                    if f.endswith(".rs"):
                        sources += read(os.path.join(root, f))
        for key in self.theme["oledcare"]:
            self.assertIn('"%s"' % key, sources, "oledcare.%s is read by the compositor or the helper" % key)


if __name__ == "__main__":
    unittest.main()
