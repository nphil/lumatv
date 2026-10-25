#!/usr/bin/env python3
"""Makes card/splash.png from the main app's splash (assets/splash.png): the same ground and tile, the words changed.

    python3 card/gen-splash.py

The ground is a smooth diagonal gradient, so the old words are painted out by interpolating each row between two clean columns, and the
new ones are set in Manrope SemiBold (payload/fonts) in the Lucent ink and foam colours.
"""
import os

from PIL import Image, ImageDraw, ImageFont

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
img = Image.open(os.path.join(ROOT, "assets", "splash.png")).convert("RGB")
w, h = img.size
px = img.load()
LEFT, RIGHT, TOP, BOTTOM = 640, 1280, 720, 890
for y in range(TOP, BOTTOM):
    a, b = px[LEFT, y], px[RIGHT, y]
    for x in range(LEFT, RIGHT):
        t = (x - LEFT) / (RIGHT - LEFT)
        px[x, y] = tuple(round(a[c] + (b[c] - a[c]) * t) for c in range(3))
draw = ImageDraw.Draw(img)
font = os.path.join(ROOT, "payload", "fonts", "Manrope-SemiBold.ttf")
for text, size, y, colour in (("LumaTV Card", 76, 740, (224, 222, 244)), ("A 64-bit app in its own window", 34, 846, (156, 207, 216))):
    f = ImageFont.truetype(font, size)
    width = draw.textlength(text, font=f)
    draw.text(((w - width) / 2, y), text, font=f, fill=colour)
img.save(os.path.join(ROOT, "card", "splash.png"), optimize=True)
print("wrote card/splash.png", img.size)
