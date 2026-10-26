"""Run with: python3 -m unittest tests.test_maliit_patch

tools/maliit_patch.py against a copy of maliit's QML (LG's on-screen keyboard, 85 files as copied from the set in round 11: the census copy on
this workstation, else the archive in docs/research/round9/r11-2026-10-26/): what it changes, that it changes style and nothing LG's code does,
that every colour of the style sheets has a role, that every pair of colours it writes reads (4.5:1 for words, 3:1 for the focus ring and the
accent mark, composited), that a missing or doubled anchor is fatal and leaves the tree as LG's, and (with Xvfb and PyQt5) that the patched
keyboard loads, takes the keys the way LG's does and lays out at 100 % and 200 % text, in the dark and the light material, through the offline host.
"""
import atexit
import difflib
import filecmp
import importlib.util
import json
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
TOOL = os.path.join(ROOT, "tools", "maliit_patch.py")
TOKENS = os.path.join(ROOT, "tools", "tokens.py")
SHIPPED = os.path.join(ROOT, "payload", "theme", "theme.json")
FONTS = os.path.join(ROOT, "payload", "fonts")
HOST = os.path.join(HERE, "qmlhost", "maliit.py")
ARCHIVE = os.path.join(ROOT, "docs", "research", "round9", "r11-2026-10-26", "maliit-qml.tgz")

sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(ROOT, "tools"))
from xdisplay import DISPLAY, HAVE_QT  # noqa: E402
from tokens import contrast, parse  # noqa: E402


def census():
    p = "/data/home/lgtv/uiinv/maliit/qml"
    if os.path.isdir(p):
        return p
    if not os.path.isfile(ARCHIVE):
        return None
    d = tempfile.mkdtemp(prefix="maliit-census-")
    atexit.register(shutil.rmtree, d, True)
    with tarfile.open(ARCHIVE) as t:
        t.extractall(d)
    return os.path.join(d, "qml")


CENSUS = census()

spec = importlib.util.spec_from_file_location("maliit_patch", TOOL)
maliit_patch = importlib.util.module_from_spec(spec)
spec.loader.exec_module(maliit_patch)

S = "keyboard-common/style/"
P = "keyboard-common/style-portrait/"
TOUCHED = sorted(
    ["keyboard-common/FocusPanel.qml", "keyboard-common/RightFunctionArea.qml"]
    + [d + n + ".qml" for d in (S, P) for n in ("ButtonStyle", "FocusStyle", "MoreButtonStyle", "NumberKeyboardStyle", "PageCountButtonStyle", "PanelButtonStyle",
                                                 "PredictionPanelButtonStyle", "PredictionPanelStyle", "Style", "SymbolButtonStyle")]
    + ["view-chinese/ChinesePredictionPanelStyle.qml", "view-chinese/ChineseStyle.qml", "view-chinese/ChineseStylePortrait.qml",
       "view-chinese/NavButtonStyle.qml", "view-chinese/PredictionItemStyle.qml", "view-chinese/HidPredictionItem.qml",
       "view-global/GlobalStyle.qml", "view-global/GlobalStylePortrait.qml", "view-global/MoreButtonStyle.qml", "view-global/HidPredictionItem.qml",
       "view-global/PredictionPanelButtonStyle.qml", "view-global/PredictionPanelStyle.qml",
       "view-japanese/JapanesePredictionPanelStyle.qml", "view-japanese/JapaneseStyle.qml", "view-japanese/JapaneseStylePortrait.qml",
       "view-japanese/HidPredictionItem.qml"])
STYLE_SHEETS = [t for t in TOUCHED if re.search(r"Style[A-Za-z]*\.qml$", t)]

# What LG's code does, as opposed to how it looks: handlers, keys, signals, service reads and calls, focus. A patch that is only style leaves
# every such line exactly as it was (the one function it adds is the face rule).
BEHAVIOUR = re.compile(r"function |Keys\.|\bon[A-Z]\w*\s*:|Connections|signal |forceActiveFocus|KeyNavigation|\bfocus:|\.handle|\.launch|\.speak|Service\b|PluginProxy|Prediction\.")
ADDED_BEHAVIOUR = {"function lumaFamily(langCode, family) {"}


def palette(light=False):
    user = os.devnull
    tmp = None
    if light:
        with open(SHIPPED) as f:
            tmp = tempfile.NamedTemporaryFile("w", suffix=".json", delete=False)
            json.dump({"dark": json.load(f)["light"]}, tmp)
            tmp.close()
        user = tmp.name
    try:
        return json.loads(subprocess.run([sys.executable, TOKENS, SHIPPED, user, "qml"], capture_output=True, text=True, check=True).stdout)
    finally:
        if tmp:
            os.unlink(tmp.name)


def lines(path):
    with open(path) as f:
        return f.read().splitlines()


@unittest.skipUnless(CENSUS and os.path.isdir(CENSUS), "needs the census copy of LG's maliit tree or its archive")
class MaliitPatch(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.dir = os.path.join(self.tmp.name, "qml")
        shutil.copytree(CENSUS, self.dir)
        self.pal = palette()

    def tearDown(self):
        self.tmp.cleanup()

    def run_tool(self, pal=None, fonts=FONTS):
        r = subprocess.run([sys.executable, TOOL, self.dir, json.dumps(pal or self.pal), fonts], capture_output=True, text=True)
        self.stdout = r.stdout
        return r.returncode, r.stderr

    def read(self, rel):
        with open(os.path.join(self.dir, rel)) as f:
            return f.read()

    def write(self, rel, text):
        with open(os.path.join(self.dir, rel), "w") as f:
            f.write(text)

    def stock(self, rel):
        with open(os.path.join(CENSUS, rel)) as f:
            return f.read()

    def differing(self):
        out = []
        for base, _, names in os.walk(CENSUS):
            for n in names:
                rel = os.path.relpath(os.path.join(base, n), CENSUS)
                if not filecmp.cmp(os.path.join(CENSUS, rel), os.path.join(self.dir, rel), shallow=False):
                    out.append(rel)
        return sorted(out)

    def test_patches_the_real_tree(self):
        rc, err = self.run_tool()
        self.assertEqual(rc, 0, err)
        roles = maliit_patch.roles(self.pal)
        style = self.read(S + "Style.qml")
        self.assertTrue(style.startswith("// Luma: patched"))
        for face in ("Regular", "Medium", "SemiBold", "Bold"):
            self.assertIn('FontLoader { source: "file://%s/Manrope-%s.ttf" }' % (os.path.realpath(FONTS), face), style)
        self.assertIn('property string defaultFontFamily: "Manrope"', style)
        self.assertIn('property color bgColor: "%s"' % roles["sheet"], style)
        self.assertIn('property color shiftColor : "%s"' % roles["dotOn"], style)
        self.assertIn('property color bgColor: "%s"' % roles["accent"], style)           # the primary key
        self.assertIn('property color fgColor: "%s"' % roles["accentInk"], style)
        button = self.read(S + "ButtonStyle.qml")
        self.assertIn('property color bgColor: "%s"' % roles["tile"], button)
        self.assertIn('property color fgColor: "%s"' % roles["ink"], button)
        self.assertIn("property int radius: %d * 2 / resolution" % self.pal["rowRadius"], button)
        self.assertIn("property int fontWeight: 57", button)
        self.assertIn('property color bgColor: "%s"' % roles["lit"], self.read(S + "FocusStyle.qml"))
        self.assertIn('property color borderColor: "%s"' % roles["ring"], self.read(S + "FocusStyle.qml"))
        self.assertIn('buttonColor: style.enterButton.normal.bgColor', self.read("keyboard-common/RightFunctionArea.qml"))
        self.assertIn('"%s"' % roles["accent"], self.read(S + "NumberKeyboardStyle.qml"))

    def test_it_changes_the_files_it_names_and_no_other_and_prints_them(self):
        self.run_tool()
        self.assertEqual(self.differing(), TOUCHED)
        self.assertEqual(self.stdout.split("\n")[:-1], TOUCHED)

    def test_every_colour_of_LGs_style_sheets_is_a_role_of_the_palette_or_transparent(self):
        self.run_tool()
        ours = set(maliit_patch.roles(self.pal).values())
        for rel in STYLE_SHEETS:
            for key, b, e, val in maliit_patch.style_colors(self.read(rel)):
                self.assertTrue(val in ours or val == "transparent", "%s: %s is %s" % (rel, key, val))
        # and no stock literal is left anywhere in the tree the patch touched
        stock = {"#3B4045", "#42484E", "#545A64", "#E6E6E6", "#62676A", "#6E7276", "#ABAEB3", "#4C5059", "#248351", "#30AD6B", "#31363A"}
        for rel in TOUCHED:
            self.assertEqual([c for c in stock if c.lower() in self.read(rel).lower()], [], rel)

    def test_only_style_is_changed_and_every_line_of_LGs_behaviour_is_still_there(self):
        self.run_tool()
        for rel in TOUCHED:
            old, new = lines(os.path.join(CENSUS, rel)), lines(os.path.join(self.dir, rel))
            old_b = sorted(l.strip() for l in old if BEHAVIOUR.search(l))
            new_b = sorted(l.strip() for l in new if BEHAVIOUR.search(l) and l.strip() not in ADDED_BEHAVIOUR)
            self.assertEqual(old_b, new_b, rel)
            removed = [l[1:] for l in difflib.unified_diff(old, new, lineterm="", n=0) if l.startswith("-") and not l.startswith("---")]
            for l in removed:
                self.assertIsNone(BEHAVIOUR.search(l), "%s: removes `%s`" % (rel, l.strip()))

    def test_no_live_shadow_comes_in_and_the_focus_boxs_is_off(self):
        self.run_tool()
        self.assertIn("    layer.enabled: true\n    layer.effect: DropShadow {", self.stock("keyboard-common/FocusPanel.qml"))
        self.assertIn("    layer.enabled: false\n    layer.effect: DropShadow {", self.read("keyboard-common/FocusPanel.qml"))
        for rel in TOUCHED:
            old, new = lines(os.path.join(CENSUS, rel)), lines(os.path.join(self.dir, rel))
            added = [l for l in difflib.unified_diff(old, new, lineterm="", n=0) if l.startswith("+") and not l.startswith("+++")]
            for l in added:
                self.assertNotRegex(l, r"DropShadow|layer\.enabled: true|ShaderEffect|Glow|GaussianBlur|FastBlur|OpacityMask", rel)

    def test_the_focus_box_is_opaque_and_ringed_and_covers_the_key_under_it(self):
        for light in (False, True):
            pal = palette(light)
            shutil.rmtree(self.dir)
            shutil.copytree(CENSUS, self.dir)
            self.assertEqual(self.run_tool(pal)[0], 0)
            for rel in (S + "FocusStyle.qml", P + "FocusStyle.qml"):
                text = self.read(rel)
                self.assertRegex(text, r'property color bgColor: "#[0-9A-F]{6}"', rel)         # six digits: alpha 1
                self.assertIn("property int borderWidth: Math.max(1, Math.round(3 * 2 / resolution))", text)
            self.assertIn("    border.width: focusStyle.borderWidth\n    border.color: focusStyle.borderColor\n", self.read("keyboard-common/FocusPanel.qml"))

    def test_every_pair_of_colours_it_writes_reads_composited(self):
        """4.5:1 for every run of words and 3:1 for the marks (the focus ring, the shift dot, the glyphs): in both materials, on the sheet, the
        tile, the recessed tile, the focus fill and the accent key, which is where LG's words and the patch's colours meet."""
        for light in (False, True):
            pal = palette(light)
            r = maliit_patch.roles(pal)

            def c(a, b):
                return contrast(parse(r[a]), parse(r[b]))
            where = "light" if light else "dark"
            for words, ground in (("ink", "tile"), ("ink", "fn"), ("ink", "lit"), ("ink", "sheet"), ("ink", "well"), ("dim", "tile"), ("dim", "sheet"), ("dim", "fn"),
                                  ("accentInk", "accent")):
                self.assertGreaterEqual(c(words, ground), 4.5, "%s on %s, %s material" % (words, ground, where))
            for mark, ground in (("ring", "tile"), ("ring", "lit"), ("ring", "sheet"), ("dotOn", "fn"), ("dotOn", "lit"), ("dotOn", "sheet"), ("accent", "sheet")):
                self.assertGreaterEqual(c(mark, ground), 3.0, "%s on %s, %s material" % (mark, ground, where))

    def test_a_light_sheet_uses_the_dark_glyph_pictures(self):
        self.run_tool()
        self.assertIn("readonly property bool lumaLightSheet: false", self.read(S + "Style.qml"))
        light = palette(light=True)
        shutil.rmtree(self.dir)
        shutil.copytree(CENSUS, self.dir)
        self.assertEqual(self.run_tool(light)[0], 0)
        for rel in (S + "Style.qml", P + "Style.qml"):
            self.assertIn("readonly property bool lumaLightSheet: true", self.read(rel))
        self.assertIn('property color bgColor: "%s"' % maliit_patch.roles(light)["sheet"], self.read(S + "Style.qml"))

    def test_every_glyph_the_patch_swaps_has_both_pictures_at_every_size(self):
        glyphs = re.search(r"lumaGlyphs: \(\{(.*?)\}\)", self.run_and_read_style()).group(1)
        names = re.findall(r"(\w+): 1", glyphs)
        self.assertGreaterEqual(len(names), 9)
        for res in ("hd", "2k", "4k"):
            for n in names:
                for f in (n + ".png", n + "_f.png"):
                    self.assertTrue(os.path.isfile(os.path.join(CENSUS, "keyboard-common", "style", "images", res, f)), "%s/%s" % (res, f))
        # the picture with no pair (the portrait drag handle) is not in the swap
        self.assertNotIn("ic_drag", glyphs)

    def run_and_read_style(self):
        self.assertEqual(self.run_tool()[0], 0)
        return self.read(S + "Style.qml")

    def test_manrope_is_for_latin_locales_only(self):
        style = self.run_and_read_style()
        self.assertIn('LanguageStyle.getStyle(langCode).info.languageClass === "latin" ? "Manrope" : family', style)
        self.assertIn("root.defaultFontFamily = lumaFamily(langCode, getFontForLanguage(langCode));", style)
        for rel in (S + "SymbolButtonStyle.qml", S + "PredictionPanelStyle.qml", "view-chinese/ChinesePredictionPanelStyle.qml"):
            self.assertNotIn("LG Smart UI", self.read(rel), rel)

    def test_the_candidate_bar_of_a_hardware_keyboard_fits_its_word_and_its_label_into_their_own_lines(self):
        self.run_tool()
        for rel in ("view-global", "view-chinese", "view-japanese"):
            text = self.read(rel + "/HidPredictionItem.qml")
            self.assertGreaterEqual(text.count("fontSizeMode: Text.Fit") + text.count("fontSizeMode:Text.Fit"), 2, rel)
        self.assertIn("property int wordHeight: 108 / resolution", self.read("view-global/GlobalStyle.qml"))

    ANCHORS = [
        (S + "Style.qml", 'property string defaultFontFamily: "LG Smart UI"'),
        (S + "Style.qml", "    function getImage(name) {\n        switch(root.resolution) {\n"),
        (S + "Style.qml", "        root.defaultFontFamily = getFontForLanguage(langCode);"),
        (S + "Style.qml", "    property var panelButton: PanelButtonStyle {resolution: root.resolution}\n\n"),
        (P + "Style.qml", "        root.defaultFontFamily = getFontForLanguage(langCode);"),
        (S + "ButtonStyle.qml", "    property int radius: "),
        (S + "FocusStyle.qml", "    property int bottomMargin:  6 / resolution\n"),
        ("keyboard-common/FocusPanel.qml", "    layer.enabled: true\n"),
        ("keyboard-common/FocusPanel.qml", "    color: focusStyle.bgColor\n    radius: focusStyle.radius\n"),
        ("keyboard-common/RightFunctionArea.qml", '            buttonColor: "#248351"\n'),
        ("keyboard-common/RightFunctionArea.qml", '            objectName: "enterButton"\n            buttonStyle: root.buttonStyle\n'),
        ("view-global/HidPredictionItem.qml", "        font.pixelSize: buttonStyle.normal.fontSize\n        elide: Text.ElideRight\n"),
        ("view-chinese/HidPredictionItem.qml", "        font.pixelSize: buttonStyle.hotKey.fontSize\n"),
        ("view-japanese/HidPredictionItem.qml", "        font.pixelSize: buttonStyle.hotKey.fontSize\n        color: buttonStyle.hotKey.fgColor\n"),
    ]

    def test_a_missing_anchor_is_fatal_says_which_file_and_leaves_LGs_tree_as_it_was(self):
        for rel, anchor in self.ANCHORS:
            shutil.rmtree(self.dir)
            shutil.copytree(CENSUS, self.dir)
            text = self.read(rel)
            self.assertEqual(text.count(anchor), 1, "the test's own anchor: " + anchor)
            self.write(rel, text.replace(anchor, anchor.replace(anchor.strip()[-1], "Z", 1) if anchor[-1] == "\n" else anchor[:-1] + "Z"))
            rc, err = self.run_tool()
            self.assertEqual(rc, 1, rel + ": " + anchor)
            self.assertIn("maliit_patch: ", err)
            self.assertIn(os.path.basename(rel), err)
            self.assertIn("anchor found 0 times", err)
            # all or nothing: a late failure writes none of the early files
            self.assertEqual([r for r in self.differing() if r != rel], [], rel)

    def test_a_doubled_anchor_is_fatal_too(self):
        text = self.read("keyboard-common/RightFunctionArea.qml")
        anchor = '            buttonColor: "#248351"\n'
        self.write("keyboard-common/RightFunctionArea.qml", text.replace(anchor, anchor + anchor))
        rc, err = self.run_tool()
        self.assertEqual(rc, 1)
        self.assertIn("anchor found 2 times, expected 1", err)

    def test_a_colour_that_has_no_role_is_fatal_so_a_firmware_update_cannot_slip_one_through(self):
        text = self.read(S + "ButtonStyle.qml")
        self.write(S + "ButtonStyle.qml", text.replace("    property var normal: QtObject {\n", '    property var normal: QtObject {\n        property color glowColor: "#123456"\n', 1))
        rc, err = self.run_tool()
        self.assertEqual(rc, 1)
        self.assertIn("ButtonStyle.qml: normal.glowColor (#123456) has no role", err)
        self.assertEqual(self.differing(), [S + "ButtonStyle.qml"])

    def test_a_sheet_that_had_no_colour_and_got_one_is_fatal_too(self):
        text = self.read(S + "PanelFocusStyle.qml")
        self.write(S + "PanelFocusStyle.qml", text.replace("    widthOffset:", '    property color edgeColor: "#abcdef"\n    widthOffset:', 1))
        rc, err = self.run_tool()
        self.assertEqual(rc, 1)
        self.assertIn("PanelFocusStyle.qml: edgeColor (#abcdef) has no role", err)

    def test_a_role_for_a_colour_that_is_gone_is_fatal_too(self):
        text = self.read(S + "FocusStyle.qml")
        self.write(S + "FocusStyle.qml", text.replace("property color fgColor:", "property color textColor:", 1))
        rc, err = self.run_tool()
        self.assertEqual(rc, 1)
        self.assertRegex(err, r"has no role|roles for colours that are not there")

    def test_style_sheets_that_lost_the_numbers_it_rewrites_are_fatal(self):
        for rel in STYLE_SHEETS:
            if rel.startswith(("keyboard-common", "view-global", "view-chinese", "view-japanese")):
                text = self.read(rel)
                self.write(rel, re.sub(r"borderWidth: 4 / resolution", "borderWidth: 3 / resolution", text))
        rc, err = self.run_tool()
        self.assertEqual(rc, 1)
        self.assertIn("`border` sites found", err)

    def test_a_missing_file_is_fatal(self):
        os.unlink(os.path.join(self.dir, "keyboard-common", "FocusPanel.qml"))
        rc, err = self.run_tool()
        self.assertEqual(rc, 1)
        self.assertIn("FocusPanel.qml: LG's tree has no such file", err)

    def test_patching_twice_is_refused(self):
        self.assertEqual(self.run_tool()[0], 0)
        rc, err = self.run_tool()
        self.assertEqual(rc, 1)
        self.assertIn("already patched", err)

    def test_a_palette_without_what_it_needs_is_refused(self):
        for key in ("sheetBottom", "row", "focusEdge", "hairline", "ink", "inkDim", "accent", "accentInk", "radius", "rowRadius"):
            pal = dict(self.pal)
            del pal[key]
            rc, err = self.run_tool(pal)
            self.assertEqual(rc, 1, key)
            self.assertIn("palette has no " + key, err)
        self.assertEqual(self.differing(), [])

    def test_fonts_that_are_not_there_are_refused_so_the_keys_are_never_drawn_in_a_fallback_face(self):
        empty = os.path.join(self.tmp.name, "fonts")
        os.makedirs(empty)
        rc, err = self.run_tool(fonts=empty)
        self.assertEqual(rc, 1)
        self.assertIn("Manrope-Regular.ttf", err)
        self.assertEqual(self.differing(), [])

    def test_usage(self):
        r = subprocess.run([sys.executable, TOOL, self.dir], capture_output=True, text=True)
        self.assertEqual(r.returncode, 2)


@unittest.skipUnless(CENSUS and os.path.isdir(CENSUS) and DISPLAY and HAVE_QT, "needs the census copy of LG's tree, Xvfb and PyQt5")
class PatchedKeyboardInTheHost(unittest.TestCase):
    """LG's own keyboard QML behind stand-ins for what is native, in Qt 5.15 on software GL: the patched keys must load, warn of nothing new, take
    the keys as LG's do and send what LG's send."""

    SCENES = ("letters", "shift", "symbols", "numbers", "numbers-symbols", "candidates", "candidates-focus", "hid", "focus-enter", "focus-shift",
              "focus-delete", "focus-space", "focus-language", "focus-voice")

    def setUp(self):
        self.shots = tempfile.TemporaryDirectory()
        self.png = os.path.join(self.shots.name, "shot.png")

    def tearDown(self):
        self.shots.cleanup()

    def run_host(self, *args, patched=True):
        r = subprocess.run([sys.executable, HOST] + (["--patched"] if patched else []) + list(args), env=dict(os.environ, DISPLAY=DISPLAY),
                           capture_output=True, text=True, timeout=int(os.environ.get("HOST_TIMEOUT", "180")))
        return r.returncode, r.stdout + r.stderr

    def probe(self, out, name, prop):
        m = re.search(r"\[probe\] %s\.%s=(.*)" % (re.escape(name), re.escape(prop)), out)
        self.assertIsNotNone(m, "no probe %s.%s in: %s" % (name, prop, out[-800:]))
        return m.group(1)

    def sent(self, out):
        return re.findall(r"\[(?:key|keysym|text|luna)\] .*", out)

    def known(self, out):
        m = re.search(r"\[known\] (\d+)", out)
        return int(m.group(1)) if m else None

    def test_every_scene_loads_clean_stock_and_patched_in_both_materials(self):
        for scene in self.SCENES:
            for patched, extra in ((False, []), (True, []), (True, ["--light"])):
                if not patched and scene not in ("letters", "numbers", "candidates", "hid"):
                    continue
                rc, out = self.run_host("--scene", scene, "--wait", "1000", *extra, patched=patched)
                self.assertEqual(rc, 0, "%s patched=%s %s: %s" % (scene, patched, extra, "\n".join(l for l in out.splitlines() if "file://" in l)[-1500:]))

    def test_the_patch_adds_no_warning_of_LGs_kind_either(self):
        """LG's tree has a handful of its own (a radius read off a style that has none, an assignment of `undefined`); the patched tree has the same."""
        for scene in ("letters", "numbers", "candidates", "hid"):
            counts = [self.known(self.run_host("--scene", scene, "--wait", "800", patched=p)[1]) for p in (False, True)]
            self.assertIsNotNone(counts[0])
            self.assertEqual(counts[0], counts[1], scene)

    def test_text_is_drawn_in_manrope_when_patched_and_not_when_stock(self):
        for patched, family in ((True, "Manrope"), (False, '"LG Smart UI"')):
            rc, out = self.run_host("--scene", "letters", "--wait", "600", "--probe", "300:mainGlPlugin:style.defaultFontFamily", "--probe", "300:mainGlPlugin:style.button.fontFamily",
                                    "--probe", "300:mainGlPlugin:style.symbolButton.fontFamily", "--probe", "300:mainGlPlugin:style.predictPanel.panelButton.fontFamily", patched=patched)
            self.assertEqual(rc, 0, out[-800:])
            self.assertEqual(self.probe(out, "mainGlPlugin", "style.defaultFontFamily"), family)
            self.assertEqual(self.probe(out, "mainGlPlugin", "style.button.fontFamily"), family)
            if patched:
                self.assertEqual(self.probe(out, "mainGlPlugin", "style.symbolButton.fontFamily"), "Manrope")
                self.assertEqual(self.probe(out, "mainGlPlugin", "style.predictPanel.panelButton.fontFamily"), "Manrope")

    def test_the_focus_box_is_lit_ringed_opaque_and_has_no_shadow(self):
        rc, out = self.run_host("--scene", "letters", "--wait", "600", "--probe", "300:focusRect:color", "--probe", "300:focusRect:border.width", "--probe", "300:focusRect:layer.enabled",
                                "--probe", "300:focusRect:focusTxt", "--probe", "300:focusRect:visible")
        self.assertEqual(rc, 0, out[-800:])
        roles = maliit_patch.roles(palette())
        self.assertEqual(self.probe(out, "focusRect", "color").lower(), roles["lit"].lower())
        self.assertEqual(self.probe(out, "focusRect", "border.width"), "3")
        self.assertEqual(self.probe(out, "focusRect", "layer.enabled"), "false")
        self.assertEqual((self.probe(out, "focusRect", "visible"), self.probe(out, "focusRect", "focusTxt")), ("true", "g"))
        rc, out = self.run_host("--scene", "letters", "--wait", "600", "--probe", "300:focusRect:color", "--probe", "300:focusRect:layer.enabled", patched=False)
        self.assertEqual((self.probe(out, "focusRect", "color").lower(), self.probe(out, "focusRect", "layer.enabled")), ("#e6e6e6", "true"))

    def test_the_primary_key_is_the_accent_and_says_search_when_the_field_asks_for_it(self):
        rc, out = self.run_host("--scene", "candidates", "--wait", "600", "--probe", "300:enterButton:buttonColor", "--probe", "300:enterButton:text")
        self.assertEqual(rc, 0, out[-800:])
        roles = maliit_patch.roles(palette())
        self.assertEqual(self.probe(out, "enterButton", "buttonColor").lower(), roles["accent"].lower())
        self.assertEqual(self.probe(out, "enterButton", "text"), "Search")

    def test_the_keys_send_what_LGs_send_for_the_same_keys(self):
        """From the default focus (g): a letter, a shifted letter, Search, delete, Clear all, the space bar and a symbol; whatever the stock
        keyboard sends for the same keys, the patched one sends."""
        seqs = {"letter": "OK", "shifted": "DOWN,LEFT,LEFT,LEFT,LEFT,LEFT,OK,RIGHT,OK", "search": "RIGHT,RIGHT,RIGHT,RIGHT,RIGHT,RIGHT,UP,OK",
                "delete": "RIGHT,RIGHT,RIGHT,RIGHT,RIGHT,RIGHT,UP,UP,OK", "clear": "RIGHT,RIGHT,RIGHT,RIGHT,RIGHT,RIGHT,DOWN,OK", "space": "DOWN,DOWN,OK",
                "symbol": "LEFT,LEFT,LEFT,LEFT,LEFT,OK,RIGHT,OK"}
        seen = {}
        for patched in (False, True):
            seen[patched] = {}
            for label, keys in seqs.items():
                rc, out = self.run_host("--scene", "letters", "--wait", "1000", "--press", "300:" + keys, patched=patched)
                self.assertEqual(rc, 0, out[-800:])
                seen[patched][label] = self.sent(out)
        self.assertEqual(seen[False], seen[True])
        self.assertEqual(seen[True]["letter"], ["[key] 103 false 2"])
        self.assertEqual(seen[True]["shifted"], ["[key] 35 true 2"])
        self.assertEqual(seen[True]["search"], ["[key] 28 false 2"])
        self.assertEqual(seen[True]["delete"], ["[key] 14 false 2"])
        self.assertEqual(seen[True]["clear"], ["[luna] clearAll"])
        self.assertEqual(seen[True]["space"], ["[key] 57 false 2"])
        self.assertEqual(seen[True]["symbol"], ["[text] -"])

    def test_the_number_pad_sends_what_LGs_sends(self):
        seen = {}
        for patched in (False, True):
            rc, out = self.run_host("--scene", "numbers", "--wait", "1000", "--press", "300:OK,RIGHT,OK,RIGHT,RIGHT,RIGHT,RIGHT,RIGHT,RIGHT,RIGHT,RIGHT,OK", patched=patched)
            self.assertEqual(rc, 0, out[-800:])
            seen[patched] = self.sent(out)
        self.assertEqual(seen[False], seen[True])
        self.assertTrue(seen[True], "the number pad sent nothing")

    def test_the_shift_key_marks_its_state_with_the_accent(self):
        rc, out = self.run_host("--scene", "shift", "--wait", "1500", "--probe", "1300:shiftButton:dotColor")
        self.assertEqual(rc, 0, out[-800:])
        roles = maliit_patch.roles(palette())
        self.assertEqual(self.probe(out, "shiftButton", "dotColor").lower(), roles["dotOn"].lower())
        rc, out = self.run_host("--scene", "letters", "--wait", "600", "--probe", "300:shiftButton:dotColor")
        self.assertEqual(self.probe(out, "shiftButton", "dotColor").lower(), roles["dotOff"].lower())

    def test_no_text_overlaps_at_100_and_200_percent_text_dark_and_light(self):
        """Lucent's universal rule, run on every restyled screen: no two runs of words on top of each other, and none cut by its key or the window,
        at the size LG draws and at twice it (a label fits its key: `fontSizeMode: Text.Fit`, which is what lets a larger type shrink into its cell)."""
        for scene in ("letters", "symbols", "numbers", "candidates", "hid", "focus-enter", "focus-space"):
            for text in ("100", "200"):
                for extra in ([], ["--light"]):
                    rc, out = self.run_host("--scene", scene, "--wait", "1600", "--text", text, "--audit", "overlap,clipped,overflow", "--shot", "1500:" + self.png, *extra)
                    self.assertEqual(rc, 0, "%s at %s%% %s: %s" % (scene, text, extra, "\n".join(l for l in out.splitlines() if "layout" in l or "warning" in l)[-1500:]))

    def test_every_run_of_words_reads_on_its_key_dark_and_light(self):
        """Composited contrast in the picture itself: 4.5:1 for words under 48 px (the audit's own measure, on the finished pixels)."""
        for scene in ("letters", "symbols", "numbers", "candidates", "hid", "focus-enter", "focus-shift", "focus-space"):
            for extra in ([], ["--light"]):
                rc, out = self.run_host("--scene", scene, "--wait", "1600", "--audit", "contrast", "--shot", "1500:" + self.png, *extra)
                self.assertEqual(rc, 0, "%s %s: %s" % (scene, extra, "\n".join(l for l in out.splitlines() if "layout" in l)[-1500:]))

    def test_a_label_that_is_longer_than_a_key_shrinks_into_it_at_200_percent(self):
        rc, out = self.run_host("--scene", "focus-enter", "--wait", "1600", "--text", "200", "--audit", "overlap,clipped,overflow,elided", "--shot", "1500:" + self.png)
        self.assertEqual(rc, 0, "\n".join(l for l in out.splitlines() if "layout" in l)[-1500:])


if __name__ == "__main__":
    unittest.main()
