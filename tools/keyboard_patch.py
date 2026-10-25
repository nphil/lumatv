#!/usr/bin/env python3
"""Patches a copy of LG's `StarfishKeyboardView.qml` (the compositor view that hosts the on-screen keyboard's window) so that the
keyboard comes and goes with the Lucent layer motion instead of LG's 600 ms jelly.

The keyboard's keys are not drawn here: the IME (maliit, `com.webos.service.ime`) draws them in a window of its own and this view
only places that window at the bottom of the screen, tells the compositor where the panel is (`panelRect`) and plays an animation
while it opens and closes. LG's animation is a "pudding": a Beziergon (a native mesh item) draws a live texture of the keyboard,
squashed by a 600 ms OutCubic on its top edge, while the panel itself overshoots on an OutBack. That is the one thing about it
that is Luma's to change from here, and it is what this does:

  * Open: the panel rises from below its own height to its place in the theme's layer time (`motion.layerMs`) on the card easing,
    where LG's rises in 600 ms with an overshoot.
  * Close: it sinks the same distance in the same time on the matching ease-in, then hides, where LG hides it at once and shows
    the squashed texture for 600 ms.
  * The Beziergon and its offscreen texture are removed, and with them the import of the native plugin that provides it: nothing
    else reads them, and the open and close animations were the only things that ever made them visible.

The view's contract with `BaseView` (the `openAnimation` and `closeAnimation` it runs, the `opened` and `closed` it then emits),
the drag masks, the rotation, `panelRect` and `inputMethod.allowed` are LG's, untouched. The panel's own look (the key caps, the
language bar, the candidate list) is maliit's and is not in anything copied from the set so far: see docs/research/lg-restyles-result.md.

Every edit is anchored to an exact block of LG's source and must find it exactly once: a firmware update that rewrites this view
fails here, loudly, and the caller leaves LG's file in force.

usage: keyboard_patch.py <StarfishKeyboardView.qml> <theme.json>
Exit status: 0 patched in place, 1 an anchor is missing or the theme has no usable motion, 2 usage.
"""
import json
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from notification_patch import sub  # noqa: E402
from tokens import EASING  # noqa: E402

MARKER = "Luma"

PUDDING = """    Eos.Beziergon {
        id: pudding
        property real sides
        property int offset: 3
        anchors.fill: draggableMask
        visible: draggableMask.isTransitioning
        sourceItem: ShaderEffectSource {
            id: fbo
            hideSource: pudding.visible
            sourceItem: currentItem
        }

        color: "#00ff0000"
        clampToEdge: true
        controlTopLeft: Qt.point(width / 2, 0)
        topLeft: Qt.point(0,sides)
        topRight: Qt.point(width,sides)
        //make it taller than the height so we don't show a gap
        // when doing Easing.*Back
        bottomLeft: Qt.point(0,height * 2 + offset)
        bottomRight: Qt.point(width,height * 2 + offset)

    }

"""

OPEN = """    openAnimation: SequentialAnimation {
        id: keyboardOpenAnimation
        PropertyAction { target: currentItem; property: "visible";  value: true; }
        ParallelAnimation {
            PropertyAnimation {
                target: pudding
                easing.type: Easing.OutCubic
                properties: "sides"
                from: 600
                to: 0
                duration: 600
            }

            PropertyAnimation {
                target: draggableMask
                easing.type: Easing.OutBack
                properties: "anchors.bottomMargin"
                duration: 600
            }

            NumberAnimation {
                target: draggableMask
                from: -draggableMask.height
                to: 0
                properties: "anchors.bottomMargin"
                duration: 600
                easing.type: Easing.OutBack
            }
        }
    }
"""

CLOSE = """    closeAnimation: SequentialAnimation {
        id: keyboardCloseAnimation
        PropertyAction { target: pudding; property: "visible"; value: true }
        PropertyAction { target: currentItem; property: "visible"; value: false }
        ParallelAnimation {
            PropertyAnimation {
                target: pudding
                easing.type: Easing.OutCubic
                properties: "sides"
                from: 0
                to: 300
                duration: 600
            }
            PropertyAnimation {
                target: draggableMask
                easing.type: Easing.OutCubic
                properties: "anchors.bottomMargin"
                duration: 600
            }
        }
    }
"""


def ease_in(name):
    """The ease-in that matches a layer's ease-out: OutCubic -> InCubic. Linear and the symmetric curves stay."""
    return "In" + name[3:] if name.startswith("Out") else name


def new_open(ms, ease):
    return (
        "    openAnimation: SequentialAnimation {\n"
        "        id: keyboardOpenAnimation\n"
        "        PropertyAction { target: currentItem; property: \"visible\";  value: true; }\n"
        "        NumberAnimation {\n"
        "            target: draggableMask\n"
        "            from: -draggableMask.height\n"
        "            to: 0\n"
        "            properties: \"anchors.bottomMargin\"\n"
        "            duration: %d\n"
        "            easing.type: Easing.%s\n"
        "        }\n"
        "    }\n" % (ms, ease))


def new_close(ms, ease):
    return (
        "    closeAnimation: SequentialAnimation {\n"
        "        id: keyboardCloseAnimation\n"
        "        NumberAnimation {\n"
        "            target: draggableMask\n"
        "            from: 0\n"
        "            to: -draggableMask.height\n"
        "            properties: \"anchors.bottomMargin\"\n"
        "            duration: %d\n"
        "            easing.type: Easing.%s\n"
        "        }\n"
        "        PropertyAction { target: currentItem; property: \"visible\"; value: false }\n"
        "        PropertyAction { target: draggableMask; property: \"anchors.bottomMargin\"; value: 0 }\n"
        "    }\n" % (ms, ease_in(ease)))


def motion(theme):
    m = theme["motion"]
    ms, ease = m["layerMs"], m["cardEasing"]
    if not isinstance(ms, int) or not 0 < ms <= 1000:
        raise ValueError("theme motion.layerMs is %r, expected 1..1000 ms" % (ms,))
    if ease not in EASING:
        raise ValueError("theme motion.cardEasing is %r, expected one of %s" % (ease, ", ".join(sorted(EASING))))
    return ms, ease


def patch(text, theme):
    name = "StarfishKeyboardView.qml"
    if MARKER + ": patched" in text:
        raise ValueError("already patched: start from a fresh copy of LG's file")
    ms, ease = motion(theme)
    text = sub(name, text, "import Eos.Items 0.1 as Eos\n", "")
    text = sub(name, text, PUDDING, "")
    text = sub(name, text, OPEN, new_open(ms, ease))
    text = sub(name, text, CLOSE, new_close(ms, ease))
    return "// Luma: patched, see tools/keyboard_patch.py\n" + text


def main(argv):
    if len(argv) != 3:
        print(__doc__.split("\n\n")[0], file=sys.stderr)
        print("usage: keyboard_patch.py <StarfishKeyboardView.qml> <theme.json>", file=sys.stderr)
        return 2
    try:
        with open(argv[2]) as f:
            theme = json.load(f)
        path = pathlib.Path(argv[1])
        path.write_text(patch(path.read_text(), theme))
    except (ValueError, KeyError, OSError) as e:
        print("keyboard_patch: " + str(e), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
