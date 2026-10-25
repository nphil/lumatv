#!/usr/bin/env python3
"""Patches a staged copy of LG's notification view (toasts, alerts, input popups, PIN prompt) into
the Luma look and removes what costs a frame.

LG's `SystemUI/Notification/NotificationView.qml` is loaded by the compositor as the item behind the
view name `notificationView` and keeps a documented contract (properties, signals and the close
functions; PLAN.md, Surfaces, and docs/research/surfaces-p2-result.md). That contract and the
data layer behind it (`Services.qml`: the luna subscriptions and the four models) are kept as they
are, so every kind of notification arrives exactly as LG's code expects. What changes:

  * Manrope in every language style the view asks for (the style object names a font per weight).
  * The toast's own colours: dark glass in place of LG's grey, ink in place of its off-white, the
    kit's row radius.
  * No drop shadows. Toast, button and the PIN prompt's focused key each set `layer.enabled` with a
    `DropShadow`: a live blurred offscreen copy per item, which is what makes a stack of toasts or an
    alert with two buttons cost more than a frame. A layer that is not enabled is never built, so
    the blocks stay and only the switch changes.

Every edit is anchored to an exact fragment of LG's source and a missing anchor is fatal: a
half-patched view is worse than an unpatched one, and a firmware update that rewrites these files must
fail here, loudly, instead of shipping a broken alert. The caller (`lumatv notifications on`) stages a
fresh copy of the stock tree every time and falls back to LG's view when this fails.

usage: notification_patch.py <staged Notification dir> <palette JSON from `tokens.py qml`>
Exit status: 0 patched, 1 an anchor is missing, 2 usage.
"""
import json
import pathlib
import re
import sys

MARKER = "Luma"

FONT_RE = re.compile(r'root\.languageStyle\.(?:normal|cond)\d+\.fontFamily\[0\]')
MIN_FONT_SITES = 6

# `layer.enabled` followed by its DropShadow, in the three files that have one per item.
SHADOW_RE = re.compile(r'layer\.enabled:\s*(?:true|root\.activeFocus)(\s*layer\.effect:\s*DropShadow\s*\{)')
SHADOW_FILES = ["Toast.qml", "SimpleButton.qml", "PincodePromptNumButton.qml"]

# Inside the style's `toast` object only: other objects reuse "#ffffff" and "#e6e6e6".
TOAST_START = "property QtObject toast: QtObject {"


def toast_edits(pal):
    return [
        ('property color  backgroundColor:        "#787F86"', 'property color  backgroundColor:        "%s"' % pal["sheetBottom"]),
        ('property color  foregroundColor:        "#ffffff"', 'property color  foregroundColor:        "%s"' % pal["ink"]),
        ('property color  textColor:              "#e6e6e6"', 'property color  textColor:              "%s"' % pal["ink"]),
        ('property int    backgroundRadius:       getRValue(12)', 'property int    backgroundRadius:       getRValue(%d)' % pal["rowRadius"]),
    ]


def block(text, start):
    """(begin, end) of the brace-balanced object whose header is `start`."""
    i = text.index(start)
    depth, j = 0, text.index("{", i)
    for k in range(j, len(text)):
        if text[k] == "{":
            depth += 1
        elif text[k] == "}":
            depth -= 1
            if depth == 0:
                return i, k + 1
    raise ValueError("unbalanced braces after " + start)


def patch_style(text, pal):
    text, n = FONT_RE.subn('"Manrope"', text)
    if n < MIN_FONT_SITES:
        raise ValueError("NotificationStyle.qml: %d font sites found, expected at least %d" % (n, MIN_FONT_SITES))
    try:
        b, e = block(text, TOAST_START)
    except ValueError:
        raise ValueError("NotificationStyle.qml: the toast style object is gone")
    toast = text[b:e]
    for old, new in toast_edits(pal):
        if old not in toast:
            raise ValueError("NotificationStyle.qml: toast anchor missing: " + old)
        toast = toast.replace(old, new, 1)
    return text[:b] + toast + text[e:]


def patch_shadows(name, text):
    text, n = SHADOW_RE.subn(r'layer.enabled: false\1', text)
    if n == 0:
        raise ValueError(name + ": no DropShadow layer found to switch off")
    return text


def patch_tree(root, pal):
    root = pathlib.Path(root)
    style = root / "NotificationStyle.qml"
    text = style.read_text()
    if MARKER + ": patched" in text:
        raise ValueError("already patched: stage a fresh copy of LG's tree")
    style.write_text("// Luma: patched, see tools/notification_patch.py\n" + patch_style(text, pal))
    for name in SHADOW_FILES:
        f = root / name
        f.write_text(patch_shadows(name, f.read_text()))


def main(argv):
    if len(argv) != 3:
        print(__doc__.split("\n\n")[0], file=sys.stderr)
        return 2
    try:
        pal = json.loads(argv[2])
        for k in ("sheetBottom", "ink", "rowRadius"):
            if k not in pal:
                raise ValueError("palette has no " + k)
        patch_tree(argv[1], pal)
    except (ValueError, OSError) as e:
        print("notification_patch: " + str(e), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
