#!/usr/bin/env python3
"""Patches a copy of LG's `CoverUI` module (the startup cover) so that its picture is LumaTV's (docs/mocks/26-startup-cover) and its
sequencing is LG's.

`bootd` opens CoverUI at power-on; it shows the first app's name in 120 px with "Loading..." on black, launches that app, emits
`smart-service-ready` and swallows keys until the first app has a window. The compositor loads it as `import CoverUI 1.0`
(`views/cover/StarfishCoverUI.qml`), so a copy of the module earlier on QML2_IMPORT_PATH (the shadow module, `tools/lumatv build_shadow`) is
what loads. Only `UserInterfaceLayer/Containers/MainView.qml` is touched, and in two places:

  * the root's `color: "black"` becomes the cover's own near-black (the mock's #06050B);
  * the first `Column` (the name and "Loading...") is replaced by `payload/cover/block.qml`: the first app's tile, its name at 40 px,
    a ring with "Starting where you left off" (or "Starting" when the app is not the one the TV was on), and the brand's mark at the
    lower left. The ids `title` and `logoText` stay: the close animation fades `logoText`.

Everything else in the file, and in the module, is LG's: the launch of the first app, the stolen-device screen (`blockUI`), the fade, the
power-off after two minutes. A `Column` that is not found exactly once (a firmware update changed the file) fails here, loudly, and the
caller leaves LG's module in force. The two tiles (`cover-input.png`, `cover-app.png`) are copied beside MainView.qml.

usage: cover_patch.py <CoverUI directory> <palette json> <display name> [<payload/cover directory>]
Exit status: 0 patched in place, 1 an anchor is missing, 2 usage.
"""
import json
import pathlib
import re
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from notification_patch import sub  # noqa: E402

MARKER = "Luma"
MAIN = "UserInterfaceLayer/Containers/MainView.qml"
BACKGROUND = "#06050B"

COLOR = '    color: "black"\n\n    signal readyToLaunchFirstApp();\n'
# the name and "Loading..." of LG's cover: from the first Column to the one that holds the stolen-device screen
START = "    Column {\n\n        anchors.centerIn: parent\n        width: parent.width\n        height: childrenRect.height\n        spacing: 10\n        visible: !blockUI.visible\n"
END = "    Column {\n\n        id: blockUI\n"


def block(src, pal, name):
    pal = dict(pal)
    pal["name"] = name.replace("\\", "\\\\").replace('"', '\\"')
    text = (pathlib.Path(src) / "block.qml").read_text()

    def fill(m):
        if m.group(1) not in pal:
            raise ValueError("block.qml: palette has no %s" % m.group(1))
        return str(pal[m.group(1)])
    return re.sub(r"@@(\w+)@@", fill, text)


def patch(text, pal, name, src):
    if MARKER + ": patched" in text:
        raise ValueError("already patched: start from a fresh copy of LG's module")
    text = text.lstrip("\ufeff")
    text = sub("MainView.qml", text, COLOR, COLOR.replace('"black"', '"%s"' % BACKGROUND))
    a = text.find(START)
    b = text.find(END)
    if text.count(START) != 1 or text.count(END) != 1 or not 0 <= a < b:
        raise ValueError("MainView.qml: LG's name-and-loading block was not found once and before the stolen-device screen")
    return "// Luma: patched, see tools/cover_patch.py\n" + text[:a] + block(src, pal, name) + text[b:]


def main(argv):
    if len(argv) not in (4, 5):
        print(__doc__.split("\n\n")[0], file=sys.stderr)
        print("usage: cover_patch.py <CoverUI directory> <palette json> <display name> [<payload/cover directory>]", file=sys.stderr)
        return 2
    root = pathlib.Path(argv[1])
    src = pathlib.Path(argv[4]) if len(argv) == 5 else pathlib.Path(__file__).resolve().parent.parent / "payload" / "cover"
    try:
        main_qml = root / MAIN
        main_qml.write_text(patch(main_qml.read_text(), json.loads(argv[2]), argv[3], src))
        for pic in ("cover-input.png", "cover-app.png"):
            (main_qml.parent / pic).write_bytes((src / pic).read_bytes())
    except (ValueError, OSError) as e:
        print("cover_patch: " + str(e), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
