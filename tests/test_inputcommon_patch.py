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
LUMA = "UserInterfaceLayer/Components/LumaStyle.qml"   # new: the patched banners' own style, not a file of LG's
BG = "UserInterfaceLayer/Containers/BgViewLayer/"
TOUCHED = [STYLE, BANNER + "InputBanner/SimpleBanner.qml", BANNER + "InputBanner/InputBanner.qml", BANNER + "InputBanner/DetailInfoBanner.qml",
           BANNER + "BannerItems/OptionButton.qml", BANNER + "BannerItems/BannerIcon.qml", BANNER + "BannerItems/NsuProgressBar.qml",
           BANNER + "ChannelBanner/ZappingBanner.qml", BANNER + "ChannelBanner/ChannelBanner.qml", BANNER + "ChannelBanner/DetailInfoBanner.qml",
           BANNER + "BannerItems/BannerEditItem.qml", BANNER + "BannerEditView.qml",
           "UserInterfaceLayer/Components/SimpleButton.qml", "UserInterfaceLayer/Containers/PopupLayer/AlertPopupView.qml",
           BG + "Screensaver.qml", BG + "ScreensaverMainText.qml", BG + "ScreensaverSubText.qml",
           # not edited for their looks: they only read, from LumaStyle, what the patch sets (the clock, the progress bars, the channel card's lines...)
           BANNER + "BannerItems/BannerClock.qml", BANNER + "BannerItems/BannerListView.qml", BANNER + "BannerItems/BannerTextIcon.qml",
           BANNER + "BannerItems/DetailInfoItem.qml", BANNER + "BannerItems/ProgressBar.qml", BANNER + "BannerItems/RecInfoIcon.qml",
           BANNER + "ChannelBanner/MiniBanner.qml", BANNER + "InputBanner/MiniBanner.qml", BANNER + "DvrInfoView.qml", BANNER + "SimilarListView.qml"]

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
        self.assertIn('property string inputBannerEdge: "%s"' % self.pal["hairline"], self.read(LUMA))
        self.assertNotIn("inputBannerEdge", style)
        self.assertIn('DINGBAT_COLOR: "%s"' % self.pal["ink"], style)
        self.assertNotIn('infoBannerColor: "#575E66"', style)
        self.assertNotIn('nsuBgColor: "#575E66"', style)

    def test_it_changes_the_files_it_names_and_no_other_and_prints_them(self):
        self.run_tool()
        self.assertEqual(self.differing(), sorted(TOUCHED))
        self.assertEqual(self.stdout.split("\n")[:-1], sorted(TOUCHED + [LUMA]))

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
        for rel in (BANNER + "InputBanner/SimpleBanner.qml", BANNER + "InputBanner/InputBanner.qml", BANNER + "ChannelBanner/ZappingBanner.qml",
                    BANNER + "ChannelBanner/ChannelBanner.qml", BANNER + "BannerItems/BannerEditItem.qml"):
            text = self.read(rel)
            self.assertEqual(text.count("id: bgMiddle"), 1)
            self.assertEqual(text.count("id: bgRight"), 1)
            self.assertEqual(text.count('source: ""'), 2, rel)
        # LG's own rule for when its background shows is the rule the new one follows: the pill where it showed alone, the
        # lit inset where it showed focused
        self.assertIn("visible: root.needFocusHandle || root.displayOnlyZapping", self.stock(BANNER + "InputBanner/SimpleBanner.qml"))
        for rel in ("InputBanner/SimpleBanner.qml", "ChannelBanner/ZappingBanner.qml"):
            patched = self.read(BANNER + rel)
            self.assertIn("visible: root.displayOnlyZapping", patched, rel)
        self.assertIn("visible: root.needFocusHandle", self.read(BANNER + "InputBanner/SimpleBanner.qml"))
        self.assertIn("visible: root.hasFocusImage", self.read(BANNER + "ChannelBanner/ZappingBanner.qml"))

    def test_the_no_signal_screen_keeps_its_mover_and_its_off_timer_and_loses_the_animation(self):
        self.run_tool()
        text = self.read(BG + "Screensaver.qml")
        self.assertNotIn("screensaver12frame.gif", text)
        # LG's burn-in guard: the block hops to a new place on a timer, the ten seconds LG chose
        self.assertIn("moveDelay: isPhotoRunning ? 50 : 10000", text)
        self.assertIn("isRepeatedMoving: !isPhotoRunning", text)
        self.assertIn("isMoving: isDynamic && systemProperties.isAppActivated", text)
        # one ink for words on the black, one on the sheet of the photo mode
        self.assertIn("lumaStyle.noSignalColor", self.read(BG + "ScreensaverMainText.qml"))
        self.assertIn("lumaStyle.noSignalDimColor", self.read(BG + "ScreensaverSubText.qml"))

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
        self.assertIn('property string inputBannerColor: "%s"' % light["sheetBottom"], self.read(LUMA))

    ANCHORS = [
        (STYLE, "property int inputSimpleMinWidth: 582"),
        (STYLE, 'BG_COLOR: "#CCCCCC"'),
        (STYLE, 'TEXT_KEYHELP_TEXT_COLOR: "#E6E6E6"'),
        (STYLE, 'bodyTextColor: "#000000"'),
        (STYLE, "property int zappingBannerHeight: 156"),
        (BANNER + "InputBanner/SimpleBanner.qml", "property int sideMargin: styleSheet.banner.zappingBannerSideMargin"),
        (BANNER + "InputBanner/InputBanner.qml", 'source: root.visible ? styleSheet.banner.rightBg : ""'),
        (BANNER + "InputBanner/DetailInfoBanner.qml", "leftMargin: styleSheet.banner.infoBannerOutSideMargin"),
        (BANNER + "BannerItems/OptionButton.qml", "buttonType: s_type"),
        (BANNER + "BannerItems/BannerIcon.qml", 'text: iconVisible && dingbatEnable ? dingbat : ""'),
        (BANNER + "BannerItems/NsuProgressBar.qml", "        target: bg\n"),
        (BANNER + "ChannelBanner/ZappingBanner.qml", 'source: root.visible ? (hasFocusImage ? styleSheet.banner.rightFocusedBg : styleSheet.banner.rightBg) : ""'),
        (BANNER + "ChannelBanner/ChannelBanner.qml", 'source: root.visible ? styleSheet.banner.middleBg : ""'),
        (BANNER + "ChannelBanner/DetailInfoBanner.qml", "leftMargin: styleSheet.banner.infoBannerOutSideMargin"),
        (BANNER + "BannerItems/BannerEditItem.qml", "    height: bgRight.height\n"),
        (BANNER + "BannerEditView.qml", "        x: 0\n        y: 0\n        useMirroring: true\n"),
        ("UserInterfaceLayer/Components/SimpleButton.qml", 'bgFocusedColor: "#e6e6e6"'),
        ("UserInterfaceLayer/Containers/PopupLayer/AlertPopupView.qml", "    ImageShadow {\n        target: alertBackground\n    }\n"),
        (BG + "Screensaver.qml", 'source: visible && !systemProperties.isBNOTV? "../../Images/screensaver/screensaver12frame.gif":""'),
        (BG + "ScreensaverMainText.qml", '    color: "#E6E6E6"\n'),
        (BG + "ScreensaverSubText.qml", "font.pixelSize: styleSheet.fontSize(35)"),
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
        for key in ("sheetBottom", "row", "focus", "focusEdge", "hairline", "ink", "inkDim", "focusInk", "accent", "radius", "rowRadius"):
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

    def test_a_patched_banner_asks_the_sheet_only_for_what_LG_ships(self):
        """A process restored from a CRIU dump has the stock sheet in memory (round 17), so a patched banner file that reads a name only the
        patched sheet has gets undefined: its own style comes from LumaStyle."""
        self.run_tool()
        stock_names = set(re.findall(r"property\s+[\w<>]+\s+(\w+)\s*:", self.stock(STYLE)))
        own = set(re.findall(r"property\s+\w+\s+(\w+)\s*:", self.read(LUMA)))
        for rel in TOUCHED[1:]:
            text = self.read(rel)
            for name in re.findall(r"styleSheet\.banner\.(\w+)", text):
                self.assertIn(name, stock_names, "%s reads styleSheet.banner.%s, which LG's sheet does not have" % (rel, name))
            for name in re.findall(r"lumaStyle\.(\w+)", text):
                self.assertIn(name, own, "%s reads lumaStyle.%s, which LumaStyle.qml does not define" % (rel, name))
            if "lumaStyle." in text:
                self.assertIn("LumaStyle { id: lumaStyle }", text, rel)

    def test_no_file_reads_from_the_style_sheet_a_value_the_patch_changed(self):
        """Round 18 on the set: the INFO cards were LG's flat grey sheet with LG's inks, because the patched card read `infoBannerColor`,
        `infoBannerRadius` and `infoBannerOpacity` from the sheet: LG's file has those names, so the check above passed, and a restored process
        holds LG's values for them. A name the patch sets is read from LumaStyle by every file, whichever way it is spelled."""
        self.run_tool()

        def banner_block(text):
            return text[text.index("property var banner: QtObject {"):text.index("    property var dvr: ({")]

        def props(text):
            return dict(re.findall(r"^\s*property\s+[\w<>]+\s+(\w+)\s*:\s*(.*)$", banner_block(text), re.M))
        def block(text, name):
            return re.search(r"property var %s: \(\{.*?\}\)" % name, text, re.S).group(0)
        stock, patched = props(self.stock(STYLE)), props(self.read(STYLE))
        changed = {n for n in patched if stock.get(n) != patched[n]}
        # a font built by the sheet's font functions has the same line in both sheets and another face in them
        changed |= {n for n, e in patched.items() if re.match(r"(light|regular|semiBold|bold|number|condRegular)Font\(\d+\)$", e.strip())}
        changed |= {n for n in ("bannerIcon", "bannerTextIcon") if block(self.stock(STYLE), n) != block(self.read(STYLE), n)}
        changed = sorted(changed)
        self.assertGreater(len(changed), 40, "the test's own list of what the patch changes")
        for needed in ("infoBannerColor", "infoBannerRadius", "infoBannerOpacity", "clockTextColor", "infoDescColor", "infoDescFont", "bannerIcon", "bannerTextIcon"):
            self.assertIn(needed, changed)
        names = "|".join(changed)
        own = set(re.findall(r"property\s+\w+\s+(\w+)\s*:", self.read(LUMA)))
        self.assertEqual(sorted(set(changed) - own), [], "LumaStyle does not carry what the patch changed")
        for base, _, files in os.walk(self.dir):
            for f in files:
                rel = os.path.relpath(os.path.join(base, f), self.dir)
                if not f.endswith(".qml") or rel in (STYLE, LUMA):
                    continue
                text = self.read(rel)
                self.assertIsNone(re.search(r"styleSheet\.banner\.(%s)\b" % names, text), rel)
                self.assertIsNone(re.search(r"ChUtils\.getChannel(Name)?ColorObj\(styleSheet\.banner,", text), rel)
                for alias in re.findall(r"property var (\w+): styleSheet\.banner\n", text):
                    self.assertIsNone(re.search(r"\b%s\.(%s)\b" % (alias, names), text), "%s reads `%s.` a value the patch changed" % (rel, alias))

    def test_the_no_signal_words_read_on_black_in_both_materials(self):
        """4.5:1 for the line and its dimmer partner, composited on black (the panel's dark; the burn-in guard is why they are not brighter)."""
        for light in (False, True):
            pal = palette(light)
            self.assertEqual(self.run_tool(pal)[0], 0)
            style = self.read(LUMA)
            for name in ("noSignalColor", "noSignalDimColor"):
                m = re.search(r'property string %s: "#([0-9A-F]{2})([0-9A-F]{2})([0-9A-F]{2})([0-9A-F]{2})"' % name, style)
                self.assertIsNotNone(m, name)
                a, r, g, b = (int(v, 16) for v in m.groups())
                # alpha over black, in the colour space the panel is driven in, then the relative luminance
                lin = [((c * a / 255 / 255) / 12.92 if (c * a / 255 / 255) <= 0.04045 else (((c * a / 255 / 255) + 0.055) / 1.055) ** 2.4) for c in (r, g, b)]
                over_black = 0.2126 * lin[0] + 0.7152 * lin[1] + 0.0722 * lin[2]
                self.assertGreaterEqual((over_black + 0.05) / 0.05, 4.5, "%s in the %s material" % (name, "light" if light else "dark"))
            shutil.rmtree(self.dir)
            shutil.copytree(CENSUS, self.dir)


@unittest.skipUnless(os.path.isdir(CENSUS) and DISPLAY and HAVE_QT, "needs the census copy of LG's tree, Xvfb and PyQt5")
class PatchedBannersInTheHost(unittest.TestCase):
    """LG's own banner components behind stand-ins for what is native, in Qt 5.15 on software GL: the patched banners must
    load, warn of nothing, take the keys as LG's do and call what LG's call."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()

    def tearDown(self):
        self.tmp.cleanup()

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

    def stale(self, scene, *probes, wait="900"):
        args = ["--scene", scene, "--stale-sheet", "--wait", wait]
        for p in probes:
            args += ["--probe", "700:" + p]
        return self.run_host(*args)[1]

    def test_the_pill_draws_its_glass_when_the_style_sheet_is_LGs_own(self):
        """Round 17 on the set: LG restores the input app from a CRIU dump, so the process held the stock style sheet and read the patched
        banner from the overlay. Every name the patched sheet adds was undefined: the pill was 0 high and its words sat bare at x 0."""
        out = self.stale("pill", "lumaPill:visible", "lumaPill:width", "lumaPill:height", "lumaPill:color.a", "lumaPill:x",
                         "inputName:inside:lumaPill", "inputLabel:inside:lumaPill", "simpleBanner:height", "lumaBar:height")
        self.assertEqual(self.probe(out, "lumaPill", "visible"), "true")
        self.assertEqual(self.probe(out, "lumaPill", "height"), "88")
        self.assertGreater(int(self.probe(out, "lumaPill", "width")), 320 - 1)
        self.assertGreater(float(self.probe(out, "lumaPill", "color.a")), 0.5)
        self.assertEqual(self.probe(out, "lumaBar", "height"), "88")
        self.assertEqual(self.probe(out, "simpleBanner", "height"), "88")
        self.assertEqual(self.probe(out, "inputName", "inside:lumaPill"), "true")
        self.assertEqual(self.probe(out, "inputLabel", "inside:lumaPill"), "true")

    def test_the_lit_inset_and_the_channel_strip_survive_the_stock_sheet_too(self):
        out = self.stale("focused-pill", "lumaFocus:visible", "lumaFocus:inside:lumaPill", "lumaFocus:height")
        self.assertEqual(self.probe(out, "lumaFocus", "visible"), "true")
        self.assertEqual(self.probe(out, "lumaFocus", "inside:lumaPill"), "true")
        self.assertGreater(int(self.probe(out, "lumaFocus", "height")), 0)
        out = self.stale("channel", "lumaPill:height", "lumaPill:width", "lumaPill:color.a")
        self.assertGreater(int(self.probe(out, "lumaPill", "height")), 0)
        self.assertGreater(int(self.probe(out, "lumaPill", "width")), 0)
        self.assertGreater(float(self.probe(out, "lumaPill", "color.a")), 0.5)

    CARD_PROBES = ("lumaCard:color.r", "lumaCard:color.g", "lumaCard:color.b", "lumaCard:color.a", "lumaCard:radius", "lumaCard:opacity",
                   "lumaCard:border.color.a", "time:color.r", "time:color.g", "time:color.b", "time:color.a", "time:font.family", "time:font.pixelSize",
                   "resolution:iconColor", "ratio:iconColor")
    CHANNEL_CARD_PROBES = ("eventDescription:color.r", "eventDescription:color.a", "eventDescription:font.family", "zappingBanner:height",
                           "channelDetailInfoBanner:height")

    def fresh(self, scene, *probes, wait="900"):
        args = ["--scene", scene, "--wait", wait]
        for p in probes:
            args += ["--probe", "700:" + p]
        return self.run_host(*args)

    def test_the_info_cards_wear_the_glass_and_the_ink_when_the_style_sheet_is_LGs_own(self):
        """Round 18 on the set (9efedab): the INFO card on HDMI 2 and on Live TV was a flat grey sheet in LG's colours with no hairline. LG's
        process holds LG's sheet, and the card read its colour, radius and opacity, the clock's ink and the chips' colours from it."""
        pal = palette()
        r, g, b, a = (int(pal["sheetBottom"][i:i + 2], 16) / 255 for i in (3, 5, 7, 1))
        for scene, probes in (("detail", self.CARD_PROBES), ("channel-detail", self.CARD_PROBES + self.CHANNEL_CARD_PROBES)):
            rc, out = self.run_host("--scene", scene, "--stale-sheet", "--wait", "900", *sum((["--probe", "700:" + p] for p in probes), []))
            self.assertEqual(rc, 0, "%s: a banner file warned with LG's sheet in memory: %s" % (scene, out[-1500:]))
            for name, want in (("color.r", r), ("color.g", g), ("color.b", b), ("color.a", a)):
                self.assertAlmostEqual(float(self.probe(out, "lumaCard", name)), want, delta=0.01, msg="%s card %s" % (scene, name))
            self.assertEqual(int(self.probe(out, "lumaCard", "radius")), pal["radius"], scene)
            self.assertEqual(float(self.probe(out, "lumaCard", "opacity")), 1.0, scene)
            self.assertGreater(float(self.probe(out, "lumaCard", "border.color.a")), 0.05, "%s: the hairline" % scene)
            self.assertEqual(self.probe(out, "time", "font.family"), "Manrope", scene)
            # the clock's ink is the palette's, not LG's #E6E6E6
            self.assertAlmostEqual(float(self.probe(out, "time", "color.r")), int(pal["ink"][3:5], 16) / 255, delta=0.01, msg=scene)
            # and every value read is the one a process with the patched sheet reads
            fresh = self.fresh(scene, *probes)[1]
            for p in probes:
                name, prop = p.split(":", 1)
                self.assertEqual(self.probe(out, name, prop), self.probe(fresh, name, prop), "%s: %s with LG's sheet and with the patched one" % (scene, p))

    def test_every_scene_reads_the_same_with_LGs_sheet_in_memory_as_with_the_patched_one(self):
        """The banners' own stale-sheet check, over every scene of the input app but the prompts: pixels, bar the antialiasing of a second render."""
        try:
            from PIL import Image, ImageChops
        except ImportError:
            self.skipTest("needs Pillow")
        for scene in ("pill", "focused-pill", "ok", "detail", "update", "channel", "channel-ok", "channel-detail", "banner-edit"):
            shots = []
            for stale in (False, True):
                path = os.path.join(self.tmp.name, "%s-%s.png" % (scene, stale))
                args = ["--scene", scene, "--wait", "900", "--shot", "900:" + path] + (["--stale-sheet"] if stale else [])
                rc, out = self.run_host(*args)
                self.assertEqual(rc, 0, "%s stale=%s: %s" % (scene, stale, out[-1200:]))
                shots.append(Image.open(path).convert("L"))
            diff = ImageChops.difference(*shots)
            self.assertLess(max(diff.tobytes()), 24, "%s: LG's sheet in memory draws it differently (%s)" % (scene, diff.getbbox()))

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

    LIVE_TV_SCENES = ("channel", "channel-ok", "channel-detail", "nosignal", "nosignal-photo", "banner-edit", "alert")

    def test_the_live_tv_the_no_signal_and_the_prompt_scenes_load_clean_stock_and_patched(self):
        for scene in self.LIVE_TV_SCENES:
            for patched in (False, True):
                rc, out = self.run_host("--scene", scene, "--wait", "900", patched=patched)
                self.assertEqual(rc, 0, "%s patched=%s: %s" % (scene, patched, out[-1200:]))

    def test_no_text_overlaps_at_100_and_200_percent_text_dark_and_light(self):
        """Lucent's universal rule, run on every restyled screen: no two runs of words on top of each other, at the size LG draws
        and at twice it (the TV has no text-size setting; this is the layout's own margin)."""
        for scene in ("pill", "ok", "detail", "update") + self.LIVE_TV_SCENES:
            for text in ("100", "200"):
                for extra in ([], ["--light"]):
                    rc, out = self.run_host("--scene", scene, "--wait", "900", "--text", text, "--audit", "overlap", *extra)
                    self.assertEqual(rc, 0, "%s at %s%% %s: %s" % (scene, text, extra, "\n".join(l for l in out.splitlines() if "layout" in l or "warning" in l)[-1500:]))

    def test_the_channel_strip_is_156_high_and_grows_with_the_type(self):
        heights = {}
        for text in ("100", "200"):
            rc, out = self.run_host("--scene", "channel", "--wait", "900", "--text", text, "--probe", "700:zappingBanner:height")
            self.assertEqual(rc, 0, out[-800:])
            heights[text] = int(self.probe(out, "zappingBanner", "height"))
        self.assertEqual(heights["100"], 156)
        self.assertGreater(heights["200"], 200)
        rc, out = self.run_host("--scene", "channel", "--wait", "900", "--probe", "700:zappingBanner:height", patched=False)
        self.assertEqual(int(self.probe(out, "zappingBanner", "height")), 156)

    def test_the_channel_banner_takes_the_keys_as_LGs_does(self):
        seen = {}
        for patched in (False, True):
            seen[patched] = {}
            for label, keys in (("name", "OK"), ("options", "RIGHT,OK"), ("there and back", "RIGHT,LEFT,OK")):
                rc, out = self.run_host("--scene", "channel-ok", "--wait", "1800", "--press", "600:" + keys, "--probe", "1500:option:activeFocus", patched=patched)
                self.assertEqual(rc, 0, out[-800:])
                seen[patched][label] = (self.luna(out), self.probe(out, "option", "activeFocus"))
        self.assertEqual(seen[False], seen[True])
        self.assertEqual(seen[True]["name"], (["handleBannerPressed"], "false"))
        self.assertEqual(seen[True]["options"], (["launchMoreActions"], "true"))

    def test_the_prompt_answers_what_LGs_does_for_the_same_keys(self):
        seen = {}
        for patched in (False, True):
            seen[patched] = {}
            for label, keys in (("first", "OK"), ("second", "DOWN,OK")):
                rc, out = self.run_host("--scene", "alert", "--wait", "1800", "--press", "600:" + keys, patched=patched)
                self.assertEqual(rc, 0, out[-800:])
                seen[patched][label] = self.luna(out)
        self.assertEqual(seen[False], seen[True])
        self.assertEqual(seen[True], {"first": ["yes"], "second": ["no"]})

    def test_the_banner_position_is_saved_as_LGs_does(self):
        seen = {}
        for patched in (False, True):
            seen[patched] = {}
            for label, keys in (("here", "OK"), ("right", "RIGHT,OK"), ("up", "UP,OK")):
                rc, out = self.run_host("--scene", "banner-edit", "--wait", "1800", "--press", "600:" + keys, patched=patched)
                self.assertEqual(rc, 0, out[-800:])
                seen[patched][label] = re.findall(r"\[luna\] (setSystemSettings .*)", out)
        self.assertEqual(seen[False], seen[True])
        self.assertEqual(seen[True]["right"], ['setSystemSettings {"bannerPosition":"rightbottom"}'])
        self.assertEqual(seen[True]["up"], ['setSystemSettings {"bannerPosition":"lefttop"}'])

    def test_the_no_signal_block_still_hops_every_ten_seconds_and_the_animation_is_gone(self):
        rc, out = self.run_host("--scene", "nosignal", "--wait", "12000", "--probe", "800:randomMoveItem:isMoving", "--probe", "800:animation:source",
                                "--probe", "800:randomMoveItem:x", "--probe", "800:randomMoveItem:y", "--probe", "11800:randomMoveItem:x", "--probe", "11800:randomMoveItem:y")
        self.assertEqual(rc, 0, out[-800:])
        self.assertEqual(self.probe(out, "randomMoveItem", "isMoving"), "true")
        self.assertEqual(self.probe(out, "animation", "source"), "")
        before, after = re.findall(r"\[probe\] randomMoveItem\.([xy])=(\S+)", out)[:2], re.findall(r"\[probe\] randomMoveItem\.([xy])=(\S+)", out)[2:]
        self.assertEqual(len(before), 2)
        self.assertNotEqual(before, after, "the block did not move in eleven seconds")

    def test_light_and_dark_agree_on_what_the_no_signal_words_sit_on(self):
        """The screen is black on every theme, so its words are the theme's light colour: dark text on black would not read."""
        for extra in ([], ["--light"]):
            with tempfile.TemporaryDirectory() as tmp:
                png = os.path.join(tmp, "nosignal.png")
                rc, out = self.run_host("--scene", "nosignal", "--wait", "900", "--shot", "800:" + png, *extra)
                self.assertEqual(rc, 0, out[-800:])
                from PyQt5.QtGui import QImage
                img = QImage(png)
                lit = max(img.pixelColor(x, y).lightness() for x in range(0, 1920, 3) for y in range(300, 700, 3))
            self.assertGreater(lit, 140, "the words are not light on black: %s" % extra)


if __name__ == "__main__":
    unittest.main()
