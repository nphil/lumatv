#!/usr/bin/env python3
"""Writes payload/store/Manrope-store.woff2: the one face the Content Store theme carries.

    python3 generators/gen-store-font.py

The store's page is LG's cloud page, not a file of the app's directory, and a page on an https origin cannot load a font from
file://. The theme's script therefore carries the face itself, as a data URI, so the face is small: the pinned variable Manrope
(generators/fonts/Manrope-Variable.ttf, provenance in payload/fonts/Manrope.SOURCE) cut to weights 400-700 and to Latin, Latin
Extended-A, Greek, Cyrillic and the punctuation, currency and arrow marks a store page uses. Any other script falls back to the
set's own fonts. One file, one @font-face with a weight range, so the weights the page asks for all come from it.

Needs fontTools and brotli (pip install fonttools brotli).
"""
import hashlib
import io
import os
import sys

from fontTools import subset
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer

HERE = os.path.dirname(os.path.abspath(__file__))
SOURCE = os.path.join(HERE, "fonts", "Manrope-Variable.ttf")
OUT = os.path.join(HERE, "..", "payload", "store", "Manrope-store.woff2")
PINNED_SHA256 = "3ae11c49db0455a3cc33e37d380f20fdb8c7f8b41dc07625c177e3d87a9d6ae6"

UNICODES = (list(range(0x20, 0x7F)) + list(range(0xA0, 0x180)) + list(range(0x370, 0x400)) + list(range(0x400, 0x530)) +
            [0x2010, 0x2011, 0x2012, 0x2013, 0x2014, 0x2018, 0x2019, 0x201A, 0x201C, 0x201D, 0x201E, 0x2022, 0x2026, 0x2030,
             0x2039, 0x203A, 0x20AC, 0x2122, 0x2190, 0x2191, 0x2192, 0x2193, 0x2212, 0x00D7])


def build():
    with open(SOURCE, "rb") as f:
        digest = hashlib.sha256(f.read()).hexdigest()
    if digest != PINNED_SHA256:
        sys.exit("generators/fonts/Manrope-Variable.ttf is not the pinned file (sha256 %s)" % digest)
    font = TTFont(SOURCE)
    opts = subset.Options()
    opts.layout_features = ["kern", "liga", "ccmp", "locl", "mark", "mkmk", "tnum", "case", "calt"]
    opts.notdef_outline = True
    opts.hinting = False
    opts.name_IDs = [1, 2, 3, 4, 6]
    sub = subset.Subsetter(opts)
    sub.populate(unicodes=UNICODES)
    sub.subset(font)
    # The weight range is cut after the glyphs are: instancing the whole font first leaves a variation table the subsetter trips on.
    font = instancer.instantiateVariableFont(font, {"wght": (400, 700)}, inplace=False, updateFontNames=False)
    font.flavor = "woff2"
    buf = io.BytesIO()
    font.save(buf)
    return buf.getvalue()


def main():
    data = build()
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "wb") as f:
        f.write(data)
    print("%s  %d bytes  sha256 %s" % (os.path.relpath(OUT), len(data), hashlib.sha256(data).hexdigest()))


if __name__ == "__main__":
    main()
