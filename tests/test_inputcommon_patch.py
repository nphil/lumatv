"""Run with: python3 -m unittest tests.test_inputcommon_patch

tools/inputcommon_patch.py against a copy of LG's `com.webos.app.inputcommon` QML (the census copy of the set's banner tree;
skipped when it is not on this workstation): what it changes, that it changes style and nothing LG's code does, that a missing
or doubled anchor is fatal and leaves the tree as LG's, and (with Xvfb and PyQt5) that the patched banners load and still
answer the keys the way LG's do, through the offline host.
"""
import difflib
import filecmp
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
TOOL = os.path.join(ROOT, "tools", "inputcommon_patch.py")
TOKENS = os.path.join(ROOT, "tools", "tokens.py")
SHIPPED = os.path.join(ROOT, "payload", "theme", "theme.json")
FONTS = os.path.join(ROOT, "payload", "fonts")
HOST = os.path.join(HERE, "qmlhost", "inputcommon.py")
CENSUS = next((p for p in ("/data/home/lgtv/uiinv/inputcommon/qml", "/tmp/tvsrc/inputcommon/qml") if os.path.isdir(p)), "/data/home/lgtv/uiinv/inputcommon/qml")

sys.path.insert(0, HERE)
from xdisplay import DISPLAY, HAVE_QT  # noqa: E402

BANNER = "UserInterfaceLayer/Containers/MainViewLayer/"
STYLE = "UserInterfaceLayer/GlobalStyle.qml"
TOUCHED = [STYLE, BANNER + "InputBanner/SimpleBanner.qml", BANNER + "InputBanner/InputBanner.qml", BANNER + "InputBanner/DetailInfoBanner.qml",
           BANNER + "BannerItems/OptionButton.qml", BANNER + "BannerItems/BannerIcon.qml", BANNER + "BannerItems/NsuProgressBar.qml"]

# What LG's code does, as opposed to how it looks: handlers, keys, interface reads and calls, focus. A patch that is only
# style leaves every such line exactly as it was.
BEHAVIOUR = re.compile(r"function |Keys\.|interfaces\[|\bon[A-Z]\w*\s*:|Connections|signal |forceActiveFocus|KeyNavigation|\bfocus:|\.handle|\.launch|\.hide")


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


@unittest.skipUnless(os.path.isdir(CENSUS), "needs the census copy of LG's inputcommon tree")
class InputcommonPatch(unittest.TestCase):
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
        style = self.read(STYLE)
        self.assertTrue(style.startswith("// Luma: patched"))
        self.assertNotRegex(style, r'"family": fontProp\("(smart|cond|number)_')
        self.assertGreaterEqual(style.count('"family": "Manrope"'), 6)
        # the one face LG's style sheet still names is the dingbat font: glyphs (the Dolby marks), not letters
        self.assertIn('fontProp("dingbat_Regular", "family")', style)
        for face in ("Regular", "Medium", "SemiBold", "Bold"):
            self.assertIn('FontLoader { source: "file://%s/Manrope-%s.ttf" }' % (os.path.realpath(FONTS), face), style)
        self.assertIn('property string inputColor: "%s"' % self.pal["ink"], style)
        self.assertIn('property string infoBannerColor: "%s"' % self.pal["sheetBottom"], style)
        self.assertIn("property int infoBannerRadius: %d" % self.pal["radius"], style)
        self.assertIn('property string inputBannerEdge: "%s"' % self.pal["hairline"], style)
        self.assertIn('DINGBAT_COLOR: "%s"' % self.pal["ink"], style)
        self.assertNotIn('infoBannerColor: "#575E66"', style)
        self.assertNotIn('nsuBgColor: "#575E66"', style)

    def test_it_changes_the_seven_files_it_names_and_no_other_and_prints_them(self):
        self.run_tool()
        self.assertEqual(self.differing(), sorted(TOUCHED))
        self.assertEqual(self.stdout.split("\n")[:-1], sorted(TOUCHED))

    def test_only_style_is_removed_and_every_line_of_LGs_behaviour_is_still_there(self):
        self.run_tool()
        for rel in TOUCHED:
            old, new = lines(os.path.join(CENSUS, rel)), lines(os.path.join(self.dir, rel))
            self.assertEqual(sorted(l for l in old if BEHAVIOUR.search(l)), sorted(l for l in new if BEHAVIOUR.search(l)), rel)
            removed = [l[1:] for l in difflib.unified_diff(old, new, lineterm="", n=0) if l.startswith("-") and not l.startswith("---")]
            for l in removed:
                self.assertIsNone(BEHAVIOUR.search(l), "%s: removes `%s`" % (rel, l.strip()))

    def test_the_pictures_stay_and_only_lose_their_source(self):
        self.run_tool()
        for rel in (BANNER + "InputBanner/SimpleBanner.qml", BANNER + "InputBanner/InputBanner.qml"):
            text = self.read(rel)
            self.assertEqual(text.count("id: bgMiddle"), 1)
            self.assertEqual(text.count("id: bgRight"), 1)
            self.assertEqual(text.count('source: ""'), 2, rel)
        # LG's own rule for when its background shows is the rule the new one follows: the pill where it showed alone, the
        # lit inset where it showed focused
        self.assertIn("visible: root.needFocusHandle || root.displayOnlyZapping", self.stock(BANNER + "InputBanner/SimpleBanner.qml"))
        patched = self.read(BANNER + "InputBanner/SimpleBanner.qml")
        self.assertIn("visible: root.displayOnlyZapping", patched)
        self.assertIn("visible: root.needFocusHandle", patched)

    def test_no_live_shadow_comes_in_and_the_two_that_were_there_are_off(self):
        self.run_tool()
        self.assertIn("shadowVisible: false", self.read(BANNER + "BannerItems/OptionButton.qml"))
        self.assertIn("        active: false\n", self.read(BANNER + "BannerItems/NsuProgressBar.qml"))
        for rel in TOUCHED:
            old, new = lines(os.path.join(CENSUS, rel)), lines(os.path.join(self.dir, rel))
            added = [l for l in difflib.unified_diff(old, new, lineterm="", n=0) if l.startswith("+") and not l.startswith("+++")]
            for l in added:
                self.assertNotRegex(l, r"DropShadow|layer\.enabled|ShaderEffect|Glow|GaussianBlur|FastBlur|OpacityMask", rel)

    def test_the_options_glyph_is_the_one_that_reads_on_the_sheet(self):
        self.run_tool()
        style = self.read(STYLE)
        self.assertIn('optionFocusedImg: Qt.resolvedUrl("./Images/banner/option/ic_moreoption_n.png")', style)
        light = palette(light=True)
        shutil.rmtree(self.dir)
        shutil.copytree(CENSUS, self.dir)
        self.assertEqual(self.run_tool(light)[0], 0)
        style = self.read(STYLE)
        self.assertIn('optionImg: Qt.resolvedUrl("./Images/banner/option/ic_moreoption_f.png")', style)
        self.assertIn('optionFocusedImg: Qt.resolvedUrl("./Images/banner/option/ic_moreoption_f.png")', style)
        self.assertIn('property string inputBannerColor: "%s"' % light["sheetBottom"], style)

    ANCHORS = [
        (STYLE, "property int inputSimpleMinWidth: 582"),
        (STYLE, 'BG_COLOR: "#CCCCCC"'),
        (BANNER + "InputBanner/SimpleBanner.qml", "property int sideMargin: styleSheet.banner.zappingBannerSideMargin"),
        (BANNER + "InputBanner/InputBanner.qml", 'source: root.visible ? styleSheet.banner.rightBg : ""'),
        (BANNER + "InputBanner/DetailInfoBanner.qml", "leftMargin: styleSheet.banner.infoBannerOutSideMargin"),
        (BANNER + "BannerItems/OptionButton.qml", "buttonType: s_type"),
        (BANNER + "BannerItems/BannerIcon.qml", 'text: iconVisible && dingbatEnable ? dingbat : ""'),
        (BANNER + "BannerItems/NsuProgressBar.qml", "        target: bg\n"),
    ]

    def test_a_missing_anchor_is_fatal_says_which_file_and_leaves_LGs_tree_as_it_was(self):
        for rel, anchor in self.ANCHORS:
            shutil.rmtree(self.dir)
            shutil.copytree(CENSUS, self.dir)
            text = self.read(rel)
            self.assertEqual(text.count(anchor), 1, "the test's own anchor: " + anchor)
            self.write(rel, text.replace(anchor, anchor[:-1] + "Z"))
            rc, err = self.run_tool()
            self.assertEqual(rc, 1, rel)
            self.assertIn(os.path.basename(rel), err)
            self.assertIn("anchor found 0 times", err)
            # all or nothing: a late failure writes none of the early files
            self.assertEqual([r for r in self.differing() if r != rel], [], rel)

    def test_a_doubled_anchor_is_fatal_too(self):
        text = self.read(STYLE)
        self.write(STYLE, text.replace("property int inputSimpleMinWidth: 582\n", "property int inputSimpleMinWidth: 582\n        property int inputSimpleMinWidth: 582\n"))
        rc, err = self.run_tool()
        self.assertEqual(rc, 1)
        self.assertIn("anchor found 2 times, expected 1", err)

    def test_a_style_sheet_that_lost_its_font_functions_is_fatal(self):
        self.write(STYLE, re.sub(r'"family": fontProp\(', '"family": fontPropX(', self.read(STYLE)))
        rc, err = self.run_tool()
        self.assertEqual(rc, 1)
        self.assertIn("font sites found", err)

    def test_a_missing_file_is_fatal(self):
        os.unlink(os.path.join(self.dir, BANNER + "BannerItems/BannerIcon.qml"))
        rc, err = self.run_tool()
        self.assertEqual(rc, 1)
        self.assertIn("BannerIcon.qml: LG's tree has no such file", err)

    def test_patching_twice_is_refused(self):
        self.assertEqual(self.run_tool()[0], 0)
        rc, err = self.run_tool()
        self.assertEqual(rc, 1)
        self.assertIn("already patched", err)

    def test_a_palette_without_what_it_needs_is_refused(self):
        for key in ("sheetBottom", "focus", "focusEdge", "hairline", "ink", "inkDim", "focusInk", "accent", "radius", "rowRadius"):
            pal = dict(self.pal)
            del pal[key]
            rc, err = self.run_tool(pal)
            self.assertEqual(rc, 1, key)
            self.assertIn("palette has no " + key, err)
        self.assertEqual(self.differing(), [])

    def test_fonts_that_are_not_there_are_refused_so_the_banner_is_never_drawn_in_a_fallback_face(self):
        empty = os.path.join(self.tmp.name, "fonts")
        os.makedirs(empty)
        rc, err = self.run_tool(fonts=empty)
        self.assertEqual(rc, 1)
        self.assertIn("Manrope-Regular.ttf", err)
        self.assertEqual(self.differing(), [])

    def test_usage(self):
        r = subprocess.run([sys.executable, TOOL, self.dir], capture_output=True, text=True)
        self.assertEqual(r.returncode, 2)


@unittest.skipUnless(os.path.isdir(CENSUS) and DISPLAY and HAVE_QT, "needs the census copy of LG's tree, Xvfb and PyQt5")
class PatchedBannersInTheHost(unittest.TestCase):
    """LG's own banner components behind stand-ins for what is native, in Qt 5.15 on software GL: the patched banners must
    load, warn of nothing, take the keys as LG's do and call what LG's call."""

    def run_host(self, *args, patched=True):
        r = subprocess.run([sys.executable, HOST] + (["--patched"] if patched else []) + list(args), env=dict(os.environ, DISPLAY=DISPLAY),
                           capture_output=True, text=True, timeout=120)
        return r.returncode, r.stdout + r.stderr

    def probe(self, out, name, prop):
        m = re.search(r"\[probe\] %s\.%s=(.*)" % (re.escape(name), re.escape(prop)), out)
        self.assertIsNotNone(m, "no probe %s.%s in: %s" % (name, prop, out[-800:]))
        return m.group(1)

    def luna(self, out):
        return re.findall(r"\[luna\] (\w+)", out)

    def test_every_scene_loads_clean_stock_and_patched_in_both_materials(self):
        for scene in ("pill", "focused-pill", "ok", "detail", "update"):
            for patched, extra in ((False, []), (True, []), (True, ["--light"])):
                rc, out = self.run_host("--scene", scene, "--wait", "900", *extra, patched=patched)
                self.assertEqual(rc, 0, "%s patched=%s %s: %s" % (scene, patched, extra, out[-1200:]))

    def test_the_pill_is_88_high_where_LGs_is_156(self):
        for patched, height in ((False, "156"), (True, "88")):
            rc, out = self.run_host("--scene", "pill", "--wait", "900", "--probe", "700:simpleBanner:height", "--probe", "700:inputBannerView:height", patched=patched)
            self.assertEqual(rc, 0, out[-800:])
            self.assertEqual(self.probe(out, "simpleBanner", "height"), height)
            self.assertEqual(self.probe(out, "inputBannerView", "height"), height)

    def test_the_detail_card_has_its_own_size(self):
        rc, out = self.run_host("--scene", "detail", "--wait", "900", "--probe", "700:inputDetailInfoBanner:width", "--probe", "700:inputDetailInfoBanner:height")
        self.assertEqual(rc, 0, out[-800:])
        self.assertEqual((self.probe(out, "inputDetailInfoBanner", "width"), self.probe(out, "inputDetailInfoBanner", "height")), ("1100", "184"))

    def test_text_is_drawn_in_manrope_when_patched_and_not_when_stock(self):
        for patched, family in ((True, "Manrope"), (False, "LG Smart UI")):
            rc, out = self.run_host("--scene", "pill", "--wait", "900", "--probe", "700:inputName:font.family", "--probe", "700:inputLabel:font.family", patched=patched)
            self.assertEqual(rc, 0, out[-800:])
            self.assertEqual(self.probe(out, "inputName", "font.family"), family)
            self.assertEqual(self.probe(out, "inputLabel", "font.family"), family)

    def test_the_keys_do_what_LGs_do_and_call_what_LGs_call(self):
        """The OK banner opens focused on the input name; Right moves to the options key; OK on each calls its own method.
        Whatever the stock banner does for the same keys, the patched one does."""
        seen = {}
        for patched in (False, True):
            runs = {}
            for label, keys in (("name", "OK"), ("options", "RIGHT,OK"), ("there and back", "RIGHT,LEFT,OK")):
                rc, out = self.run_host("--scene", "ok", "--wait", "1800", "--press", "600:" + keys, "--probe", "1500:option:activeFocus",
                                        "--probe", "1500:simpleBanner:activeFocus", patched=patched)
                self.assertEqual(rc, 0, out[-800:])
                runs[label] = (self.luna(out), self.probe(out, "option", "activeFocus"), self.probe(out, "simpleBanner", "activeFocus"))
            seen[patched] = runs
        self.assertEqual(seen[False], seen[True])
        self.assertEqual(seen[True]["name"], (["handleBannerPressed"], "false", "true"))
        self.assertEqual(seen[True]["options"], (["launchMoreActions"], "true", "false"))
        self.assertEqual(seen[True]["there and back"], (["handleBannerPressed"], "false", "true"))

    def test_focus_is_lit_on_the_item_that_has_it(self):
        rc, out = self.run_host("--scene", "ok", "--wait", "1500", "--press", "600:RIGHT", "--probe", "1200:option:hasFocus", "--probe", "1200:simpleBanner:hasFocus")
        self.assertEqual(rc, 0, out[-800:])
        self.assertEqual((self.probe(out, "option", "hasFocus"), self.probe(out, "simpleBanner", "hasFocus")), ("true", "false"))
        rc, out = self.run_host("--scene", "ok", "--wait", "1500", "--probe", "1200:option:hasFocus", "--probe", "1200:simpleBanner:hasFocus")
        self.assertEqual((self.probe(out, "option", "hasFocus"), self.probe(out, "simpleBanner", "hasFocus")), ("false", "true"))


if __name__ == "__main__":
    unittest.main()
