#!/usr/bin/env python3
"""Patches a staged copy of `com.webos.app.voice`'s QML (LG's Search app: the search field, the keyword rows, the buttons, the option
and confirm popups, the style sheet every other screen of the app reads) into the Luma look and removes what costs a frame.

LG's Search is one native process that draws every screen from a tree of QML (202 files, `qml/` in its app directory); its screens
are fed by LG's cloud search and its voice services, none of which the set runs under the `lean` profile, so what is reachable is
the search field with the keyboard under it, the option popup, the confirm and alert popups, and, with the services on, the keyword
rows and result cards. The style sheet (`Style.qml`) holds almost every colour of those, by object, so most of the work is a table of
roles over it; the few colours written into a component, the focus fills drawn as a white rectangle with a drop shadow, the picture
buttons and the search screen's own layout are anchored edits. What changes, all of it style (docs/research/lg-restyles-result.md,
docs/research/restyle-fixes/search/result.md):

  * Manrope for Latin text: LG's `reloadFontFamily` takes the families per locale from its language-style table; for a Latin
    locale the four faces (loaded by `FontLoader`s in the style sheet, this process does not share the compositor's fonts) take the
    place of LG Smart UI, any other script keeps LG's face. Light falls to Regular.
  * The palette's sheet under every dim backdrop and popup, ink and dim ink for text, a wash for every grey chip and a lit
    wash with a 1 px rim for the focused one, in place of LG's white fill with dark text. The chip and button rectangles that LG draws only
    as the source of a drop shadow are drawn plainly (the shadow is a blurred offscreen copy per focused item: no live shadow).
  * Mock 09 (docs/mocks/09-search-keyboard): the search field is a well 84 high with a search glyph, the typed words in 32 px semibold,
    an accent caret and, where LG shows its hint (an empty field that is not being typed in), the mic hint at its right end; the keyword
    chips and the typed suggestions are 64 high tiles on the ink wash, the focused one lights and is rimmed and its plate grows while the
    words stay put; the genre cards are photographs with rounded corners, a scrim under the foot and the name over it
    (payload/voice/Views/Components/LumaPhoto.qml, one shader quad per card); the rows are marked by one glyph each (history, star),
    tinted from the palette. The glyphs are Lucide's (the compositor's icons/lucide PNGs), recoloured here so that no shader is needed.
  * The top row (back, profile, more, close) is four 64 px round targets with Lucide glyphs on the same wash, in the field's row.
  * The AI keyword dot and highlight are the accent, read at 4.5:1 on every material.
  * The typed words of a suggestion are in the accent where they stand in it (mock 09's mark); the no-results notice's button may be 520 px
    wide, since Manrope sets wider than LG Smart UI and LG's 250 px cut "Go to Internet".
  * The mic hint says what the mock says ("Hold the mic key to speak"), where LG's string names the AI button.

Every edit is anchored to an exact fragment of LG's source and must find it exactly as many times as it expects: a missing or doubled anchor
is fatal, and so is a colour in the style sheet that has no role here (a firmware update that adds one must be looked at, not guessed).
The files this tool adds (payload/voice, the glyphs) are laid beside LG's and never over one. The caller (`tools/voice.sh stage`) works on
a fresh copy every time and leaves LG's tree in force when this fails.

usage: voice_patch.py <staged qml dir> <palette JSON from `tokens.py qml`> <fonts dir with Manrope-*.ttf>
Prints the paths it rewrote or added (relative to the staged dir), one a line.
Exit status: 0 patched, 1 an anchor is missing or the palette or fonts lack something, 2 usage.
"""
import json
import pathlib
import re
import struct
import sys
import zlib

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from notification_patch import sub, edit_blocks, blocks  # noqa: E402
from tokens import argb, contrast, luminance, parse, toward  # noqa: E402

MARKER = "Luma"

# The four faces the style sheet loads.
FACES = ("Manrope-Regular.ttf", "Manrope-Medium.ttf", "Manrope-SemiBold.ttf", "Manrope-Bold.ttf")

# Palette names the edits draw on.
NEEDED = ("sheetBottom", "row", "focus", "focusEdge", "hairline", "ink", "inkDim", "focusInk", "accent", "accentInk", "danger", "radius", "rowRadius")

# Where our own files are: the repository's payload/ (the package copies the whole tree beside tools/, so the same path holds on the set).
ROOT = pathlib.Path(__file__).resolve().parent.parent
OWN_DIR = ROOT / "payload" / "voice"
ICON_DIR = ROOT / "payload" / "compositor" / "lumatv" / "icons" / "lucide"

# What each colour of LG's style sheet becomes: `object.property` -> palette name. `None` as the object is the root of the sheet.
# A colour that is in none of these is a refusal; the ones in KEEP are LG's on purpose (see the comment by each).
ROLES = {
    "sheet": """keywordsView.bgColor mainView.bgColor contentsView.bgColor simpleContentsView.bgColor weatherRec.bgColor popup.bgColor
        micBackground.bgColor livetvRec.bgRecColor livetvRec.titleBgColor verticalRec.titleBgColor freeviewRec.titleBgColor
        qnaRec.titleBgColor youtubeRec.titleBgColor localSearchRec.titleBg_color teamSportsRec.eventBgRecColor
        individualSportsRec.eventBgRecColor popupWithCloseBtn.bg_color cpSelectionPopup.popup_bg_color epgSchedulePopup.popup_bg_color
        nextProgramView.bgColor nextProgramView.popup_bg_color alarmView.popup_bg_color searchView.listView_dim_color
        notificationPopup.backgroundColor root.normalColor""",
    "ink": """keywordsView.itemNormalTextColor keywordsView.itemFocusTextColor keywordsView.itemRelatedFocusTextColor
        focusableText.normalFontColor focusableText.focusedFontColor focusableSwitch.normalFontColor focusableSwitch.focusedFontColor
        focusableSwitch.handle_enabled_checked_color livetvRec.textColor livetvRec.titleTextColorOnair livetvRec.focusedTextColor
        verticalRec.textColor verticalRec.focusedTextColor appRec.titleTextColor appRec.textFocusColor recentAppRec.titleTextColor
        recentAppRec.textFocusColor webRec.titleTextColor freeviewRec.textColor freeviewRec.focusedTextColor weatherRec.textNormalColor
        weatherRec.textFocusedColor timeRec.textNormalColor timeRec.textFocusedColor youtubeRec.textColor localSearchRec.textColor
        iotRec.textTitleColor iotRec.textDetailColor iotRec.textDisconnColor teamSportsRec.titleColor teamSportsRec.eventTextColor
        individualSportsRec.titleColor individualSportsRec.playerTextColor individualSportsRec.eventTextColor
        noResultItem.textFocusedColor popupWithCloseBtn.titleFont_color popupWithCloseBtn.description_color popup.textColor
        cpSelectionPopup.cpName_normalColor cpSelectionPopup.cpName_focusColor cpSelectionPopup.titleFont_color
        optionPopup.itemFocusTextColor optionPopup.itemNormalTextColor optionPopup.titleFont_color epgSchedulePopup.titleFont_color
        nextProgramView.titleFont_color alarmView.titleFont_color searchView.systemFeedback_color tooltip.fontColor
        buttonFlex.normalFontColor buttonFlex.focusedFontColor curtainPageButton.normalFontColor curtainPageButton.focusedFontColor
        knowMeButton.normalFontColor notiView.messageFontColor descriptionBtn.normalFontColor descriptionBtn.focusedFontColor
        notificationPopup.foregroundColor""",
    "dim": """contentsView.dividerFontColor simpleContentsView.dividerFontColor focusableSwitch.handle_enabled_unchecked_color
        focusableSwitch.handle_disabled_color livetvRec.titleTextColorProgram appRec.genreTextColor weatherRec.textDeviderColor
        timeRec.dateTextColor iotRec.textValueColor iotRec.textDisabledColor noResultItem.textNormalColor
        cpSelectionPopup.cpInfo_normalColor""",
    "hairline": """keywordsView.itemNormalbgColor contentsView.dividerLineColor simpleContentsView.dividerLineColor
        searchInputBox.textinputwrapper_border_normal_color
        focusableSwitch.groove_disabled_color livetvRec.timebarBgColor weatherRec.selectRecColor weatherRec.frameDividerColor
        timeRec.selectRecColor iotRec.recColor iotRec.recDisconnColor iotRec.deviderColor teamSportsRec.bgRecColor
        teamSportsRec.timebarBgColor individualSportsRec.bgRecColor individualSportsRec.timebarBgColor popupWithCloseBtn.divider_color
        cpSelectionPopup.popup_bgBorder_color epgSchedulePopup.popup_bgBorder_color nextProgramView.popup_bgBorder_color
        alarmView.popup_bgBorder_color buttonFlex.normalColor buttonFlex.borderColor curtainPageButton.normalColor
        knowMeButton.normalColor descriptionBtn.normalColor descriptionBtn.borderColor""",
    "lit": """keywordsView.itemFocusbgColor focusableText.focusedColor
        focusableSwitch.focusedColor weatherRec.selectRecFocusedColor timeRec.selectRecFocusedColor iotRec.focusedRecColor
        iotRec.btnFocusedColor optionPopup.focusedColor buttonFlex.focusedColor curtainPageButton.focusedColor
        knowMeButton.focusedColor descriptionBtn.focusedColor""",
    "ring": """curtainPageButton.borderColor""",
    "edge": """focusableSwitch.groove_enabled_unchecked_color iotRec.indicatorColor popupWithCloseBtn.bgBorder_color""",
    "accent": """focusableSwitch.groove_enabled_checked_color livetvRec.timebarColor
        teamSportsRec.timebarColor individualSportsRec.timebarColor""",
    # Mock 09: the search field is a well, and its edge brightens a little while it is the one that takes the words.
    "well": """searchInputBox.textinputwrapper_normal_color searchInputBox.textinputwrapper_focused_color""",
    "edgeRaised": """searchInputBox.textinputwrapper_border_activefocus_color""",
    # The AI keywords' words are the accent, moved toward the ink where the material needs it for 4.5:1.
    "accentText": """keywordsView.itemRelatedNormalTextColor""",
    "danger": "mic.focusedColor",
}
# LG's on purpose: transparent is nothing, `scrimColor` and the picture backdrops under a card are black behind a picture, the pink is
# the select colour of a list nobody on this set reaches, the three orange ones are the IP channel's mark, the blue is the web tile's
# brand colour.
KEEP = """root.baseColor root.disabledColor root.selectColor root.scrimColor verticalRec.thumbnailBgColor freeviewRec.thumbnailBgColor
    qnaRec.thumbnailBgColor youtubeRec.thumbnailBgColor webRec.bgColor livetvRec.ipChannelColor teamSportsRec.ipChannelColor
    individualSportsRec.ipChannelColor nextProgramView.ipChannelColor iotRec.btnNormalColor notiView.color mic.bgColor""".split()

PAL_OF = {"sheet": "sheetBottom", "ink": "ink", "dim": "inkDim", "hairline": "hairline", "lit": "focusEdge", "ring": "inkDim", "edge": "focusEdge",
          "accent": "accent", "danger": "danger", "well": "well", "edgeRaised": "edgeRaised", "accentText": "accentText"}

COLOR_RE = re.compile(r'^\s*property\s+(?:color|string)\s+(?P<name>\w+)\s*:\s*"(?P<val>#[0-9a-fA-F]{3,8}|transparent|white|black)"', re.M)
BLOCK_RE = re.compile(r'property\s+(?:var\s+)?(?:QtObject\s+)?(\w+)\s*:\s*QtObject\s*\{')


# ---- the palette the edits draw on: the shipped roles and the few more mock 09 needs, all computed from them ------------------------------

def over(top, bottom):
    """`top` ((r, g, b, a)) drawn over the opaque `bottom` ((r, g, b)), in the colour space Qt blends in."""
    r, g, b, a = top
    return tuple(int(round(t * a + u * (1 - a))) for t, u in zip((r, g, b), bottom))


def readable(fg, grounds, ink, need=4.5):
    """`fg` ((r, g, b)) taken toward the ink until it reads `need`:1 on every ground; the colour itself when it already does."""
    for _ in range(12):
        if all(contrast(fg, g) >= need for g in grounds):
            break
        fg = toward(fg, ink, 0.12)
    return fg


def derive(pal):
    """`pal` and, beside its roles, the ones the Search look draws with (values of docs/mocks/lucent.css, dark and light):
    `wash` the tile wash (--tile-ink-wash, 10 % of the ink), `washLit` the same glass brought forward by focus, `edgeRaised` the raised edge,
    `well` (--well) with the two lips of its inset (--well-inset), and `accentText`, the accent as words."""
    ink = parse(pal["ink"])[:3]
    sheet = parse(pal["sheetBottom"])[:3]
    dark = luminance(*sheet) < 0.5
    out = dict(pal)
    out["wash"] = argb(*ink, 0.10)
    out["washLit"] = argb(*ink, 0.18)
    out["edgeRaised"] = argb(*ink, 0.20)
    if dark:
        out["well"], out["lipDark"], out["lipLight"] = argb(8, 6, 16, 0.34), argb(0, 0, 0, 0.32), argb(255, 255, 255, 0.08)
    else:
        out["well"], out["lipDark"], out["lipLight"] = argb(23, 19, 31, 0.08), argb(23, 19, 31, 0.12), argb(255, 255, 255, 0.86)
    lit = over(parse(out["washLit"]), sheet)
    out["accentText"] = argb(*readable(parse(pal["accent"])[:3], (sheet, lit), ink), 1.0)
    return out


# ---- glyphs: Lucide's white-on-transparent PNGs, recoloured so that the app needs no shader to tint them ---------------------------------

# name of the glyph -> the palette role it is drawn in
ICONS = {"search": "inkDim", "mic": "inkDim", "history": "inkDim", "star": "inkDim",
         "x": "ink", "ellipsis": "ink", "arrow-left": "ink", "user": "ink"}
ICON_SUBDIR = "Views/images/luma"
PNG_MAGIC = b"\x89PNG\r\n\x1a\n"


def png_read(data):
    """(width, height, rows) of an 8-bit RGBA PNG without interlace, each row a bytearray of r, g, b, a."""
    if data[:8] != PNG_MAGIC:
        raise ValueError("not a PNG")
    pos, idat, head = 8, b"", None
    while pos < len(data):
        n, kind = struct.unpack(">I4s", data[pos:pos + 8])
        body = data[pos + 8:pos + 8 + n]
        if kind == b"IHDR":
            head = struct.unpack(">IIBBBBB", body)
        elif kind == b"IDAT":
            idat += body
        pos += 12 + n
    if head is None or head[2:] != (8, 6, 0, 0, 0):
        raise ValueError("not an 8-bit RGBA PNG without interlace")
    w, h = head[0], head[1]
    raw = zlib.decompress(idat)
    stride, rows, prev = w * 4, [], bytearray(w * 4)
    for y in range(h):
        kind, line = raw[y * (stride + 1)], bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
        for i in range(stride):
            a = line[i - 4] if i >= 4 else 0
            b = prev[i]
            c = prev[i - 4] if i >= 4 else 0
            if kind == 1:
                line[i] = (line[i] + a) & 255
            elif kind == 2:
                line[i] = (line[i] + b) & 255
            elif kind == 3:
                line[i] = (line[i] + ((a + b) >> 1)) & 255
            elif kind == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                line[i] = (line[i] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
            elif kind != 0:
                raise ValueError("bad PNG filter %d" % kind)
        rows.append(line)
        prev = line
    return w, h, rows


def png_write(w, h, rows):
    def chunk(kind, body):
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body) & 0xFFFFFFFF)
    raw = b"".join(b"\x00" + bytes(r) for r in rows)
    return PNG_MAGIC + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")


def png_tint(data, colour):
    """A Lucide glyph (white strokes, alpha is the shape) in `colour` ((r, g, b, a) with a in 0..1: the alpha scales the glyph's own)."""
    w, h, rows = png_read(data)
    r, g, b, a = colour
    out = []
    for line in rows:
        row = bytearray(len(line))
        for x in range(0, len(line), 4):
            row[x], row[x + 1], row[x + 2] = r, g, b
            row[x + 3] = int(round(line[x + 3] * a))
        out.append(row)
    return png_write(w, h, out)


def icon_files(root, pal):
    """{path: bytes} of every glyph in the role it is drawn in."""
    out = {}
    for name, role in sorted(ICONS.items()):
        src = ICON_DIR / (name + ".png")
        if not src.is_file():
            raise ValueError("icons: %s has no %s.png" % (ICON_DIR, name))
        out[root / ICON_SUBDIR / (name + ".png")] = png_tint(src.read_bytes(), parse(pal[role]))
    return out


def own_files(root):
    """{path: bytes} of payload/voice laid into the tree at the same relative path. A path LG's tree has is fatal: it would replace its file."""
    if not OWN_DIR.is_dir():
        raise ValueError("%s: our own QML files are not there" % OWN_DIR)
    out = {}
    for f in sorted(OWN_DIR.rglob("*")):
        if f.is_file():
            dest = root / f.relative_to(OWN_DIR)
            if dest.exists():
                raise ValueError("%s: LG's tree already has this file" % f.relative_to(OWN_DIR))
            out[dest] = f.read_bytes()
    return out


# ---- the style sheet ----------------------------------------------------------------------------------------------------------------------

def style_colors(text):
    """(object, name, begin of value, end of value) of every quoted colour of the style sheet, where the object is `root` outside
    every QtObject and the first-level object's name inside one."""
    blocks = [(m.start(), m.group(1)) for m in BLOCK_RE.finditer(text)]
    out = []
    for m in COLOR_RE.finditer(text):
        owner = "root"
        for at, name in blocks:
            if at < m.start():
                owner = name
        out.append((owner, m.group("name"), m.start("val") - 1, m.end("val") + 1, m.group("val")))
    return out


def recolor_style(text, pal):
    """Every quoted colour of the sheet to its role. `property color` values that are not quoted (bindings) are LG's wiring and stay."""
    role_of = {}
    for role, names in ROLES.items():
        for n in names.split():
            if n in role_of:
                raise ValueError("Style.qml: %s is given two roles" % n)
            role_of[n] = role
    for n in KEEP:
        if n in role_of:
            raise ValueError("Style.qml: %s is both kept and given a role" % n)
    found = style_colors(text)
    seen = set()
    edits = []
    for owner, name, b, e, val in found:
        key = owner + "." + name
        if key in seen:
            raise ValueError("Style.qml: %s found twice" % key)
        seen.add(key)
        if key in KEEP:
            continue
        if key not in role_of:
            if val == "transparent":
                continue
            raise ValueError("Style.qml: %s (%s) has no role in tools/voice_patch.py" % (key, val))
        edits.append((b, e, '"%s"' % pal[PAL_OF[role_of[key]]]))
    unused = [n for n in list(role_of) + KEEP if n not in seen]
    if unused:
        raise ValueError("Style.qml: roles for colours that are not there: " + ", ".join(sorted(unused)))
    for b, e, new in sorted(edits, reverse=True):
        text = text[:b] + new + text[e:]
    return text


def style_edits(pal):
    """The sheet's own additions and the numbers that are not colours."""
    return [
        ("    property bool isUS: serviceCountry === \"US\"\n",
         "    property bool isUS: serviceCountry === \"US\"\n"
         "\n"
         "    // Luma: the palette, for the components that draw their own rectangles (tools/voice_patch.py)\n"
         "    property color lumaSheet: \"%s\"\n"
         "    property color lumaInk: \"%s\"\n"
         "    property color lumaInkDim: \"%s\"\n"
         "    property color lumaHairline: \"%s\"\n"
         "    property color lumaLit: \"%s\"\n"
         "    property color lumaRing: \"%s\"\n"
         "    property color lumaAccent: \"%s\"\n"
         "    property color lumaAccentText: \"%s\"\n"
         "    property color lumaAccentInk: \"%s\"\n"
         "    property color lumaWash: \"%s\"\n"
         "    property color lumaWashLit: \"%s\"\n"
         "    property color lumaEdgeRaised: \"%s\"\n"
         "    property color lumaWell: \"%s\"\n"
         "    property color lumaLipDark: \"%s\"\n"
         "    property color lumaLipLight: \"%s\"\n"
         "    property int lumaSheetRadius: %d\n"
         "    property int lumaRowRadius: %d\n"
         % (pal["sheetBottom"], pal["ink"], pal["inkDim"], pal["hairline"], pal["focusEdge"], pal["inkDim"], pal["accent"], pal["accentText"], pal["accentInk"],
            pal["wash"], pal["washLit"], pal["edgeRaised"], pal["well"], pal["lipDark"], pal["lipLight"], pal["radius"], pal["rowRadius"])),
        ('property color focusColor: supportBNO ? "#68819b" : "#E6E6E6"', 'property color focusColor: "%s"' % pal["focusEdge"]),
    ]


def fonts_edit(text, fonts):
    """The faces, and Manrope as the family of every Latin locale. LG's `reloadFontFamily` copies four families out of its language table;
    each assignment goes through `lumaFamily`, which keeps LG's for any other script."""
    name = "Style.qml"
    loaders = "".join('FontLoader { source: "file://%s/%s" }, ' % (fonts, f) for f in FACES).rstrip(", ")
    text = sub(name, text, "    property var fontSet\n\n    property var languageStyle\n",
               "    property var fontSet\n\n    property var languageStyle\n\n"
               "    // Luma: the four Manrope faces; this process does not share the compositor's\n"
               "    property list<QtObject> lumaFonts: [%s]\n\n"
               "    function lumaFamily(style, family) {\n"
               "        return style.info.languageClass === \"latin\" ? \"Manrope\" : family;\n"
               "    }\n" % loaders)
    for w in ("300", "400", "600", "700"):
        text = sub(name, text, "normal%s_fontFamily = style.normal%s.fontFamily[0];" % (w, w),
                   "normal%s_fontFamily = lumaFamily(style, style.normal%s.fontFamily[0]);" % (w, w))
    return text


def patch_style(text, pal, fonts):
    name = "Style.qml"
    text = recolor_style(text, pal)
    for old, new in style_edits(pal):
        text = sub(name, text, old, new)
    # The search field is a well, whole at rest (the sheet's palette values carry their own translucency).
    text = edit_blocks(name, text, "property QtObject searchInputBox: QtObject {", [0], [
        ("property real textinputwrapper_normal_opacity: 0.2", "property real textinputwrapper_normal_opacity: 1"),
    ])
    # The sheets LG lays at 70-85 % under its white words are the whole sheet (the palette's carries its own translucency).
    for header, old, new in (("property QtObject keywordsView: QtObject {", "property real bgOpacity: root.transluscent ? 1 : 0.7", "property real bgOpacity: 1"),
                             ("property QtObject mainView: QtObject {", "property real bgOpacity: root.transluscent ? 1 : 0.8", "property real bgOpacity: 1"),
                             ("property QtObject contentsView: QtObject {", "property real bgOpacity: root.transluscent ? 1 : 0.8", "property real bgOpacity: 1"),
                             ("property QtObject simpleContentsView: QtObject {", "property real bgOpacity : 0.8", "property real bgOpacity : 1")):
        text = edit_blocks(name, text, header, [0], [(old, new)])
    # A sheet that is already translucent is not made translucent again.
    text = edit_blocks(name, text, "property QtObject cpSelectionPopup: QtObject {", [0], [
        ("property real popup_bg_opacity: 0.9", "property real popup_bg_opacity: 1"),
    ])
    text = edit_blocks(name, text, "property QtObject notificationPopup: QtObject {", [0], [
        ("property real   backgroundOpacity:      0.95", "property real   backgroundOpacity:      1"),
        ("property int    radius:                 getRelativeWidth(12)", "property int    radius:                 lumaSheetRadius"),
    ])
    for header in ("property QtObject buttonFlex: QtObject {", "property QtObject focusableText: QtObject {"):
        text = edit_blocks(name, text, header, [0], [
            ("property int radius: getRelativeHeight(6)", "property int radius: lumaRowRadius"),
        ])
    # Manrope sets wider than LG Smart UI, and LG's 250 px ceiling cut the notice's "Go to Internet" to "Go to Int..."; the ceiling is 520, and
    # as wide again at twice the text.
    text = edit_blocks(name, text, "property QtObject buttonFlex: QtObject {", [0], [
        ("property int flexMaxWidth: getRelativeHeight(250)",
         "property int flexMaxWidth: Math.round(getRelativeHeight(520) * getRelativeFontSize(36) / 36)"),
    ])
    return fonts_edit(text, fonts)


def patch_main(text):
    name = "Main.qml"
    for w in ("300", "400", "600", "700"):
        text = sub(name, text, 'property string normal%s_fontFamily: "LG Smart UI"' % w, 'property string normal%s_fontFamily: "Manrope"' % w)
    return text


def patch_stringset(text):
    """The mic hint of the empty field: LG's sentence names the AI button; the mock's is shorter and names the key."""
    return sub("StringSet.qml", text, 'qsTr("Hold the AI button on the remote and search with your voice.")', 'qsTr("Hold the mic key to speak")')


def recolor(name, text, table):
    """`table`: (quoted literal, replacement expression, how many times it is there)."""
    for old, new, count in table:
        text = sub(name, text, old, new, count)
    return text


INK = "voiceStyle.lumaInk"
DIM = "voiceStyle.lumaInkDim"
HAIR = "voiceStyle.lumaHairline"
FOCUS = "voiceStyle.lumaLit"
EDGE = "voiceStyle.lumaRing"
SHEET = "voiceStyle.lumaSheet"
ACCENT = "voiceStyle.lumaAccent"
ROW_RADIUS = "voiceStyle.lumaRowRadius"
WASH = "voiceStyle.lumaWash"
WASH_LIT = "voiceStyle.lumaWashLit"

# 1080p pixels, mock 09's and the TV profile's: the field and the top row share a centre line, the keyword rows start where the field does
# and end at the safe edge, five chips or six cards across (312 x 5 + 22 x 4 = 258 x 6 + 20 x 5 = 1648).
SAFE_X = 96
FIELD_X, FIELD_TOP, FIELD_W, FIELD_H = 176, 54, 1384, 84
FIELD_PX, HINT_PX, CHIP_PX, CARD_PX = 32, 24, 27, 28
CHIP_W, CHIP_H, CHIP_GAP = 312, 64, 22
CARD_W, CARD_H, CARD_GAP = 258, 96, 20
BUTTON = 64             # the top row's targets (the TV's 64)
BUTTON_GAP = 24
CHIP_SCALE, CARD_SCALE = 1.04, 1.03       # theme.json focus.buttonScale and cardScale: the plate grows, the words stay
RIM = 1                 # px: the rim of a focused plate (Nitin 2026-10-23: no line under the focused item, a lifted brighter glass with a rim)


def shadow_off(name, text, source_id, visible_expr):
    """The drop shadow whose `source` is `source_id`: off. `visible_expr` is what LG shows it on. The shadow's item is blurred offscreen
    for every frame it is visible, which is the cost the frame gate forbids."""
    found = [(b, e) for b, e in blocks(text, "DropShadow {") if re.search(r"^\s*source: %s\s*$" % re.escape(source_id), text[b:e], re.M)]
    if len(found) != 1:
        raise ValueError("%s: %d drop shadows with source %s, expected 1" % (name, len(found), source_id))
    b, e = found[0]
    part = sub("%s DropShadow(%s)" % (name, source_id), text[b:e], "visible: %s\n" % visible_expr, "visible: false\n")
    return text[:b] + part + text[e:]


def patch_searchinputbox(text):
    name = "SearchInputBox.qml"
    # The caret is the accent, a little taller than LG's and rounded (mock 09: 3 x 38).
    text = sub(name, text, '                width: getRelativeWidth(3)\n                height: getRelativeHeight(35)\n                color: "#4C5059"\n',
               '                width: getRelativeWidth(3)\n                height: getRelativeHeight(38)\n                radius: 2\n                color: %s\n' % ACCENT)
    # The words are 32 px semibold, whether the field has the focus or not: LG's weight change on focus moved every glyph.
    text = sub(name, text,
               '            tmpColor= "#E6E6E6";\n            tmpPixelSize = getRelativeFontSize(30);\n            tmpFamily = normal400_fontFamily;\n            tmpWeight = normal400_fontWeight;\n',
               '            tmpColor= "#E6E6E6";\n            tmpPixelSize = getRelativeFontSize(%d);\n            tmpFamily = normal600_fontFamily;\n            tmpWeight = normal600_fontWeight;\n' % FIELD_PX)
    text = sub(name, text, '            tmpColor= "#4C5059";\n            tmpPixelSize = getRelativeFontSize(30);\n',
               '            tmpColor= "#4C5059";\n            tmpPixelSize = getRelativeFontSize(%d);\n' % FIELD_PX)
    text = sub(name, text,
               '            font.pixelSize: getRelativeFontSize(30)\n            font.family: root.isFocused?normal600_fontFamily:normal400_fontFamily\n'
               '            font.weight: root.isFocused?normal600_fontWeight:normal400_fontWeight\n\n            clip: false\n',
               '            font.pixelSize: getRelativeFontSize(%d)\n            font.family: normal600_fontFamily\n'
               '            font.weight: normal600_fontWeight\n\n            clip: false\n' % FIELD_PX)
    # The hint is at the field's right end with the mic before it, where LG's runs from the left edge over the caret. It shows where LG shows it
    # (an empty field that is not being typed in): the mic key is Home's, and LG's Search has no listener behind it on this set.
    text = sub(name, text,
               '                anchors {\n                    left: parent.left\n                    verticalCenter: parent.verticalCenter\n                }\n'
               '                width: parent.width\n                font.pixelSize: getRelativeFontSize(30)\n'
               '                font.family: root.isFocused?normal600_fontFamily:normal400_fontFamily\n'
               '                font.weight: root.isFocused?normal600_fontWeight:normal400_fontWeight\n'
               '                fontColor: root.isFocused? "#664C5059":"#66E6E6E6"\n'
               '                text: stringSet.getQstr("search_230")\n'
               '                visible: (globalVars.isTyping === false && textInput.text.length === 0)\n            }\n',
               '                anchors {\n                    right: parent.right\n                    verticalCenter: parent.verticalCenter\n                }\n'
               '                width: Math.min(implicitWidth, parent.width - getRelativeWidth(%d))\n'
               '                font.pixelSize: getRelativeFontSize(%d)\n'
               '                font.family: normal400_fontFamily\n'
               '                font.weight: normal400_fontWeight\n'
               '                fontColor: %s\n'
               '                text: stringSet.getQstr("search_230")\n'
               '                visible: (globalVars.isTyping === false && textInput.text.length === 0)\n            }\n\n'
               '            Image {\n                anchors {\n                    right: defaultText.left; rightMargin: getRelativeWidth(10)\n                    verticalCenter: parent.verticalCenter\n                }\n'
               '                width: getRelativeWidth(26)\n                height: getRelativeWidth(26)\n                sourceSize.width: 52\n                sourceSize.height: 52\n'
               '                source: "../images/luma/mic.png"\n                smooth: true\n                visible: defaultText.visible\n            }\n' % (26 + 10, HINT_PX, DIM))
    text = recolor(name, text, [
        ('"#4C5059"', INK, 3),
        ('"#E6E6E6"', INK, 4),
        ('"#999999"', DIM, 1),
        ('"#575E66"', INK, 1),
    ])
    # The field: a well (mock 09), its search glyph at the left, the words after it; the lips are the two edges of the inset.
    text = sub(name, text,
               '    Rectangle {\n        id: listSearchBg\n        anchors.fill: textInputWrapper\n'
               '        color: root.isFocused?root.style.textinputwrapper_focused_color:root.style.textinputwrapper_normal_color\n'
               '        opacity: root.isFocused?root.style.textinputwrapper_focused_opacity:root.style.textinputwrapper_normal_opacity\n'
               '        border.color: root.isFocused?root.style.textinputwrapper_border_activefocus_color:root.style.textinputwrapper_border_normal_color\n'
               '        border.width: getRelativeWidth(2)\n        clip: true\n        radius: getRelativeWidth(6)\n    }\n',
               '    Rectangle {\n        id: listSearchBg\n        anchors.fill: textInputWrapper\n'
               '        color: root.isFocused?root.style.textinputwrapper_focused_color:root.style.textinputwrapper_normal_color\n'
               '        opacity: root.isFocused?root.style.textinputwrapper_focused_opacity:root.style.textinputwrapper_normal_opacity\n'
               '        border.color: root.isFocused?root.style.textinputwrapper_border_activefocus_color:root.style.textinputwrapper_border_normal_color\n'
               '        border.width: 1\n        clip: true\n        radius: %(row)s\n        antialiasing: true\n\n'
               '        // the inset: a dark lip along the top, a light return along the bottom, between the corners\n'
               '        Rectangle { x: parent.radius; y: 1; width: parent.width - 2 * parent.radius; height: 2; color: voiceStyle.lumaLipDark }\n'
               '        Rectangle { x: parent.radius; y: parent.height - 3; width: parent.width - 2 * parent.radius; height: 2; color: voiceStyle.lumaLipLight }\n'
               '    }\n\n'
               '    Image {\n        x: getRelativeWidth(26)\n        anchors.verticalCenter: listSearchBg.verticalCenter\n'
               '        width: getRelativeWidth(32)\n        height: getRelativeWidth(32)\n        sourceSize.width: 64\n        sourceSize.height: 64\n'
               '        source: "../images/luma/search.png"\n        smooth: true\n    }\n' % {"row": ROW_RADIUS})
    text = sub(name, text, '            fill: textInputWrapper\n            leftMargin: getRelativeWidth(24)\n            rightMargin: getRelativeWidth(24)\n',
               '            fill: textInputWrapper\n            leftMargin: getRelativeWidth(%d)\n            rightMargin: getRelativeWidth(26)\n' % (26 + 32 + 18 - 4))
    # The typed suggestions under the field: full-width rows of the wash with a search glyph, the focused one lit and ringed.
    text = sub(name, text, '            height: getRelativeHeight(48)\n            property int textSideMargin: getRelativeWidth(24)\n',
               '            height: Math.max(getRelativeHeight(%d), Math.round(getRelativeFontSize(%d) * 1.4) + getRelativeHeight(20))\n'
               '            property int textSideMargin: getRelativeWidth(24)\n' % (CHIP_H, CHIP_PX))
    text = sub(name, text, "        header: Item {  //Gap\n            width: getRelativeWidth(1)\n            height: getRelativeHeight(30)\n",
               "        header: Item {  //Gap\n            width: getRelativeWidth(1)\n            height: getRelativeHeight(24)\n")
    text = sub(name, text, '                width: textDelegateRoot.getWidth()\n                normalColor: "#33FFFFFF"\n',
               '                width: textDelegateRoot.width\n                normalColor: %s\n' % HAIR)
    text = sub(name, text,
               '                Rectangle {\n                    anchors.fill: parent\n                    color: focusableItem.isFocused ? parent.focusedColor : parent.normalColor\n'
               '                    radius: getRelativeWidth(6)\n                    border.color: "#33E6E6E6"\n                    border.width: getRelativeWidth(1)\n                }\n',
               '                Rectangle {\n                    anchors.fill: parent\n                    color: focusableItem.isFocused ? %s : %s\n'
               '                    radius: %s\n                    border.color: %s\n                    border.width: focusableItem.isFocused ? %d : 0\n                    antialiasing: true\n                }\n\n'
               '                Image {\n                    x: getRelativeWidth(22)\n                    anchors.verticalCenter: parent.verticalCenter\n'
               '                    width: getRelativeWidth(28)\n                    height: getRelativeWidth(28)\n                    sourceSize.width: 56\n                    sourceSize.height: 56\n'
               '                    source: "../images/luma/search.png"\n                    smooth: true\n                }\n' % (WASH_LIT, WASH, ROW_RADIUS, EDGE, RIM))
    text = sub(name, text,
               '                        leftMargin: textDelegateRoot.textSideMargin\n                        rightMargin: textDelegateRoot.textSideMargin\n',
               '                        leftMargin: textDelegateRoot.textSideMargin + getRelativeWidth(44)\n                        rightMargin: textDelegateRoot.textSideMargin\n')
    text = sub(name, text,
               '                    font.family: focusableItem.isFocused ? normal600_fontFamily:normal400_fontFamily\n'
               '                    font.weight: focusableItem.isFocused? normal600_fontWeight:normal400_fontWeight\n',
               '                    font.family: normal600_fontFamily\n                    font.weight: normal600_fontWeight\n')
    # The typed words, where they stand in a suggestion, in the accent (mock 09's mark). The row's own MarqueeText stays for what LG does with
    # it (the audio guidance reads its text, the marquee runs on it) and is not drawn: a row is as wide as the field, so a suggestion does not
    # reach the marquee's length.
    esc = '.replace(/&/g, "&amp;").replace(/</g, "&lt;")'
    words = ('                Text {\n'
             '                    anchors {\n'
             '                        left: parent.left; leftMargin: textDelegateRoot.textSideMargin + getRelativeWidth(44)\n'
             '                        right: parent.right; rightMargin: textDelegateRoot.textSideMargin\n'
             '                        verticalCenter: parent.verticalCenter\n'
             '                    }\n'
             '                    elide: Text.ElideRight\n'
             '                    textFormat: Text.StyledText\n'
             '                    font.pixelSize: getRelativeFontSize(%d)\n'
             '                    font.family: normal600_fontFamily\n'
             '                    font.weight: normal600_fontWeight\n'
             '                    color: %s\n'
             '                    readonly property string typed: textInput.displayText\n'
             '                    readonly property int at: typed === "" ? -1 : keyword.toLowerCase().indexOf(typed.toLowerCase())\n'
             '                    text: at < 0 ? keyword%s : keyword.substring(0, at)%s + "<font color=\\"" + voiceStyle.lumaAccentText + "\\">" + keyword.substr(at, typed.length)%s + "</font>" + keyword.substring(at + typed.length)%s\n'
             '                }\n\n') % (CHIP_PX, INK, esc, esc, esc, esc)
    text = sub(name, text, '                MarqueeText {\n                    id: textKeyword\n                    objectName: category + "TextKeyword"\n                    clip: true\n',
               words + '                MarqueeText {\n                    id: textKeyword\n                    objectName: category + "TextKeyword"\n                    visible: false\n                    clip: true\n')
    return text


def patch_searchview(text):
    name = "SearchView.qml"
    text = sub(name, text, 'searchInputBox.textColor = searchInputBox.isFocused?"#4C5059":"#E6E6E6";', "searchInputBox.textColor = " + INK + ";")
    # LG's search is drawn straight over the picture in white; ink that follows the theme needs the sheet under it, whole, on every theme.
    # The dim backdrop that LG shows once there are results becomes the sheet too, and while there are none (the full-screen search field)
    # the sheet is drawn under the field.
    text = sub(name, text, '                color: "#000000"\n                opacity: 0.85\n', "                color: %s\n                opacity: 1\n" % SHEET)
    text = sub(name, text, "        FocusScope{\n            id: genreListWrapper\n            clip: true\n            anchors.fill: parent\n",
               "        FocusScope{\n            id: genreListWrapper\n            clip: true\n            anchors.fill: parent\n\n"
               "            Rectangle {\n                id: lumaSheet\n                anchors.fill: parent\n                color: %s\n                visible: globalVars.isFullMode\n            }\n" % SHEET)
    # The field: mock 09's 84 px well, where the keyword rows start; the top row's targets are on its centre line.
    text = sub(name, text,
               "                top: parent.top; topMargin: getRelativeHeight(51)\n"
               "                left: parent.left; leftMargin: (main.isPortraitMode ? getRelativeWidth(135) : getRelativeWidth(600))\n"
               "            }\n"
               "            width: main.isPortraitMode ? getRelativeWidth(537) : getRelativeWidth(720)\n"
               "            height: getRelativeHeight(66)\n",
               "                top: parent.top; topMargin: getRelativeHeight(%d)\n"
               "                left: parent.left; leftMargin: (main.isPortraitMode ? getRelativeWidth(135) : getRelativeWidth(%d))\n"
               "            }\n"
               "            width: main.isPortraitMode ? getRelativeWidth(537) : getRelativeWidth(%d)\n"
               "            height: Math.max(getRelativeHeight(%d), Math.round(getRelativeFontSize(%d) * 1.4) + getRelativeHeight(28))\n"
               % (FIELD_TOP, FIELD_X, FIELD_W, FIELD_H, FIELD_PX))
    text = sub(name, text, "                top: searchInputBox.bottom;topMargin: getRelativeHeight(165/2)\n",
               "                top: searchInputBox.bottom;topMargin: getRelativeHeight(40)\n")
    return patch_toprow(name, text)


def patch_toprow(name, text):
    """Back, profile, more and close: 64 px targets on one centre line with the field, the profile's initial in the ink that reads on its colour."""
    centre = FIELD_TOP + FIELD_H // 2
    top = centre - BUTTON // 2
    # the close key is the last of the row; the others are placed from it
    text = sub(name, text, "                top: parent.top; topMargin: getRelativeHeight(66)\n                right: parent.right; rightMargin: getRelativeWidth(72)\n",
               "                top: parent.top; topMargin: getRelativeHeight(%d)\n                right: parent.right; rightMargin: getRelativeWidth(%d)\n" % (top, SAFE_X))
    text = sub(name, text, "                right: quitButton.left; rightMargin: getRelativeWidth(64)\n",
               "                right: quitButton.left; rightMargin: getRelativeWidth(%d)\n" % BUTTON_GAP)
    text = sub(name, text, "                right: moreButton.left; rightMargin: getRelativeWidth(114/2)\n",
               "                right: moreButton.left; rightMargin: getRelativeWidth(%d)\n" % BUTTON_GAP)
    text = sub(name, text, "                left: parent.left; leftMargin: getRelativeWidth(72)\n",
               "                left: parent.left; leftMargin: getRelativeWidth(%d)\n" % SAFE_X)
    # each of the four ButtonIcons: our glyph in both states, the 64 px plate (the plate's look is ButtonIcon's own, patched below); the
    # more key is the horizontal ellipsis turned a quarter, as LG's is vertical
    for old_n, old_f, glyph_name, extra in (
            ('imagesPath + "ic_back_n.png"', 'imagesPath + "ic_back_f.png"', "arrow-left", ""),
            ('imagesPath + "searchView/ic_profile_n.png"', 'imagesPath + "searchView/ic_profile_f.png"', "user", ""),
            ('imagesPath + "searchView/ic_more_option_n.png"', 'imagesPath + "searchView/ic_more_option_f.png"', "ellipsis", "            rotation: 90\n"),
            ('imagesPath + "ic_close_n.png"', 'imagesPath + "ic_close_f.png"', "x", "")):
        text = sub(name, text, "            normalImage: %s\n            focusedImage: %s\n" % (old_n, old_f),
                   "            hasContainer: true\n            lumaPlate: true\n            width: getRelativeWidth(%d)\n            height: getRelativeWidth(%d)\n            mouseAreaMargins: 0\n%s"
                   "            normalImage: Qt.resolvedUrl(\"images/luma/%s.png\")\n            focusedImage: normalImage\n" % (BUTTON, BUTTON, extra, glyph_name))
    # the profile when there is one: a round 64 px target in the profile's colour, its focus the ring on a lifted plate, its initial in
    # whichever of the dark and the light ink reads on that colour
    text = sub(name, text, "            width: getRelativeWidth(78/2)\n            height: getRelativeHeight(78/2)\n",
               "            width: getRelativeWidth(%d)\n            height: getRelativeWidth(%d)\n" % (BUTTON, BUTTON))
    text = sub(name, text,
               "                width: getRelativeWidth(63)\n                height: getRelativeHeight(63)\n                color: \"#E6E6E6\"\n                radius: getRelativeHeight(6)\n",
               "                width: Math.round(parent.width * 1.06)\n                height: Math.round(parent.height * 1.06)\n                color: %s\n"
               "                border.width: %d\n                border.color: %s\n                radius: width / 2\n                antialiasing: true\n" % (WASH_LIT, RIM, EDGE))
    text = sub(name, text,
               "                text: loginUserData.iconNick||\"\"\n                font.pixelSize: getRelativeFontSize(48/2)\n",
               "                text: loginUserData.iconNick||\"\"\n                font.pixelSize: getRelativeFontSize(28)\n")
    text = sub(name, text, '                font.weight: normal600_fontWeight\n                color:"#E6E6E6"\n',
               "                font.weight: normal600_fontWeight\n"
               "                property color lumaBg: Qt.lighter(loginUserData.profileBg || \"#000000\", 1.0)\n"
               "                color: (0.2126 * Math.pow(lumaBg.r, 2.2) + 0.7152 * Math.pow(lumaBg.g, 2.2) + 0.0722 * Math.pow(lumaBg.b, 2.2)) > 0.18 ? voiceStyle.lumaAccentInk : %s\n" % INK)
    return text


def patch_notiview(text):
    name = "NotiView.qml"
    text = sub(name, text, '            fontColor: "#E6E6E6"\n            style: Text.Raised\n            styleColor: "#333333"\n',
               "            fontColor: %s\n" % INK)
    return text


def patch_keywordsview(text):
    name = "KeywordsView.qml"
    # The rows start where the field does and end at the safe edge: five chips or six cards fill the line exactly.
    text = sub(name, text, "    readonly property int leftGap: getRelativeWidth(174)\n", "    readonly property int leftGap: getRelativeWidth(%d)\n" % FIELD_X)
    text = sub(name, text, "                displayMarginEnd: allListView.leftGap\n", "                displayMarginEnd: getRelativeWidth(%d)\n" % SAFE_X)
    text = sub(name, text, '                spacing: (name === "genreKeywordsList")?getRelativeWidth(12):getRelativeWidth(21)\n',
               '                spacing: (name === "genreKeywordsList")?getRelativeWidth(%d):getRelativeWidth(%d)\n' % (CARD_GAP, CHIP_GAP))
    text = sub(name, text, "    // Style\n    property var style: voiceStyle.keywordsView\n",
               "    // Style\n    property var style: voiceStyle.keywordsView\n\n"
               "    // Luma: a chip is 64 high, and more where the words need it (text at 200 %%); a card is 96\n"
               "    property int lumaChipHeight: Math.max(getRelativeHeight(%d), Math.round(getRelativeFontSize(%d) * 1.5) + getRelativeHeight(16))\n"
               "    property int lumaCardHeight: Math.max(getRelativeHeight(%d), Math.round(getRelativeFontSize(%d) * 1.5) + getRelativeHeight(56))\n"
               % (CHIP_H, CHIP_PX, CARD_H, CARD_PX))
    text = sub(name, text,
               '        case "relatedKeywordsList":\n            return getRelativeHeight(42);\n        case "genreKeywordsList":\n            return getRelativeHeight(87);\n'
               '        case "recommendKeywordsList":\n            return getRelativeHeight(48+6+21);\n        case "previousKeywordsList":\n        default:\n            return getRelativeHeight(48);\n',
               '        case "relatedKeywordsList":\n            return Math.max(getRelativeHeight(42), Math.round(getRelativeFontSize(24) * 1.45));\n        case "genreKeywordsList":\n            return lumaCardHeight;\n'
               '        case "recommendKeywordsList":\n            return lumaChipHeight;\n        case "previousKeywordsList":\n        default:\n            return lumaChipHeight;\n')
    text = sub(name, text, "            return getRelativeWidth(503/2);\n", "            return getRelativeWidth(%d);\n" % CARD_W)
    text = sub(name, text, "        default:\n            return getRelativeWidth(297);\n", "        default:\n            return getRelativeWidth(%d);\n" % CHIP_W)
    # The gap under a row (the row's 5 px, its pad and the list's 24): 36 under the chips, 8 under the chips when the AI row follows, 28 under it.
    text = sub(name, text, "                        return (getRelativeHeight(63) - allListView.spacing)\n",
               "                        return (getRelativeHeight(31) - allListView.spacing)\n")
    text = sub(name, text, "                    if(root.isRelatedKeywordExist)\n                        return 0;\n",
               "                    if(root.isRelatedKeywordExist)\n                        return -getRelativeHeight(21);\n")
    text = sub(name, text, "            return (getRelativeHeight(39) - allListView.spacing);\n",
               "            return (getRelativeHeight(23) - allListView.spacing);\n")
    text = sub(name, text, "                else\n                    return (getRelativeHeight(36) - allListView.spacing);\n",
               "                else\n                    return (getRelativeHeight(31) - allListView.spacing);\n")
    # The row's mark: one glyph, where LG draws a picture at the foot of the row.
    text = sub(name, text,
               "                anchors{\n                    right: categoryListView.left;  rightMargin: getRelativeWidth(18)\n"
               "                    bottom: categoryListView.bottom;bottomMargin: getRelativeHeight(2)\n                }\n"
               '                source: imagesPath + "keywordsView/"+getSectionIcon(name)\n',
               "                anchors{\n                    right: categoryListView.left;  rightMargin: getRelativeWidth(32)\n"
               "                    verticalCenter: categoryListView.verticalCenter\n                }\n"
               "                width: getRelativeWidth(32)\n                height: getRelativeWidth(32)\n                sourceSize.width: 64\n                sourceSize.height: 64\n"
               '                smooth: true\n                source: "images/luma/" + (name == "previousKeywordsList" ? "history" : "star") + ".png"\n')
    return text


def patch_keywordsrec(text):
    name = "KeywordsRec.qml"
    text = sub(name, text, '                color: "#7F1EFF"\n                radius: height/2\n                border.color: "#E6E6E6"\n                border.width: getRelativeWidth(2)\n',
               "                color: %s\n                radius: height/2\n                border.color: %s\n                border.width: %d\n" % (WASH_LIT, EDGE, RIM))
    # LG's chip is a picture (a pair of PNGs, the focused one white) and its genre tile a picture with a white frame; both are our plates,
    # and the genre picture is only the source of the card's photograph (it is not drawn itself).
    text = sub(name, text, '                    return isFocused? imagesPath + "keywordsView/genre_focus.png":bgImgUrl||imagesPath + "keywordsView/genre_default.png";',
               '                    return bgImgUrl||imagesPath + "keywordsView/genre_default.png";')
    text = sub(name, text, '                    return imagesPath + "keywordsView/"+"btn_search_keyword_"+(isFocused?"f.png":"n.png");',
               '                    return "";')
    text = sub(name, text, "            visible: (name !== \"relatedKeywordsList\")\n",
               "            visible: false\n"
               "            width: parent.width\n            height: parent.height\n            fillMode: Image.PreserveAspectCrop\n"
               "            sourceSize.width: getRelativeWidth(%d)\n            asynchronous: true\n" % (2 * CARD_W))
    plate = (
        "        // Luma: the plate. It grows when it is focused and the words do not (an item that scales moves its words to non-integral pixels).\n"
        "        Item {\n"
        "            id: lumaCard\n"
        "            anchors.fill: parent\n"
        "            visible: name !== \"relatedKeywordsList\"\n"
        "            scale: isFocused ? (name == \"genreKeywordsList\" ? %(card_scale)s : %(chip_scale)s) : 1\n"
        "            Behavior on scale { NumberAnimation { duration: 150; easing.type: Easing.OutCubic } }\n"
        "\n"
        "            Rectangle {\n"
        "                anchors.fill: parent\n"
        "                radius: %(row)s\n"
        "                color: (name == \"genreKeywordsList\") ? %(wash)s : (isFocused ? %(lit)s : %(wash)s)\n"
        "                border.width: (name == \"genreKeywordsList\") ? 0 : (isFocused ? %(rim)d : 0)\n"
        "                border.color: %(ring)s\n"
        "                antialiasing: true\n"
        "            }\n"
        "            LumaPhoto {\n"
        "                anchors.fill: parent\n"
        "                visible: name == \"genreKeywordsList\" && bgImg.status === Image.Ready\n"
        "                picture: bgImg\n"
        "                cornerRadius: %(row)s\n"
        "                scrimColor: %(sheet)s\n"
        "            }\n"
        "            Rectangle {\n"
        "                anchors.fill: parent\n"
        "                visible: name == \"genreKeywordsList\"\n"
        "                radius: %(row)s\n"
        "                color: \"transparent\"\n"
        "                border.width: isFocused ? %(rim)d : 0\n"
        "                border.color: %(ring)s\n"
        "                antialiasing: true\n"
        "            }\n"
        "        }\n"
        % {"row": ROW_RADIUS, "wash": WASH, "lit": WASH_LIT, "ring": EDGE, "sheet": SHEET, "chip_scale": CHIP_SCALE, "card_scale": CARD_SCALE, "rim": RIM})
    text = sub(name, text,
               "        Rectangle{\n            id:bgImgBorderRect\n\n            anchors.fill: bgImg\n            visible: bgImg.visible\n            color: \"transparent\"\n"
               "            border.color: (name == \"genreKeywordsList\")?\"#CCE6E6E6\":\"#33E6E6E6\"\n            border.width: getRelativeWidth(2)\n            radius: getRelativeWidth(6)\n        }\n",
               plate)
    text = sub(name, text, "            visible: ((!bgImg.visible)&&(textKeyword.text !==\"\"))\n",
               "            visible: ((name === \"relatedKeywordsList\")&&(textKeyword.text !==\"\"))\n")
    text = sub(name, text, '                GradientStop { position: 0.0; color: "#813ADC" }\n                GradientStop { position: 1.0; color: "#4056D6" }\n',
               "                GradientStop { position: 0.0; color: %s }\n                GradientStop { position: 1.0; color: %s }\n" % (ACCENT, ACCENT))
    text = sub(name, text, '            border.color: "#CCFFFFFF"\n', "            border.color: %s\n" % INK)
    text = sub(name, text, '            color: "#949494"\n', "            color: %s\n" % DIM)
    # The words: 27 px semibold in the chip, 28 on the card's foot, the AI keywords' 24 as LG has them.
    text = sub(name, text,
               "            anchors {\n                fill: (name == \"recommendKeywordsList\")?bgImg:parent\n"
               "                leftMargin: (name == \"relatedKeywordsList\")?getRelativeWidth(15+15+9):getRelativeWidth(24)\n"
               "                rightMargin: getRelativeWidth(24)\n            }\n            horiCenter:true\n",
               "            anchors {\n                fill: parent\n"
               "                topMargin: (name == \"genreKeywordsList\") ? Math.min(parent.height * 0.45, parent.height - getRelativeHeight(8) - Math.round(getRelativeFontSize(%d) * 1.45)) : 0\n"
               "                bottomMargin: (name == \"genreKeywordsList\") ? getRelativeHeight(8) : 0\n"
               "                leftMargin: (name == \"relatedKeywordsList\")?getRelativeWidth(15+15+9):getRelativeWidth(22)\n"
               "                rightMargin: getRelativeWidth(22)\n            }\n            horiCenter: name !== \"genreKeywordsList\"\n" % CARD_PX)
    text = sub(name, text, "                case \"genreKeywordsList\":\n                    return getRelativeFontSize(30);\n",
               "                case \"genreKeywordsList\":\n                    return getRelativeFontSize(%d);\n" % CARD_PX)
    text = sub(name, text,
               "            font.family: (name == \"relatedKeywordsList\")?normal400_fontFamily:(isFocused ?normal600_fontFamily:normal400_fontFamily)\n"
               "            font.weight: (name == \"relatedKeywordsList\")?normal400_fontWeight:(isFocused ?normal600_fontWeight:normal400_fontWeight)\n",
               "            font.family: (name == \"relatedKeywordsList\")?normal400_fontFamily:normal600_fontFamily\n"
               "            font.weight: (name == \"relatedKeywordsList\")?normal400_fontWeight:normal600_fontWeight\n")
    text = sub(name, text, "        source: focusableItem\n        color: \"#80000000\"\n        verticalOffset: getRelativeHeight(20)\n        radius: getRelativeWidth(12)\n        samples: 1 + radius * 2\n        visible: root.isFocused\n",
               "        source: focusableItem\n        color: \"#80000000\"\n        verticalOffset: getRelativeHeight(20)\n        radius: getRelativeWidth(12)\n        samples: 1 + radius * 2\n        visible: false\n")
    return text


def patch_buttonflex(text):
    name = "ButtonFlex.qml"
    text = sub(name, text, "        color: root.isFocused ? root.focusedColor : root.normalColor\n        visible: !root.isFocused\n    }\n",
               "        color: root.isFocused ? root.focusedColor : root.normalColor\n"
               "        border.width: 1\n        border.color: root.isFocused ? %s : %s\n    }\n" % (EDGE, HAIR))
    return shadow_off(name, text, "buttonRec", "root.isFocused")


def patch_focusrect(name, shadow_source="focusRec"):
    """FocusableText, FocusableSwitch and FocusableSwitchWithText: the focus pad."""
    def fn(text):
        text = sub(name, text, "        color: root.isFocused ? %s : root.normalColor\n        visible: false\n    }\n" % ("(root.disabled ? root.disabledColor : root.focusedColor)" if name != "FocusableSwitch.qml" else "root.focusedColor"),
                   "        color: root.isFocused ? %s : root.normalColor\n        border.width: root.isFocused ? %d : 0\n        border.color: %s\n    }\n"
                   % ("(root.disabled ? root.disabledColor : root.focusedColor)" if name != "FocusableSwitch.qml" else "root.focusedColor", RIM, EDGE))
        return shadow_off(name, text, shadow_source, "root.isFocused")
    return fn


def patch_buttonicon(text, pal):
    """The picture buttons: LG's `_n` picture is the light glyph and `_f` the dark one on its white box [INFERENCE: read off the names, the
    pictures are not in the census copy]; the sheet decides which of the two reads, and the focus is the wash. The ones with a container
    (the Search screen's top row) are round plates of their own size with a 32 px glyph, ringed when focused; the others are LG's size."""
    name = "ButtonIcon.qml"
    light_sheet = luminance(*parse(pal["sheetBottom"])[:3]) >= 0.5
    if light_sheet:
        text = sub(name, text, "            PropertyChanges { target: root; source: normalImage }\n", "            PropertyChanges { target: root; source: focusedImage }\n")
        text = sub(name, text, "            PropertyChanges { target: root; source: pressedImage }\n", "            PropertyChanges { target: root; source: focusedImage }\n")
    else:
        text = sub(name, text, "            PropertyChanges { target: root; source: focusedImage }\n", "            PropertyChanges { target: root; source: normalImage }\n")
        text = sub(name, text, "            PropertyChanges { target: root; source: pressedImage }\n", "            PropertyChanges { target: root; source: normalImage }\n")
    text = sub(name, text, 'property color bgFocusColor: bgColor !== "" ? bgColor : "#E6E6E6"', "property color bgFocusColor: bgColor !== \"\" ? bgColor : " + WASH_LIT)
    text = sub(name, text, "        color: bgNormalColor\n        radius: getRelativeWidth(6)\n",
               "        color: bgNormalColor\n        border.width: (root.state === \"focused\" || root.state === \"pressed\") ? %d : 0\n        border.color: %s\n        radius: width / 2\n        antialiasing: true\n" % (RIM, EDGE))
    text = sub(name, text, 'property color bgNormalColor: hasContainer ? "#7D848C" : "transparent"', "property color bgNormalColor: hasContainer ? " + WASH + " : \"transparent\"")
    text = sub(name, text, "        source: background\n        opacity: (root.state === \"focused\" || root.state === \"pressed\") ? 1 : 0\n",
               "        source: background\n        visible: false\n        opacity: (root.state === \"focused\" || root.state === \"pressed\") ? 1 : 0\n")
    # the plate (a button that asks for it with `lumaPlate`: the Search screen's top row): the button's own size, a little more when focused,
    # the glyph 32 px in its middle; every other ButtonIcon is as LG draws it
    text = sub(name, text, "    property bool hasContainer: false\n", "    property bool hasContainer: false\n    property bool lumaPlate: false\n")
    text = sub(name, text, "            PropertyChanges { target: background; width: isLarge ? getRelativeWidth(72) : getRelativeWidth(54) }\n"
                           "            PropertyChanges { target: background; height: isLarge ? getRelativeHeight(72) : getRelativeHeight(54) }\n",
               "            PropertyChanges { target: background; width: lumaPlate ? root.width : (isLarge ? getRelativeWidth(72) : getRelativeWidth(54)) }\n"
               "            PropertyChanges { target: background; height: lumaPlate ? root.height : (isLarge ? getRelativeHeight(72) : getRelativeHeight(54)) }\n", 2)
    text = sub(name, text, "            PropertyChanges { target: background; width: isLarge ? getRelativeWidth(87) : getRelativeWidth(63) }\n"
                           "            PropertyChanges { target: background; height: isLarge ? getRelativeHeight(87) : getRelativeHeight(63) }\n",
               "            PropertyChanges { target: background; width: lumaPlate ? Math.round(root.width * 1.06) : (isLarge ? getRelativeWidth(87) : getRelativeWidth(63)) }\n"
               "            PropertyChanges { target: background; height: lumaPlate ? Math.round(root.height * 1.06) : (isLarge ? getRelativeHeight(87) : getRelativeHeight(63)) }\n")
    text = sub(name, text, "    Image {\n        anchors.fill: parent\n        source: root.source\n        mirror: root.mirror\n    }\n",
               "    Image {\n        anchors.centerIn: parent\n        width: lumaPlate ? getRelativeWidth(32) : parent.width\n        height: lumaPlate ? getRelativeWidth(32) : parent.height\n"
               "        sourceSize.width: lumaPlate ? 64 : 0\n        sourceSize.height: lumaPlate ? 64 : 0\n"
               "        smooth: true\n        source: root.source\n        mirror: root.mirror\n    }\n")
    return text


def patch_optionpopup(text):
    name = "optionPopup.qml"
    text = sub(name, text, '        id:bgRect\n        anchors.fill: root\n        color: "#000000"\n        opacity: 0.7\n',
               "        id:bgRect\n        anchors.fill: root\n        color: %s\n        opacity: 0.7\n" % SHEET)
    text = sub(name, text, "        opacity: root.style.popup_bg_opacity\n        radius: getRelativeWidth(12)\n",
               "        opacity: root.style.popup_bg_opacity\n        border.width: 1\n        border.color: %s\n        radius: voiceStyle.lumaSheetRadius\n" % HAIR)
    return shadow_off(name, text, "popupView", "parent.visible")


def patch_mainview(text):
    return _mainview("MainView.qml", text)


def _mainview(name, text):
    # The dim behind the app while it is a half-height drawer over the picture: the sheet, whole (LG's is 60 % black under white words).
    text = sub(name, text, '        color: "#000000"\n        opacity: 0.6\n        visible: resultsViewComponent', "        color: %s\n        opacity: 1\n        visible: resultsViewComponent" % SHEET)
    text = sub(name, text, '            color: "#000000"\n            opacity: 0.6\n            visible: false\n', "            color: %s\n            opacity: 1\n            visible: false\n" % SHEET)
    return text


def patch_minibg(text):
    # The drawer's top edge is a black gradient picture on the set; the sheet starts at the top instead.
    return sub("MiniBg.qml", text, '        source: imagesPath + "mainView/bg_nlp_search_result"+(main.isPortraitMode ? "_portrait.png" : ".png")\n', '        source: ""\n')


def patch_utterance(text):
    name = "UtteranceUIView.qml"
    return sub(name, text, '"<font color=\\"#C97EFA\\">"', '"<font color=\\"" + voiceStyle.lumaAccentText + "\\">"')


# file under the staged tree -> edit
FILES = (
    ("Main.qml", lambda t, pal: patch_main(t)),
    ("Utils/StringSet.qml", lambda t, pal: patch_stringset(t)),
    ("Views/SearchView.qml", lambda t, pal: patch_searchview(t)),
    ("Views/SearchView/SearchInputBox.qml", lambda t, pal: patch_searchinputbox(t)),
    ("Views/SearchView/NotiView.qml", lambda t, pal: patch_notiview(t)),
    ("Views/KeywordsView.qml", lambda t, pal: patch_keywordsview(t)),
    ("Views/ContentsList/KeywordsRec.qml", lambda t, pal: patch_keywordsrec(t)),
    ("Views/Components/ButtonFlex.qml", lambda t, pal: patch_buttonflex(t)),
    ("Views/Components/ButtonIcon.qml", patch_buttonicon),
    ("Views/Components/FocusableText.qml", lambda t, pal: patch_focusrect("FocusableText.qml")(t)),
    ("Views/Components/FocusableSwitch.qml", lambda t, pal: patch_focusrect("FocusableSwitch.qml")(t)),
    ("Views/Components/FocusableSwitchWithText.qml", lambda t, pal: patch_focusrect("FocusableSwitchWithText.qml")(t)),
    ("Views/PopupView/optionPopup.qml", lambda t, pal: patch_optionpopup(t)),
    ("Views/MainView.qml", lambda t, pal: patch_mainview(t)),
    ("Views/Components/MiniBg.qml", lambda t, pal: patch_minibg(t)),
    ("Views/UtteranceUIView.qml", lambda t, pal: patch_utterance(t)),
)
STYLE_FILE = "Style.qml"


def patch_tree(root, pal, fonts):
    root = pathlib.Path(root)
    fonts = pathlib.Path(fonts).resolve()
    for f in FACES:
        if not (fonts / f).is_file():
            raise ValueError("fonts: %s has no %s" % (fonts, f))
    style = root / STYLE_FILE
    text = style.read_text()
    if MARKER + ": patched" in text:
        raise ValueError("already patched: stage a fresh copy of LG's tree")
    pal = derive(pal)
    # Everything is computed before anything is written: a late anchor failure leaves the tree as LG's.
    out = {style: "// Luma: patched, see tools/voice_patch.py\n" + patch_style(text, pal, fonts)}
    for rel, fn in FILES:
        f = root / rel
        if not f.is_file():
            raise ValueError("%s: LG's tree has no such file" % rel)
        out[f] = fn(f.read_text(), pal)
    added = own_files(root)
    added.update(icon_files(root, pal))
    for f, text in out.items():
        f.write_text(text)
    for f, data in added.items():
        f.parent.mkdir(parents=True, exist_ok=True)
        f.write_bytes(data)
    return sorted(str(f.relative_to(root)) for f in list(out) + list(added))


def main(argv):
    if len(argv) != 4:
        print(__doc__.split("\n\n")[0], file=sys.stderr)
        print("usage: voice_patch.py <staged qml dir> <palette JSON> <fonts dir>", file=sys.stderr)
        return 2
    try:
        pal = json.loads(argv[2])
        for k in NEEDED:
            if k not in pal:
                raise ValueError("palette has no " + k)
        changed = patch_tree(argv[1], pal, argv[3])
    except (ValueError, OSError) as e:
        print("voice_patch: " + str(e), file=sys.stderr)
        return 1
    # What was rewritten or added, one path a line: the caller lays exactly these over LG's tree.
    print("\n".join(changed))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
