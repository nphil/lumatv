#!/usr/bin/env python3
"""Rasterises the Home's icon set to PNG at the exact pixel sizes the QML draws them.

The compositor's Qt has no SVG plugin, so every glyph ships as a PNG rendered
once here, at its display size, so it is never resampled on the set. Stroke
icons come in two inks (dark material = light ink, light material = dark ink);
weather "now" glyphs are coloured and material-independent. Output: payload/compositor/lumatv/icons/.
"""
import os
import cairosvg

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "payload", "compositor", "lumatv", "icons")
os.makedirs(OUT, exist_ok=True)

INK = {"d": "rgba(242,239,246,0.66)", "l": "rgba(23,19,31,0.66)"}
INK_FULL = {"d": "#F2EFF6", "l": "#17131F"}

# Lucide-style 24-unit stroke paths, stroke 2, round caps/joins.
STROKE = {
    "bell": '<path d="M18 8a6 6 0 0 0-12 0c0 7-3 9-3 9h18s-3-2-3-9"/><path d="M13.7 21a2 2 0 0 1-3.4 0"/>',
    "settings": '<circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z"/>',
    "search": '<circle cx="11" cy="11" r="7"/><path d="m21 21-4.3-4.3"/>',
    "generic": '<circle cx="12" cy="12" r="8"/>',
    # weather, small rows
    "wx-sun": '<circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M2 12h2M20 12h2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"/>',
    "wx-moon": '<path d="M20 15.5A8.5 8.5 0 0 1 8.5 4a8.5 8.5 0 1 0 11.5 11.5z"/>',
    "wx-partly": '<path d="M12 2v2M4.9 4.9l1.4 1.4M2 12h2M19.1 4.9l-1.4 1.4M20 12h2"/><path d="M8.5 13a4 4 0 1 1 7.6-1.7"/><path d="M17.5 21a3.5 3.5 0 0 0 .4-7 5 5 0 0 0-9.6 1.4A3 3 0 0 0 8 21z"/>',
    "wx-partly-night": '<path d="M9.5 4.5A5 5 0 0 0 15 10"/><path d="M17.5 21a3.5 3.5 0 0 0 .4-7 5 5 0 0 0-9.6 1.4A3 3 0 0 0 8 21z"/>',
    "wx-cloud": '<path d="M17.5 19a4.5 4.5 0 0 0 .5-9 7 7 0 0 0-13.5 2A4 4 0 0 0 6 19z"/>',
    "wx-fog": '<path d="M17.5 15a4.5 4.5 0 0 0 .5-9 7 7 0 0 0-13.5 2A4 4 0 0 0 6 15z"/><path d="M5 19h14M8 22h8"/>',
    "wx-rain": '<path d="M17.5 17a4.5 4.5 0 0 0 .5-9 7 7 0 0 0-13.5 2A4 4 0 0 0 6 17z"/><path d="M8 19v2M12 19v3M16 19v2"/>',
    "wx-drizzle": '<path d="M17.5 17a4.5 4.5 0 0 0 .5-9 7 7 0 0 0-13.5 2A4 4 0 0 0 6 17z"/><path d="M8 19v1M12 19v1M16 19v1"/>',
    "wx-snow": '<path d="M17.5 17a4.5 4.5 0 0 0 .5-9 7 7 0 0 0-13.5 2A4 4 0 0 0 6 17z"/><path d="M8 19h.01M12 21h.01M16 19h.01M10 22h.01M14 22h.01"/>',
    "wx-storm": '<path d="M17.5 16a4.5 4.5 0 0 0 .5-9 7 7 0 0 0-13.5 2A4 4 0 0 0 6 16z"/><path d="M13 13l-2 4h3l-2 4"/>',
}


def stroke_svg(body, colour, width=2):
    return ('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="%s" '
            'stroke-width="%s" stroke-linecap="round" stroke-linejoin="round">%s</svg>' % (colour, width, body))


def render(svg, size, name):
    cairosvg.svg2png(bytestring=svg.encode(), write_to=os.path.join(OUT, name), output_width=size, output_height=size)


# status bar: 26px in ink2, focused variant in full ink
for ink in ("d", "l"):
    for key, w in (("bell", 2), ("settings", 2), ("search", 2.2), ("generic", 2)):
        render(stroke_svg(STROKE[key], INK[ink], w), 26, "%s_%s.png" % (key, ink))
        render(stroke_svg(STROKE[key], INK_FULL[ink], w), 26, "%s_%s_on.png" % (key, ink))
    for key in [k for k in STROKE if k.startswith("wx-")]:
        render(stroke_svg(STROKE[key], INK[ink]), 26, "%s_%s.png" % (key, ink))

# weather "now": 96px, coloured flat glyphs in the mock's style
SUN = "#FFD166"
CLOUD = "#E9ECF3"
MOON = "#E9ECF3"
RAIN = "#7FB8FF"
BIG = {
    "sun": '<circle cx="32" cy="32" r="13" fill="%s"/><g stroke="%s" stroke-width="4" stroke-linecap="round"><path d="M32 6v8M32 50v8M6 32h8M50 32h8M13.6 13.6l5.7 5.7M44.7 44.7l5.7 5.7M13.6 50.4l5.7-5.7M44.7 19.3l5.7-5.7"/></g>' % (SUN, SUN),
    "moon": '<path d="M44 40A18 18 0 0 1 24 12a18 18 0 1 0 20 28z" fill="%s"/>' % MOON,
    "partly": '<circle cx="40" cy="22" r="11" fill="%s"/><path d="M18 50h27a9 9 0 0 0 1-18 12 12 0 0 0-23 2 8 8 0 0 0-5 16z" fill="%s"/>' % (SUN, CLOUD),
    "partly-night": '<path d="M46 26a10 10 0 0 1-12-14 10 10 0 1 0 12 14z" fill="%s"/><path d="M18 50h27a9 9 0 0 0 1-18 12 12 0 0 0-23 2 8 8 0 0 0-5 16z" fill="%s"/>' % (MOON, CLOUD),
    "cloud": '<path d="M18 46h27a9 9 0 0 0 1-18 12 12 0 0 0-23 2 8 8 0 0 0-5 16z" fill="%s"/>' % CLOUD,
    "fog": '<path d="M18 38h27a9 9 0 0 0 1-18 12 12 0 0 0-23 2 8 8 0 0 0-5 16z" fill="%s"/><g stroke="%s" stroke-width="4" stroke-linecap="round"><path d="M14 46h36M20 54h24"/></g>' % (CLOUD, CLOUD),
    "rain": '<path d="M18 40h27a9 9 0 0 0 1-18 12 12 0 0 0-23 2 8 8 0 0 0-5 16z" fill="%s"/><g stroke="%s" stroke-width="4" stroke-linecap="round"><path d="M22 46v6M32 46v10M42 46v6"/></g>' % (CLOUD, RAIN),
    "drizzle": '<path d="M18 40h27a9 9 0 0 0 1-18 12 12 0 0 0-23 2 8 8 0 0 0-5 16z" fill="%s"/><g stroke="%s" stroke-width="4" stroke-linecap="round"><path d="M22 46v3M32 46v5M42 46v3"/></g>' % (CLOUD, RAIN),
    "snow": '<path d="M18 40h27a9 9 0 0 0 1-18 12 12 0 0 0-23 2 8 8 0 0 0-5 16z" fill="%s"/><g fill="#FFFFFF"><circle cx="22" cy="48" r="2.5"/><circle cx="32" cy="54" r="2.5"/><circle cx="42" cy="48" r="2.5"/></g>' % CLOUD,
    "storm": '<path d="M18 38h27a9 9 0 0 0 1-18 12 12 0 0 0-23 2 8 8 0 0 0-5 16z" fill="%s"/><path d="M34 34l-6 12h6l-4 10 10-14h-6l4-8z" fill="%s"/>' % (CLOUD, SUN),
}
for key, body in BIG.items():
    render('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64" fill="none">%s</svg>' % body, 96, "wxbig-%s.png" % key)

print("wrote", len(os.listdir(OUT)), "icons to", OUT)
