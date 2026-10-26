#!/usr/bin/env python3
"""Contact sheets of the GeForce NOW app's renders: one JPEG per material and text size, six screens each.

    GFN_SHOTS=all cargo test -p lumatv-rustui --lib gfn::shots     (in rustui/; writes rustui/build/gfn-shots/<material>-<percent>/*.png)
    python3 rustui/tools/gfn-sheets.py [OUT_DIR]                      (default docs/research/gfn-app)
"""
import os
import sys

from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
SHOTS = os.path.join(HERE, "..", "build", "gfn-shots")
SCREENS = ["library-art", "library-header", "settings", "settings-help-stream", "signin", "overlay"]
TILE = (960, 540)


def sheet(variant, out_dir):
    src = os.path.join(SHOTS, variant)
    sheet = Image.new("RGB", (2 * TILE[0], 3 * TILE[1]))
    for i, name in enumerate(SCREENS):
        im = Image.open(os.path.join(src, name + ".png")).convert("RGB").resize(TILE, Image.LANCZOS)
        sheet.paste(im, ((i % 2) * TILE[0], (i // 2) * TILE[1]))
    path = os.path.join(out_dir, f"shots-{variant}.jpg")
    sheet.save(path, quality=80, optimize=True)
    return path


def main():
    out_dir = sys.argv[1] if len(sys.argv) > 1 else os.path.join(HERE, "..", "..", "docs", "research", "gfn-app")
    os.makedirs(out_dir, exist_ok=True)
    for variant in ("dark-100", "dark-200", "light-100", "light-200"):
        print(sheet(variant, out_dir))


if __name__ == "__main__":
    main()
