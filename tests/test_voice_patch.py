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
sys.path.insert(0, os.path.join(ROOT, "tools"))
from xdisplay import DISPLAY, HAVE_QT  # noqa: E402
import voice_patch as vp  # noqa: E402

TOUCHED = sorted(["Style.qml", "Main.qml", "Utils/StringSet.qml", "Views/SearchView.qml", "Views/SearchView/SearchInputBox.qml", "Views/SearchView/NotiView.qml",
                  "Views/KeywordsView.qml", "Views/ContentsList/KeywordsRec.qml", "Views/Components/ButtonFlex.qml", "Views/Components/ButtonIcon.qml",
                  "Views/Components/FocusableText.qml", "Views/Components/FocusableSwitch.qml", "Views/Components/FocusableSwitchWithText.qml",
                  "Views/PopupView/optionPopup.qml", "Views/MainView.qml", "Views/Components/MiniBg.qml", "Views/UtteranceUIView.qml"])
# What the patch adds beside LG's files: the photo card's shader (payload/voice) and the glyphs, Lucide's, in the palette's ink.
GLYPHS = ["arrow-left", "ellipsis", "history", "mic", "search", "star", "user", "x"]
ADDED = sorted(["Views/Components/LumaPhoto.qml"] + ["Views/images/luma/%s.png" % g for g in GLYPHS])
# What the tool prints, and what the staging lays over LG's directory.
PRINTED = sorted(TOUCHED + ADDED)

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
        ours = {v.upper() for v in vp.derive(self.pal).values() if isinstance(v, str)}
        left = [(n, v) for n, v in re.findall(r'property (?:color|string) (\w+)\s*:\s*"(#[0-9a-fA-F]{3,8})"', style) if v.upper() not in ours]
        # kept: the web tile's brand blue, the IP channel's orange (four of them), the select pink, the black behind a thumbnail
        self.assertEqual(sorted(set(n for n, _ in left)), ["bgColor", "ipChannelColor", "selectColor", "thumbnailBgColor"], left)
        self.assertEqual(sorted(set(v for n, v in left if n == "bgColor")), ["#359BDB"])

    def test_it_changes_the_files_it_names_and_no_other_and_prints_them(self):
        self.run_tool()
        self.assertEqual(self.differing(), TOUCHED)
        self.assertEqual(self.stdout.split("\n")[:-1], PRINTED)

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

    def test_the_mic_hint_says_what_mock_09_says_and_no_other_string_changes(self):
        self.run_tool()
        old, new = lines(os.path.join(CENSUS, "Utils/StringSet.qml")), lines(os.path.join(self.dir, "Utils/StringSet.qml"))
        changed = [(a, b) for a, b in zip(old, new) if a != b]
        self.assertEqual(len(old), len(new))
        self.assertEqual(len(changed), 1, changed)
        self.assertIn('qsTr("Hold the AI button on the remote and search with your voice.")', changed[0][0])
        self.assertIn('qsTr("Hold the mic key to speak")', changed[0][1])

    def test_the_glyphs_are_lucides_in_the_palettes_ink_and_the_shape_is_lucides_own(self):
        self.run_tool()
        for name, role in vp.ICONS.items():
            with open(os.path.join(self.dir, "Views/images/luma/%s.png" % name), "rb") as f:
                w, h, rows = vp.png_read(f.read())
            with open(os.path.join(vp.ICON_DIR, name + ".png"), "rb") as f:
                sw, sh, src = vp.png_read(f.read())
            self.assertEqual((w, h), (sw, sh), name)
            r, g, b, a = vp.parse(self.pal[role])
            for line, ref in zip(rows, src):
                for x in range(0, len(line), 4):
                    self.assertEqual(tuple(line[x:x + 3]), (r, g, b), name)
                    self.assertLessEqual(abs(line[x + 3] - round(ref[x + 3] * a)), 1, name)
        self.assertEqual(sorted(vp.ICONS), GLYPHS)

    def test_png_tinting_reads_every_filter_a_png_can_use(self):
        """A PNG of 5 rows, one per filter type, built here from known pixels: read back, the pixels are the ones put in."""
        import struct
        import zlib
        w, h = 6, 5
        px = [bytearray((x * 40 + y * 7) % 256 for x in range(w * 4)) for y in range(h)]
        raw = b""
        for y, row in enumerate(px):
            prev = px[y - 1] if y else bytearray(w * 4)
            out = bytearray()
            for i, v in enumerate(row):
                a, b, c = (row[i - 4] if i >= 4 else 0), prev[i], (prev[i - 4] if i >= 4 else 0)
                if y == 0:
                    pred = 0
                elif y == 1:
                    pred = a
                elif y == 2:
                    pred = b
                elif y == 3:
                    pred = (a + b) >> 1
                else:
                    p = a + b - c
                    pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                    pred = a if pa <= pb and pa <= pc else b if pb <= pc else c
                out.append((v - pred) & 255)
            raw += bytes([y]) + bytes(out)

        def chunk(kind, body):
            return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body) & 0xFFFFFFFF)
        data = vp.PNG_MAGIC + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b"")
        rw, rh, rows = vp.png_read(data)
        self.assertEqual((rw, rh), (w, h))
        self.assertEqual([bytes(r) for r in rows], [bytes(r) for r in px])
        tinted = vp.png_read(vp.png_tint(data, (10, 20, 30, 0.5)))[2]
        for line, ref in zip(tinted, px):
            for x in range(0, w * 4, 4):
                self.assertEqual((line[x], line[x + 1], line[x + 2], line[x + 3]), (10, 20, 30, int(round(ref[x + 3] * 0.5))))

    def test_our_files_are_laid_beside_LGs_and_never_over_one(self):
        self.write("Views/Components/LumaPhoto.qml", "// LG's, the day it exists\n")
        rc, err = self.run_tool()
        self.assertEqual(rc, 1)
        self.assertIn("LumaPhoto.qml: LG's tree already has this file", err)
        self.assertEqual(self.read("Views/Components/LumaPhoto.qml"), "// LG's, the day it exists\n")
        self.assertEqual([r for r in self.differing() if r != "Views/Components/LumaPhoto.qml"], [])

    def test_only_the_search_screens_four_picture_buttons_take_the_64_px_plate(self):
        self.run_tool()
        self.assertIn("    property bool lumaPlate: false\n", self.read("Views/Components/ButtonIcon.qml"))
        self.assertEqual(self.read("Views/SearchView.qml").count("lumaPlate: true"), 4)
        self.assertEqual(self.read("Views/ContentsList/MusicSearchRec.qml"), self.stock("Views/ContentsList/MusicSearchRec.qml"))

    def test_the_photo_card_is_one_shader_quad_with_no_offscreen_target(self):
        with open(os.path.join(ROOT, "payload", "voice", "Views", "Components", "LumaPhoto.qml")) as f:
            text = f.read()
        code = "\n".join(l for l in text.splitlines() if not l.lstrip().startswith("//"))
        self.assertEqual(len(re.findall(r"^ShaderEffect \{", code, re.M)), 1)
        self.assertNotRegex(code, r"DropShadow|layer\.enabled|ShaderEffectSource|Glow|GaussianBlur|FastBlur|OpacityMask|Canvas")
        self.assertIn("visible: picture !== null && picture.status === Image.Ready", text)

    def test_the_rows_fill_the_line_from_the_field_to_the_safe_edge_and_the_top_row_shares_the_fields_centre(self):
        line = 1920 - vp.FIELD_X - vp.SAFE_X
        self.assertEqual(5 * vp.CHIP_W + 4 * vp.CHIP_GAP, line)
        self.assertEqual(6 * vp.CARD_W + 5 * vp.CARD_GAP, line)
        self.assertEqual(vp.FIELD_TOP + vp.FIELD_H // 2, (vp.FIELD_TOP + vp.FIELD_H // 2 - vp.BUTTON // 2) + vp.BUTTON // 2)
        # the field ends before the first of the three targets on the right and starts after the back key on the left
        self.assertLessEqual(vp.FIELD_X + vp.FIELD_W + vp.BUTTON_GAP, 1920 - vp.SAFE_X - 3 * vp.BUTTON - 2 * vp.BUTTON_GAP)
        self.assertGreaterEqual(vp.FIELD_X, vp.SAFE_X + vp.BUTTON)
        # a target is the TV profile's 64, and so is a chip
        self.assertGreaterEqual(vp.BUTTON, 64)
        self.assertGreaterEqual(vp.CHIP_H, 64)

    def test_a_plate_grows_when_focused_and_the_words_are_not_inside_it(self):
        self.run_tool()
        text = self.read("Views/ContentsList/KeywordsRec.qml")
        card = vp.blocks(text, "        Item {\n            id: lumaCard\n")
        self.assertEqual(len(card), 1)
        b, e = card[0]
        self.assertIn("scale: isFocused", text[b:e])
        self.assertNotIn("MarqueeText", text[b:e])
        self.assertEqual(text.count("scale:"), 1)

    def test_every_palette_value_of_the_new_roles_is_derived_from_the_palette_and_is_readable(self):
        for light in (False, True):
            pal = palette(light)
            derived = vp.derive(pal)
            sheet = vp.parse(pal["sheetBottom"])[:3]
            lit = vp.over(vp.parse(derived["washLit"]), sheet)
            for name in ("ink", "accentText"):
                self.assertGreaterEqual(vp.contrast(vp.parse(derived[name])[:3], lit), 4.5, (light, name))
            ground = vp.over(vp.parse(derived["well"]), sheet)
            self.assertGreaterEqual(vp.contrast(vp.over(vp.parse(derived["inkDim"]), ground), ground), 4.5, light)

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
        ("Utils/StringSet.qml", 'qsTr("Hold the AI button on the remote and search with your voice.")'),
        ("Views/KeywordsView.qml", "    readonly property int leftGap: getRelativeWidth(174)\n"),
        ("Views/KeywordsView.qml", "                        return (getRelativeHeight(63) - allListView.spacing)\n"),
        ("Views/ContentsList/KeywordsRec.qml", "        Rectangle{\n            id:bgImgBorderRect\n"),
        ("Views/SearchView.qml", "                top: parent.top; topMargin: getRelativeHeight(66)\n"),
        ("Views/SearchView.qml", "            width: getRelativeWidth(78/2)\n"),
        ("Views/SearchView/SearchInputBox.qml", "        id: listSearchBg\n"),
        ("Views/SearchView/SearchInputBox.qml", '                visible: (globalVars.isTyping === false && textInput.text.length === 0)\n'),
        ("Views/Components/ButtonIcon.qml", "    Image {\n        anchors.fill: parent\n        source: root.source\n"),
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
        for key in ("sheetBottom", "row", "focus", "focusEdge", "hairline", "ink", "inkDim", "focusInk", "accent", "accentInk", "danger", "radius", "rowRadius"):
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

    SCENES = ("search", "search-tv", "search-focus", "genre", "chip", "back", "auto", "keywords", "noti", "option", "confirm")

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

    def test_the_search_field_has_the_focus_when_the_screen_opens(self):
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

    # Every kind of the layout audit but `elided`: a keyword that does not fit its chip is LG's marquee (it runs while the chip has the focus),
    # and the one test below says that it is the only words that are ever cut short.
    SQUARE = "scaled,overflow,clipped,outside,tight,overlap,off-centre,icon-level,contrast"

    def test_the_new_look_reads_and_sits_square_at_100_percent_dark_and_light(self):
        """Words that read (4.5:1 over the well, the chips and the foot of a photograph, 3:1 for a one-glyph initial), none in a scaled item
        (a focus that moved them sideways), centred where they should be and level with their glyphs."""
        for scene in ("search", "search-tv", "search-focus", "genre", "chip", "back", "auto"):
            for extra in ([], ["--light"]):
                rc, out = self.run_host("--scene", scene, "--audit", self.SQUARE, *extra)
                self.assertEqual(rc, 0, "%s %s: %s" % (scene, extra, "\n".join(l for l in out.splitlines() if "layout" in l or "warning" in l)[-1500:]))

    def test_the_card_names_read_at_4_5_to_1_over_the_worst_picture_the_scrim_can_have(self):
        """Pure white under the dark sheet's scrim, pure black under the light one's."""
        with tempfile.TemporaryDirectory() as d:
            for extra, level in (([], 255), (["--light"], 0)):
                pic = os.path.join(d, "%d.png" % level)
                with open(pic, "wb") as f:
                    f.write(vp.png_write(64, 36, [bytearray(bytes([level, level, level, 255]) * 64) for _ in range(36)]))
                rc, out = self.run_host("--scene", "genre", "--genres", pic, "--audit", "contrast", *extra)
                self.assertEqual(rc, 0, "%s: %s" % (extra, "\n".join(l for l in out.splitlines() if "layout" in l or "warning" in l)[-1500:]))

    def test_only_a_keyword_that_does_not_fit_its_chip_is_cut_short(self):
        for scene in ("search", "genre", "auto", "noti"):
            rc, out = self.run_host("--scene", scene, "--audit", "elided")
            found = [l for l in out.splitlines() if l.startswith("[layout")]
            self.assertTrue(all("KeywordsListTextKeyword" in l for l in found), "%s: %s" % (scene, found))

    def test_the_mic_hint_is_whole_while_the_field_is_empty_and_gone_when_a_word_is_typed(self):
        rc, out = self.run_host("--scene", "search-tv", "--probe", "2500:DefaultText:visible", "--probe", "2500:DefaultText:width", "--probe", "2500:DefaultText:implicitWidth")
        self.assertEqual(rc, 0, out[-800:])
        self.assertEqual(self.probe(out, "DefaultText", "visible"), "true")
        self.assertAlmostEqual(float(self.probe(out, "DefaultText", "width")), float(self.probe(out, "DefaultText", "implicitWidth")), delta=1)
        rc, out = self.run_host("--scene", "auto", "--probe", "2500:DefaultText:visible")
        self.assertEqual(rc, 0, out[-800:])
        self.assertEqual(self.probe(out, "DefaultText", "visible"), "false")

    def test_the_field_the_rows_and_the_top_row_are_where_the_layout_says(self):
        rc, out = self.run_host("--scene", "search-tv", "--probe", "2500:SearchInputBox:x", "--probe", "2500:SearchInputBox:y", "--probe", "2500:SearchInputBox:width",
                                "--probe", "2500:SearchInputBox:height", "--probe", "2500:quitButton:width")
        self.assertEqual(rc, 0, out[-800:])
        self.assertEqual([float(self.probe(out, "SearchInputBox", p)) for p in ("x", "y", "width", "height")], [vp.FIELD_X, vp.FIELD_TOP, vp.FIELD_W, vp.FIELD_H])
        self.assertEqual(float(self.probe(out, "quitButton", "width")), vp.BUTTON)


if __name__ == "__main__":
    unittest.main()
