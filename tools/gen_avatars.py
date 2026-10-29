#!/usr/bin/env python3
"""Makes the bundled avatar gallery: payload/avatars/{gallery.json,NOTICE.md,<id>.png}.

usage: python3 tools/gen_avatars.py            (needs the network and PyQt5; the result is committed, the TV never fetches an avatar)

Two sets of twelve, both from DiceBear's styles that are CC0 1.0 (https://www.dicebear.com/licenses/, checked 2026-10-29): People (the
Lorelei style, a remix of Lorelei by Lisa Wischofsky) and Critters (DiceBear's own). CC0 asks for no credit; the artists are named in
NOTICE.md anyway. Each avatar is the style's picture on one of the profile colours (tools of `helper/app/src/profiles.rs`, COLORS), drawn
at 320 x 320 by Qt's SVG renderer, a square: the screen that shows it gives it its own shape. The seeds are chosen by eye from a contact
sheet and are the whole recipe: the same seeds and options give the same pictures from the same DiceBear version (API 10.x).

gallery.json is what the helper reads (`profiles.rs`: `Store::gallery`) and what `/v1/profiles/gallery` lists: {"avatars": [{id, set, file}],
"sets": [{id, name, style, licence, source}]}. `lumatv apply` copies the directory to `<data dir>/avatars/`.
"""
import json
import os
import sys
import urllib.parse
import urllib.request

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
from PyQt5 import QtCore, QtGui, QtSvg  # noqa: E402

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
OUT = os.path.join(ROOT, "payload", "avatars")
API = "https://api.dicebear.com/10.x/%s/svg?seed=%s&backgroundType=solid&backgroundColor=%s"
SIZE = 320
COLORS = ["A79BF5", "EB6F92", "F6C177", "9CCFD8", "3E8FB0", "EA9A97", "C4A7E7", "908CAA"]

SETS = [
    {"id": "people", "name": "People", "style": "lorelei", "licence": "CC0 1.0",
     "source": "DiceBear Lorelei, a remix of Lorelei by Lisa Wischofsky (https://www.dicebear.com/styles/lorelei/)",
     "seeds": ["Nitin", "Isabel", "Leo", "Mia", "Ava", "Sam", "Zoe", "Max", "Ivy", "Kai", "Dev", "Lia"]},
    {"id": "critters", "name": "Critters", "style": "critters", "licence": "CC0 1.0",
     "source": "DiceBear Critters, by DiceBear (https://www.dicebear.com/styles/critters/)",
     "seeds": ["Mia", "Leo", "Max", "Ivy", "Zoe", "Kai", "Noor", "Eli", "Ava", "Sam", "Nitin", "Isabel"]},
]


def render(svg):
    renderer = QtSvg.QSvgRenderer(QtCore.QByteArray(svg))
    image = QtGui.QImage(SIZE, SIZE, QtGui.QImage.Format_ARGB32)
    image.fill(QtGui.QColor(0, 0, 0, 0))
    painter = QtGui.QPainter(image)
    painter.setRenderHint(QtGui.QPainter.Antialiasing)
    renderer.render(painter, QtCore.QRectF(0, 0, SIZE, SIZE))
    painter.end()
    return image


def main():
    app = QtGui.QGuiApplication(sys.argv[:1])  # noqa: F841 (Qt needs one to draw)
    os.makedirs(OUT, exist_ok=True)
    avatars, sets = [], []
    for s in SETS:
        sets.append({k: s[k] for k in ("id", "name", "style", "licence", "source")})
        for i, seed in enumerate(s["seeds"]):
            ident = "%s-%02d" % (s["id"], i + 1)
            url = API % (s["style"], urllib.parse.quote(seed), COLORS[(i + (3 if s["id"] == "critters" else 0)) % len(COLORS)])
            with urllib.request.urlopen(url, timeout=30) as r:
                svg = r.read()
            path = os.path.join(OUT, ident + ".png")
            if not render(svg).save(path, "PNG"):
                raise SystemExit("could not write " + path)
            avatars.append({"id": ident, "set": s["id"], "file": ident + ".png"})
            print(ident, os.path.getsize(path))
    with open(os.path.join(OUT, "gallery.json"), "w") as f:
        json.dump({"v": 1, "avatars": avatars, "sets": sets}, f, indent=2)
        f.write("\n")
    with open(os.path.join(OUT, "NOTICE.md"), "w") as f:
        f.write("# Avatars\n\nThe pictures in this folder are the bundled avatar gallery of LumaTV's profiles. Both sets are CC0 1.0 (public domain\n"
                "dedication, https://creativecommons.org/publicdomain/zero/1.0/); nothing is required, and the artists are named anyway.\n\n")
        for s in sets:
            f.write("- **%s** (`%s-NN.png`): %s. Licence: %s.\n" % (s["name"], s["id"], s["source"], s["licence"]))
        f.write("\nMade by `tools/gen_avatars.py` from DiceBear's API (10.x) on the profile colours; the DiceBear code is MIT licensed, each style's art has its\n"
                "own licence (https://www.dicebear.com/licenses/). A person's own photo is not in this folder: it is kept with the profile.\n")


if __name__ == "__main__":
    main()
