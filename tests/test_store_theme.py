"""Run with: python3 -m unittest tests.test_store_theme

The Content Store theme (tools/store_theme.py, payload/store): what the builder writes, what the stylesheet may contain, what the script
does in a real browser, and the stand-in pages of docs/mocks/store drawn with the theme on and checked for overlapping text, contrast and
the focus ring at 100 % and 200 % text in both materials.

The browser parts need the system Chromium and playwright-core (the ones docs/mocks/render.mjs uses) and are skipped without them. The
pages are stand-ins: the store's page is LG's cloud page and is not in the firmware snapshot, so what these prove is the mechanism and the
sheet's own contributions, and not that every selector meets LG's markup (docs/research/store-result.md, step 0).
"""
import base64
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


def build(parts="", material="auto", user="/dev/null"):
    return store_theme.build_script(PAYLOAD, SHIPPED, user, store_theme.parse_parts(parts), material)


def css_of(text):
    """The stylesheet a built script carries, as the script's own JSON string."""
    m = re.search(r'var CSS = ("(?:[^"\\]|\\.)*");', text)
    assert m, "no stylesheet in the script"
    return json.loads(m.group(1))


def strip_comments(css):
    return re.sub(r"/\*.*?\*/", "", css, flags=re.S)


class Parts(unittest.TestCase):
    def test_the_default_is_look_and_motion(self):
        self.assertEqual(store_theme.parse_parts(""), ("look", "motion"))
        self.assertEqual(store_theme.parse_parts("  "), ("look", "motion"))

    def test_parts_come_back_in_canonical_order_commas_or_spaces(self):
        self.assertEqual(store_theme.parse_parts("lazy, look"), ("look", "lazy"))
        self.assertEqual(store_theme.parse_parts("motion look lazy"), ("look", "motion", "lazy"))

    def test_an_unknown_part_is_refused(self):
        with self.assertRaises(ValueError):
            store_theme.parse_parts("look,sparkle")

    def test_lazy_pictures_are_never_on_by_default(self):
        self.assertNotIn("lazy", store_theme.DEFAULT_PARTS)


class Build(unittest.TestCase):
    def test_the_markers_are_filled_and_the_script_parses(self):
        text, options = build()
        for marker in (store_theme.OPTIONS_MARKER, store_theme.CSS_MARKER):
            self.assertNotIn(marker, text)
        self.assertNotIn("@@", text)
        self.assertEqual(options["parts"], ["look", "motion"])
        self.assertEqual(options["capMs"], 180)
        with tempfile.NamedTemporaryFile("w", suffix=".js", delete=False) as f:
            f.write(text)
        try:
            r = subprocess.run(["node", "--check", f.name], capture_output=True, text=True)
            self.assertEqual(r.returncode, 0, r.stderr)
        finally:
            os.unlink(f.name)

    def test_the_font_is_data_and_the_four_static_faces_are_gone(self):
        css = css_of(build()[0])
        self.assertNotIn('url("Manrope-', css)
        m = re.search(r"url\(data:font/woff2;base64,([A-Za-z0-9+/=]+)\)", css)
        self.assertTrue(m)
        data = base64.b64decode(m.group(1))
        self.assertEqual(data[:4], b"wOF2")
        self.assertLess(len(data), 64 * 1024, "the face rides in the script on every page the store loads")
        self.assertEqual(css.count("@font-face"), 1)

    def test_the_ink_is_put_on_every_word_of_the_page_in_the_light_material_only(self):
        dark, light = css_of(build(material="dark")[0]), css_of(build(material="light")[0])
        self.assertNotIn(":where(body, body *)", dark, "the dark canvas keeps the colours the page gave its own words")
        self.assertIn(":where(body, body *)", light)

    def test_the_light_material_is_chosen_by_the_build_not_by_an_attribute(self):
        dark, light = css_of(build(material="dark")[0]), css_of(build(material="light")[0])
        self.assertNotIn('[data-theme="light"]', light)
        self.assertGreater(light.count(":root {"), dark.count(":root {"))
        self.assertIn("color-scheme: light;", light)
        # the dark blocks stay in front: the light ones come after and win where both name a property
        self.assertLess(light.index("color-scheme: dark;"), light.index("color-scheme: light;"))

    def test_the_material_follows_the_theme_then_what_home_decided_then_dark(self):
        with tempfile.TemporaryDirectory() as d:
            user = os.path.join(d, "theme.json")
            with open(user, "w") as f:
                json.dump({"material": "auto"}, f)
            self.assertEqual(build(user=user)[1]["material"], "dark")
            with open(os.path.join(d, "tint.json"), "w") as f:
                json.dump({"light": True, "luma": 0.8}, f)
            self.assertEqual(build(user=user)[1]["material"], "light")
            with open(user, "w") as f:
                json.dump({"material": "dark"}, f)
            self.assertEqual(build(user=user)[1]["material"], "dark")
            self.assertEqual(build(material="light", user=user)[1]["material"], "light")

    def test_without_look_nothing_is_painted_and_no_face_is_carried(self):
        text, options = build("motion")
        self.assertEqual(css_of(text), "")
        self.assertEqual(options["parts"], ["motion"])
        self.assertLess(len(text), 8000)

    def test_lazy_is_in_the_script_only_when_named(self):
        self.assertNotIn("lazy", build()[1]["parts"])
        self.assertIn("lazy", build("look,lazy")[1]["parts"])

    def test_the_focus_ring_keeps_lucents_three_to_one_in_both_materials(self):
        theme, _menu, accent, _ink = tokens.resolve(SHIPPED, "/dev/null")
        for scheme in ("dark", "light"):
            m = theme[scheme]
            ink, canvas, raised = tokens.parse(m["ink"])[:3], tokens.parse(m["canvas"])[:3], tokens.parse(m["glassFocus"])
            a = store_theme.ring_alpha(ink, canvas, raised)
            fill = store_theme.composite(raised[:3], raised[3], canvas)
            ring = store_theme.composite(ink, a, fill)
            self.assertGreaterEqual(tokens.contrast(ring, canvas), 3.0, scheme)
            self.assertGreaterEqual(tokens.contrast(ring, fill), 3.0, scheme)
            self.assertLess(a, 0.8, "a ring that is not nearly the full ink is a ring and not a pill")

    def test_the_popup_sheet_is_opaque(self):
        css = css_of(build()[0])
        for scheme_block in re.findall(r"--st-sheet-solid: (#[0-9A-Fa-f]{6});", css):
            self.assertEqual(len(scheme_block), 7)
        self.assertEqual(len(re.findall(r"--st-sheet-solid:", css)), 2)

    def test_a_broken_payload_is_an_error_and_not_a_script(self):
        with tempfile.TemporaryDirectory() as d:
            for name in ("store.css", "Manrope-store.woff2"):
                with open(os.path.join(PAYLOAD, name), "rb") as src, open(os.path.join(d, name), "wb") as dst:
                    dst.write(src.read())
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
            self.assertEqual([f for f in os.listdir(d)], ["userScript.js"], "no half-written file stays")

    def test_the_tool_prints_the_parts_it_would_use(self):
        r = subprocess.run([sys.executable, TOOL, "parts", ""], capture_output=True, text=True)
        self.assertEqual((r.returncode, r.stdout.strip()), (0, "look,motion"))
        r = subprocess.run([sys.executable, TOOL, "parts", "x"], capture_output=True, text=True)
        self.assertEqual(r.returncode, 2)


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
        for name, body in (("tools/store_theme.py", "x = '/*{{OPTIONS}}*/ null'\n"), ("payload/store/userScript.js", "// {{name}} placeholders\n")):
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

    def test_every_store_tool_is_one_the_build_copies(self):
        with open(BUILD) as f:
            build = f.read()
        tools = [rel for rel in store_files() if rel.startswith("tools/")]
        self.assertGreaterEqual(len(tools), 3)
        for rel in tools:
            self.assertIn(rel, build, "tools/build-ipk.sh does not name " + rel)


class StoreCss(unittest.TestCase):
    """The rules the sheet keeps: presentation only, nothing hidden, nothing heavy, tokens and not constants."""

    @classmethod
    def setUpClass(cls):
        with open(os.path.join(PAYLOAD, "store.css")) as f:
            cls.sheet = strip_comments(f.read())
        with open(os.path.join(PAYLOAD, "store-light.css")) as f:
            cls.light_sheet = strip_comments(f.read())
        cls.built = css_of(build()[0])

    def test_nothing_is_hidden_removed_moved_or_added(self):
        for bad in (r"display\s*:\s*none", r"visibility\s*:\s*hidden", r"opacity\s*:\s*0\s*[;!]", r"pointer-events", r"@import", r"url\(",
                    r"\bposition\s*:\s*fixed", r"\bvisibility\b", r"\bz-index\b", r"\bfloat\b"):
            self.assertIsNone(re.search(bad, self.sheet), bad)
        # nothing is sized but the tab's underline bar, whose height is the token's
        self.assertEqual(re.findall(r"(?<![-\w])(?:min-|max-)?(?:width|height)\s*:\s*([^;]+);", self.sheet), ["var(--st-underline)"])
        # the one generated box is the tab's underline: it carries no words
        self.assertEqual([m.strip() for m in re.findall(r"\bcontent\s*:\s*([^;]+);", self.sheet)], ['""'])

    def test_nothing_costs_a_blur_a_filter_or_an_animation(self):
        for bad in (r"(?<![-\w])filter\s*:", r"mix-blend-mode", r"@keyframes", r"\banimation"):
            self.assertIsNone(re.search(bad, self.sheet), bad)
        # will-change only ever says auto: no layer is promoted for a row, and the ones LG promotes for its focus are let go
        for value in re.findall(r"will-change\s*:\s*([^;]+);", self.sheet):
            self.assertTrue(value.strip().startswith("auto"), value)
        # the one backdrop-filter in the sheet takes the page's away
        self.assertEqual([v.strip() for v in re.findall(r"backdrop-filter\s*:\s*([^;!]+)", self.sheet)], ["none", "none"])

    def test_transitions_are_on_transform_alone_and_never_longer_than_the_card_settle(self):
        for m in re.finditer(r"transition\s*:\s*([^;]+);", self.sheet):
            value = m.group(1).replace("!important", "").strip()
            self.assertTrue(value == "none" or value.startswith("transform "), value)
        theme, *_ = tokens.resolve(SHIPPED, "/dev/null")
        m = theme["motion"]
        self.assertLessEqual(max(m["focusMs"], m["pressMs"], m["restMs"] + m["focusMs"]), m["cardMs"] + m["restMs"])

    def test_the_focus_rule_the_highlight_at_once_and_the_lift_after_the_rest(self):
        # the focused state declares the delayed transition; the resting state declares none, so losing focus cancels a pending lift
        self.assertRegex(self.sheet, r":focus,?[^{]*\{[^}]*transition:\s*transform var\(--lu-focus-ms\) var\(--lu-ease-focus\) var\(--lu-rest-ms\)")
        self.assertRegex(self.sheet, r"a, button \{\s*transition: none;")
        # no colour or outline of the highlight is part of that transition
        for m in re.finditer(r"transition\s*:\s*([^;]+);", self.sheet):
            self.assertNotRegex(m.group(1), r"(background|color|outline|shadow)")

    def test_no_colour_or_size_is_a_constant_in_a_rule(self):
        self.assertIsNone(re.search(r"#[0-9A-Fa-f]{3,8}\b", self.sheet))
        for m in re.finditer(r"rgba?\(([^)]*)\)", self.sheet):
            self.fail("a colour constant: " + m.group(0))
        for m in re.finditer(r"border-radius\s*:\s*([^;]+);", self.sheet):
            self.assertIn("var(--lu-radius-", m.group(1))

    def test_every_variable_the_sheet_reads_is_defined_by_the_build_or_the_sheet(self):
        defined = set(re.findall(r"(--[a-z0-9-]+)\s*:", self.built))
        used = set(re.findall(r"var\((--[a-z0-9-]+)", self.sheet + self.light_sheet))
        self.assertEqual(sorted(used - defined), [])
        self.assertIsNone(re.search(r"#[0-9A-Fa-f]{3,8}\b|rgba?\(", self.light_sheet))

    def test_the_sandstone_names_it_matches_are_ones_lgs_own_build_has(self):
        """The class parts the selectors name, against the compiled CSS of LG's webOS 10 Settings build when this workstation has it."""
        path = "/data/home/lgtv/research/lgsettings/main.css"
        if not os.path.exists(path):
            self.skipTest("LG's Settings stylesheet is not on this workstation")
        with open(path, errors="replace") as f:
            lg = f.read()
        wanted = set(re.findall(r'\[class\*="([A-Za-z]+_[A-Za-z]+)__"\]', self.sheet))
        # components LG's Settings build does not draw but the store's own page might (TabGroup's tabs, Header's title and subtitle,
        # the layer's scrim, a contextual popup): named for the cloud page, [INFERENCE], not checked here
        for_the_cloud_page = {"TabGroup_tab", "TabGroup_bg", "Header_title", "Header_subtitle", "Marquee_text", "ContextualPopup_contextualPopup",
                              "FloatingLayer_scrim", "scrimTranslucent_", "RadioItem_bg", "CheckboxItem_bg", "SwitchItem_bg", "Input_inputArea"}
        missing = sorted(n for n in wanted if n not in for_the_cloud_page and ("_" + n.split("_")[1] + "__") not in lg and n not in lg)
        self.assertEqual(missing, [], "selectors that match nothing in LG's own Sandstone build")


@unittest.skipUnless(BROWSER, "needs the system Chromium and playwright-core (docs/mocks/render.mjs uses the same)")
class InTheBrowser(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        d = cls.tmp.name
        files = {}
        for name, args in (("dark", ["--material", "dark"]), ("light", ["--material", "light"]), ("look", ["--parts", "look"]), ("lazy", ["--parts", "look,motion,lazy"])):
            files[name] = os.path.join(d, name + ".js")
            subprocess.run([sys.executable, TOOL, "build", PAYLOAD, SHIPPED, "/dev/null", files[name]] + args, check=True, capture_output=True)
        r = subprocess.run(["node", os.path.join(HERE, "store_behaviour.mjs"), files["dark"], files["light"], files["look"], files["lazy"]],
                           capture_output=True, text=True, timeout=180)
        if r.returncode != 0:
            raise AssertionError("the browser run failed: " + r.stderr[-1500:])
        cls.r = json.loads(r.stdout)

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def test_the_look_is_in_place_before_the_page_runs_and_the_face_is_loaded(self):
        d = self.r["dark"]
        self.assertEqual((d["mark"], d["attr"], d["sheets"]), ("look,motion", "look,motion", 1))
        self.assertEqual((d["htmlBg"], d["h1"]), ("rgb(25, 23, 36)", "rgb(242, 239, 246)"))
        self.assertEqual(d["bodyBg"], "rgba(0, 0, 0, 0)", "the page paints nothing of its own; the canvas is the root's")
        self.assertTrue(d["family"].startswith("Manrope"))
        self.assertEqual(d["loaded"], 1)
        self.assertEqual(d["errors"], [])

    def test_the_light_material_needs_no_attribute_on_the_page(self):
        light = self.r["light"]
        self.assertEqual((light["htmlBg"], light["h1"], light["scheme"], light["theme"]), ("rgb(250, 244, 237)", "rgb(23, 19, 31)", "light", None))

    def test_a_page_that_forbids_inline_styles_takes_the_theme_all_the_same(self):
        self.assertEqual((self.r["csp"]["htmlBg"], self.r["csp"]["h1"]), ("rgb(25, 23, 36)", "rgb(242, 239, 246)"))

    def test_a_long_transition_is_played_faster_and_still_ends(self):
        self.assertGreater(self.r["stockMs"], 560)
        # frame times are the browser's and a busy workstation stretches them, so the check is the ratio and not a number of milliseconds
        self.assertLess(self.r["cappedMs"], self.r["stockMs"] * 0.7)
        self.assertGreater(self.r["lookOnlyMs"], 560, "without the motion part LG's transitions run as written")

    def test_script_made_animations_are_asked_for_the_cap_and_forever_ones_are_left_alone(self):
        a = self.r["animate"]
        self.assertEqual((a["long"], a["number"], a["short"], a["forever"], a["keeps"]), (180, 180, 100, 800, 30))
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
    def test_every_page_in_both_materials_and_both_text_sizes_has_no_overlap_no_weak_contrast_and_a_ring(self):
        r = subprocess.run(["node", os.path.join(ROOT, "docs", "mocks", "store", "render.mjs"), "--audit-only"], capture_output=True, text=True, timeout=300)
        try:
            summary = json.loads(r.stdout.strip().splitlines()[-1])
        except (ValueError, IndexError):
            self.fail("no audit: " + r.stderr[-1500:])
        self.assertEqual(len(summary["results"]), 20, "five pages, two materials, two text sizes")
        bad = [(x["page"], x["material"], x["scale"], x["overlaps"][:2], x["lowContrast"][:2]) for x in summary["results"] if x["overlaps"] or x["lowContrast"]]
        self.assertEqual(bad, [])
        for x in summary["results"]:
            self.assertGreaterEqual(x["ring"]["width"], 2, (x["page"], x["material"]))
            self.assertGreaterEqual(x["ring"]["vsCanvas"], 3.0, (x["page"], x["material"]))
            self.assertGreaterEqual(x["ring"]["vsFill"], 3.0, (x["page"], x["material"]))
            self.assertGreaterEqual(x["minContrast"], 4.5, (x["page"], x["material"]))


if __name__ == "__main__":
    unittest.main()
