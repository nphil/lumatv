"""Run with: python3 -m unittest tests.test_voice_patch

tools/voice_patch.py against a copy of LG's `com.webos.app.voice` QML (the census copy of LG's Search app; skipped when it is not on this
workstation): what it changes, that it changes style and nothing LG's code does, that every colour of the style sheet has a role, that a
missing or doubled anchor is fatal and leaves the tree as LG's, and (with Xvfb and PyQt5) that the patched screens load, take the keys the
way LG's do and lay out at 100 % and 200 % text, through the offline host.
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
TOOL = os.path.join(ROOT, "tools", "voice_patch.py")
TOKENS = os.path.join(ROOT, "tools", "tokens.py")
SHIPPED = os.path.join(ROOT, "payload", "theme", "theme.json")
FONTS = os.path.join(ROOT, "payload", "fonts")
HOST = os.path.join(HERE, "qmlhost", "voice.py")
CENSUS = next((p for p in ("/data/home/lgtv/uiinv/voice/qml", "/tmp/tvsrc/apps/com.webos.app.voice/qml") if os.path.isdir(p)), "/data/home/lgtv/uiinv/voice/qml")

sys.path.insert(0, HERE)
from xdisplay import DISPLAY, HAVE_QT  # noqa: E402

TOUCHED = sorted(["Style.qml", "Main.qml", "Views/SearchView.qml", "Views/SearchView/SearchInputBox.qml", "Views/SearchView/NotiView.qml",
                  "Views/ContentsList/KeywordsRec.qml", "Views/Components/ButtonFlex.qml", "Views/Components/ButtonIcon.qml",
                  "Views/Components/FocusableText.qml", "Views/Components/FocusableSwitch.qml", "Views/Components/FocusableSwitchWithText.qml",
                  "Views/PopupView/optionPopup.qml", "Views/MainView.qml", "Views/Components/MiniBg.qml", "Views/UtteranceUIView.qml"])

# What LG's code does, as opposed to how it looks: handlers, keys, service reads and calls, focus.
BEHAVIOUR = re.compile(r"function |Keys\.|\bon[A-Z]\w*\s*:|Connections|signal |forceActiveFocus|KeyNavigation|\bfocus:|\.handle|\.launch|\.speak|Service\b|util\.")


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


@unittest.skipUnless(os.path.isdir(CENSUS), "needs the census copy of LG's Search tree")
class VoicePatch(unittest.TestCase):
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
        style = self.read("Style.qml")
        self.assertTrue(style.startswith("// Luma: patched"))
        for face in ("Regular", "Medium", "SemiBold", "Bold"):
            self.assertIn('FontLoader { source: "file://%s/Manrope-%s.ttf" }' % (os.path.realpath(FONTS), face), style)
        self.assertEqual(style.count("lumaFamily(style, style.normal"), 4)
        self.assertIn('property color lumaInk: "%s"' % self.pal["ink"], style)
        self.assertIn('property color lumaSheet: "%s"' % self.pal["sheetBottom"], style)
        self.assertIn('property color popup_bg_color: "%s"' % self.pal["sheetBottom"], style)
        self.assertIn('property string normal400_fontFamily: "Manrope"', self.read("Main.qml"))

    def test_every_white_and_grey_of_LGs_style_sheet_is_gone_except_the_ones_kept_on_purpose(self):
        self.run_tool()
        style = self.read("Style.qml")
        ours = {v.upper() for v in self.pal.values() if isinstance(v, str)}
        left = [(n, v) for n, v in re.findall(r'property (?:color|string) (\w+)\s*:\s*"(#[0-9a-fA-F]{3,8})"', style) if v.upper() not in ours]
        # kept: the web tile's brand blue, the IP channel's orange (four of them), the select pink, the black behind a thumbnail
        self.assertEqual(sorted(set(n for n, _ in left)), ["bgColor", "ipChannelColor", "selectColor", "thumbnailBgColor"], left)
        self.assertEqual(sorted(set(v for n, v in left if n == "bgColor")), ["#359BDB"])

    def test_it_changes_the_files_it_names_and_no_other_and_prints_them(self):
        self.run_tool()
        self.assertEqual(self.differing(), TOUCHED)
        self.assertEqual(self.stdout.split("\n")[:-1], TOUCHED)

    def test_only_style_is_removed_and_every_line_of_LGs_behaviour_is_still_there(self):
        self.run_tool()
        for rel in TOUCHED:
            old, new = lines(os.path.join(CENSUS, rel)), lines(os.path.join(self.dir, rel))
            old_b = sorted(l for l in old if BEHAVIOUR.search(l))
            new_b = sorted(l for l in new if BEHAVIOUR.search(l))
            # the four lines that copy a font family out of LG's language table are rewritten to go through one function, and said so
            if rel == "Style.qml":
                old_b = [l for l in old_b if "fontFamily[0]" not in l]
                new_b = [l for l in new_b if "fontFamily[0]" not in l and "lumaFamily" not in l]
            self.assertEqual(old_b, new_b, rel)

    def test_no_live_shadow_comes_in_and_every_one_that_was_there_in_a_touched_file_is_off(self):
        self.run_tool()
        for rel in TOUCHED:
            old, new = lines(os.path.join(CENSUS, rel)), lines(os.path.join(self.dir, rel))
            added = [l for l in difflib.unified_diff(old, new, lineterm="", n=0) if l.startswith("+") and not l.startswith("+++")]
            for l in added:
                self.assertNotRegex(l, r"DropShadow|layer\.enabled|ShaderEffect|Glow|GaussianBlur|FastBlur|OpacityMask", rel)
        for rel in ("Views/Components/ButtonFlex.qml", "Views/Components/ButtonIcon.qml", "Views/Components/FocusableText.qml", "Views/Components/FocusableSwitch.qml",
                    "Views/Components/FocusableSwitchWithText.qml", "Views/PopupView/optionPopup.qml", "Views/ContentsList/KeywordsRec.qml"):
            text = self.read(rel)
            shadows = re.findall(r"DropShadow \{.*?\n    \}\n", text, re.S)
            self.assertTrue(shadows, rel)
            for s in shadows:
                self.assertRegex(s, r"\n\s*visible: false\n", rel)

    def test_the_focus_pads_are_drawn_and_ringed_where_LG_drew_a_white_box_with_a_shadow(self):
        self.run_tool()
        for rel in ("Views/Components/FocusableText.qml", "Views/Components/FocusableSwitch.qml", "Views/Components/FocusableSwitchWithText.qml"):
            self.assertIn("border.color: voiceStyle.lumaRing", self.read(rel), rel)
            self.assertNotIn("        visible: false\n    }\n\n    DropShadow", self.read(rel), rel)
            self.assertIn("        visible: false\n    }\n\n    DropShadow", self.stock(rel), rel)

    def test_a_light_sheet_uses_the_glyph_pictures_the_other_way_round(self):
        self.run_tool()
        dark = self.read("Views/Components/ButtonIcon.qml")
        self.assertEqual(dark.count("PropertyChanges { target: root; source: normalImage }"), 3)
        light = palette(light=True)
        shutil.rmtree(self.dir)
        shutil.copytree(CENSUS, self.dir)
        self.assertEqual(self.run_tool(light)[0], 0)
        text = self.read("Views/Components/ButtonIcon.qml")
        self.assertEqual(text.count("PropertyChanges { target: root; source: focusedImage }"), 3)
        self.assertIn('property color lumaSheet: "%s"' % light["sheetBottom"], self.read("Style.qml"))

    def test_manrope_is_for_latin_locales_only(self):
        self.run_tool()
        style = self.read("Style.qml")
        self.assertIn('return style.info.languageClass === "latin" ? "Manrope" : family;', style)

    ANCHORS = [
        ("Style.qml", 'property color focusColor: supportBNO ? "#68819b" : "#E6E6E6"'),
        ("Style.qml", "property real textinputwrapper_normal_opacity: 0.2"),
        ("Style.qml", "normal700_fontFamily = style.normal700.fontFamily[0];"),
        ("Main.qml", 'property string normal600_fontFamily: "LG Smart UI"'),
        ("Views/SearchView.qml", 'searchInputBox.textColor = searchInputBox.isFocused?"#4C5059":"#E6E6E6";'),
        ("Views/SearchView/SearchInputBox.qml", 'normalColor: "#33FFFFFF"'),
        ("Views/SearchView/NotiView.qml", 'styleColor: "#333333"'),
        ("Views/ContentsList/KeywordsRec.qml", 'GradientStop { position: 0.0; color: "#813ADC" }'),
        ("Views/Components/ButtonFlex.qml", "        visible: !root.isFocused\n"),
        ("Views/Components/ButtonIcon.qml", 'property color bgNormalColor: hasContainer ? "#7D848C" : "transparent"'),
        ("Views/Components/FocusableText.qml", "        visible: false\n    }\n"),
        ("Views/Components/FocusableSwitch.qml", "        visible: false\n    }\n"),
        ("Views/Components/FocusableSwitchWithText.qml", "        visible: false\n    }\n"),
        ("Views/PopupView/optionPopup.qml", "        id:bgRect\n"),
        ("Views/MainView.qml", "            visible: false\n"),
        ("Views/Components/MiniBg.qml", 'source: imagesPath + "mainView/bg_nlp_search_result"'),
        ("Views/UtteranceUIView.qml", '<font color=\\"#C97EFA\\">'),
    ]

    def test_a_missing_anchor_is_fatal_says_which_file_and_leaves_LGs_tree_as_it_was(self):
        for rel, anchor in self.ANCHORS:
            shutil.rmtree(self.dir)
            shutil.copytree(CENSUS, self.dir)
            text = self.read(rel)
            self.assertGreaterEqual(text.count(anchor), 1, "the test's own anchor: " + anchor)
            self.write(rel, text.replace(anchor, anchor.replace(anchor[-2], "Z", 1) if anchor[-1] == "\n" else anchor[:-1] + "Z"))
            rc, err = self.run_tool()
            self.assertEqual(rc, 1, rel + ": " + anchor)
            self.assertIn("voice_patch: ", err)
            # all or nothing: a late failure writes none of the early files
            self.assertEqual([r for r in self.differing() if r != rel], [], rel)

    def test_a_doubled_anchor_is_fatal_too(self):
        text = self.read("Views/SearchView/NotiView.qml")
        anchor = '            fontColor: "#E6E6E6"\n            style: Text.Raised\n            styleColor: "#333333"\n'
        self.write("Views/SearchView/NotiView.qml", text.replace(anchor, anchor + anchor))
        rc, err = self.run_tool()
        self.assertEqual(rc, 1)
        self.assertIn("anchor found 2 times, expected 1", err)

    def test_a_colour_that_has_no_role_is_fatal_so_a_firmware_update_cannot_slip_one_through(self):
        text = self.read("Style.qml")
        self.write("Style.qml", text.replace("    property QtObject searchView: QtObject {\n", '    property QtObject searchView: QtObject {\n        property color newColor: "#123456"\n'))
        rc, err = self.run_tool()
        self.assertEqual(rc, 1)
        self.assertIn("searchView.newColor (#123456) has no role", err)

    def test_a_role_for_a_colour_that_is_gone_is_fatal_too(self):
        text = self.read("Style.qml")
        self.write("Style.qml", text.replace('property color titleBgColor: "#000000"', 'property color titleBgColorX: "#000000"', 1))
        rc, err = self.run_tool()
        self.assertEqual(rc, 1)
        self.assertRegex(err, r"has no role|roles for colours that are not there")

    def test_a_missing_file_is_fatal(self):
        os.unlink(os.path.join(self.dir, "Views/Components/ButtonFlex.qml"))
        rc, err = self.run_tool()
        self.assertEqual(rc, 1)
        self.assertIn("ButtonFlex.qml: LG's tree has no such file", err)

    def test_patching_twice_is_refused(self):
        self.assertEqual(self.run_tool()[0], 0)
        rc, err = self.run_tool()
        self.assertEqual(rc, 1)
        self.assertIn("already patched", err)

    def test_a_palette_without_what_it_needs_is_refused(self):
        for key in ("sheetBottom", "row", "focus", "focusEdge", "hairline", "ink", "inkDim", "focusInk", "accent", "danger", "radius", "rowRadius"):
            pal = dict(self.pal)
            del pal[key]
            rc, err = self.run_tool(pal)
            self.assertEqual(rc, 1, key)
            self.assertIn("palette has no " + key, err)
        self.assertEqual(self.differing(), [])

    def test_fonts_that_are_not_there_are_refused_so_the_screen_is_never_drawn_in_a_fallback_face(self):
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
class PatchedSearchInTheHost(unittest.TestCase):
    """LG's own Search views behind stand-ins for what is native, in Qt 5.15 on software GL: the patched screens must load, warn of
    nothing, take the keys as LG's do and call what LG's call."""

    SCENES = ("search", "search-focus", "auto", "keywords", "noti", "option", "confirm")

    def run_host(self, *args, patched=True):
        r = subprocess.run([sys.executable, HOST] + (["--patched"] if patched else []) + list(args), env=dict(os.environ, DISPLAY=DISPLAY),
                           capture_output=True, text=True, timeout=120)
        return r.returncode, r.stdout + r.stderr

    def probe(self, out, name, prop):
        m = re.search(r"\[probe\] %s\.%s=(.*)" % (re.escape(name), re.escape(prop)), out)
        self.assertIsNotNone(m, "no probe %s.%s in: %s" % (name, prop, out[-800:]))
        return m.group(1)

    def luna(self, out):
        return re.findall(r"\[luna\] (.*)", out)

    def test_every_scene_loads_clean_stock_and_patched_in_both_materials(self):
        for scene in self.SCENES:
            for patched, extra in ((False, []), (True, []), (True, ["--light"])):
                rc, out = self.run_host("--scene", scene, *extra, patched=patched)
                self.assertEqual(rc, 0, "%s patched=%s %s: %s" % (scene, patched, extra, out[-1200:]))

    def test_text_is_drawn_in_manrope_when_patched_and_not_when_stock(self):
        for patched, family in ((True, "Manrope"), (False, "LG Smart UI")):
            rc, out = self.run_host("--scene", "search", "--probe", "2500:SearchInputBox:font.family", "--probe", "2500:previousKeywordsListTextKeyword:font.family", patched=patched)
            self.assertEqual(rc, 0, out[-800:])
            self.assertEqual(self.probe(out, "previousKeywordsListTextKeyword", "font.family"), family)

    def test_the_search_field_is_a_pill_and_the_focus_ring_is_the_palettes(self):
        rc, out = self.run_host("--scene", "search", "--probe", "2500:SearchInputBox:isFocused")
        self.assertEqual(rc, 0, out[-800:])
        self.assertEqual(self.probe(out, "SearchInputBox", "isFocused"), "true")

    def test_the_keys_do_what_LGs_do_and_call_what_LGs_call(self):
        """From the field: Right three times reaches the close key, OK on it asks to leave the app; in the option popup OK on the second row
        opens LG's user guide and on the third asks to quit. Whatever the stock screens do for the same keys, the patched ones do."""
        seen = {}
        for patched in (False, True):
            runs = {}
            rc, out = self.run_host("--scene", "search", "--press", "1500:RIGHT,RIGHT,RIGHT", "--press", "2300:OK", "--probe", "2200:quitButton:activeFocus",
                                    "--probe", "2200:moreButton:activeFocus", patched=patched)
            self.assertEqual(rc, 0, out[-800:])
            runs["close"] = (self.luna(out), self.probe(out, "quitButton", "activeFocus"), self.probe(out, "moreButton", "activeFocus"))
            for label, keys in (("guide", "DOWN,OK"), ("quit", "DOWN,DOWN,OK"), ("switch", "OK")):
                rc, out = self.run_host("--scene", "option", "--wait", "3000", "--press", "1500:" + keys, patched=patched)
                self.assertEqual(rc, 0, out[-800:])
                runs[label] = self.luna(out)
            for label, keys in (("yes", "OK"), ("no", "RIGHT,OK")):
                rc, out = self.run_host("--scene", "confirm", "--wait", "3000", "--press", "1500:" + keys, patched=patched)
                self.assertEqual(rc, 0, out[-800:])
                runs[label] = self.luna(out)
            seen[patched] = runs
        self.assertEqual(seen[False], seen[True])
        self.assertEqual(seen[True]["close"], (["deactivate exit_button"], "true", "false"))
        self.assertEqual(seen[True]["quit"], ["deactivate quit_app_button"])
        self.assertEqual(len(seen[True]["guide"]), 1)
        self.assertIn("tvuserguide", seen[True]["guide"][0])
        self.assertIn("firstuse-overlay", seen[True]["yes"][0])
        self.assertEqual(seen[True]["no"], [])

    def test_no_text_overlaps_at_100_and_200_percent_text_dark_and_light(self):
        """Lucent's universal rule, run on every restyled screen: no two runs of words on top of each other, at the size LG draws and at twice it."""
        for scene in self.SCENES:
            for text in ("100", "200"):
                for extra in ([], ["--light"]):
                    rc, out = self.run_host("--scene", scene, "--text", text, "--audit", "overlap", *extra)
                    self.assertEqual(rc, 0, "%s at %s%% %s: %s" % (scene, text, extra, "\n".join(l for l in out.splitlines() if "layout" in l or "warning" in l)[-1500:]))

    def test_the_marquee_never_leaves_a_run_of_words_clipped_at_200_percent(self):
        rc, out = self.run_host("--scene", "search", "--text", "200", "--audit", "clipped,overflow")
        self.assertEqual(rc, 0, out[-1200:])


if __name__ == "__main__":
    unittest.main()
