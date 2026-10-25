#!/usr/bin/env python3
"""Patches a staged copy of LG's notification view (toasts, alerts, input popups, PIN prompt) into
the Luma look and removes what costs a frame.

LG's `SystemUI/Notification/NotificationView.qml` is loaded by the compositor as the item behind the
view name `notificationView` and keeps a documented contract (properties, signals and the close
functions; PLAN.md, Surfaces, and docs/research/surfaces-p2-result.md). That contract and the
data layer behind it (`Services.qml`: the luna subscriptions and the four models) are kept as they
are, so every kind of notification arrives exactly as LG's code expects. What changes:

  * Manrope in every language style the view asks for (the style object names a font per weight).
  * The toast is LumaTV's own `Toast.qml` (the overlay directory, palette markers filled in): a glass
    pill at the top right with an icon tile and a title over a dimmer subtitle, where LG draws a grey
    bar at the bottom. ToastView's three placement lines are patched to put it there.
  * The alert is LG's, in the palette: dark glass for LG's grey, ink for its dark text, the row
    radius, a hairline edge, no translucency on top of the glass's own. Its buttons (`SimpleButton`)
    hard-code their colours, so those lines are patched too: a focused button is a lighter glass
    with a light edge in place of LG's near-white fill.
  * The alert sits in the middle of the screen, over the palette's scrim, where LG puts it above the
    bottom edge, over a black one.
  * LG's focus colour (`defaultFocusColor`, a red) is the palette's accent.
  * No live drop shadows. Button and the PIN prompt's focused key each set `layer.enabled` with a
    `DropShadow`: a blurred offscreen copy per item, which is what makes an alert with two buttons
    cost more than a frame. A layer that is not enabled is never built, so the blocks stay and only
    the switch changes. (The alert card's own shadow is nine static images, which cost nothing.)

Every edit is anchored to an exact fragment of LG's source and must find it exactly as many times as
it expects: a missing or doubled anchor is fatal, because a half-patched view is worse than an
unpatched one, and a firmware update that rewrites these files must fail here, loudly, instead of
shipping a broken alert. The caller (`lumatv notifications on`) stages a fresh copy of the stock tree
every time and falls back to LG's view when this fails.

usage: notification_patch.py <staged Notification dir> <palette JSON from `tokens.py qml`> [<overlay dir>]
Exit status: 0 patched, 1 an anchor is missing or the palette lacks a name, 2 usage.
"""
import json
import pathlib
import re
import sys

MARKER = "Luma"

FONT_RE = re.compile(r'root\.languageStyle\.(?:normal|cond)\d+\.fontFamily\[0\]')
MIN_FONT_SITES = 6

# `layer.enabled` followed by its DropShadow, in the files that have one per item.
SHADOW_RE = re.compile(r'layer\.enabled:\s*(?:true|root\.activeFocus)(\s*layer\.effect:\s*DropShadow\s*\{)')
SHADOW_FILES = ["SimpleButton.qml", "PincodePromptNumButton.qml"]

# Palette names the edits and the overlay draw on.
NEEDED = ("sheetBottom", "row", "focus", "focusEdge", "hairline", "ink", "inkDim", "accent", "rowRadius", "scrim")

# Where the toast goes: below the status bar, right edges level with it (the mock, 07-toast).
TOAST_TOP = 137
TOAST_RIGHT = 64
TOAST_WIDTH = 760


def sub(name, text, old, new, count=1):
    """`old` replaced by `new`, where `old` must be there exactly `count` times."""
    found = text.count(old)
    if found != count:
        raise ValueError("%s: anchor found %d times, expected %d: %s" % (name, found, count, old.strip()))
    return text.replace(old, new)


def blocks(text, header):
    """(begin, end) of every brace-balanced object whose header is `header`, in file order."""
    out, at = [], 0
    while True:
        i = text.find(header, at)
        if i < 0:
            return out
        depth, j = 0, text.index("{", i)
        for k in range(j, len(text)):
            if text[k] == "{":
                depth += 1
            elif text[k] == "}":
                depth -= 1
                if depth == 0:
                    out.append((i, k + 1))
                    at = k + 1
                    break
        else:
            raise ValueError("unbalanced braces after " + header)


def edit_blocks(name, text, header, which, edits):
    """Apply `edits` ((old, new) pairs, each found once) inside the `which`-th blocks of `header`."""
    found = blocks(text, header)
    if len(found) <= max(which):
        raise ValueError("%s: %d of the style objects `%s` found, expected at least %d" % (name, len(found), header, max(which) + 1))
    for n in sorted(which, reverse=True):
        b, e = found[n]
        part = text[b:e]
        for old, new in edits:
            part = sub("%s %s #%d" % (name, header, n), part, old, new)
        text = text[:b] + part + text[e:]
    return text


def alert_edits(pal):
    return [
        ('property color  backgroundColor:        "#CACBCC"', 'property color  backgroundColor:        "%s"' % pal["sheetBottom"]),
        ('property color  borderColor:            "#bbbbbb"', 'property color  borderColor:            "%s"' % pal["hairline"]),
        ('property int    radius:                 getRValue(12)', 'property int    radius:                 getRValue(%d)' % pal["rowRadius"]),
        ('property real   backgroundOpacity:      0.95', 'property real   backgroundOpacity:      1'),
        ('property color  alertTextColor:         "#2E3239"', 'property color  alertTextColor:         "%s"' % pal["ink"]),
        ('property color  foregroundColor:        "#4d4d4d"', 'property color  foregroundColor:        "%s"' % pal["ink"]),
        ('property color  subtitleColor:          "#ABAEB3"', 'property color  subtitleColor:          "%s"' % pal["inkDim"]),
        ('property color  titleRulerColor:        "#a7a7a7"', 'property color  titleRulerColor:        "%s"' % pal["hairline"]),
    ]


def button_edits(pal):
    return [
        ('property color  backgroundColor:        "#686868"', 'property color  backgroundColor:        "%s"' % pal["row"]),
    ]


def patch_style(text, pal):
    name = "NotificationStyle.qml"
    text, n = FONT_RE.subn('"Manrope"', text)
    if n < MIN_FONT_SITES:
        raise ValueError("%s: %d font sites found, expected at least %d" % (name, n, MIN_FONT_SITES))
    text = sub(name, text, 'property color defaultFocusColor: "#cf0652"', 'property color defaultFocusColor: "%s"' % pal["accent"])
    text = sub(name, text, 'property string scrimColor: "black"', 'property string scrimColor: "%s"' % pal["scrim"])
    text = sub(name, text, "property real scrimOpacity: 0.3", "property real scrimOpacity: 1")
    text = edit_blocks(name, text, "property QtObject toast: QtObject {", [0],
                       [("property int    width:                  getRValue(900)", "property int    width:                  getRValue(%d)" % TOAST_WIDTH)])
    # The base object and `variations.defaultView` hold the same values and the second is bound over
    # the first when the view starts; `partialView` is the Atelier screens' and not this set's.
    text = edit_blocks(name, text, "property QtObject alert: QtObject {", [0, 1], alert_edits(pal))
    text = edit_blocks(name, text, "property QtObject button: QtObject {", [0, 1], button_edits(pal))
    return text


def patch_toastview(text):
    name = "ToastView.qml"
    text = sub(name, text, "anchors.topMargin: root.style.toast.viewMargins", "anchors.topMargin: root.style.getRValue(%d)" % TOAST_TOP)
    text = sub(name, text, "anchors.rightMargin: root.style.toast.viewMargins", "anchors.rightMargin: root.style.getRValue(%d)" % TOAST_RIGHT)
    text = sub(name, text,
               'x: (type === "light") ? toastStack.width - width - root.style.getRValue(38) : (root.width - style.toast.width) / 2',
               "x: toastStack.width - width")
    return text


def patch_simplebutton(text, pal):
    name = "SimpleButton.qml"
    text = sub(name, text, 'property color textColor: root.hasFocus ? "#4C5059" : "#E6E6E6"', 'property color textColor: "%s"' % pal["ink"])
    text = sub(name, text, 'property color bgColor: root.hasFocus ? "#E6E6E6" : (noBackgroundColor ? "transparent" : "#7D848C")',
               'property color bgColor: root.hasFocus ? "%s" : (noBackgroundColor ? "transparent" : "%s")' % (pal["focus"], pal["row"]))
    text = sub(name, text, "        radius: 6\n", "        radius: root.style.getRValue(%d)\n" % pal["rowRadius"])
    text = sub(name, text, '        border.width: useStroke ? 1 : 0\n        border.color: "#FFFFFF"\n',
               '        border.width: root.hasFocus ? 2 : (useStroke ? 1 : 0)\n        border.color: root.hasFocus ? "%s" : "#FFFFFF"\n' % pal["focusEdge"])
    text = sub(name, text, 'PropertyChanges {target: label; color: "#4C5059"}', 'PropertyChanges {target: label; color: "%s"}' % pal["ink"])
    text = sub(name, text, 'PropertyChanges { target: label; color: "#E6E6E6" }', 'PropertyChanges { target: label; color: "%s" }' % pal["ink"])
    return text


def patch_alert(text):
    """The card in the middle of the screen (the mock, 06-confirm-dialog) where LG puts it 180 px above
    the bottom edge. A full alert and the partial-screen layout keep their own placement."""
    name = "Alert.qml"
    return sub(name, text,
               "        anchors.bottom: isFullAlert ? undefined: isPartialScreen ? undefined : parent.bottom\n"
               "        anchors.bottomMargin: isFullAlert ? 0 : root.style.getRValue(180)\n",
               "        anchors.verticalCenter: isFullAlert ? undefined : isPartialScreen ? undefined : parent.verticalCenter\n"
               "        anchors.verticalCenterOffset: 0\n")


def patch_shadows(name, text):
    text, n = SHADOW_RE.subn(r'layer.enabled: false\1', text)
    if n == 0:
        raise ValueError(name + ": no DropShadow layer found to switch off")
    return text


def overlay(root, src, pal):
    """Our own files laid over the tree. Each replaces a file of LG's of the same name (a missing
    original is fatal: it would mean the view no longer has the part we draw in place of)."""
    for f in sorted(pathlib.Path(src).glob("*.qml")):
        if not (root / f.name).is_file():
            raise ValueError("%s: LG's tree has no such file to replace" % f.name)

        def fill(m):
            if m.group(1) not in pal:
                raise ValueError("%s: palette has no %s" % (f.name, m.group(1)))
            return str(pal[m.group(1)])
        (root / f.name).write_text(re.sub(r"@@(\w+)@@", fill, f.read_text()))


def patch_tree(root, pal, overlay_dir=None):
    root = pathlib.Path(root)
    style = root / "NotificationStyle.qml"
    text = style.read_text()
    if MARKER + ": patched" in text:
        raise ValueError("already patched: stage a fresh copy of LG's tree")
    style.write_text("// Luma: patched, see tools/notification_patch.py\n" + patch_style(text, pal))
    for name, fn in (("ToastView.qml", patch_toastview), ("SimpleButton.qml", lambda t: patch_simplebutton(t, pal)), ("Alert.qml", patch_alert)):
        f = root / name
        f.write_text(fn(f.read_text()))
    for name in SHADOW_FILES:
        f = root / name
        f.write_text(patch_shadows(name, f.read_text()))
    if overlay_dir:
        overlay(root, overlay_dir, pal)


def main(argv):
    if len(argv) not in (3, 4):
        print(__doc__.split("\n\n")[0], file=sys.stderr)
        return 2
    try:
        pal = json.loads(argv[2])
        for k in NEEDED:
            if k not in pal:
                raise ValueError("palette has no " + k)
        patch_tree(argv[1], pal, argv[3] if len(argv) == 4 else None)
    except (ValueError, OSError) as e:
        print("notification_patch: " + str(e), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
