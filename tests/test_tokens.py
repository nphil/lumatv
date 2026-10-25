"""Run with: python3 -m unittest discover -s tests -p 'test_*.py'

tools/tokens.py and tools/theme.py: the merge, the resolved menu palette, the accent modes, the
contrast guard and the generated CSS and Tokens.js.
"""
import json
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
SHIPPED = os.path.join(ROOT, "payload", "theme", "theme.json")
TOKENS = os.path.join(ROOT, "tools", "tokens.py")
THEME = os.path.join(ROOT, "tools", "theme.py")


def run(tool, *args):
    return subprocess.run([sys.executable, tool, *args], capture_output=True, text=True, check=True).stdout


class Tokens(unittest.TestCase):
    def write(self, d, name, obj):
        path = os.path.join(d, name)
        with open(path, "w") as f:
            json.dump(obj, f)
        return path

    def test_menu_palette_of_the_shipped_theme(self):
        menu = json.loads(run(TOKENS, SHIPPED, SHIPPED, "qml"))
        self.assertEqual(menu["accent"], "#FFA79BF5")
        self.assertEqual((menu["radius"], menu["rowRadius"]), (34, 22))
        self.assertEqual(menu["sheetBottom"], "#ED120E1A")
        self.assertEqual(menu["danger"], "#FFEB6F92")        # the staged notification view's destructive colour
        self.assertEqual(menu["accentInk"], "#FF17131F")     # the ink that reads on an accent fill

    def test_a_light_sheet_has_a_dark_hairline_and_the_shipped_dark_one_keeps_its_white_one(self):
        with tempfile.TemporaryDirectory() as d:
            user = self.write(d, "theme.json", {"dark": {"glass": "#FFFAF3", "ink": "#17131F"}})
            self.assertEqual(json.loads(run(TOKENS, SHIPPED, user, "qml"))["hairline"], "#1F000000")
        self.assertEqual(json.loads(run(TOKENS, SHIPPED, SHIPPED, "qml"))["hairline"][3:], "FFFFFF")

    def test_a_user_file_overrides_only_what_it_says(self):
        with tempfile.TemporaryDirectory() as d:
            user = self.write(d, "theme.json", {"radii": {"sheet": 40}, "dark": {"accent": "#112233"}})
            menu = json.loads(run(TOKENS, SHIPPED, user, "qml"))
            self.assertEqual(menu["radius"], 40)
            self.assertEqual(menu["rowRadius"], 22)           # the shipped radius survives beside it
            self.assertEqual(menu["accent"], "#FF112233")

    def test_a_missing_or_broken_user_file_leaves_the_shipped_tokens(self):
        with tempfile.TemporaryDirectory() as d:
            broken = os.path.join(d, "theme.json")
            with open(broken, "w") as f:
                f.write("{not json")
            for user in (os.path.join(d, "absent.json"), broken):
                self.assertEqual(json.loads(run(TOKENS, SHIPPED, user, "qml"))["accent"], "#FFA79BF5")

    def test_accent_modes(self):
        with tempfile.TemporaryDirectory() as d:
            fixed = self.write(d, "fixed.json", {"accent": "#334455"})
            self.assertEqual(json.loads(run(TOKENS, SHIPPED, fixed, "qml"))["accent"], "#FF334455")
            wall = self.write(d, "theme.json", {"accent": "wallpaper"})
            self.assertEqual(json.loads(run(TOKENS, SHIPPED, wall, "qml"))["accent"], "#FFA79BF5")   # no tint.json yet
            with open(os.path.join(d, "tint.json"), "w") as f:
                json.dump({"accent": "#9898E1"}, f)
            self.assertEqual(json.loads(run(TOKENS, SHIPPED, wall, "qml"))["accent"], "#FF9898E1")
            tile = self.write(d, "tile.json", {"accent": "tile"})
            self.assertEqual(json.loads(run(TOKENS, SHIPPED, tile, "qml"))["accent"], "#FFA79BF5")

    def test_the_contrast_guard_darkens_a_pale_sheet(self):
        with tempfile.TemporaryDirectory() as d:
            pale = self.write(d, "theme.json", {"dark": {"glass": "#66E0E0F0", "ink": "#F2EFF6"}})
            menu = json.loads(run(TOKENS, SHIPPED, pale, "qml"))
            sheet = tuple(int(menu["sheetBottom"][i:i + 2], 16) for i in (3, 5, 7))
            ink = (0xF2, 0xEF, 0xF6)

            def lum(c):
                f = lambda v: (v / 255) / 12.92 if v / 255 <= 0.03928 else (((v / 255) + 0.055) / 1.055) ** 2.4
                return 0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2])
            ratio = (lum(ink) + 0.05) / (lum(sheet) + 0.05)
            self.assertGreaterEqual(ratio, 7.0)

    def test_css_names_and_values(self):
        css = run(TOKENS, SHIPPED, SHIPPED, "css")
        for want in ("--lu-ink: rgba(242, 239, 246, 1.000);", "--lu-accent: #A79BF5;", "--lu-accent-ink: #17131F;",
                     "--lu-font: Manrope, system-ui, sans-serif;", "--lu-radius-sheet: 34px;", "--lu-weight-semibold: 600;",
                     "--lu-type-title: 40px;", "--lu-menu-scrim:", "--sand-toggle-on-bg-color: #A79BF5;",
                     "--sand-focus-bg-color-rgb: 242, 237, 232;"):
            self.assertIn(want, css)
        self.assertNotIn("Inter", css)

    def test_the_web_stylesheet_uses_only_tokens_the_generator_writes(self):
        css = run(TOKENS, SHIPPED, SHIPPED, "css")
        defined = {line.split(":")[0].strip() for line in css.splitlines() if line.strip().startswith("--")}
        with open(os.path.join(ROOT, "payload", "webapps", "lumatv.css")) as f:
            sheet = f.read()
        import re
        used = set(re.findall(r"var\((--[a-z0-9-]+)", sheet))
        self.assertEqual(sorted(used - defined), [])
        self.assertNotRegex(sheet, r"#[0-9A-Fa-f]{6}\b")

    def test_web_tokens_have_both_materials_and_the_four_faces(self):
        css = run(TOKENS, SHIPPED, SHIPPED, "web")
        cut = css.index('[data-theme="light"]')
        dark, light = css[:cut], css[cut:]
        self.assertIn(':root, [data-theme="dark"] {', dark)
        self.assertIn("color-scheme: dark;", dark)
        self.assertIn("--lu-ink: rgba(242, 239, 246, 1.000);", dark)
        self.assertIn("--lu-accent: #A79BF5;", dark)
        self.assertIn("color-scheme: light;", light)
        self.assertIn("--lu-ink: rgba(23, 19, 31, 1.000);", light)
        self.assertIn("--lu-accent: #907AA9;", light)          # the palette's own: the wallpaper tints Home, not a web page
        self.assertIn("--lu-accent-ink: #17131F;", light)          # whichever of the palette's inks reads best on it, as Lu.qml picks
        for want in ("--lu-font: Manrope, system-ui, sans-serif;", "--lu-radius-sheet: 34px;", "--lu-type-body: 28px;", "--lu-card-ms: 180ms;",
                     "--lu-ease-card: cubic-bezier(0.215, 0.61, 0.355, 1);", "--lu-focus-wash: rgba(23, 19, 31, 0.060);"):
            self.assertIn(want, css)
        for face in ("Regular", "Medium", "SemiBold", "Bold"):
            self.assertIn('url("Manrope-%s.ttf")' % face, css)
        self.assertNotIn("Inter", css)

    def test_a_fixed_accent_is_both_materials_and_the_wallpapers_is_the_dark_ones(self):
        with tempfile.TemporaryDirectory() as d:
            fixed = self.write(d, "fixed.json", {"accent": "#334455"})
            css = run(TOKENS, SHIPPED, fixed, "web")
            self.assertEqual(css.count("--lu-accent: #334455;"), 2)
            wall = self.write(d, "theme.json", {"accent": "wallpaper"})
            with open(os.path.join(d, "tint.json"), "w") as f:
                json.dump({"accent": "#9898E1"}, f)
            css = run(TOKENS, SHIPPED, wall, "web")
            self.assertIn("--lu-accent: #9898E1;", css[:css.index('[data-theme="light"]')])
            self.assertIn("--lu-accent: #907AA9;", css[css.index('[data-theme="light"]'):])

    def test_the_frame_stylesheet_uses_only_tokens_the_web_output_writes(self):
        import re
        css = run(TOKENS, SHIPPED, SHIPPED, "web")
        defined = {line.split(":")[0].strip() for line in css.splitlines() if line.strip().startswith("--")}
        with open(os.path.join(ROOT, "frame", "frame.css")) as f:
            sheet = f.read()
        self.assertEqual(sorted(set(re.findall(r"var\((--[a-z0-9-]+)", sheet)) - defined), [])
        self.assertNotRegex(sheet, r"#[0-9A-Fa-f]{6}\b")
        self.assertNotRegex(sheet, r"rgba?\(")
        with open(os.path.join(ROOT, "frame", "index.html")) as f:
            page = f.read()
        self.assertLess(page.index("theme.css"), page.index("frame.css"))       # the tokens are loaded before the sheet that reads them

    def test_tokens_js_is_a_pragma_library_with_both_materials(self):
        js = run(TOKENS, SHIPPED, SHIPPED, "js")
        self.assertTrue(js.startswith(".pragma library"))
        data = json.loads(js[js.index("{"):js.rindex("}") + 1])
        self.assertEqual(data["font"], "Manrope")
        self.assertIn("accent", data["dark"])
        self.assertIn("accent", data["light"])
        self.assertEqual(data["radii"]["pill"], 999)


class ThemeFile(unittest.TestCase):
    def test_minimize_keeps_only_differences_and_drops_retired_keys(self):
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "theme.json")
            with open(SHIPPED) as f:
                shipped = json.load(f)
            user = json.loads(json.dumps(shipped))
            user["version"] = 1
            user["radius"] = 26
            user["dark"]["scrim"] = [0.1, 0.18, 0.46]
            user["wallpaperMotion"]["amplitude"] = 11
            user["tiles"]["plates"]["x.y"] = "#112233"
            user["custom"] = {"a": 1}
            with open(path, "w") as f:
                json.dump(user, f)
            run(THEME, path, "minimize", SHIPPED)
            with open(path) as f:
                slim = json.load(f)
            self.assertEqual(slim, {"wallpaperMotion": {"amplitude": 11}, "tiles": {"plates": {"x.y": "#112233"}}, "custom": {"a": 1}})

    def test_an_unedited_file_becomes_empty_and_a_second_run_changes_nothing(self):
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "theme.json")
            with open(SHIPPED) as f, open(path, "w") as g:
                g.write(f.read())
            run(THEME, path, "minimize", SHIPPED)
            with open(path) as f:
                self.assertEqual(json.load(f), {})
            before = os.stat(path).st_mtime_ns
            run(THEME, path, "minimize", SHIPPED)
            self.assertEqual(os.stat(path).st_mtime_ns, before)

    def test_set_merges_objects_and_replaces_the_rest(self):
        with tempfile.TemporaryDirectory() as d:
            path = os.path.join(d, "theme.json")
            with open(path, "w") as f:
                json.dump({"wallpaperMotion": {"amplitude": 4, "speed": 1}, "accent": "tile"}, f)
            run(THEME, path, "set", json.dumps({"wallpaperMotion": {"amplitude": 9}, "accent": "wallpaper"}))
            with open(path) as f:
                self.assertEqual(json.load(f), {"wallpaperMotion": {"amplitude": 9, "speed": 1}, "accent": "wallpaper"})


if __name__ == "__main__":
    unittest.main()
