#!/usr/bin/env python3
"""Patches copies of LG's `StarfishImageSplash.qml` (the launch splash) and `StarfishSpinnerView.qml` (the launch spinner), the compositor
views, so that what an app launch shows is LumaTV's (docs/mocks/15-launch-splash, 15-spinner).

Launch splash. LG draws one full-screen picture from the app's own `splashBackground` (47 of the set's 149 apps ship one, Netflix, YouTube,
Prime Video and Plex among them) and, for the others, a grey default with eight dots. The view's `open(appId, img)` gets `img` empty exactly
for the others (`ImageUtils.applyImagePath` then falls back to the default). The patch leaves LG's picture alone and adds one item above
the wiper that draws LumaTV's launch screen (the app's picture as a rounded square, its name, "Opening" and a ring) while the wiper is open
and `img` was empty. An app with its own art shows it, as the plan says ("LG art only where an app ships its own"); the Home launch splash
(`panels/Splash.qml`) is LumaHome's own and is not this view.

Launch spinner. LG's is a 72 px grey disc with a GIF over a 500 ms delay, input-blocking, 120 s at most. The patch replaces the disc and the
GIF by the dimmed screen, a ring and, after a second, "Loading" in a dark pill; the timers, the mouse area that blocks input and the
open and close of the view are LG's.

The patched views exist only in the shadow module (`tools/lumatv build_shadow`), so LG's are back whenever that module is not the one loaded.
Every anchor must be found exactly once in both files; one that is not (a firmware update changed a view) fails here, loudly, nothing is
written, and the caller leaves LG's views in force.

usage: splash_patch.py <views directory> <palette json> [<payload/splash directory>]
Exit status: 0 both patched in place, 1 an anchor is missing, 2 usage.
"""
import json
import pathlib
import re
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from notification_patch import sub  # noqa: E402

MARKER = "Luma"
SPLASH = "StarfishImageSplash.qml"
SPINNER = "StarfishSpinnerView.qml"

PROPS_AT = "    property bool wasCancelled: false\n"
CARD_AT = "    MouseArea {\n        anchors.fill: parent\n        enabled: root.focus\n        hoverEnabled: enabled\n    }\n"
OPEN_AT = "            theImage.fillMode = theImage.updateFillMode();\n            theImage.color ="
# LG's disc and GIF, up to the function that opens the view
DISC = "    Rectangle {\n        id: spinnerBackground\n"
DISC_END = "    function open(appId) {\n"

PROPS = ('    // Luma: set by open() below. No picture of its own (`img` empty) means LG draws its grey default, and LumaTV draws its launch\n'
         '    // screen over it (tools/splash_patch.py).\n'
         '    property bool lumaCard: false\n'
         '    property string lumaName: ""\n'
         '    property url lumaIcon: ""\n')
OPEN = ('            // Luma: the card is drawn for an app with no picture of its own; the name and picture are the app\'s (folderPath and the file named by\n'
        '            // largeIcon or icon, which the app manager gives relative to it)\n'
        '            var lumaInfo = LS.applicationManager.appInfoList[appId] || ({});\n'
        '            var lumaFile = lumaInfo.largeIcon || lumaInfo.icon || "";\n'
        '            root.lumaName = lumaInfo.title || appId;\n'
        '            root.lumaIcon = lumaFile === "" ? "" : (lumaFile.charAt(0) === "/" ? "file://" + lumaFile : (lumaInfo.folderPath ? "file://" + lumaInfo.folderPath + "/" + lumaFile : ""));\n'
        '            root.lumaCard = !img;\n')


def fill(text, pal, name):
    def one(m):
        if m.group(1) not in pal:
            raise ValueError("%s: palette has no %s" % (name, m.group(1)))
        return str(pal[m.group(1)])
    return re.sub(r"@@(\w+)@@", one, text)


def patch_splash(text, pal, card):
    if MARKER + ": patched" in text:
        raise ValueError("%s: already patched: start from a fresh copy of LG's file" % SPLASH)
    text = text.lstrip("\ufeff")
    text = sub(SPLASH, text, PROPS_AT, PROPS_AT + PROPS)
    text = sub(SPLASH, text, CARD_AT, fill(card, pal, "card.qml") + CARD_AT)
    text = sub(SPLASH, text, OPEN_AT, OPEN_AT.replace("            theImage.color =", OPEN + "            theImage.color ="))
    return "// Luma: patched, see tools/splash_patch.py\n" + text


def patch_spinner(text, pal, disc):
    if MARKER + ": patched" in text:
        raise ValueError("%s: already patched: start from a fresh copy of LG's file" % SPINNER)
    text = text.lstrip("\ufeff")
    if text.count(DISC) != 1 or text.count(DISC_END) != 1 or text.index(DISC) > text.index(DISC_END):
        raise ValueError("%s: LG's disc was not found once and before open()" % SPINNER)
    a, b = text.index(DISC), text.index(DISC_END)
    return "// Luma: patched, see tools/splash_patch.py\n" + text[:a] + fill(disc, pal, "spinner.qml") + text[b:]


def main(argv):
    if len(argv) not in (3, 4):
        print(__doc__.split("\n\n")[0], file=sys.stderr)
        print("usage: splash_patch.py <views directory> <palette json> [<payload/splash directory>]", file=sys.stderr)
        return 2
    views = pathlib.Path(argv[1])
    src = pathlib.Path(argv[3]) if len(argv) == 4 else pathlib.Path(__file__).resolve().parent.parent / "payload" / "splash"
    try:
        pal = json.loads(argv[2])
        splash = patch_splash((views / SPLASH).read_text(), pal, (src / "card.qml").read_text())
        spinner = patch_spinner((views / SPINNER).read_text(), pal, (src / "spinner.qml").read_text())
        (views / SPLASH).write_text(splash)
        (views / SPINNER).write_text(spinner)
    except (ValueError, OSError) as e:
        print("splash_patch: " + str(e), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
