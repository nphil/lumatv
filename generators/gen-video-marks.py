#!/usr/bin/env python3
"""Writes the format badge's mark sources, `generators/video-logos/*.svg`, which `gen-audio-logos.py --set video` reduces to PNGs.

    python3 generators/gen-video-marks.py

Every mark is one path in `currentColor` on a frame 112 units tall, so the shell tints it with the look's ink. Two kinds:

- a format's name in a frame (4K, FHD, HD, SDR, HDR, HDR10, HDR10+, HLG, 120Hz, 144Hz, PCM, DTS, DTS-HD, DTS:X, 5.1, 7.1): a rounded
  rectangle from 8 to 104, its line 8 thick, and the name inside in Manrope 800 with caps from 28 to 84; 5.1 and 7.1 lead with a speaker;
- Dolby's: the double-D from 8 to 104 beside a small DOLBY (caps 8 to 40) stacked over the kind (caps 52 to 104: VISION, ATMOS, DIGITAL,
  DIGITAL+, AUDIO, TRUEHD), so a badge with two of them stays short.

Every edge that runs across or along the frame sits on a multiple of 4 units: at 28 px (4 units a pixel) the frame's lines are 2 px on whole
pixels, and at 56 and 112 px 4 and 8. Each mark's width is a multiple of 4 units for the same reason. The words are outlines of the pinned
Manrope (`generators/fonts/`), at fixed instances of its weight axis, without kerning.
"""
import os

from fontTools.pens.boundsPen import BoundsPen
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.pens.transformPen import TransformPen
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer

HERE = os.path.dirname(os.path.abspath(__file__))
FONT = os.path.join(HERE, "fonts", "Manrope-Variable.ttf")
OUT = os.path.join(HERE, "video-logos")
FRAME_H = 112
BOX_TOP, BOX_BOTTOM, LINE = 8, 104, 8
OUTER_R = 20
CAPS = (28, 84)              # a framed name's caps, top and baseline
PAD = 16                     # from the frame's inner edge to the name's ink
DOLBY_SMALL = (8, 40)        # DOLBY over the kind
DOLBY_KIND = (52, 104)
DD_W, DD_GAP = 104, 24       # the double-D's width and the air before its words

FRAMED = {
    "uhd": "4K", "fhd": "FHD", "hd": "HD", "sdr": "SDR", "hdr": "HDR", "hdr10": "HDR10", "hdr10plus": "HDR10+", "hlg": "HLG",
    "120hz": "120Hz", "144hz": "144Hz", "pcm": "PCM", "dts": "DTS", "dts-hd": "DTS-HD", "dtsx": "DTS:X", "5.1": "5.1", "7.1": "7.1",
}
SPEAKER = {"5.1", "7.1"}
DOLBY = {"dolby-vision": "VISION", "atmos": "ATMOS", "dolby-digital": "DIGITAL", "dolby-digital-plus": "DIGITAL+", "dolby-audio": "AUDIO",
         "dolby-truehd": "TRUEHD"}

_fonts = {}


def font(weight):
    if weight not in _fonts:
        _fonts[weight] = instancer.instantiateVariableFont(TTFont(FONT), {"wght": weight})
    return _fonts[weight]


def fmt(v):
    s = "%.2f" % v
    return s.rstrip("0").rstrip(".") if "." in s else s


def words(text, weight, caps, x0):
    """`text`'s outlines with its caps from `caps[0]` down to the baseline `caps[1]` and its ink starting at `x0`: (path, ink right)."""
    f = font(weight)
    glyphs, cmap, hmtx = f.getGlyphSet(), f.getBestCmap(), f["hmtx"]
    scale = (caps[1] - caps[0]) / f["OS/2"].sCapHeight
    names = [cmap[ord(c)] for c in text]
    # where the ink starts and ends, in font units along the run
    bounds, pen_x = [], 0
    for n in names:
        bp = BoundsPen(glyphs)
        glyphs[n].draw(bp)
        if bp.bounds:
            bounds.append((pen_x + bp.bounds[0], pen_x + bp.bounds[2]))
        pen_x += hmtx[n][0]
    left, right = min(b[0] for b in bounds), max(b[1] for b in bounds)
    sp = SVGPathPen(glyphs, ntos=fmt)
    pen_x = 0
    for n in names:
        tx = x0 + (pen_x - left) * scale
        glyphs[n].draw(TransformPen(sp, (scale, 0, 0, -scale, tx, caps[1])))
        pen_x += hmtx[n][0]
    return sp.getCommands(), x0 + (right - left) * scale


def rounded(x0, y0, x1, y1, r, clockwise):
    """A rounded rectangle; the inner one of a frame runs the other way round, so under the nonzero rule only the frame's line is filled."""
    if clockwise:
        pts = (x0 + r, y0, x1 - r, r, r, x1, y0 + r, y1 - r, r, r, x1 - r, y1, x0 + r, r, r, x0, y1 - r, y0 + r, r, r, x0 + r, y0)
        return "M%s %sH%sA%s %s 0 0 1 %s %sV%sA%s %s 0 0 1 %s %sH%sA%s %s 0 0 1 %s %sV%sA%s %s 0 0 1 %s %sZ" % tuple(map(fmt, pts))
    pts = (x1 - r, y0, x0 + r, r, r, x0, y0 + r, y1 - r, r, r, x0 + r, y1, x1 - r, r, r, x1, y1 - r, y0 + r, r, r, x1 - r, y0)
    return "M%s %sH%sA%s %s 0 0 0 %s %sV%sA%s %s 0 0 0 %s %sH%sA%s %s 0 0 0 %s %sV%sA%s %s 0 0 0 %s %sZ" % tuple(map(fmt, pts))


def speaker(x0):
    """A speaker as tall as the caps from `x0`, a box and its cone: (path, right)."""
    top, bottom = CAPS
    mid = (top + bottom) / 2
    return "M%s %sH%sL%s %sV%sL%s %sH%sZ" % tuple(map(fmt, (x0, mid - 10, x0 + 12, x0 + 28, top, bottom, x0 + 12, mid + 10, x0))), x0 + 28


def framed(key, text):
    lead = 28 + 12 if key in SPEAKER else 0
    _, right = words(text, 800, CAPS, 0)
    inner = lead + right
    width = -(-(inner + 2 * (PAD + LINE)) // 4) * 4
    # the contents in the middle of the frame: the room the width is rounded up by is shared on both sides
    x = (width - inner) / 2
    parts = []
    if lead:
        parts.append(speaker(x)[0])
    parts.append(words(text, 800, CAPS, x + lead)[0])
    frame = rounded(0, BOX_TOP, width, BOX_BOTTOM, OUTER_R, True) + rounded(LINE, BOX_TOP + LINE, width - LINE, BOX_BOTTOM - LINE, OUTER_R - LINE, False)
    return width, frame + "".join(parts)


def dolby(kind):
    dd = "M0 8A48 48 0 0 1 0 104ZM104 8A48 48 0 0 0 104 104Z"
    x = DD_W + DD_GAP
    small, r1 = words("DOLBY", 800, DOLBY_SMALL, x)
    big, r2 = words(kind, 700, DOLBY_KIND, x)
    return -(-max(r1, r2) // 4) * 4, dd + small + big


def main():
    os.makedirs(OUT, exist_ok=True)
    marks = {k: framed(k, t) for k, t in FRAMED.items()}
    marks.update({k: dolby(t) for k, t in DOLBY.items()})
    for key, (width, d) in marks.items():
        w = int(width)
        with open(os.path.join(OUT, key + ".svg"), "w", encoding="utf-8") as f:
            f.write('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 %d %d" width="%d" height="%d">\n' % (w, FRAME_H, w, FRAME_H))
            f.write('<path fill="currentColor" fill-rule="nonzero" d="%s"/>\n</svg>\n' % d)
        print(key, w)


if __name__ == "__main__":
    main()
