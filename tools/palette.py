#!/usr/bin/env python3
"""Derives the settings-menu palette from the Luma theme.

The menus have to be tinted by whatever wallpaper and theme are in use, not by
constants baked into a patch, or they stop matching the moment the wallpaper
changes. Everything here comes out of the data dir's theme/theme.json:

  dark.glass      the Home cards' material -> the sheet tint
  dark.ink        body text
  dark.glassEdge  the hairline along a lit edge
  accent          toggles, sliders, selection
  radius          sheet corners; tileRadius is the rows and buttons

The theme's glass is deliberately sheer - it sits over a wallpaper the
compositor already drew. A settings sheet can be over live picture instead, so
the same hue is used at a much higher alpha: the tint tracks the theme, the
contrast does not depend on what is behind it.

usage: palette.py <theme.json> qml|css
Runs on the set (python 3.10). Called by tools/lumatv.
"""
import json
import sys

# Used when the theme is missing a key, and equal to the shipped dark theme, so
# a stock install looks the same whether or not the file parses.
FALLBACK = {
    "glass": "#66120E1A",
    "ink": "#F2EFF6",
    "glassEdge": "#21FFFFFF",
    "accent": "#A79BF5",
    "radius": 26,
    "tileRadius": 22,
}


def parse(colour, default):
    """#AARRGGBB or #RRGGBB -> (r, g, b, a) with a in 0..1. QML's order."""
    c = (colour or "").lstrip("#")
    try:
        if len(c) == 8:
            a, r, g, b = (int(c[i:i + 2], 16) for i in (0, 2, 4, 6))
            return r, g, b, a / 255.0
        if len(c) == 6:
            r, g, b = (int(c[i:i + 2], 16) for i in (0, 2, 4))
            return r, g, b, 1.0
    except ValueError:
        pass
    return parse(default, "#66120E1A") if colour != default else (18, 14, 26, 0.4)


def argb(r, g, b, a):
    return "#%02X%02X%02X%02X" % (max(0, min(255, int(round(a * 255)))), r, g, b)


def lift(r, g, b, amount):
    """Toward white, for the top of a sheet's gradient."""
    return tuple(int(round(v + (255 - v) * amount)) for v in (r, g, b))


theme = {}
try:
    theme = json.load(open(sys.argv[1]))
except Exception:
    pass

dark = theme.get("dark") or {}
glass = dark.get("glass") or FALLBACK["glass"]
ink = dark.get("ink") or FALLBACK["ink"]
edge = dark.get("glassEdge") or FALLBACK["glassEdge"]
accent = theme.get("accent") or FALLBACK["accent"]
if not isinstance(accent, str) or not accent.startswith("#"):
    accent = FALLBACK["accent"]           # "tile" means per-tile on Home, which a menu has no equivalent of
radius = int(theme.get("radius") or FALLBACK["radius"])
tile_radius = int(theme.get("tileRadius") or FALLBACK["tileRadius"])

gr, gg, gb, _ = parse(glass, FALLBACK["glass"])
ir, ig, ib, _ = parse(ink, FALLBACK["ink"])
er, eg, eb, ea = parse(edge, FALLBACK["glassEdge"])
ar, ag, ab, _ = parse(accent, FALLBACK["accent"])

# A menu sheet is not a Home card: it can be over a bright film as easily as
# over the wallpaper, and it carries small text. So the theme supplies the hue
# and the sheet supplies its own contrast.
#
# The guard below is what stops a pale or washed-out theme producing a menu
# nobody can read: the sheet is darkened (or lightened, for a genuinely light
# theme) until its body text clears a 7:1 contrast ratio against it - AAA for
# normal text, which is the right target for text read across a room.
def luminance(r, g, b):
    def channel(v):
        v /= 255.0
        return v / 12.92 if v <= 0.03928 else ((v + 0.055) / 1.055) ** 2.4
    return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)


def contrast(a, b):
    la, lb = luminance(*a), luminance(*b)
    hi, lo = max(la, lb), min(la, lb)
    return (hi + 0.05) / (lo + 0.05)


def toward(rgb, target, amount):
    return tuple(int(round(v + (t - v) * amount)) for v, t in zip(rgb, target))


sheet = (gr, gg, gb)
ink_rgb = (ir, ig, ib)
# Which way to push: away from the ink, whichever end of the scale that is.
target = (0, 0, 0) if luminance(*ink_rgb) > 0.5 else (255, 255, 255)
steps = 0
while contrast(sheet, ink_rgb) < 7.0 and steps < 20:
    sheet = toward(sheet, target, 0.12)
    steps += 1

# The sheet is nearly opaque on purpose. Sheer glass works on Home because the
# compositor drew the wallpaper behind it; a settings sheet can be over motion,
# and text over motion is unreadable at the alphas the Home cards use.
top = lift(*sheet, 0.06) if luminance(*sheet) < 0.5 else toward(sheet, (0, 0, 0), 0.06)
# Sheer enough to read as glass over the wallpaper, opaque enough that what is
# behind one pane cannot tint it differently from the pane beside it.
sheet_top = argb(*top, 0.93)
sheet_bottom = argb(*sheet, 0.93)
# Rows, buttons and cards inside a sheet: sheer, because the sheet is behind them.
row = argb(*toward(sheet, (255, 255, 255) if luminance(*sheet) < 0.5 else (0, 0, 0), 0.10), 0.35)
# Focus is the same material, lifted: brighter glass, a stronger edge and a
# shadow under it, rather than a solid light fill. It keeps the panel reading as
# one set of surfaces, and it keeps text and icons light, which is what lets the
# rest of the theme stay dark.
# Selection is the same glass, lifted - not a block of another colour. A thin
# wash of light over the sheet keeps the hue identical, so the only things that
# separate a selected row from the surface are the lit edge and the shadow
# under it. Anything more opaque reads as a background swatch sitting on top.
focus = argb(255, 255, 255, 0.07) if luminance(*sheet) < 0.5 else argb(0, 0, 0, 0.06)
focus_edge = argb(255, 255, 255, 0.28) if luminance(*sheet) < 0.5 else argb(0, 0, 0, 0.22)
hairline = argb(er, eg, eb, min(0.22, max(0.10, ea)))

palette = {
    "sheetTop": sheet_top,
    "sheetBottom": sheet_bottom,
    "row": row,
    "focus": focus,
    "focusEdge": focus_edge,
    "hairline": hairline,
    "ink": argb(ir, ig, ib, 1.0),
    "inkDim": argb(ir, ig, ib, 0.66),
    "focusInk": argb(ir, ig, ib, 1.0),
    "accent": argb(ar, ag, ab, 1.0),
    "radius": radius + 8,          # a sheet is bigger than a card, and reads better slightly rounder
    "rowRadius": tile_radius,
    "scrim": argb(*lift(gr, gg, gb, 0.0), 0.42),
}


def css_rgba(value):
    r, g, b, a = parse(value, value)
    return "rgba(%d, %d, %d, %.3f)" % (r, g, b, a)


if sys.argv[2] == "qml":
    print(json.dumps(palette))
else:
    print(""":root {
  --luma-ink: %s;
  --luma-ink-2: %s;
  --luma-glass: %s;
  --luma-glass-edge: %s;
  --luma-sheet-top: %s;
  --luma-sheet-bottom: %s;
  --luma-focus: %s;
  --luma-focus-edge: %s;
  --luma-focus-ink: %s;
  --luma-accent: %s;
  --luma-scrim: %s;
  --luma-radius-sheet: %dpx;
  --luma-radius-row: %dpx;
  --luma-radius-pill: 999px;
}""" % (
        css_rgba(palette["ink"]), css_rgba(palette["inkDim"]), css_rgba(palette["row"]),
        css_rgba(palette["hairline"]), css_rgba(palette["sheetTop"]), css_rgba(palette["sheetBottom"]),
        css_rgba(palette["focus"]), css_rgba(palette["focusEdge"]),
        css_rgba(palette["focusInk"]), css_rgba(palette["accent"]),
        css_rgba(palette["scrim"]), palette["radius"], palette["rowRadius"]))
