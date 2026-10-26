#!/usr/bin/env python3
"""Patches a staged copy of maliit's QML (`/usr/lib/maliit/plugins/qml` on the set: LG's on-screen keyboard, 85 files) into the Luma look:
the key caps, the function keys, the focus, the candidate rows and the number pad, in the palette and in Manrope, with no live shadow.

maliit is the IME process (`MaliitServer`, `maliit-server.service`): LG's plugins (`libplugin-global.so` for every Latin language, `-chinese`,
`-japanese`) draw the keyboard from QML on disk, in a window the compositor places at the bottom of the screen. What the keys do (the
navigation between them, the key codes, the shift and symbol states, the prediction list, the text-to-speech lines) is LG's and stays as it
is; the style is in two places: the style sheets (`keyboard-common/style/*.qml`, `style-portrait/`, and the views' own) and a few literals
in the components. What changes, all of it style (docs/research/lg-restyles-result.md):

  * A role for every colour of the style sheets. LG draws a flat grey-blue panel (#3B4045), tiles (#42484E) with a 2 px edge, a white focus
    box with dark text and a green Enter key. Here the panel is the palette's sheet, made opaque (a keyboard is text over whatever is
    behind, and an opaque sheet makes every contrast a number that does not depend on the picture), the character keys are the lifted tile
    (the hairline wash on the sheet), the function keys the recessed one (the row wash), the candidates tiles on a well, the primary key
    (Search, Enter) the accent with its ink, the shift and accent-letter marks the accent when on. Every composite is computed here from the
    palette and written as an opaque colour: the focus box is opaque (it covers the key under it, LG's design) and a translucent wash
    would show the key's own words through it.
  * Focus is a lit tile with a ring and the same ink, where LG's is a white box with dark words: the focus rectangle's fill is the focus
    wash over the sheet, its ring the ink (12:1 against the tile). The glyph pictures (delete, shift, mic, space, arrows, more) are
    LG's pairs, a light one and a dark one for its white box: the one that reads on the sheet is used in both states.
  * The focus rectangle's DropShadow (a blurred offscreen copy of the box, `layer.enabled`) is off: no live shadow.
  * Manrope for Latin locales: `setDefaultFont` goes through LG's language table (`languageClass`), the four faces are loaded by FontLoaders
    in the style sheet, and the literals that name LG Smart UI (symbols, the candidate row) are Manrope. Keys are Medium.
  * The corner is the row radius of the palette, the edge one pixel (LG's is two), and the sizes of LG's grid (cells, labels, offsets) are
    untouched: 87 x 66 px keys at 1080p with 39 px letters are readable at 10 feet, and every label is `fontSizeMode: Text.Fit`, so a larger
    type shrinks into its key and never meets its neighbour.

Every colour of every style sheet has a role here, and so does every literal the patch rewrites: a colour LG adds in a firmware update is a
refusal (the role table says which are expected), and so is a missing or doubled anchor. The caller (`tools/maliit.sh stage`) works on a
fresh copy every time and leaves LG's tree in force when this fails.

usage: maliit_patch.py <staged qml dir> <palette JSON from `tokens.py qml`> <fonts dir with Manrope-*.ttf>
Prints the paths it rewrote (relative to the staged dir), one a line.
Exit status: 0 patched, 1 an anchor is missing or the palette or fonts lack something, 2 usage.
"""
import json
import pathlib
import re
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from notification_patch import sub, blocks  # noqa: E402
from tokens import contrast, luminance, parse, toward  # noqa: E402

MARKER = "Luma"

# The four faces the style sheet loads; LG's fonts are the fallback while one is still loading.
FACES = ("Manrope-Regular.ttf", "Manrope-Medium.ttf", "Manrope-SemiBold.ttf", "Manrope-Bold.ttf")

# Palette names the edits draw on.
NEEDED = ("sheetBottom", "row", "focusEdge", "hairline", "ink", "inkDim", "accent", "accentInk", "radius", "rowRadius")

# The candidate bar's lines (LG's 78 and 42, in the style sheets' own units): see generic().
HID_WORD = 108
HID_HOTKEY = 50

# Key labels are Medium (Qt's Font.Medium, 57: Manrope's 500, the weight Lucent's body is set in at a distance).
KEY_WEIGHT = 57
# The keyboard counts in pixels of its own resolution: LG's `N / resolution` is N / 2 at 1080p (resolution 2), N / 1 at 4K. The palette's
# radii and widths are 1080p pixels, so they are written `N * 2 / resolution`.
GRID = 2
RING_WIDTH = 3          # 1080p pixels: the focus ring
EDGE_WIDTH = 1          # the key's own edge

ROLE_NAMES = ("sheet", "tile", "tileEdge", "fn", "fnEdge", "well", "lit", "ring", "ink", "dim", "accent", "accentInk", "dotOff", "dotOn")


def over(top, bottom):
    """`top` ((r, g, b, a)) drawn over the opaque `bottom` ((r, g, b)), in the colour space Qt blends in."""
    r, g, b, a = top
    return tuple(int(round(t * a + u * (1 - a))) for t, u in zip((r, g, b), bottom))


def hexrgb(rgb):
    return "#%02X%02X%02X" % tuple(rgb)


def dot_on(accent, ink, grounds):
    """The mark of an active shift or accent key: the accent, taken toward the ink until it reads 3:1 on the focus fill, the recessed key and the
    sheet (the accent alone does on the dark material and not on the light one)."""
    for step in range(21):
        mark = toward(accent, ink, step / 20.0)
        if all(contrast(mark, g) >= 3.0 for g in grounds):
            return mark
    return ink


def roles(pal):
    """Every colour the patch writes, opaque, from the palette's own layers: the sheet and the washes drawn over it."""
    sheet = parse(pal["sheetBottom"])[:3]
    hair, row, edge = parse(pal["hairline"]), parse(pal["row"]), parse(pal["focusEdge"])
    dim, ink = parse(pal["inkDim"]), parse(pal["ink"])
    tile = over(hair, sheet)
    fn = over(row, sheet)
    lit = over(edge, sheet)
    out = {
        "sheet": sheet,
        "tile": tile,
        "tileEdge": over(hair, tile),
        "fn": fn,
        "fnEdge": tile,
        "well": fn,
        "lit": lit,
        "ring": ink[:3],
        "ink": ink[:3],
        "dim": over(dim, tile),
        "accent": parse(pal["accent"])[:3],
        "accentInk": parse(pal["accentInk"])[:3],
        "dotOff": tile,
        "dotOn": dot_on(parse(pal["accent"])[:3], ink[:3], (lit, fn, sheet)),
    }
    assert set(out) == set(ROLE_NAMES)
    return {k: hexrgb(v) for k, v in out.items()}


# ---- the style sheets' colours: file stem -> "object.path.property" -> role
BUTTON = {"bgMerge": "sheet", "normal.bgColor": "tile", "normal.fgColor": "ink", "normal.borderColor": "tileEdge",
          "disable.bgColor": "tile", "disable.fgColor": "tile", "disable.borderColor": "tileEdge"}
CHIP = {"bgMerge": "well", "normal.bgColor": "tile", "normal.fgColor": "ink", "normal.borderColor": "tileEdge"}
HID = {"normal.bgColor": "tile", "normal.fgColor": "ink", "normal.borderColor": "tileEdge",
       "disable.fgColor": "tile", "disable.borderColor": "tileEdge", "hotKey.fgColor": "dim"}
NAV = {"normal.bgColor": "tile", "normal.fgColor": "ink", "normal.borderColor": "tileEdge",
       "disable.bgColor": "tile", "disable.fgColor": "dim", "disable.borderColor": "tileEdge"}


def prefixed(prefix, table):
    return {prefix + k: v for k, v in table.items()}


NUMBER_KEY = {"bgMerge": "sheet", "normal.bgColor": "tile", "normal.fgColor": "ink", "normal.borderColor": "tileEdge"}
NUMBER_FN = {"bgMerge": "sheet", "normal.bgColor": "fn", "normal.fgColor": "ink", "normal.borderColor": "fnEdge"}
STYLE_ROLES = {
    "keyboard-common/style": {
        "Style": {"bgColor": "sheet", "shiftButton.normalColor": "dotOff", "shiftButton.shiftColor": "dotOn"},
        "PanelFocusStyle": {},
        "SymbolButtonStyle": {},
        "FocusStyle": {"bgColor": "lit", "fgColor": "ink", "hotKey.fgColor": "ink"},
        "ButtonStyle": BUTTON,
        "PanelButtonStyle": {"normal.bgColor": "fn", "normal.fgColor": "ink", "normal.borderColor": "fnEdge", "disable.fgColor": "dim", "disable.borderColor": "fnEdge"},
        "MoreButtonStyle": BUTTON,
        "PageCountButtonStyle": {"normal.bgColor": "tile", "normal.fgColor": "ink", "normal.borderColor": "tileEdge",
                                 "disable.bgColor": "tile", "disable.fgColor": "dim", "disable.borderColor": "tileEdge"},
        "PredictionPanelStyle": {"bgColor": "well"},
        "PredictionPanelButtonStyle": CHIP,
        "NumberKeyboardStyle": dict({"bgColor": "sheet"}, **prefixed("symbolButton.", NUMBER_FN), **prefixed("numberButton.", NUMBER_KEY),
                                    **prefixed("deleteButton.", NUMBER_FN),
                                    **{"enterButton.bgMerge": "sheet", "enterButton.normal.bgColor": "accent", "enterButton.normal.fgColor": "accentInk",
                                       "enterButton.normal.borderColor": "accent"}),
    },
    "view-global": {
        "GlobalStyle": prefixed("hidPredItem.", HID),
        "MoreButtonStyle": BUTTON,
        "PredictionPanelStyle": {"bgColor": "well"},
        "PredictionPanelButtonStyle": CHIP,
    },
    "view-chinese": {
        "ChineseStyle": prefixed("hid.hidPredItem.", HID),
        "NavButtonStyle": NAV,
        "PredictionItemStyle": dict(CHIP, **{"disable.fgColor": "dim"}),
        "ChinesePredictionPanelStyle": {},
    },
    "view-japanese": {
        "JapaneseStyle": prefixed("hid.hidPredItem.", HID),
        "JapanesePredictionPanelStyle": dict({"bgColor": "well"}, **prefixed("panelButton.", CHIP), **prefixed("moreButton.", BUTTON)),
    },
}
# The portrait sheet of each of these is the same objects in another geometry (`<Name>Portrait.qml`, or style-portrait/).
PORTRAIT_OF = ("GlobalStyle", "ChineseStyle", "JapaneseStyle")


def style_files():
    """(relative path, role table) of every style sheet the patch recolours."""
    out = []
    for folder, files in STYLE_ROLES.items():
        for stem, table in files.items():
            out.append(("%s/%s.qml" % (folder, stem), table))
            if folder == "keyboard-common/style":
                out.append(("keyboard-common/style-portrait/%s.qml" % stem, table))
            elif stem in PORTRAIT_OF:
                out.append(("%s/%sPortrait.qml" % (folder, stem), table))
    return out


HEAD_RE = re.compile(r'property\s+\w+\s+(\w+)\s*:\s*(?:\w+\s*)?\{')
COLOR_RE = re.compile(r'property\s+color\s+(?P<name>\w+)\s*:\s*"(?P<val>#[0-9A-Fa-f]{3,8}|transparent|grey|gray|white|black)"')


def object_spans(text):
    """(name, begin, end) of every `property <type> <name>: [<Type>] { ... }` object, brace-balanced, in file order."""
    out = []
    for m in HEAD_RE.finditer(text):
        depth, j = 0, m.end() - 1
        for k in range(j, len(text)):
            if text[k] == "{":
                depth += 1
            elif text[k] == "}":
                depth -= 1
                if depth == 0:
                    out.append((m.group(1), m.start(), k + 1))
                    break
        else:
            raise ValueError("unbalanced braces after " + m.group(0))
    return out


def style_colors(text):
    """(object path . name, begin of the value, end of the value, value) of every quoted colour: the path is the names of the objects around it."""
    spans = object_spans(text)
    out = []
    for m in COLOR_RE.finditer(text):
        path = [n for n, b, e in spans if b < m.start() < e]
        out.append((".".join(path + [m.group("name")]), m.start("val") - 1, m.end("val") + 1, m.group("val")))
    return out


def recolor(name, text, table, rolemap):
    """Every quoted colour of one style sheet to its role. `transparent` stays nothing; any other colour with no role is a refusal, and so
    is a role for a colour that is not there."""
    found = style_colors(text)
    seen, edits = set(), []
    for key, b, e, val in found:
        if key in seen:
            raise ValueError("%s: %s found twice" % (name, key))
        seen.add(key)
        if key in table:
            edits.append((b, e, '"%s"' % rolemap[table[key]]))
        elif val != "transparent":
            raise ValueError("%s: %s (%s) has no role in tools/maliit_patch.py" % (name, key, val))
    unused = sorted(set(table) - seen)
    if unused:
        raise ValueError("%s: roles for colours that are not there: %s" % (name, ", ".join(unused)))
    for b, e, new in sorted(edits, reverse=True):
        text = text[:b] + new + text[e:]
    return text


# ---- everything else the style sheets say: sizes of corners, edges and weights, the faces
def px(n):
    """`n` 1080p pixels in the keyboard's own units."""
    return "%d * %d / resolution" % (n, GRID)


def generic(name, text, pal):
    """The numbers that hold for every style sheet: the corner, the edge, the weight of the candidate row, the faces. Each is counted: a sheet
    that LG rewrote so that none matches is a refusal at the end (`MIN_SITES`)."""
    hits = {}

    def rx(pattern, repl, key):
        nonlocal text
        text, n = re.subn(pattern, repl, text)
        hits[key] = hits.get(key, 0) + n

    rx(r"(property int radius:\s*)12 / resolution", lambda m: m.group(1) + "%d * %d / resolution" % (pal["rowRadius"], GRID), "radius")
    rx(r"(property int borderWidth:\s*)4 / resolution", lambda m: m.group(1) + "Math.max(1, Math.round(%d * %d / resolution))" % (EDGE_WIDTH, GRID), "border")
    rx(r"(property int fontWeight:\s*)100 / resolution", lambda m: m.group(1) + str(KEY_WEIGHT), "weight")
    rx(r'(property string (?:symbolFontFamily|defaultFontFamily):\s*)"LG Smart UI"', lambda m: m.group(1) + '"Manrope"', "face")
    # The candidate bar of a hardware keyboard: LG's chip holds a word in a 39 px line (with a 39 px face the text is taller than its line and
    # sits up against the F-key's label in the corner) and the label in a 21 px line (a 36 px label is two lines of 18). A line that holds its
    # text, the word a few pixels lower, is the same chip with the two apart (HidPredictionItem.qml shrinks either to its line at larger type).
    rx(r"(property int wordHeight:\s*)78 / resolution", lambda m: m.group(1) + "%d / resolution" % HID_WORD, "hid")
    for header, pattern in (("property var word: QtObject {", r"(property int height:\s*)78 / resolution"),
                            ("property var hotKey: QtObject {", r"(property int height:\s*)42 / resolution")):
        for b, e in reversed(blocks(text, header)):
            part, n = re.subn(pattern, lambda m: m.group(1) + "%d / resolution" % (HID_WORD if "word" in header else HID_HOTKEY), text[b:e])
            text = text[:b] + part + text[e:]
            hits["hid"] = hits.get("hid", 0) + n
    return text, hits


# Sites the patch expects over the whole tree (a count that falls is LG's change).
MIN_SITES = {"radius": 8, "border": 25, "weight": 5, "face": 6, "hid": 8}


# ---- the anchored edits of single files
def font_loaders(fonts):
    return "".join('FontLoader { source: "file://%s/%s" }, ' % (fonts, f) for f in FACES).rstrip(", ")


def patch_style(name, text, pal, rolemap, fonts):
    """`Style.qml`, landscape and portrait: the faces, the glyph pictures, the primary key's style."""
    text = sub(name, text, '    property string defaultFontFamily: "LG Smart UI"\n',
               '    property string defaultFontFamily: "Manrope"\n'
               "    // Luma: the four Manrope faces; the keyboard's process does not load them anywhere else\n"
               "    property list<QtObject> lumaFonts: [%s]\n" % font_loaders(fonts))
    text = sub(name, text, "    function setDefaultFont(langCode) {\n        root.defaultFontFamily = getFontForLanguage(langCode);\n    }\n",
               "    function setDefaultFont(langCode) {\n        root.defaultFontFamily = lumaFamily(langCode, getFontForLanguage(langCode));\n    }\n\n"
               "    // Luma: Manrope for the locales LG's language table calls Latin, LG's face for every other script\n"
               "    function lumaFamily(langCode, family) {\n"
               "        try {\n"
               '            return LanguageStyle.getStyle(langCode).info.languageClass === "latin" ? "Manrope" : family;\n'
               "        } catch (e) {\n"
               "            return family;\n"
               "        }\n"
               "    }\n")
    # LG's pictures come in pairs: the light one (back.png) and the dark one for its white focus box (back_f.png). The one that reads on the
    # sheet is used in both states; a name without a pair (the portrait drag handle) is left alone.
    light = luminance(*parse(pal["sheetBottom"])[:3]) >= 0.5
    text = sub(name, text, "    function getImage(name) {\n        switch(root.resolution) {\n",
               "    // Luma: the glyphs LG draws as a light and a dark picture; one of them reads on the sheet, in both states\n"
               "    readonly property var lumaGlyphs: ({more: 1, clear: 1, back: 1, next: 1, voice_mike: 1, space_bar: 1, shift_off: 1, shift_on: 1, shift_keep_on: 1})\n"
               "    readonly property bool lumaLightSheet: %s\n\n"
               "    function getImage(name) {\n"
               '        var plain = name.replace(/_f$/, "");\n'
               "        if (lumaGlyphs[plain] === 1)\n"
               '            name = lumaLightSheet ? plain + "_f" : plain;\n'
               "        switch(root.resolution) {\n" % ("true" if light else "false"))
    enter = ('    // Luma: the primary key (Search, Enter): the accent with its ink; the rest of a panel key\n'
             '    property var enterButton: PanelButtonStyle {\n'
             '        resolution: root.resolution\n'
             '        property var normal: QtObject {\n'
             '            property color bgColor: "%s"\n'
             '            property color fgColor: "%s"\n'
             '            property color borderColor: "%s"\n'
             '            property int borderWidth: Math.max(1, Math.round(%d * %d / resolution))\n'
             '            property int fontSize: 70 / resolution\n'
             '        }\n'
             '    }\n\n' % (rolemap["accent"], rolemap["accentInk"], rolemap["accent"], EDGE_WIDTH, GRID))
    return sub(name, text, "    property var panelButton: PanelButtonStyle {resolution: root.resolution}\n\n",
               "    property var panelButton: PanelButtonStyle {resolution: root.resolution}\n\n" + enter)


def patch_buttonstyle(name, text):
    """The key's weight: ButtonBase reads `buttonStyle.fontWeight` when there is one."""
    return sub(name, text, "    property int radius: ", "    property int fontWeight: %d\n    property int radius: " % KEY_WEIGHT)


def patch_focusstyle(name, text, rolemap):
    """The ring: two pixels of the dimmed ink over the lit fill (FocusPanel.qml draws it)."""
    text = sub(name, text, "    property int bottomMargin:  6 / resolution\n",
               "    property int bottomMargin:  6 / resolution\n"
               '    property color borderColor: "%s"\n'
               "    property int borderWidth: %s\n" % (rolemap["ring"], "Math.max(1, Math.round(%d * %d / resolution))" % (RING_WIDTH, GRID)))
    return text


def patch_focuspanel(name, text):
    """The focus box: a ring on the lit fill, and no DropShadow (a blurred offscreen copy of the box for every frame it moves)."""
    text = sub(name, text, "    color: focusStyle.bgColor\n    radius: focusStyle.radius\n",
               "    color: focusStyle.bgColor\n    radius: focusStyle.radius\n"
               "    border.width: focusStyle.borderWidth\n    border.color: focusStyle.borderColor\n")
    return sub(name, text, "    layer.enabled: true\n    layer.effect: DropShadow {\n", "    layer.enabled: false\n    layer.effect: DropShadow {\n")


def patch_rightarea(name, text):
    """The Search / Enter key is the primary key."""
    text = sub(name, text, "            objectName: \"enterButton\"\n            buttonStyle: root.buttonStyle\n",
               "            objectName: \"enterButton\"\n            buttonStyle: style.enterButton\n")
    return sub(name, text, '            buttonColor: "#248351"\n', "            buttonColor: style.enterButton.normal.bgColor\n")


def patch_hiditem(name, text, word=True):
    """The candidate chip of a hardware keyboard: the word and the F-key's label each shrink into their own line at a larger type, where LG's
    run over each other (its Chinese item already has the word's)."""
    text = sub(name, text, "        font.pixelSize: buttonStyle.hotKey.fontSize\n        color: buttonStyle.hotKey.fgColor\n",
               "        font.pixelSize: buttonStyle.hotKey.fontSize\n        color: buttonStyle.hotKey.fgColor\n        fontSizeMode: Text.Fit\n")
    if word:
        text = sub(name, text, "        font.pixelSize: buttonStyle.normal.fontSize\n        elide: Text.ElideRight\n",
                   "        font.pixelSize: buttonStyle.normal.fontSize\n        elide: Text.ElideRight\n        fontSizeMode: Text.Fit\n")
    return text


# file under the staged tree -> edit; the style sheets are in style_files()
CODE_FILES = (
    ("view-global/HidPredictionItem.qml", lambda n, t, pal, rm, fonts: patch_hiditem(n, t)),
    ("view-chinese/HidPredictionItem.qml", lambda n, t, pal, rm, fonts: patch_hiditem(n, t, word=False)),
    ("view-japanese/HidPredictionItem.qml", lambda n, t, pal, rm, fonts: patch_hiditem(n, t)),
    ("keyboard-common/FocusPanel.qml", lambda n, t, pal, rm, fonts: patch_focuspanel(n, t)),
    ("keyboard-common/RightFunctionArea.qml", lambda n, t, pal, rm, fonts: patch_rightarea(n, t)),
)
STYLE_EDITS = {
    "keyboard-common/style/Style.qml": lambda n, t, pal, rm, fonts: patch_style(n, t, pal, rm, fonts),
    "keyboard-common/style-portrait/Style.qml": lambda n, t, pal, rm, fonts: patch_style(n, t, pal, rm, fonts),
    "keyboard-common/style/ButtonStyle.qml": lambda n, t, pal, rm, fonts: patch_buttonstyle(n, t),
    "keyboard-common/style-portrait/ButtonStyle.qml": lambda n, t, pal, rm, fonts: patch_buttonstyle(n, t),
    "keyboard-common/style/FocusStyle.qml": lambda n, t, pal, rm, fonts: patch_focusstyle(n, t, rm),
    "keyboard-common/style-portrait/FocusStyle.qml": lambda n, t, pal, rm, fonts: patch_focusstyle(n, t, rm),
}
STYLE_FILE = "keyboard-common/style/Style.qml"


def patch_tree(root, pal, fonts):
    root = pathlib.Path(root)
    fonts = pathlib.Path(fonts).resolve()
    for f in FACES:
        if not (fonts / f).is_file():
            raise ValueError("fonts: %s has no %s" % (fonts, f))
    marker = root / STYLE_FILE
    if not marker.is_file():
        raise ValueError("%s: LG's tree has no such file" % STYLE_FILE)
    if MARKER + ": patched" in marker.read_text(encoding="utf-8"):
        raise ValueError("already patched: stage a fresh copy of LG's tree")
    rolemap = roles(pal)
    totals = {}
    out = {}

    def load(rel):
        f = root / rel
        if not f.is_file():
            raise ValueError("%s: LG's tree has no such file" % rel)
        return f, f.read_text(encoding="utf-8")

    # Everything is computed before anything is written: a late anchor failure leaves the tree as LG's.
    for rel, table in style_files():
        f, original = load(rel)
        name = rel.rsplit("/", 1)[1]
        text = recolor(name, original, table, rolemap)
        if rel in STYLE_EDITS:
            text = STYLE_EDITS[rel](name, text, pal, rolemap, fonts)
        text, hits = generic(name, text, pal)
        for k, v in hits.items():
            totals[k] = totals.get(k, 0) + v
        if text != original:
            out[f] = text
    for rel, fn in CODE_FILES:
        f, text = load(rel)
        out[f] = fn(rel.rsplit("/", 1)[1], text, pal, rolemap, fonts)
    for key, least in MIN_SITES.items():
        if totals.get(key, 0) < least:
            raise ValueError("style sheets: %d `%s` sites found, expected at least %d" % (totals.get(key, 0), key, least))
    out[marker] = "// Luma: patched, see tools/maliit_patch.py\n" + out[marker]
    for f, text in out.items():
        f.write_text(text, encoding="utf-8")
    return sorted(str(f.relative_to(root)) for f in out)


def main(argv):
    if len(argv) != 4:
        print(__doc__.split("\n\n")[0], file=sys.stderr)
        print("usage: maliit_patch.py <staged qml dir> <palette JSON> <fonts dir>", file=sys.stderr)
        return 2
    try:
        pal = json.loads(argv[2])
        for k in NEEDED:
            if k not in pal:
                raise ValueError("palette has no " + k)
        changed = patch_tree(argv[1], pal, argv[3])
    except (ValueError, OSError) as e:
        print("maliit_patch: " + str(e), file=sys.stderr)
        return 1
    # What was rewritten, one path a line: the caller lays exactly these over LG's tree.
    print("\n".join(changed))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
