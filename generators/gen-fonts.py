#!/usr/bin/env python3
"""Writes payload/fonts/Manrope-{Regular,Medium,SemiBold,Bold}.ttf from the pinned variable font.

    python3 generators/gen-fonts.py

The static faces are instantiated from generators/fonts/Manrope-Variable.ttf (Manrope 4.505, SIL
OFL 1.1; provenance in payload/fonts/Manrope.SOURCE) at weights 400, 500, 600 and 700 and named the
way Qt's FontLoader and the web @font-face rules expect: family "Manrope", the weight as the style.
LumaShow's embedded Manrope is the same file, so a Luma device renders the same glyphs everywhere.

Needs fontTools (pip install fonttools).
"""
import hashlib
import io
import os
import sys

from fontTools.ttLib import TTFont
from fontTools.varLib import instancer

HERE = os.path.dirname(os.path.abspath(__file__))
SOURCE = os.path.join(HERE, "fonts", "Manrope-Variable.ttf")
OUT = os.path.join(HERE, "..", "payload", "fonts")
PINNED_SHA256 = "3ae11c49db0455a3cc33e37d380f20fdb8c7f8b41dc07625c177e3d87a9d6ae6"
FACES = ((400, "Regular"), (500, "Medium"), (600, "SemiBold"), (700, "Bold"))


def instance(weight, style):
    font = instancer.instantiateVariableFont(TTFont(SOURCE), {"wght": weight}, inplace=False, updateFontNames=False)
    names = font["name"]
    for nid in (16, 17, 21, 22, 25):
        names.removeNames(nameID=nid)
    full = "Manrope " + style
    for nid, value in ((1, "Manrope"), (2, style), (3, "Manrope-%s;4.505" % style), (4, full), (6, "Manrope-" + style)):
        names.setName(value, nid, 3, 1, 0x409)
        names.setName(value, nid, 1, 0, 0)
    font["OS/2"].usWeightClass = weight
    if "STAT" in font:
        del font["STAT"]
    buf = io.BytesIO()
    font.save(buf)
    return buf.getvalue()


def main():
    with open(SOURCE, "rb") as f:
        digest = hashlib.sha256(f.read()).hexdigest()
    if digest != PINNED_SHA256:
        sys.exit("generators/fonts/Manrope-Variable.ttf is not the pinned file (sha256 %s)" % digest)
    for weight, style in FACES:
        path = os.path.join(OUT, "Manrope-%s.ttf" % style)
        data = instance(weight, style)
        with open(path, "wb") as f:
            f.write(data)
        print("%s  %d bytes  sha256 %s" % (os.path.relpath(path), len(data), hashlib.sha256(data).hexdigest()))


if __name__ == "__main__":
    main()
