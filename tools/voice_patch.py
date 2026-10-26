#!/usr/bin/env python3
"""Patches a staged copy of `com.webos.app.voice`'s QML (LG's Search app: the search field, the keyword rows, the buttons, the option
and confirm popups, the style sheet every other screen of the app reads) into the Luma look and removes what costs a frame.

LG's Search is one native process that draws every screen from a tree of QML (202 files, `qml/` in its app directory); its screens
are fed by LG's cloud search and its voice services, none of which the set runs under the `lean` profile, so what is reachable is
the search field with the keyboard under it, the option popup, the confirm and alert popups, and, with the services on, the keyword
rows and result cards. The style sheet (`Style.qml`) holds almost every colour of those, by object, so most of the work is a table of
roles over it; the few colours written into a component, the focus fills drawn as a white rectangle with a drop shadow, and the
picture buttons are anchored edits. What changes, all of it style (docs/research/lg-restyles-result.md):

  * Manrope for Latin text: LG's `reloadFontFamily` takes the families per locale from its language-style table; for a Latin
    locale the four faces (loaded by `FontLoader`s in the style sheet, this process does not share the compositor's fonts) take the
    place of LG Smart UI, any other script keeps LG's face. Light falls to Regular.
  * The palette's sheet under every dim backdrop and popup, ink and dim ink for text, a hairline wash for every grey chip and a lit
    wash with a ring for the focused one, in place of LG's white fill with dark text. The chip and button rectangles that LG draws only
    as the source of a drop shadow are drawn plainly (the shadow is a blurred offscreen copy per focused item: no live shadow).
  * The search field is a pill, wash and hairline at rest, lit and ringed with the focus; the keyword chips LG draws from pictures are
    rectangles; the AI keyword dot and highlight are the accent.
  * The picture buttons (close, more, back) keep LG's glyph pictures and show the one that reads on the sheet in both states (LG swaps to a
    dark glyph for its white focus box).

Every edit is anchored to an exact fragment of LG's source and must find it exactly as many times as it expects: a missing or doubled anchor
is fatal, and so is a colour in the style sheet that has no role here (a firmware update that adds one must be looked at, not guessed).
The caller (`tools/voice.sh stage`) works on a fresh copy every time and leaves LG's tree in force when this fails.

usage: voice_patch.py <staged qml dir> <palette JSON from `tokens.py qml`> <fonts dir with Manrope-*.ttf>
Prints the paths it rewrote (relative to the staged dir), one a line.
Exit status: 0 patched, 1 an anchor is missing or the palette or fonts lack something, 2 usage.
"""
import json
import pathlib
import re
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from notification_patch import sub, edit_blocks, blocks  # noqa: E402
from tokens import luminance, parse  # noqa: E402

MARKER = "Luma"

# The four faces the style sheet loads.
FACES = ("Manrope-Regular.ttf", "Manrope-Medium.ttf", "Manrope-SemiBold.ttf", "Manrope-Bold.ttf")

# Palette names the edits draw on.
NEEDED = ("sheetBottom", "row", "focus", "focusEdge", "hairline", "ink", "inkDim", "focusInk", "accent", "danger", "radius", "rowRadius")

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
        searchInputBox.textinputwrapper_normal_color searchInputBox.textinputwrapper_border_normal_color
        focusableSwitch.groove_disabled_color livetvRec.timebarBgColor weatherRec.selectRecColor weatherRec.frameDividerColor
        timeRec.selectRecColor iotRec.recColor iotRec.recDisconnColor iotRec.deviderColor teamSportsRec.bgRecColor
        teamSportsRec.timebarBgColor individualSportsRec.bgRecColor individualSportsRec.timebarBgColor popupWithCloseBtn.divider_color
        cpSelectionPopup.popup_bgBorder_color epgSchedulePopup.popup_bgBorder_color nextProgramView.popup_bgBorder_color
        alarmView.popup_bgBorder_color buttonFlex.normalColor buttonFlex.borderColor curtainPageButton.normalColor
        knowMeButton.normalColor descriptionBtn.normalColor descriptionBtn.borderColor""",
    "lit": """keywordsView.itemFocusbgColor searchInputBox.textinputwrapper_focused_color focusableText.focusedColor
        focusableSwitch.focusedColor weatherRec.selectRecFocusedColor timeRec.selectRecFocusedColor iotRec.focusedRecColor
        iotRec.btnFocusedColor optionPopup.focusedColor buttonFlex.focusedColor curtainPageButton.focusedColor
        knowMeButton.focusedColor descriptionBtn.focusedColor""",
    "ring": """searchInputBox.textinputwrapper_border_activefocus_color curtainPageButton.borderColor""",
    "edge": """focusableSwitch.groove_enabled_unchecked_color iotRec.indicatorColor popupWithCloseBtn.bgBorder_color""",
    "accent": """keywordsView.itemRelatedNormalTextColor focusableSwitch.groove_enabled_checked_color livetvRec.timebarColor
        teamSportsRec.timebarColor individualSportsRec.timebarColor""",
    "danger": "mic.focusedColor",
}
# LG's on purpose: transparent is nothing, `scrimColor` and the picture backdrops under a card are black behind a picture, the pink is
# the select colour of a list nobody on this set reaches, the three orange ones are the IP channel's mark, the blue is the web tile's
# brand colour.
KEEP = """root.baseColor root.disabledColor root.selectColor root.scrimColor verticalRec.thumbnailBgColor freeviewRec.thumbnailBgColor
    qnaRec.thumbnailBgColor youtubeRec.thumbnailBgColor webRec.bgColor livetvRec.ipChannelColor teamSportsRec.ipChannelColor
    individualSportsRec.ipChannelColor nextProgramView.ipChannelColor iotRec.btnNormalColor notiView.color mic.bgColor""".split()

PAL_OF = {"sheet": "sheetBottom", "ink": "ink", "dim": "inkDim", "hairline": "hairline", "lit": "focusEdge", "ring": "inkDim", "edge": "focusEdge",
          "accent": "accent", "danger": "danger"}

COLOR_RE = re.compile(r'^\s*property\s+(?:color|string)\s+(?P<name>\w+)\s*:\s*"(?P<val>#[0-9a-fA-F]{3,8}|transparent|white|black)"', re.M)
BLOCK_RE = re.compile(r'property\s+(?:var\s+)?(?:QtObject\s+)?(\w+)\s*:\s*QtObject\s*\{')


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
         "    property int lumaSheetRadius: %d\n"
         "    property int lumaRowRadius: %d\n"
         % (pal["sheetBottom"], pal["ink"], pal["inkDim"], pal["hairline"], pal["focusEdge"], pal["inkDim"], pal["accent"],
            pal["radius"], pal["rowRadius"])),
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
    # The search field's wash at rest was LG's 20 % of a white fill: the hairline is the whole of it.
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
    return fonts_edit(text, fonts)


def patch_main(text):
    name = "Main.qml"
    for w in ("300", "400", "600", "700"):
        text = sub(name, text, 'property string normal%s_fontFamily: "LG Smart UI"' % w, 'property string normal%s_fontFamily: "Manrope"' % w)
    return text


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
    text = recolor(name, text, [
        ('"#4C5059"', INK, 4),
        ('"#E6E6E6"', INK, 4),
        ('"#664C5059"', DIM, 1),
        ('"#66E6E6E6"', DIM, 1),
        ('"#999999"', DIM, 1),
        ('"#575E66"', INK, 1),
    ])
    # The auto-keyword chips: the hairline wash, a ring when focused, the row radius.
    text = sub(name, text, 'normalColor: "#33FFFFFF"', "normalColor: " + HAIR)
    text = sub(name, text,
               "                    radius: getRelativeWidth(6)\n                    border.color: \"#33E6E6E6\"\n                    border.width: getRelativeWidth(1)\n",
               "                    radius: %s\n                    border.color: focusableItem.isFocused ? %s : %s\n                    border.width: 1\n" % (ROW_RADIUS, EDGE, HAIR))
    # The field: a pill.
    text = sub(name, text, "        clip: true\n        radius: getRelativeWidth(6)\n    }\n",
               "        clip: true\n        radius: height / 2\n    }\n")
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
    text = sub(name, text, '                color: "#E6E6E6"\n                radius: getRelativeHeight(6)\n',
               "                color: %s\n                border.width: 2\n                border.color: %s\n                radius: getRelativeHeight(6)\n" % (FOCUS, EDGE))
    text = sub(name, text, '                font.weight: normal600_fontWeight\n                color:"#E6E6E6"\n',
               "                font.weight: normal600_fontWeight\n                color: %s\n" % INK)
    return text


def patch_notiview(text):
    name = "NotiView.qml"
    text = sub(name, text, '            fontColor: "#E6E6E6"\n            style: Text.Raised\n            styleColor: "#333333"\n',
               "            fontColor: %s\n" % INK)
    return text


def patch_keywordsrec(text):
    name = "KeywordsRec.qml"
    text = sub(name, text, '                color: "#7F1EFF"\n                radius: height/2\n                border.color: "#E6E6E6"\n',
               "                color: %s\n                radius: height/2\n                border.color: %s\n" % (FOCUS, EDGE))
    # LG's chip is a picture (a pair of PNGs, the focused one white); it is a rectangle here. The genre tiles keep their picture (the
    # server's, or LG's default) and the focus is a ring over it, the white focus picture is not used.
    text = sub(name, text, '                    return isFocused? imagesPath + "keywordsView/genre_focus.png":bgImgUrl||imagesPath + "keywordsView/genre_default.png";',
               '                    return bgImgUrl||imagesPath + "keywordsView/genre_default.png";')
    text = sub(name, text, '                    return imagesPath + "keywordsView/"+"btn_search_keyword_"+(isFocused?"f.png":"n.png");',
               '                    return "";')
    text = sub(name, text, "            visible: (name !== \"relatedKeywordsList\")\n",
               "            visible: (name !== \"relatedKeywordsList\")\n"
               "            width: parent.width\n            height: parent.height\n            fillMode: Image.PreserveAspectCrop\n")
    text = sub(name, text,
               '            color: "transparent"\n            border.color: (name == "genreKeywordsList")?"#CCE6E6E6":"#33E6E6E6"\n            border.width: getRelativeWidth(2)\n            radius: getRelativeWidth(6)\n',
               '            color: (name == "genreKeywordsList") ? "transparent" : (isFocused ? %s : %s)\n'
               '            border.color: isFocused ? %s : %s\n            border.width: isFocused ? 2 : 1\n            radius: %s\n' % (FOCUS, HAIR, EDGE, HAIR, ROW_RADIUS))
    text = sub(name, text, '                GradientStop { position: 0.0; color: "#813ADC" }\n                GradientStop { position: 1.0; color: "#4056D6" }\n',
               "                GradientStop { position: 0.0; color: %s }\n                GradientStop { position: 1.0; color: %s }\n" % (ACCENT, ACCENT))
    text = sub(name, text, '            border.color: "#CCFFFFFF"\n', "            border.color: %s\n" % INK)
    text = sub(name, text, '            color: "#949494"\n', "            color: %s\n" % DIM)
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
                   "        color: root.isFocused ? %s : root.normalColor\n        border.width: root.isFocused ? 2 : 0\n        border.color: %s\n    }\n"
                   % ("(root.disabled ? root.disabledColor : root.focusedColor)" if name != "FocusableSwitch.qml" else "root.focusedColor", EDGE))
        return shadow_off(name, text, shadow_source, "root.isFocused")
    return fn


def patch_buttonicon(text, pal):
    """The picture buttons: LG's `_n` picture is the light glyph and `_f` the dark one on its white box [INFERENCE: read off the names, the
    pictures are not in the census copy]; the sheet decides which of the two reads, and the focus is the wash."""
    name = "ButtonIcon.qml"
    light_sheet = luminance(*parse(pal["sheetBottom"])[:3]) >= 0.5
    if light_sheet:
        text = sub(name, text, "            PropertyChanges { target: root; source: normalImage }\n", "            PropertyChanges { target: root; source: focusedImage }\n")
        text = sub(name, text, "            PropertyChanges { target: root; source: pressedImage }\n", "            PropertyChanges { target: root; source: focusedImage }\n")
    else:
        text = sub(name, text, "            PropertyChanges { target: root; source: focusedImage }\n", "            PropertyChanges { target: root; source: normalImage }\n")
        text = sub(name, text, "            PropertyChanges { target: root; source: pressedImage }\n", "            PropertyChanges { target: root; source: normalImage }\n")
    text = sub(name, text, 'property color bgFocusColor: bgColor !== "" ? bgColor : "#E6E6E6"', "property color bgFocusColor: bgColor !== \"\" ? bgColor : " + FOCUS)
    text = sub(name, text, "        color: bgNormalColor\n        radius: getRelativeWidth(6)\n",
               "        color: bgNormalColor\n        border.width: (root.state === \"focused\" || root.state === \"pressed\") ? 2 : 0\n        border.color: %s\n        radius: width / 2\n" % EDGE)
    text = sub(name, text, 'property color bgNormalColor: hasContainer ? "#7D848C" : "transparent"', "property color bgNormalColor: hasContainer ? %s : \"transparent\"" % HAIR)
    text = sub(name, text, "        source: background\n        opacity: (root.state === \"focused\" || root.state === \"pressed\") ? 1 : 0\n",
               "        source: background\n        visible: false\n        opacity: (root.state === \"focused\" || root.state === \"pressed\") ? 1 : 0\n")
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
    return sub(name, text, '"<font color=\\"#C97EFA\\">"', '"<font color=\\"" + voiceStyle.lumaAccent + "\\">"')


# file under the staged tree -> edit
FILES = (
    ("Main.qml", lambda t, pal: patch_main(t)),
    ("Views/SearchView.qml", lambda t, pal: patch_searchview(t)),
    ("Views/SearchView/SearchInputBox.qml", lambda t, pal: patch_searchinputbox(t)),
    ("Views/SearchView/NotiView.qml", lambda t, pal: patch_notiview(t)),
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
    # Everything is computed before anything is written: a late anchor failure leaves the tree as LG's.
    out = {style: "// Luma: patched, see tools/voice_patch.py\n" + patch_style(text, pal, fonts)}
    for rel, fn in FILES:
        f = root / rel
        if not f.is_file():
            raise ValueError("%s: LG's tree has no such file" % rel)
        out[f] = fn(f.read_text(), pal)
    for f, text in out.items():
        f.write_text(text)
    return sorted(str(f.relative_to(root)) for f in out)


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
    # What was rewritten, one path a line: the caller lays exactly these over LG's tree.
    print("\n".join(changed))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
