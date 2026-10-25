#!/usr/bin/env python3
"""Patches a staged copy of `com.webos.app.inputcommon`'s QML (the HDMI input pill, the OK banner with its options
button, the INFO detail card and the software-update bar) into the Luma look and removes what costs a frame.

inputcommon is one native process per input (`hdmi1`-`hdmi4`, Live TV, ...) that draws every banner over the video from
a tree of QML; the banner list, its focus and key handling, the luna calls behind the options button and the audio
guide are LG's and stay as they are (docs/research/lg-restyles-result.md). What changes, all of it style:

  * Manrope in every font the style sheet builds (`GlobalStyle.qml`'s font functions name LG Smart UI through the font
    manager). The four faces are loaded by `FontLoader`s in the style sheet itself, from the fonts directory given:
    inputcommon is its own process and does not share the compositor's fonts. LG's dingbat face stays (it holds
    glyphs, the Dolby marks, not letters).
  * The pill is a glass sheet with a hairline edge where LG draws a grey bar from four PNG pieces. The four pictures
    stay in the tree with no source (their visibility logic is LG's); a rectangle drawn in their place takes the same
    `visible` condition. The focused input name in the OK banner is a lit inset (the palette's focus wash and edge)
    in place of LG's white pill with dark text, so text keeps one ink in both states.
  * The OK banner's options button is a round glass key with a lit ring on focus, and its picture is the light one in
    both states (LG swaps to a dark glyph for its white focus). Its live `DropShadow` is off: a blurred offscreen copy
    per focused item is what the frame gate forbids. So is the update bar's.
  * The input name is a small dim line over the device label, which is the title (LG sets the name small and the CEC label
    large; the order is LG's, the sizes are the type scale's). The pill is 88 high, where LG's is 156, and floats off the
    screen edge by the Home margin; its place in the banner list is LG's (and the owner's banner-position setting).
  * The update bar (and every progress bar the sheet draws) has a light track and an accent fill.
  * The detail card is a sheet of its own width and height with the palette's sheet radius and a hairline, its chips
    are lifted tiles (the palette's hairline wash) with ink text.
  * The shared style (info banner colours, chips, the clock, the option bar) moves with the input banners; the channel
    banner draws from the same sheet and gets the colours and the face, while its LG background pictures and layout are
    left to its own restyle.

Every edit is anchored to an exact fragment of LG's source and must find it exactly as many times as it expects: a
missing or doubled anchor is fatal, because a half-patched banner is worse than an unpatched one, and a firmware update
that rewrites these files must fail here, loudly, instead of shipping a broken banner. The caller
(`tools/inputcommon.sh stage`) works on a fresh copy every time and leaves LG's tree in force when this fails.

usage: inputcommon_patch.py <staged qml dir> <palette JSON from `tokens.py qml`> <fonts dir with Manrope-*.ttf>
Prints the paths it rewrote (relative to the staged dir), one a line.
Exit status: 0 patched, 1 an anchor is missing or the palette or fonts lack something, 2 usage.
"""
import json
import pathlib
import re
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from notification_patch import sub, edit_blocks  # noqa: E402
from tokens import luminance, parse  # noqa: E402

MARKER = "Luma"

# The four faces the style sheet loads; LG's fonts are the fallback while one is still loading.
FACES = ("Manrope-Regular.ttf", "Manrope-Medium.ttf", "Manrope-SemiBold.ttf", "Manrope-Bold.ttf")

# The font functions in `GlobalStyle.qml`: light, regular, semibold, bold, number and the condensed regular.
FONT_RE = re.compile(r'"family": fontProp\("(?:smart|cond|number)_\w+", "family"\)')
MIN_FONT_SITES = 6

# Palette names the edits draw on.
NEEDED = ("sheetBottom", "focus", "focusEdge", "hairline", "ink", "inkDim", "focusInk", "accent", "radius", "rowRadius")

# Geometry of the input banners, in the 1080-line units LG's style sheet counts in. The banner's height is the
# pill's; a two-line label (kicker over title) needs 22 + 28 plus air.
PILL_HEIGHT = 88
SIDE_MARGIN = 32
INSET = 64                 # off the screen edge, the Home margin
DETAIL_WIDTH = 1100
DETAIL_HEIGHT = 184
FOCUS_INSET = 8            # the lit inset inside the pill when the input name has the focus
STYLE = "property var banner: QtObject {"
NEW_STYLE_AT = "        property int inputInfoTopHeight: 181\n"


def banner_edits(pal):
    return [
        # Where the new properties live: beside LG's own input-banner block.
        (NEW_STYLE_AT, NEW_STYLE_AT +
         "        // Luma: the input banners' own measures and colours (tools/inputcommon_patch.py)\n"
         "        property int inputBannerHeight: %d\n"
         "        property int inputSideMargin: %d\n"
         "        property int inputSideInset: %d\n"
         "        property int inputInfoWidth: %d\n"
         "        property int inputInfoHeight: %d\n"
         "        property int inputFocusInset: %d\n"
         "        property string inputBannerColor: \"%s\"\n"
         "        property string inputBannerEdge: \"%s\"\n"
         "        property string inputFocusColor: \"%s\"\n"
         "        property string inputFocusEdge: \"%s\"\n"
         "        property string inputKickerColor: \"%s\"\n"
         % (PILL_HEIGHT, SIDE_MARGIN, INSET, DETAIL_WIDTH, DETAIL_HEIGHT, FOCUS_INSET,
            pal["sheetBottom"], pal["hairline"], pal["focus"], pal["focusEdge"], pal["inkDim"])),
        ("property int inputSimpleMinWidth: 582", "property int inputSimpleMinWidth: 320"),
        ('property string inputColor: "#E6E6E6"', 'property string inputColor: "%s"' % pal["ink"]),
        ('property string inputFocusedColor: "#4C5059"', 'property string inputFocusedColor: "%s"' % pal["focusInk"]),
        # The kicker (the input's name) over the title (the device's label).
        ("property font inputNameFont: semiBoldFont(33)", "property font inputNameFont: semiBoldFont(22)"),
        ("property font inputNameFont2: semiBoldFont(60)", "property font inputNameFont2: semiBoldFont(34)"),
        ("property font inputLabelFont: regularFont(45)", "property font inputLabelFont: semiBoldFont(28)"),
        ("property font inputLabelFont2: regularFont(60)", "property font inputLabelFont2: semiBoldFont(34)"),
        ("property int inputDingbatBottomMargin: 29", "property int inputDingbatBottomMargin: 32"),
        # The options key and the divider before it: a hairline for a bar, and one glyph in both states (LG swaps to a dark
        # glyph for its white focus; the focus here is a wash). LG's two pictures are light and dark: the light one on a
        # dark sheet, the dark one on a light sheet.
    ] + option_glyph(pal) + [
        ('property string optionBarColor: "#858A91"', 'property string optionBarColor: "%s"' % pal["hairline"]),
        ('property string remoteFocusedImg: Qt.resolvedUrl("./Images/banner/remote/icon_remotecontrol_F.png")',
         'property string remoteFocusedImg: Qt.resolvedUrl("./Images/banner/remote/icon_remotecontrol_N.png")'),
        # The detail card and the update bar: the sheet, whole, not translucent twice over.
        ("property int infoBannerRadius: 12", "property int infoBannerRadius: %d" % pal["radius"]),
        ("property double infoBannerOpacity: 0.9", "property double infoBannerOpacity: 1"),
        ('property string infoBannerColor: "#575E66"', 'property string infoBannerColor: "%s"' % pal["sheetBottom"]),
        ("property int nsuBgRadius: 12", "property int nsuBgRadius: %d" % pal["rowRadius"]),
        ("property double nsuBgOpacity: 0.9", "property double nsuBgOpacity: 1"),
        ('property string nsuBgColor: "#575E66"', 'property string nsuBgColor: "%s"' % pal["sheetBottom"]),
        # The progress bar (the update bar, and the channel banner's): a track, an accent fill.
        ('property string progressBarBgColor: "#858A91"', 'property string progressBarBgColor: "%s"' % pal["focusEdge"]),
        ('property string progressBarBgFocusedColor: "#858A91"', 'property string progressBarBgFocusedColor: "%s"' % pal["focusEdge"]),
        ('property string progressBarColor: "#FFFFFF"', 'property string progressBarColor: "%s"' % pal["accent"]),
        ('property string progressBarFocusedColor: "#373A41"', 'property string progressBarFocusedColor: "%s"' % pal["accent"]),
        ('property string progressBarTextColor: "#E6E6E6"', 'property string progressBarTextColor: "%s"' % pal["ink"]),
        ('property string clockTextColor: "#E6E6E6"', 'property string clockTextColor: "%s"' % pal["ink"]),
        ("property font clockTextFont: regularFont(30)", "property font clockTextFont: regularFont(26)"),
    ]


def option_glyph(pal):
    """Both states draw the glyph that reads on the palette's sheet. Edits: none when the sheet is light (LG's focused
    picture is the dark one, and the normal one is made the same), the focused picture made the normal one otherwise."""
    glyph = 'Qt.resolvedUrl("./Images/banner/option/ic_moreoption_%s.png")'
    if luminance(*parse(pal["sheetBottom"])[:3]) >= 0.5:
        return [('property string optionImg: ' + glyph % "n", 'property string optionImg: ' + glyph % "f")]
    return [('property string optionFocusedImg: ' + glyph % "f", 'property string optionFocusedImg: ' + glyph % "n")]


def chip_edits(pal):
    """The badges (resolution, HDR, audio): a lifted tile with ink on it (`row` is too sheer to show on the card's sheet),
    and the ink for the Dolby glyphs too."""
    return [
        ('BG_COLOR: "#CCCCCC"', 'BG_COLOR: "%s"' % pal["hairline"]),
        ('BG_F_COLOR: "#ABAEB2"', 'BG_F_COLOR: "%s"' % pal["focus"]),
        ("RADIUS: 3", "RADIUS: 8"),
        ('TEXT_COLOR: "#000000"', 'TEXT_COLOR: "%s",\n                                      DINGBAT_COLOR: "%s"' % (pal["ink"], pal["ink"])),
    ]


def patch_style(text, pal, fonts):
    name = "GlobalStyle.qml"
    text, n = FONT_RE.subn('"family": "Manrope"', text)
    if n < MIN_FONT_SITES:
        raise ValueError("%s: %d font sites found, expected at least %d" % (name, n, MIN_FONT_SITES))
    loaders = "".join('FontLoader { source: "file://%s/%s" }, ' % (fonts, f) for f in FACES).rstrip(", ")
    text = sub(name, text, "    property var fontManager\n",
               "    property var fontManager\n"
               "    // Luma: the four Manrope faces; this process does not share the compositor's\n"
               "    property list<QtObject> lumaFonts: [%s]\n" % loaders)
    text = edit_blocks(name, text, STYLE, [0], banner_edits(pal))
    text = edit_blocks(name, text, "property var bannerIcon: ({", [0], chip_edits(pal))
    return text


def patch_simplebanner(text):
    name = "SimpleBanner.qml"
    text = sub(name, text, "property int sideMargin: styleSheet.banner.zappingBannerSideMargin",
               "property int sideMargin: styleSheet.banner.inputSideMargin")
    text = sub(name, text, "    height: styleSheet.banner.zappingBannerHeight\n", "    height: styleSheet.banner.inputBannerHeight\n")
    # LG's four background pictures stay, empty; the pill is drawn under them with their own visibility.
    text = sub(name, text, 'source: root.visible ? (hasFocus? styleSheet.banner.middleFocusedBg : styleSheet.banner.middleBg) : ""', 'source: ""')
    text = sub(name, text, 'source: root.visible ? (hasFocus? styleSheet.banner.rightFocusedBg : styleSheet.banner.rightBg) : ""', 'source: ""')
    text = sub(name, text,
               "        anchors.left: parent.left\n\n        Image {\n\n            id: bgMiddle\n",
               "        anchors.left: parent.left\n\n"
               "        Rectangle {\n\n"
               "            id: lumaPill\n\n"
               "            anchors.fill: parent\n"
               "            radius: height / 2\n"
               "            color: styleSheet.banner.inputBannerColor\n"
               "            border.width: 1\n"
               "            border.color: styleSheet.banner.inputBannerEdge\n"
               "            visible: root.displayOnlyZapping\n"
               "        }\n\n"
               "        Rectangle {\n\n"
               "            id: lumaFocus\n\n"
               "            anchors {\n"
               "                fill: parent\n"
               "                margins: styleSheet.banner.inputFocusInset\n"
               "            }\n"
               "            radius: height / 2\n"
               "            color: styleSheet.banner.inputFocusColor\n"
               "            border.width: 2\n"
               "            border.color: styleSheet.banner.inputFocusEdge\n"
               "            visible: root.needFocusHandle\n"
               "        }\n\n"
               "        Image {\n\n            id: bgMiddle\n")
    # The name is the kicker over the label when there is one; with no label it is the title.
    text = sub(name, text,
               "                font: inputLabelItem.visible ? styleSheet.banner.inputNameFont : styleSheet.banner.inputNameFont2\n"
               "                color: needFocusHandle ? styleSheet.banner.inputFocusedColor : styleSheet.banner.inputColor\n",
               "                font: inputLabelItem.visible ? styleSheet.banner.inputNameFont : styleSheet.banner.inputNameFont2\n"
               "                color: needFocusHandle ? styleSheet.banner.inputFocusedColor : (inputLabelItem.visible ? styleSheet.banner.inputKickerColor : styleSheet.banner.inputColor)\n")
    return text


def patch_inputbanner(text):
    name = "InputBanner.qml"
    text = sub(name, text, "    height: visible ? styleSheet.banner.zappingBannerHeight + recInfoIcon.heightWithMargin : 0\n",
               "    height: visible ? styleSheet.banner.inputBannerHeight + recInfoIcon.heightWithMargin : 0\n")
    text = sub(name, text, "        height: styleSheet.banner.zappingBannerHeight\n        anchors {\n            left: parent.left\n            top: parent.top\n        }\n",
               "        height: styleSheet.banner.inputBannerHeight\n        anchors {\n            left: parent.left\n"
               "            leftMargin: styleSheet.banner.inputSideInset\n            top: parent.top\n        }\n")
    text = sub(name, text, "            sideMargin: styleSheet.banner.zappingBannerSideMargin\n", "            sideMargin: styleSheet.banner.inputSideMargin\n")
    text = sub(name, text, 'source: root.visible ? styleSheet.banner.middleBg : ""', 'source: ""')
    text = sub(name, text, 'source: root.visible ? styleSheet.banner.rightBg : ""', 'source: ""')
    text = sub(name, text,
               "        anchors.fill: bannerRow\n\n        Image {\n\n            id: bgMiddle\n",
               "        anchors.fill: bannerRow\n\n"
               "        Rectangle {\n\n"
               "            id: lumaPill\n\n"
               "            anchors.fill: parent\n"
               "            radius: height / 2\n"
               "            color: styleSheet.banner.inputBannerColor\n"
               "            border.width: 1\n"
               "            border.color: styleSheet.banner.inputBannerEdge\n"
               "        }\n\n"
               "        Image {\n\n            id: bgMiddle\n")
    return text


def patch_detail(text):
    name = "DetailInfoBanner.qml"
    text = sub(name, text, "    width: styleSheet.banner.infoBannerWidth\n    height: styleSheet.banner.infoBannerHeight\n",
               "    width: styleSheet.banner.inputInfoWidth\n    height: styleSheet.banner.inputInfoHeight\n")
    text = sub(name, text, "        leftMargin: styleSheet.banner.infoBannerOutSideMargin\n", "        leftMargin: styleSheet.banner.inputSideInset\n")
    text = sub(name, text, "        radius: styleSheet.banner.infoBannerRadius\n        color: styleSheet.banner.infoBannerColor\n",
               "        radius: styleSheet.banner.infoBannerRadius\n        color: styleSheet.banner.infoBannerColor\n"
               "        border.width: 1\n        border.color: styleSheet.banner.inputBannerEdge\n")
    text = sub(name, text, "styleSheet.banner.infoBannerInMargin", "styleSheet.banner.inputSideMargin", count=5)
    # The chips sit on the card's bottom edge, where LG's tall card had them near its lower third.
    text = sub(name, text, "            width: styleSheet.banner.infoBannerWidth\n            height: styleSheet.banner.infoBannerContentsHeight\n",
               "            width: parent.width\n            height: parent.height\n")
    return sub(name, text, "                topMargin: styleSheet.banner.infoTopMargin\n", "                topMargin: 0\n")


def patch_optionbutton(text, pal):
    name = "OptionButton.qml"
    text = sub(name, text, "    bClickEffect: true\n    buttonType: s_type\n",
               "    bClickEffect: true\n    buttonType: c_type\n"
               "    shadowVisible: false\n"
               '    bgFocusedColor: "%s"\n' % pal["focus"])
    # The lit ring, in the item's own children: ImageButton's own background has no edge.
    return sub(name, text, "    onSelected: {\n",
               "    Rectangle {\n\n"
               "        anchors.fill: parent\n"
               "        radius: width / 2\n"
               '        color: "transparent"\n'
               "        border.width: 2\n"
               '        border.color: "%s"\n'
               "        visible: root.hasFocus\n"
               "    }\n\n"
               "    onSelected: {\n" % pal["focusEdge"])


def patch_bannericon(text):
    name = "BannerIcon.qml"
    return sub(name, text, '        color: iconColor\n        text: iconVisible && dingbatEnable ? dingbat : ""\n',
               '        color: bannerIconStyle.DINGBAT_COLOR\n        text: iconVisible && dingbatEnable ? dingbat : ""\n')


def patch_nsu(text):
    name = "NsuProgressBar.qml"
    return sub(name, text, "    CommonBoxShadow {\n        target: bg\n    }\n", "    CommonBoxShadow {\n        target: bg\n        active: false\n    }\n")


# file under the staged tree -> edit
BANNER = "UserInterfaceLayer/Containers/MainViewLayer/"
FILES = (
    (BANNER + "InputBanner/SimpleBanner.qml", lambda t, pal: patch_simplebanner(t)),
    (BANNER + "InputBanner/InputBanner.qml", lambda t, pal: patch_inputbanner(t)),
    (BANNER + "InputBanner/DetailInfoBanner.qml", lambda t, pal: patch_detail(t)),
    (BANNER + "BannerItems/OptionButton.qml", patch_optionbutton),
    (BANNER + "BannerItems/BannerIcon.qml", lambda t, pal: patch_bannericon(t)),
    (BANNER + "BannerItems/NsuProgressBar.qml", lambda t, pal: patch_nsu(t)),
)
STYLE_FILE = "UserInterfaceLayer/GlobalStyle.qml"


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
    out = {style: "// Luma: patched, see tools/inputcommon_patch.py\n" + patch_style(text, pal, fonts)}
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
        print("usage: inputcommon_patch.py <staged qml dir> <palette JSON> <fonts dir>", file=sys.stderr)
        return 2
    try:
        pal = json.loads(argv[2])
        for k in NEEDED:
            if k not in pal:
                raise ValueError("palette has no " + k)
        changed = patch_tree(argv[1], pal, argv[3])
    except (ValueError, OSError) as e:
        print("inputcommon_patch: " + str(e), file=sys.stderr)
        return 1
    # What was rewritten, one path a line: the caller lays exactly these over LG's tree.
    print("\n".join(changed))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
