#!/usr/bin/env python3
"""Rasterises the Lucide icons the mocks use into white-on-transparent PNGs for the compositor's QML.

    python3 generators/gen-lucide.py [--size 64] [--out payload/compositor/lumatv/icons/lucide]

The icon source is `docs/mocks/icons.js` (Lucide 1.27, ISC licence): 24-grid stroke paths, stroke width
1.9, round caps and joins, the same icons the approved mocks draw. Each becomes `<name>.png`, 64 px
square (a 32 px icon on the TV, drawn at twice the pixels so a 1080p panel scaled to 4K stays crisp), white
on a transparent ground: `kit/LuIcon.qml` multiplies the alpha by the colour it is given, so one file
serves every colour of both materials. Rendering is Chromium's, headless, on one sheet of all icons.
`index.json` lists the names; a mock gaining an icon means re-running this.
"""
import argparse
import json
import os
import re
import subprocess
import sys
import tempfile

from PIL import Image

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
SRC = os.path.join(ROOT, "docs", "mocks", "icons.js")
STROKE = 1.9
COLS = 16


def load_icons():
    with open(SRC, encoding="utf-8") as f:
        text = f.read()
    m = re.search(r"const I=(\{.*?\});\s*function svg", text, re.S)
    if not m:
        raise SystemExit("icons.js: the icon table was not found")
    return json.loads(m.group(1))


def sheet_html(icons, size):
    cells = []
    for name in sorted(icons):
        cells.append('<svg width="%d" height="%d" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="%s" '
                     'stroke-linecap="round" stroke-linejoin="round">%s</svg>' % (size, size, STROKE, icons[name]))
    return ('<html><body style="margin:0;background:transparent"><div style="display:grid;grid-template-columns:repeat(%d,%dpx);'
            'grid-auto-rows:%dpx">%s</div></body></html>' % (COLS, size, size, "".join(cells)))


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--size", type=int, default=64)
    ap.add_argument("--out", default=os.path.join(ROOT, "payload", "compositor", "lumatv", "icons", "lucide"))
    a = ap.parse_args()
    icons = load_icons()
    names = sorted(icons)
    rows = -(-len(names) // COLS)
    with tempfile.TemporaryDirectory() as d:
        html = os.path.join(d, "sheet.html")
        png = os.path.join(d, "sheet.png")
        with open(html, "w", encoding="utf-8") as f:
            f.write(sheet_html(icons, a.size))
        subprocess.run(["chromium", "--headless=new", "--no-sandbox", "--disable-gpu", "--hide-scrollbars",
                        "--default-background-color=00000000", "--screenshot=" + png,
                        "--window-size=%d,%d" % (COLS * a.size, rows * a.size), "file://" + html],
                       check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=120)
        sheet = Image.open(png).convert("RGBA")
        os.makedirs(a.out, exist_ok=True)
        for i, name in enumerate(names):
            x, y = (i % COLS) * a.size, (i // COLS) * a.size
            cell = sheet.crop((x, y, x + a.size, y + a.size))
            # the sheet is white strokes on transparent: keep colour white and the alpha as drawn
            px = cell.load()
            for yy in range(a.size):
                for xx in range(a.size):
                    r, g, b, al = px[xx, yy]
                    px[xx, yy] = (255, 255, 255, al)
            if cell.getchannel("A").getextrema()[1] == 0:
                raise SystemExit("icon %s rendered empty" % name)
            cell.save(os.path.join(a.out, name + ".png"), optimize=True)
    with open(os.path.join(a.out, "index.json"), "w") as f:
        json.dump(names, f)
        f.write("\n")
    # The toast lays this over LG's notification tree (tools/notification_patch.py): the info glyph in the
    # accent's ink, for the tile it draws in place of LG's own white info tile.
    glyph = Image.open(os.path.join(a.out, "info.png")).convert("RGBA")
    alpha = glyph.getchannel("A")
    dark = Image.new("RGBA", glyph.size, (23, 19, 31, 0))
    dark.putalpha(alpha)
    target = os.path.join(ROOT, "payload", "notification", "images")
    os.makedirs(target, exist_ok=True)
    dark.save(os.path.join(target, "luma-info.png"), optimize=True)
    print("%d icons, %dx%d, in %s" % (len(names), a.size, a.size, a.out))


if __name__ == "__main__":
    sys.exit(main())
