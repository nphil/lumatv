#!/usr/bin/env python3
"""Renders the icons and launch splashes of LumaTV's apps, and the Marquee icon master.

    python3 generators/gen-app-icons.py [--marquee-master PATH]     # needs cairosvg and Pillow

Each app has its own hue and one bold white glyph; the finish is shared so the set reads as a family: a full-bleed opaque square
(Home rounds the corners), the same top-left gloss and bottom shade, the glyph inside the central 60 % over a soft shadow.

    LumaTV (assets/)            two glass panes, on iris violet
    LumaTV Card (card/)         a window card with a second one behind, on teal
    LumaTV Frame (frame/)       a picture frame, on sunrise orange
    GeForce NOW (gfn/package/)  the GeForce eye in NVIDIA green, on graphite
    Marquee (com.beb.plxnative) a play triangle in a ring of marquee bulbs, on magenta; lives in the Marquee repo, which cuts its
                                sizes from the master this writes (tools/mkicons.py --bleed)

Every app gets icon (80 px), largeIcon (130 px, what webOS Home lists) and extraLargeIcon (240 px, what LumaTV's dock prefers: its
tiles are 120 px on a 1080p UI and 240 on the 4K panel), each rendered at its own size so strokes land on the pixel grid; the main
app also gets icon160 (the webosbrew listing and the notification tests). Card and GeForce NOW get a 1920x1080 splash too: the main
splash's ground with the icon and the app's name set in Manrope SemiBold.

iconColor in each appinfo.json is the largeIcon's pixel (1,1), printed here; Home paints it behind the icon. Pure black is lifted to
#060606, because the compositor keys #000 as Home's scaffold. The Qt on the set has no SVG plugin, so everything ships as PNG.
"""
import io
import os
import sys

import cairosvg
from PIL import Image, ImageDraw, ImageFilter, ImageFont

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SIZES = (("icon.png", 80), ("largeIcon.png", 130), ("extraLargeIcon.png", 240))
# app -> (splash title, splash line); only the apps with a splash of their own
SPLASH = {
    "gfn": ("GeForce NOW", "Cloud gaming on the video plane"),
    "card": ("LumaTV Card", "A 64-bit app in its own window"),
}

V = 1024  # design canvas

GFN_GREEN = "#76b900"


def glyph_lumatv():
    # Two glass panes, the back one tilted: the shell's layered glass.
    return f"""
<g transform="rotate(-13 400 440)"><rect x="215" y="245" width="370" height="370" rx="104" fill="#fff" fill-opacity=".40"/></g>
<defs><linearGradient id="pane" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#fff"/><stop offset="1" stop-color="#e4d9ff"/></linearGradient></defs>
<rect x="360" y="360" width="430" height="430" rx="118" fill="url(#pane)"/>
<path d="M410 452 q0 -42 42 -42 h118" fill="none" stroke="#8f6bff" stroke-opacity=".35" stroke-width="30" stroke-linecap="round"/>
"""


def glyph_card():
    # A window card (title bar, three dots) with a second card peeking out behind.
    return """
<rect x="300" y="222" width="424" height="150" rx="64" fill="#fff" fill-opacity=".42"/>
<rect x="212" y="290" width="600" height="470" rx="84" fill="#fff"/>
<circle cx="296" cy="372" r="27" fill="#0f7e88"/>
<circle cx="372" cy="372" r="27" fill="#0f7e88" fill-opacity=".62"/>
<circle cx="448" cy="372" r="27" fill="#0f7e88" fill-opacity=".34"/>
<rect x="276" y="462" width="472" height="52" rx="26" fill="#0f7e88" fill-opacity=".30"/>
<rect x="276" y="556" width="330" height="52" rx="26" fill="#0f7e88" fill-opacity=".30"/>
<rect x="276" y="650" width="400" height="52" rx="26" fill="#0f7e88" fill-opacity=".30"/>
"""


def glyph_frame():
    # A picture frame with a sun and two hills.
    return """
<defs><clipPath id="in"><rect x="298" y="338" width="428" height="348" rx="26"/></clipPath></defs>
<rect x="226" y="266" width="572" height="492" rx="92" fill="none" stroke="#fff" stroke-width="64"/>
<g clip-path="url(#in)" fill="#fff">
<circle cx="618" cy="448" r="50"/>
<path d="M240 720 L402 540 L560 720 Z" stroke="#fff" stroke-width="30" stroke-linejoin="round"/>
<path d="M470 720 L592 590 L760 720 Z" stroke="#fff" stroke-width="30" stroke-linejoin="round" fill-opacity=".8" stroke-opacity=".8"/>
</g>
"""


def glyph_gfn():
    # The GeForce eye, adapted: heavy almond lid, iris with a swirl arc, pupil.
    return f"""
<path d="M200 512 C 330 326 694 326 824 512 C 694 698 330 698 200 512 Z" fill="{GFN_GREEN}"/>
<path d="M286 512 C 390 410 634 410 738 512 C 634 614 390 614 286 512 Z" fill="#161a14"/>
<circle cx="512" cy="512" r="132" fill="{GFN_GREEN}"/>
<path d="M418 548 A 98 98 0 0 1 512 414 A 98 98 0 0 1 606 480" fill="none" stroke="#161a14" stroke-width="26" stroke-linecap="round"/>
<circle cx="512" cy="512" r="52" fill="#161a14"/>
"""


def glyph_marquee():
    # A marquee sign: a plate of bulbs, corner bulbs included, round a play triangle.
    x0, x1, y0, y1 = 192, 832, 262, 702
    xs = [x0 + (x1 - x0) * i / 6 for i in range(7)]
    ys = [y0 + (y1 - y0) * i / 4 for i in range(5)]
    pts = [(x, y0) for x in xs] + [(x, y1) for x in xs] + [(x0, y) for y in ys[1:4]] + [(x1, y) for y in ys[1:4]]
    bulbs = "".join(
        f'<circle cx="{px:.1f}" cy="{py:.1f}" r="50" fill="#ffe7a8" fill-opacity=".30"/>'
        f'<circle cx="{px:.1f}" cy="{py:.1f}" r="34" fill="#fff6dc"/>' for px, py in pts)
    return f"""
<rect x="{x0}" y="{y0}" width="{x1 - x0}" height="{y1 - y0}" rx="44" fill="#fff" fill-opacity=".10" stroke="#fff" stroke-opacity=".30" stroke-width="16"/>
{bulbs}
<path d="M452 388 L452 576 L632 482 Z" fill="#fff" stroke="#fff" stroke-width="64" stroke-linejoin="round"/>
"""


# name -> (gradient top-left, bottom-right, glyph, shadow colour)
APPS = {
    "lumatv": ("#9a78ff", "#3a2a9c", glyph_lumatv, "#1d1160"),
    "card": ("#34d8c8", "#0b6674", glyph_card, "#053f49"),
    "frame": ("#ffc94f", "#ee5f29", glyph_frame, "#8f2c0a"),
    "gfn": ("#3a4a2c", "#0e140a", glyph_gfn, "#080a08"),
    "marquee": ("#ff5f9a", "#9c1a5a", glyph_marquee, "#5a0a30"),
}
# optical size correction: the panes and the eye cover less of the tile than a frame or a card
SCALE = {"lumatv": 1.10, "gfn": 1.08}
OUT = {"lumatv": "assets", "card": "card", "frame": "frame", "gfn": "gfn/package"}


def bg_svg(c1, c2):
    return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {V} {V}">
<defs>
<linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="{c1}"/><stop offset="1" stop-color="{c2}"/></linearGradient>
<radialGradient id="gloss" cx=".22" cy=".12" r=".75"><stop offset="0" stop-color="#fff" stop-opacity=".26"/><stop offset="1" stop-color="#fff" stop-opacity="0"/></radialGradient>
<linearGradient id="shade" x1="0" y1=".62" x2="0" y2="1"><stop offset="0" stop-color="#000" stop-opacity="0"/><stop offset="1" stop-color="#000" stop-opacity=".18"/></linearGradient>
</defs>
<rect width="{V}" height="{V}" fill="url(#g)"/><rect width="{V}" height="{V}" fill="url(#gloss)"/><rect width="{V}" height="{V}" fill="url(#shade)"/>
</svg>"""


def glyph_svg(body):
    return f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {V} {V}">{body}</svg>'


def raster(svg, n):
    png = cairosvg.svg2png(bytestring=svg.encode(), output_width=n, output_height=n)
    return Image.open(io.BytesIO(png)).convert("RGBA")


def tile(name, n):
    c1, c2, glyph, shadow = APPS[name]
    im = raster(bg_svg(c1, c2), n)
    k = SCALE.get(name, 1.0)
    body = f'<g transform="translate(512 512) scale({k}) translate(-512 -512)">{glyph()}</g>'
    g = raster(glyph_svg(body), n)
    # shadow: the glyph's alpha, blurred and dropped, in a dark shade of the tile's own hue
    a = g.getchannel("A")
    blur = max(1, round(n * 0.022))
    drop = round(n * 0.020)
    sh = a.filter(ImageFilter.GaussianBlur(blur)).point(lambda v: int(v * 0.55))
    sh_img = Image.new("RGBA", (n, n), shadow + "ff")
    sh_img.putalpha(sh)
    im.alpha_composite(sh_img, (0, drop))
    im.alpha_composite(g)
    rgb = im.convert("RGB")
    px = rgb.load()
    for y in range(n):
        for x in range(n):
            if px[x, y] == (0, 0, 0):
                px[x, y] = (6, 6, 6)
    return rgb


def hexpix(im):
    return "#%02x%02x%02x" % im.getpixel((1, 1))


def rounded(im, size, radius):
    """The tile with Home's corners, for the splash."""
    im = im.convert("RGBA").resize((size, size), Image.LANCZOS)
    s = 4
    m = Image.new("L", (size * s, size * s), 0)
    ImageDraw.Draw(m).rounded_rectangle([0, 0, size * s - 1, size * s - 1], radius * s, fill=255)
    im.putalpha(m.resize((size, size), Image.LANCZOS))
    return im


def splash(name):
    # The ground is a smooth diagonal gradient: the main splash's tile and words are painted out by interpolating each row between two
    # columns clear of both, then this app's tile and words go on.
    title, line = SPLASH[name]
    img = Image.open(os.path.join(ROOT, "assets", "splash.png")).convert("RGB")
    w, _ = img.size
    px = img.load()
    left, right = 640, 1280
    for y in range(360, 890):
        a, b = px[left, y], px[right, y]
        for x in range(left, right):
            t = (x - left) / (right - left)
            px[x, y] = tuple(round(a[c] + (b[c] - a[c]) * t) for c in range(3))
    big = rounded(tile(name, 340), 340, 340 * 22 / 120)
    img.paste(big, (790, 370), big)
    draw = ImageDraw.Draw(img)
    font = os.path.join(ROOT, "payload", "fonts", "Manrope-SemiBold.ttf")
    for text, size, y, colour in ((title, 76, 740, (224, 222, 244)), (line, 34, 846, (156, 207, 216))):
        f = ImageFont.truetype(font, size)
        draw.text(((w - draw.textlength(text, font=f)) / 2, y), text, font=f, fill=colour)
    return img


def main():
    for name, out in OUT.items():
        sizes = SIZES + ((("icon160.png", 160),) if name == "lumatv" else ())
        for fname, n in sizes:
            im = tile(name, n)
            im.save(os.path.join(ROOT, out, fname), optimize=True)
            if fname == "largeIcon.png":
                print(f"{name:8s} iconColor {hexpix(im)}  -> {out}")
        if name in SPLASH:
            splash(name).save(os.path.join(ROOT, out, "splash.png"), optimize=True)
    if "--marquee-master" in sys.argv:
        dest = sys.argv[sys.argv.index("--marquee-master") + 1]
        tile("marquee", 1254).save(dest, optimize=True)
        print(f"marquee  master {dest}  iconColor {hexpix(tile('marquee', 130))}")


if __name__ == "__main__":
    main()
