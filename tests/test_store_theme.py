"""Run with: python3 -m unittest tests.test_store_theme

The Content Store theme (tools/store_theme.py, payload/store): what the builder writes and cuts, what the stylesheet may contain, what the
script does in a real browser (the face behind LG's family names, the plate found by what it computes, the ink on it, the way back when
the focus leaves, the canvas, the motion), and the stand-in pages of docs/mocks/store drawn with the theme on and audited from their
pixels for overlapping text, contrast, the ring, the fade-out and the face, in six ways LG's plate might be built.

The browser parts need the system Chromium and playwright-core (the ones docs/mocks/render.mjs uses) and are skipped without them. The
store's markup is LG's cloud page and is not in the firmware snapshot, so the theme names none of it; what these prove is the mechanism
against every way the plate might be drawn, and the probe (tests/test_store_probe.py) is how the TV round reads the real one.
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
sys.path.insert(0, os.path.join(ROOT, "tools"))
import store_theme  # noqa: E402
import tokens  # noqa: E402

PAYLOAD = os.path.join(ROOT, "payload", "store")
SHIPPED = os.path.join(ROOT, "payload", "theme", "theme.json")
TOOL = os.path.join(ROOT, "tools", "store_theme.py")
BUILD = os.path.join(ROOT, "tools", "build-ipk.sh")
PLAYWRIGHT = "/data/home/lucent-ha/node_modules/playwright-core/index.mjs"
CHROMIUM = "/usr/bin/chromium"
BROWSER = os.path.exists(PLAYWRIGHT) and os.path.exists(CHROMIUM)


def build(parts="", probe_port=None, user="/dev/null"):
    return store_theme.build_script(PAYLOAD, SHIPPED, user, store_theme.parse_parts(parts), probe_port)


def theme():
    return tokens.resolve(SHIPPED, "/dev/null")[0]


def css_of(text):
    """The stylesheet a built script carries, as the script's own JSON string."""
    m = re.search(r'var CSS = ("(?:[^"\\]|\\.)*");', text)
    assert m, "no stylesheet in the script"
    return json.loads(m.group(1))


def strip_comments(css):
    return re.sub(r"/\*.*?\*/", "", css, flags=re.S)


def rgb(c):
    return "rgb(%d, %d, %d)" % tuple(c[:3])


class Parts(unittest.TestCase):
    def test_the_default_is_look_and_motion(self):
        self.assertEqual(store_theme.parse_parts(""), ("look", "motion"))
        self.assertEqual(store_theme.parse_parts("  "), ("look", "motion"))

    def test_parts_come_back_in_canonical_order_commas_or_spaces(self):
        self.assertEqual(store_theme.parse_parts("lazy, look"), ("look", "lazy"))
        self.assertEqual(store_theme.parse_parts("motion look lazy canvas"), ("look", "canvas", "motion", "lazy"))
        self.assertEqual(store_theme.parse_parts("probe"), ("probe",))

    def test_an_unknown_part_is_refused(self):
        with self.assertRaises(ValueError):
            store_theme.parse_parts("look,sparkle")

    def test_the_opt_in_parts_are_never_on_by_default(self):
        for part in ("lazy", "canvas", "probe"):
            self.assertNotIn(part, store_theme.DEFAULT_PARTS)

    def test_cut_keeps_a_block_for_any_part_it_names_and_drops_the_rest(self):
        text = "a\n//@@part x y\nxy\n//@@end\nb\n/*@@part z*/\nz\n/*@@end*/\nc"
        self.assertEqual(store_theme.cut(text, ("x",)), "a\nxy\nb\nc")
        self.assertEqual(store_theme.cut(text, ("y", "z")), "a\nxy\nb\nz\nc")
        self.assertEqual(store_theme.cut(text, ()), "a\nb\nc")

    def test_cut_refuses_blocks_that_do_not_close_or_nest(self):
        for bad in ("//@@part x\na", "//@@end\na", "//@@part x\n//@@part y\n//@@end\n//@@end"):
            with self.assertRaises(ValueError, msg=bad):
                store_theme.cut(bad, ("x",))


class Build(unittest.TestCase):
    def test_the_markers_are_filled_and_the_script_parses(self):
        text, options = build()
        for marker in (store_theme.OPTIONS_MARKER, store_theme.CSS_MARKER, store_theme.FONT_MARKER):
            self.assertNotIn(marker, text)
        self.assertNotIn("@@", text)
        self.assertEqual(options["parts"], ["look", "motion"])
        self.assertEqual(options["capMs"], 260)
        with tempfile.NamedTemporaryFile("w", suffix=".js", delete=False) as f:
            f.write(text)
        try:
            r = subprocess.run(["node", "--check", f.name], capture_output=True, text=True)
            self.assertEqual(r.returncode, 0, r.stderr)
        finally:
            os.unlink(f.name)

    def test_every_combination_of_parts_parses(self):
        import itertools
        names = [p for p in store_theme.PARTS if p != "probe"]
        for n in range(1, len(names) + 1):
            for combo in itertools.combinations(names, n):
                text, _ = build(",".join(combo))
                with tempfile.NamedTemporaryFile("w", suffix=".js", delete=False) as f:
                    f.write(text)
                try:
                    r = subprocess.run(["node", "--check", f.name], capture_output=True, text=True)
                    self.assertEqual(r.returncode, 0, "%s: %s" % (combo, r.stderr))
                finally:
                    os.unlink(f.name)
        text, _ = build("probe", 18742)
        self.assertIn("lumatv-probe", text)

    def test_the_default_script_stays_small(self):
        # TV round 11: the first version's 76 KB script, with 45 substring selectors, put the page's cold open at +9.3 %. The sheet is
        # a few KB now and the face 21 KB; the budget is for the script the page reads at every load.
        text, _ = build()
        self.assertLess(len(text.encode("utf-8")), 56 * 1024)
        motion, _ = build("motion")
        self.assertLess(len(motion.encode("utf-8")), 8 * 1024, "the motion part alone is the cap and nothing else")

    def test_the_face_is_one_latin_variable_font_carried_once(self):
        import base64
        path = os.path.join(PAYLOAD, "Manrope-store.woff2")
        with open(path, "rb") as f:
            data = f.read()
        self.assertEqual(data[:4], b"wOF2")
        self.assertLess(len(data), 24 * 1024, "the face rides in the script on every page the store loads (the first version's was 39 KB)")
        text, _ = build()
        self.assertEqual(text.count(base64.b64encode(data[:60]).decode().rstrip("=")), 1)

    def test_the_face_has_latin_and_not_the_scripts_the_page_falls_back_for(self):
        try:
            from fontTools.ttLib import TTFont
            import brotli  # noqa: F401
        except ImportError:
            self.skipTest("fontTools and brotli read a woff2 file")
        font = TTFont(os.path.join(PAYLOAD, "Manrope-store.woff2"))
        cmap = font.getBestCmap()
        for ch in "AZaz09 .,:;!?&()-_/'\"%+=@#$\u00e9\u00fc\u00f1\u00df\u2013\u2014\u2019\u20ac\u2122":
            self.assertIn(ord(ch), cmap, ch)
        for ch in "\u65e5\u0416\u0627\u05d0":
            self.assertNotIn(ord(ch), cmap, ch)
        axes = {a.axisTag: (a.minValue, a.maxValue) for a in font["fvar"].axes}
        self.assertEqual(axes["wght"], (300.0, 700.0))

    def test_without_look_nothing_is_painted_and_no_face_is_carried(self):
        text, options = build("motion")
        self.assertEqual(css_of(text), "")
        self.assertIn('var FONT = "";', text)
        self.assertNotIn("function plateOf", text, "no focus layer without the look part")
        self.assertNotIn("data-lu-ps", text)

    def test_the_parts_are_cut_out_of_the_script_they_are_not_in(self):
        look, motion, canvas, lazy = build("look")[0], build("motion")[0], build("look,canvas")[0], build("look,lazy")[0]
        self.assertIn("function plateOf", look)
        self.assertNotIn("transitionrun\", function (e) {\n            var d", look)
        self.assertNotIn("Element.prototype.animate", look)
        self.assertIn("Element.prototype.animate", motion)
        self.assertNotIn("data-lu-bare", look)
        self.assertIn("data-lu-bare", canvas)
        self.assertNotIn("loading", look.replace("loading=", ""))
        self.assertIn('"lazy"', lazy)
        for text in (look, motion, canvas, lazy, build()[0]):
            self.assertNotIn("lumatv-probe", text, "the probe is in a build only when it is asked for")

    def test_the_probe_needs_the_receivers_port_and_has_it_in_the_script(self):
        with self.assertRaises(ValueError):
            build("probe")
        text, options = build("look,probe", 31337)
        self.assertEqual(options["probe"], {"url": "http://127.0.0.1:31337/p", "max": store_theme.PROBE_MAX})
        self.assertIn("http://127.0.0.1:31337/p", text)

    def test_the_script_names_no_class_of_the_page(self):
        # TV round 11: the first version's forty-five selectors on Sandstone's class names missed LG's focus markup. The script and the
        # sheet match nothing by class, so a markup that is not what was assumed leaves LG's colours, never light words on a light plate.
        for name in ("userScript.js", "store.css"):
            with open(os.path.join(PAYLOAD, name), encoding="utf-8") as f:
                body = f.read()
            code = strip_comments(body) if name.endswith(".css") else re.sub(r"//[^\n]*|/\*.*?\*/", "", body, flags=re.S)
            for banned in ("ImageItem", "TabGroup", "Header_", "Button_", "[class*", "[class^", "[class$", "querySelector(\".", "querySelectorAll(\"."):
                self.assertNotIn(banned, code, "%s names the page's markup: %s" % (name, banned))

    def test_the_tokens_hold_the_contrast_floors_on_the_glass(self):
        t = theme()
        dark = t["dark"]
        ink, plate = tokens.parse(dark["ink"])[:3], tokens.parse(dark["glassFocus"])[:3]
        for alpha in store_theme.INK_TIERS:
            text = store_theme.composite(ink, alpha, plate)
            self.assertGreaterEqual(tokens.contrast(text, plate), 4.5, "ink at %.2f on the plate" % alpha)
        css = build()[0]
        sheet = css_of(css)
        self.assertIn("--st-plate: %s;" % tokens.hexrgb(*plate), sheet)
        ring = re.search(r"--st-ring: rgba\((\d+), (\d+), (\d+), ([\d.]+)\)", sheet)
        self.assertTrue(ring)
        ring_c = store_theme.composite(tuple(int(ring.group(i)) for i in (1, 2, 3)), float(ring.group(4)), plate)
        self.assertGreaterEqual(tokens.contrast(ring_c, (0, 0, 0)), 3.0)
        self.assertGreaterEqual(tokens.contrast(ring_c, plate), 3.0)
        out = re.search(r"--st-ring-out: rgba\((\d+), (\d+), (\d+), ([\d.]+)\)", sheet)
        self.assertGreaterEqual(tokens.contrast(store_theme.composite(tuple(int(out.group(i)) for i in (1, 2, 3)), float(out.group(4)), (0, 0, 0)), (0, 0, 0)), 3.0)

    def test_a_palette_that_would_break_a_floor_is_an_error(self):
        t = theme()
        t["dark"]["ink"] = "#6a6a78"
        with self.assertRaises(ValueError):
            store_theme.tokens_css(t, ("look",))

    def test_the_options_carry_the_families_the_cap_and_the_plate_threshold(self):
        _, options = build()
        families = dict((n, w) for n, w in options["faces"])
        self.assertEqual(families["Sandstone"], [300, 400, 600, 700])
        self.assertEqual(families["MuseoSans"], [100, 300, 400, 500, 700, 900])
        self.assertIn("LG Smart UI", families)
        self.assertEqual(options["plateLum"], 0.35)
        ring = options["ring"]
        self.assertEqual((ring["floor"], ring["ink"], ring["plate"]), (3.3, list(tokens.parse(theme()["dark"]["ink"])[:3]), list(tokens.parse(theme()["dark"]["glassFocus"])[:3])))
        self.assertTrue(0.2 <= ring["base"] <= 1.0 and 0.2 <= ring["baseOut"] <= 1.0)
        self.assertEqual(options["capMs"], theme()["motion"]["cardMs"])
        self.assertTrue(options["range"].startswith("U+0020-007E"))

    def test_a_broken_payload_is_an_error_and_not_a_script(self):
        with tempfile.TemporaryDirectory() as d:
            for name in ("store.css", "Manrope-store.woff2"):
                shutil.copy(os.path.join(PAYLOAD, name), os.path.join(d, name))
            with open(os.path.join(PAYLOAD, "userScript.js")) as f:
                text = f.read()
            with open(os.path.join(d, "userScript.js"), "w") as f:
                f.write(text.replace(store_theme.CSS_MARKER, '""'))
            with self.assertRaises(ValueError):
                store_theme.build_script(d, SHIPPED, "/dev/null", ("look",))

    def test_the_cli_writes_the_file_and_one_json_object(self):
        with tempfile.TemporaryDirectory() as d:
            out = os.path.join(d, "userScript.js")
            r = subprocess.run([sys.executable, TOOL, "build", PAYLOAD, SHIPPED, "/dev/null", out, "--parts", "look"], capture_output=True, text=True)
            self.assertEqual(r.returncode, 0, r.stderr)
            info = json.loads(r.stdout)
            self.assertEqual((info["ok"], info["parts"], info["bytes"]), (True, ["look"], os.path.getsize(out)))
            bad = subprocess.run([sys.executable, TOOL, "build", PAYLOAD, SHIPPED, "/dev/null", out, "--parts", "nope"], capture_output=True, text=True)
            self.assertEqual(bad.returncode, 2)
            noport = subprocess.run([sys.executable, TOOL, "build", PAYLOAD, SHIPPED, "/dev/null", out, "--parts", "probe"], capture_output=True, text=True)
            self.assertEqual(noport.returncode, 1)
            self.assertFalse(json.loads(noport.stdout)["ok"])
            withport = subprocess.run([sys.executable, TOOL, "build", PAYLOAD, SHIPPED, "/dev/null", out, "--parts", "look,probe", "--probe-port", "4242"], capture_output=True, text=True)
            self.assertEqual(withport.returncode, 0, withport.stderr)
            self.assertEqual(json.loads(withport.stdout)["probe"]["url"], "http://127.0.0.1:4242/p")
            self.assertEqual([f for f in os.listdir(d)], ["userScript.js"], "no half-written file stays")

    def test_the_tool_prints_the_parts_it_would_use(self):
        r = subprocess.run([sys.executable, TOOL, "parts", ""], capture_output=True, text=True)
        self.assertEqual((r.returncode, r.stdout.strip()), (0, "look,motion"))
        r = subprocess.run([sys.executable, TOOL, "parts", "x"], capture_output=True, text=True)
        self.assertEqual(r.returncode, 2)
        r = subprocess.run([sys.executable, TOOL, "parts", "probe look"], capture_output=True, text=True)
        self.assertEqual(r.stdout.strip(), "look,probe")


def shell_function(path, name):
    """One shell function of a script, from its `name() {` line to the closing brace in column 0."""
    out, on = [], False
    with open(path) as f:
        for line in f:
            if line.startswith(name + "() {"):
                on = True
            if on:
                out.append(line)
                if line.rstrip() == "}":
                    return "".join(out)
    raise AssertionError(name + " not found in " + path)


def store_files():
    """What the package carries for the store, as paths under the repository: payload/ goes out whole, the tools are copied by name."""
    tools = os.path.join(ROOT, "tools")
    return (["payload/store/" + n for n in sorted(os.listdir(PAYLOAD))] +
            ["tools/" + n for n in sorted(os.listdir(tools)) if n.startswith("store") and os.path.isfile(os.path.join(tools, n))])


class Packaged(unittest.TestCase):
    """What tools/build-ipk.sh does to every file it stages (its `render`): fills the brand's double-brace placeholders and fails the whole
    package on any name in double braces it finds left, in a comment or a string as much as in code. The store's builder fills markers of its
    own in its template, so neither may be spelt that way; this runs the build's own function on the store's files, as the package build
    would, and not the build's regular expression as a copy."""

    def render(self, d):
        script = ("BRAND_DISPLAY_NAME=LumaTV SHORT_ID=lumatv APP_ID=org.nphil.lumatv BRAND_DATA_DIR=/var/lib/lumatv QML_MODULE=lumatv\n"
                  + shell_function(BUILD, "render") + 'render "$1"\n')
        return subprocess.run(["sh", "-c", script, "sh", d], capture_output=True, text=True, timeout=60)

    def stage(self, d):
        for rel in store_files():
            dst = os.path.join(d, rel)
            os.makedirs(os.path.dirname(dst), exist_ok=True)
            shutil.copy(os.path.join(ROOT, rel), dst)

    def test_the_store_files_pass_the_package_builds_placeholder_check(self):
        with tempfile.TemporaryDirectory() as d:
            self.stage(d)
            r = self.render(d)
            self.assertEqual((r.returncode, r.stderr), (0, ""))
            # nothing the build fills was there to fill: the files are as they were checked in
            for rel in store_files():
                with open(os.path.join(ROOT, rel), "rb") as a, open(os.path.join(d, rel), "rb") as b:
                    self.assertEqual(a.read(), b.read(), rel)

    def test_the_check_is_live_a_placeholder_the_brand_does_not_have_fails_it(self):
        # the old markers of userScript.js, and a name spelt in a comment: both were found only when the package was built
        for name, body in (("tools/store_theme.py", "x = '/*{{OPTIONS}}*/ null'\n"), ("payload/store/userScript.js", "// {{name}} placeholders\n"),
                           ("tools/store_probe.py", "# {{port}}\n")):
            with tempfile.TemporaryDirectory() as d:
                self.stage(d)
                with open(os.path.join(d, name), "a") as f:
                    f.write(body)
                r = self.render(d)
                self.assertEqual(r.returncode, 1, name)
                self.assertIn("unresolved placeholders", r.stderr)
                self.assertIn(os.path.basename(name), r.stderr)

    def test_a_brand_placeholder_is_filled_and_not_an_error(self):
        with tempfile.TemporaryDirectory() as d:
            self.stage(d)
            with open(os.path.join(d, "payload/store/userScript.js"), "a") as f:
                f.write("// {{shortId}}\n")
            self.assertEqual(self.render(d).returncode, 0)
            with open(os.path.join(d, "payload/store/userScript.js")) as f:
                self.assertTrue(f.read().rstrip().endswith("// lumatv"))

    def test_every_store_file_is_one_the_build_requires_and_copies(self):
        with open(BUILD) as f:
            build_text = f.read()
        files = store_files()
        self.assertGreaterEqual(len(files), 6)
        for rel in files:
            self.assertIn(rel, build_text, "tools/build-ipk.sh does not name " + rel)
        # nothing it names is gone from the tree
        for rel in re.findall(r"^((?:tools/store\S*|payload/store/\S+))$", build_text, flags=re.M):
            self.assertTrue(os.path.exists(os.path.join(ROOT, rel)), rel)
        self.assertNotIn("store_dump", build_text)
        self.assertNotIn("store-light", build_text)


class StoreCss(unittest.TestCase):
    """The rules the sheet keeps: presentation only, nothing hidden, nothing heavy, tokens and not constants, no class of the page."""

    @classmethod
    def setUpClass(cls):
        with open(os.path.join(PAYLOAD, "store.css")) as f:
            cls.source = f.read()
        cls.sheet = strip_comments(cls.source)
        cls.look = strip_comments(store_theme.cut(cls.source, ("look",)))
        cls.canvas = strip_comments(store_theme.cut(cls.source, ("canvas",)))
        cls.built = css_of(build()[0])

    @staticmethod
    def rules(css):
        return [(sel.strip(), body.strip()) for sel, body in re.findall(r"([^{}]+)\{([^{}]*)\}", css)]

    def test_every_selector_is_an_attribute_the_script_sets_and_never_a_class_of_the_page(self):
        selectors = [sel for sel, _ in self.rules(self.look)]
        self.assertGreaterEqual(len(selectors), 8)
        self.assertLessEqual(len(selectors), 10)
        for sel in selectors:
            self.assertRegex(sel, r'^\[data-lu-(ps|pb|pa|fr|root|ink="[123]"|solid)\](::before|::after)?$', sel)
        self.assertEqual([sel for sel, _ in self.rules(self.canvas)], ["html, body", "[data-lu-bare]"])
        for css in (self.look, self.canvas):
            self.assertNotRegex(css, r"[.#]\w|\[class|\*\s*[{,]")

    def test_the_script_sets_exactly_the_attributes_the_sheet_reads(self):
        with open(os.path.join(PAYLOAD, "userScript.js")) as f:
            js = f.read()
        read = set(re.findall(r"data-lu-([a-z]+)", self.sheet))
        for attr in read:
            self.assertIn("data-lu-" + attr, js, "the sheet reads data-lu-%s and nothing sets it" % attr)
        for attr in set(re.findall(r'"data-lu-([a-z]+)"', js)):
            self.assertIn(attr, read, "the script sets data-lu-%s and the sheet never reads it" % attr)

    def test_nothing_is_hidden_removed_moved_or_added(self):
        for banned in ("display:", "visibility:", "content:", "position:", "width:", "height:", "margin", "padding", "transform", "top:", "left:", "inset"):
            self.assertNotIn(banned, self.sheet.replace("outline-offset", "OO"), banned)

    def test_nothing_costs_a_blur_a_filter_a_shadow_or_an_animation(self):
        for banned in ("blur(", "filter", "backdrop", "animation", "will-change", "@keyframes", "text-shadow"):
            self.assertNotIn(banned, self.sheet, banned)
        for sel, body in self.rules(self.sheet):
            for value in re.findall(r"box-shadow:\s*([^;!]+)", body):
                self.assertEqual(value.strip(), "none", sel)

    def test_a_declaration_beats_the_pages_and_a_transition_is_only_ever_taken_away(self):
        for sel, body in self.rules(self.sheet):
            if sel == ":root":
                continue
            for decl in [d.strip() for d in body.split(";") if d.strip()]:
                self.assertTrue(decl.endswith("!important"), "%s { %s } does not beat the page's own rule" % (sel, decl))
        self.assertEqual({v.strip() for v in re.findall(r"transition:\s*([^;!]+)", self.sheet)}, {"none"})

    def test_no_colour_or_size_is_a_constant_in_a_rule(self):
        for sel, body in self.rules(self.sheet):
            self.assertNotRegex(body, r"#[0-9a-fA-F]{3,8}\b|rgba?\(|hsla?\(", sel)
            for px in re.findall(r"(-?\d+(?:\.\d+)?)px", body):
                self.assertIn(px, ("2", "-2"), "%s: a size that is not the ring's" % sel)

    def test_every_variable_the_sheet_reads_is_defined_by_the_build(self):
        read = set(re.findall(r"var\((--[a-z0-9-]+)\)", self.sheet))
        defined = set(re.findall(r"(--[a-z0-9-]+):", self.built))
        self.assertEqual(sorted(read - defined), [])
        self.assertLessEqual(len(defined), 10, "a handful of tokens, not a copy of the theme")

    def test_the_built_sheet_is_the_tokens_and_the_look_rules_and_nothing_else(self):
        self.assertTrue(self.built.startswith(":root { --st-plate:"))
        self.assertLess(len(self.built), 4 * 1024)
        self.assertNotIn("html, body", self.built, "the canvas rules are in a build only with the canvas part")
        self.assertIn("html, body", css_of(build("look,canvas")[0]))


@unittest.skipUnless(BROWSER, "needs the system Chromium and playwright-core (docs/mocks/render.mjs uses the same)")
class InTheBrowser(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        d = cls.tmp.name
        files = {}
        for name, parts in (("default", "look,motion"), ("look", "look"), ("motion", "motion"), ("lazy", "look,motion,lazy"), ("canvas", "look,motion,canvas")):
            files[name] = os.path.join(d, name + ".js")
            subprocess.run([sys.executable, TOOL, "build", PAYLOAD, SHIPPED, "/dev/null", files[name], "--parts", parts], check=True, capture_output=True)
        r = subprocess.run(["node", os.path.join(HERE, "store_behaviour.mjs"), files["default"], files["look"], files["motion"], files["lazy"], files["canvas"]],
                           capture_output=True, text=True, timeout=240)
        if r.returncode != 0:
            raise AssertionError("the browser run failed: " + r.stderr[-1500:])
        cls.r = json.loads(r.stdout)
        t = theme()["dark"]
        cls.plate = rgb(tokens.parse(t["glassFocus"]))
        cls.ink = rgb(tokens.parse(t["ink"]))
        cls.canvas = rgb(tokens.parse(t["canvas"]))
        cls.ink_rgb = tuple(tokens.parse(t["ink"])[:3])

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def test_the_marker_is_there_and_the_page_runs_without_an_error(self):
        d = self.r["dark"]
        self.assertEqual((d["mark"], d["attr"], d["sheets"]), ("look,motion", "look,motion", 1))
        self.assertEqual(self.r["focus"]["errors"], [])
        self.assertEqual(d["errors"], [])

    def test_the_default_leaves_the_pages_own_canvas_and_colours(self):
        d = self.r["dark"]
        self.assertEqual((d["htmlBg"], d["bodyBg"], d["h1"]), ("rgb(0, 0, 0)", "rgb(0, 0, 0)", "rgb(236, 236, 236)"), "LG's black and LG's ink: the canvas is the opt-in part")
        self.assertEqual(self.r["canvasOff"]["header"], "rgb(0, 0, 0)")
        self.assertFalse(self.r["canvasOff"]["marked"])

    def test_the_face_is_behind_lgs_family_names_at_every_weight_and_nothing_else_is_touched(self):
        f = self.r["fonts"]
        for ident in ("p-sand", "p-300", "p-700", "p-museo", "p-lg"):
            self.assertTrue(any("Manrope" in n for n in f[ident]), (ident, f[ident]))
        self.assertNotIn("Manrope", " ".join(f["p-arial"]), "a family that is not LG's keeps its face")
        self.assertNotIn("Manrope", " ".join(f["p-cjk"]), "a character the face lacks falls back")
        self.assertEqual(self.r["dark"]["sandstone"], 8, "the page's four faces and ours, one per weight")

    def test_a_page_that_forbids_inline_styles_takes_the_theme_all_the_same(self):
        self.assertEqual((self.r["csp"]["sheets"], self.r["csp"]["mark"]), (1, "look,motion"))

    def test_a_plate_drawn_by_a_pseudo_element_becomes_the_glass_and_its_words_the_ink_in_their_tiers(self):
        s = self.r["focus"]["plate"]
        self.assertEqual(s["found"]["plates"][0].split(":")[0], "pb")
        self.assertEqual(s["plate"], {"before": self.plate, "ring": "solid", "radius": "%dpx" % theme()["radii"]["row"]})
        self.assertEqual(s["marks"]["pa"], ["pb", "root"])
        self.assertEqual((s["marks"]["pa1"], s["marks"]["pa2"], s["marks"]["pa3"]), (["ink=1"], ["ink=2", "solid"], ["ink=3", "solid"]))
        self.assertEqual(s["color"]["pa1"], self.ink)
        a2, a3 = s["color"]["pa2"], s["color"]["pa3"]
        self.assertTrue(a2.startswith("rgba(") and a2.endswith(", 0.8)"), a2)
        self.assertTrue(a3.endswith(", 0.66)"), a3)
        self.assertEqual((s["opacity"]["pa2"], s["opacity"]["pa3"]), ("1", "1"), "LG's translucent words are made solid so that the tier is what is seen")

    def test_the_plate_is_found_before_the_first_frame_whatever_way_it_arrives(self):
        self.assertLessEqual(self.r["focus"]["plate"]["found"]["ms"], 30)
        late = self.r["focus"]["late"]
        self.assertEqual(late["found"]["plates"][0].split(":")[0], "ps", "a class the page adds after the focus event, and a colour transition")
        self.assertLessEqual(late["found"]["ms"], 30)
        self.assertEqual(late["color"]["lt1"], self.ink)

    def test_the_ring_holds_its_contrast_on_a_surface_that_is_not_lgs_black(self):
        def parse(c):
            nums = [float(x) for x in re.findall(r"[\d.]+", c)]
            return nums[:3], (nums[3] if len(nums) > 3 else 1.0)

        def drawn(colour, over):
            rgba, a = parse(colour)
            return store_theme.composite(rgba, a, over)

        plate, panel, black = tuple(tokens.parse(theme()["dark"]["glassFocus"])[:3]), (95, 107, 122), (0, 0, 0)
        floor = store_theme.RING_FLOOR
        black_ring = self.r["focus"]["plate"]["ring"]
        self.assertIsNone(black_ring["onBlack"]["style"], "on black the sheet's own ring is used and nothing is written to the element")
        self.assertIsNone(black_ring["buttonOnBlack"]["style"])
        # the plate on the panel: the ring is over the plate and against the panel, both at the floor
        s = self.r["focus"]["ringPanel"]["ring"]["onPanel"]
        self.assertIn("--st-ring", s["style"], "the script raised the ring's alpha for this surface")
        ring = drawn(s["outline"], plate)
        self.assertGreaterEqual(tokens.contrast(ring, plate), floor)
        self.assertGreaterEqual(tokens.contrast(ring, panel), floor)
        # the ring of an item with no plate, outside its edge, against the panel
        b = self.r["focus"]["ringButton"]["ring"]["buttonOnPanel"]
        self.assertIn("--st-ring-out", b["style"])
        self.assertGreaterEqual(tokens.contrast(drawn(b["outline"], panel), panel), floor)
        # and it comes off when the focus leaves
        self.assertEqual(self.r["focus"]["after"]["ring"]["onPanel"]["style"] in (None, ""), True)
        self.assertEqual(self.r["focus"]["after"]["ring"]["buttonOnPanel"]["style"].count("--st-ring"), 0)

    def test_an_item_with_no_plate_keeps_lgs_colours_and_gets_the_ring_alone(self):
        for name, ident, words in (("button", "bt", ["bt"]), ("icon", "ic", ["ic1"])):
            s = self.r["focus"][name]
            self.assertIsNone(s["found"], name)
            self.assertEqual(s["marks"][ident], ["fr"], name)
            for w in words:
                self.assertEqual(s["color"][w], self.r["focus"]["before"]["color"][w], "LG's colour stays: light on light is never made")
        self.assertEqual(self.r["focus"]["icon"]["marks"]["ic-icon"], [], "the white tile behind an icon is not a plate: no words are on it")

    def test_words_on_a_surface_of_their_own_keep_their_colour(self):
        s = self.r["focus"]["badge"]
        self.assertEqual((s["marks"]["bd1"], s["marks"]["badge"]), (["ink=1"], []))
        self.assertEqual(s["color"]["badge"], "rgb(255, 255, 255)")

    def test_a_panel_the_size_of_the_screen_is_not_a_plate(self):
        s = self.r["focus"]["panel"]
        self.assertIsNone(s["found"])
        self.assertEqual(s["marks"]["big"], [])

    def test_when_the_focus_leaves_everything_the_theme_set_comes_off(self):
        a = self.r["focus"]["after"]
        self.assertEqual([i for i, m in a["marks"].items() if m], [])
        self.assertIsNone(a["found"])
        self.assertEqual(a["color"]["pa1"], self.r["focus"]["before"]["color"]["pa1"])
        again = self.r["focus"]["again"]
        self.assertEqual([i for i, m in again["marks"].items() if m], ["pa", "pa1", "pa2", "pa3"], "the item focused last is marked and nothing before it is")

    def test_without_the_look_part_the_focus_is_lgs(self):
        s = self.r["focusOff"]
        self.assertIsNone(s["found"])
        self.assertEqual(s["marks"], {"pa": [], "pa1": []})

    def test_the_canvas_part_paints_the_palettes_canvas_and_the_black_band_with_it(self):
        c = self.r["canvasOn"]
        self.assertEqual((c["html"], c["body"], c["header"]), (self.canvas, self.canvas, self.canvas))
        self.assertTrue(c["marked"])
        self.assertFalse(c["row"], "a row is not a band")

    def test_a_long_transition_is_played_faster_and_still_ends(self):
        self.assertGreater(self.r["stockMs"], 560)
        # frame times are the browser's and a busy workstation stretches them, so the check is the ratio and not a number of milliseconds
        self.assertLess(self.r["cappedMs"], self.r["stockMs"] * 0.7)
        self.assertGreater(self.r["lookOnlyMs"], 560, "without the motion part LG's transitions run as written")

    def test_script_made_animations_are_asked_for_the_cap_and_forever_ones_are_left_alone(self):
        a = self.r["animate"]
        self.assertEqual((a["long"], a["number"], a["short"], a["forever"], a["keeps"]), (260, 260, 100, 800, 30))
        self.assertEqual(self.r["spinRate"], 1, "a spinner is not sped up")

    def test_lazy_pictures_only_when_asked_for(self):
        self.assertEqual(self.r["lazyOff"], ["auto", "auto"])
        self.assertEqual(self.r["lazyOn"], ["lazy", "async", "auto", "div"])

    def test_the_script_runs_once_per_document(self):
        self.assertEqual(self.r["twice"], 1)

    def test_frames_of_the_apps_own_files_are_themed_and_an_anonymous_frame_is_not(self):
        frames = self.r["frames"]
        self.assertIsNone(frames["srcdoc"]["mark"])
        themed = [v for k, v in frames.items() if k != "srcdoc"]
        self.assertTrue(themed and all(v["mark"] == "look,motion" for v in themed))


@unittest.skipUnless(BROWSER, "needs the system Chromium and playwright-core (docs/mocks/render.mjs uses the same)")
class StandInPages(unittest.TestCase):
    def test_the_home_page_in_every_way_the_plate_might_be_built_at_the_larger_text_passes_the_pixel_audit(self):
        """Each page is two software-rendered browser loads (the theme's and LG's), a few seconds on a busy host: the other pages and the 100 % size are
        `node render.mjs`, which writes the mocks, and differ from this by content and not by the way a plate is built."""
        r = subprocess.run(["node", os.path.join(ROOT, "docs", "mocks", "store", "render.mjs"), "--audit-only", "--pages", "home", "--scales", "2"],
                           capture_output=True, text=True, timeout=600)
        try:
            summary = json.loads(r.stdout.strip().splitlines()[-1])
        except (ValueError, IndexError):
            self.fail("no audit: " + r.stderr[-1500:])
        results = summary["results"]
        self.assertEqual(len(results), 6)
        self.assertEqual(summary["failing"], [])
        self.assertEqual(sorted({x["dialect"] for x in results}), ["child", "late", "opaque", "pseudo", "self", "tile"])
        for x in results:
            where = (x["page"], x["dialect"], x["scale"])
            self.assertGreaterEqual(x["plate"]["count"], 1, where)
            self.assertLessEqual(x["plate"]["ms"], 50, where)
            self.assertGreaterEqual(x["ring"]["vsPage"], 3.0, where)
            self.assertGreaterEqual(x["ring"]["vsPlate"], 3.0, where)
            self.assertEqual(x["lowContrast"], [], where)
            self.assertEqual(x["overlaps"], [], where)
            self.assertTrue(x["iconTilesKept"], where)
            if "fade" in x:
                self.assertLessEqual(x["fade"]["themed"], 0.25, "LG's light plate shows while it fades out: %s" % (where,))


if __name__ == "__main__":
    unittest.main()
