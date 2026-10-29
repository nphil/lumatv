#!/usr/bin/env python3
"""Renders the icons and launch splashes of LumaTV's apps from the SVG masters in assets/app-icons/.

    python3 generators/gen-app-icons.py            # needs chromium on PATH and Pillow

Each app gets icon (80 px), largeIcon (130 px, what webOS Home lists) and extraLargeIcon (240 px, what LumaTV's dock prefers: its tiles
are 120 px on a 1080p UI and 240 on the 4K panel), each rendered at its own size so strokes land on the pixel grid, plus a 1920x1080
splash: the main app's ground with the icon and the app's name set in Manrope SemiBold. The main app's own icon is assets/icon.svg;
only its extraLargeIcon is made here.

Chromium draws the SVG (feDropShadow and clip paths included); the Qt on the set has no SVG plugin, so everything ships as PNG.
"""
import os
import subprocess
import sys
import tempfile

from PIL import Image, ImageDraw, ImageFont

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SRC = os.path.join(ROOT, "assets", "app-icons")
SIZES = {"icon.png": 80, "largeIcon.png": 130, "extraLargeIcon.png": 240}

# svg master -> (output directory, splash title, splash line)
APPS = {
    "gfn": ("gfn/package", "GeForce NOW", "Cloud gaming on the video plane"),
    "card": ("card", "LumaTV Card", "A 64-bit app in its own window"),
}


def render(svg, size):
    work = tempfile.mkdtemp()
    page, shot = os.path.join(work, "a.html"), os.path.join(work, "o.png")
    with open(page, "w") as f:
        f.write('<html><body style="margin:0;background:transparent"><img src="file://%s" style="display:block;width:%dpx;height:%dpx"></body></html>'
                % (svg, size, size))
    subprocess.run(["chromium", "--headless", "--no-sandbox", "--disable-gpu", "--hide-scrollbars", "--allow-file-access-from-files",
                    "--default-background-color=00000000", "--force-device-scale-factor=1", "--window-size=1200,1200",
                    "--screenshot=" + shot, "file://" + page], check=True, capture_output=True)
    return Image.open(shot).convert("RGBA").crop((0, 0, size, size))


def splash(icon, title, line):
    # The ground is a smooth diagonal gradient: the main splash's tile and words are painted out by interpolating each row between two
    # columns clear of both, then this app's tile and words go on.
    img = Image.open(os.path.join(ROOT, "assets", "splash.png")).convert("RGB")
    w, _ = img.size
    px = img.load()
    left, right = 640, 1280
    for y in range(360, 890):
        a, b = px[left, y], px[right, y]
        for x in range(left, right):
            t = (x - left) / (right - left)
            px[x, y] = tuple(round(a[c] + (b[c] - a[c]) * t) for c in range(3))
    big = render(os.path.join(SRC, icon + ".svg"), 340)
    img.paste(big, (790, 370), big)
    draw = ImageDraw.Draw(img)
    font = os.path.join(ROOT, "payload", "fonts", "Manrope-SemiBold.ttf")
    for text, size, y, colour in ((title, 76, 740, (224, 222, 244)), (line, 34, 846, (156, 207, 216))):
        f = ImageFont.truetype(font, size)
        draw.text(((w - draw.textlength(text, font=f)) / 2, y), text, font=f, fill=colour)
    return img


def main():
    for name, (out, title, line) in APPS.items():
        svg = os.path.join(SRC, name + ".svg")
        for fname, size in SIZES.items():
            render(svg, size).save(os.path.join(ROOT, out, fname), optimize=True)
        splash(name, title, line).save(os.path.join(ROOT, out, "splash.png"), optimize=True)
        print("wrote", out)
    render(os.path.join(ROOT, "assets", "icon.svg"), 240).save(os.path.join(ROOT, "assets", "extraLargeIcon.png"), optimize=True)
    print("wrote assets/extraLargeIcon.png")


if __name__ == "__main__":
    main()
