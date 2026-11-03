#!/usr/bin/env python3
"""Reduces the audio-format marks of the volume bar to one-colour PNGs for the compositor's QML.

    python3 generators/gen-audio-logos.py [--height 112] [--out compositor/assets/ui/icons/audio]

The sources sit in `generators/audio-logos/` (Wikimedia Commons, for this private build; the marks are their owners' trademarks):
    atmos.svg   File:Logo Dolby Atmos.svg   public domain
    dolby.svg   File:DolbyAudio.svg         CC BY-SA 4.0 (the Dolby Audio lockup: the double-D and the words)
    dts.svg     File:DTS logo Invert.svg    CC0
    dtsx.png    File:DTS X B&W.png          CC BY-SA 4.0
Each is drawn black on white, turned into an alpha mask (white where the ink was, transparent elsewhere: `kit/LuLogo.qml` multiplies it by the
colour it is given, as LuIcon does, so a mark follows the bar's ink in both materials), cleared of its (R) and TM signs, which are specks at the
bar's size, cropped to the ink and scaled to `--height` px tall: 4x the 28 px the bar draws it at, so a 1080p surface scaled to 4K and text at
200 % stay crisp. Rendering of the SVGs is Chromium's, headless. `index.json` lists each key with its width and height in px.
"""
import argparse
import json
import os
import re
import subprocess
import tempfile

from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
SRC = os.path.join(HERE, "audio-logos")
RENDER_W = 2400          # px the SVGs are rasterised at before they are cropped and scaled down

# key -> (source, boxes (left, top, right, bottom as fractions of the source) holding the (R) or TM sign to clear)
LOGOS = {
    "atmos": ("atmos.svg", [(0.972, 0.38, 1.0, 0.56), (0.86, 0.58, 0.92, 0.75)]),
    "dolby": ("dolby.svg", [(0.97, 0.0, 1.0, 0.40)]),
    "dts": ("dts.svg", [(0.88, 0.0, 1.0, 0.40)]),
    "dtsx": ("dtsx.png", []),
}


def rasterise(svg, path):
    """The SVG on white at RENDER_W px wide, as a greyscale image."""
    with tempfile.TemporaryDirectory() as d:
        html = os.path.join(d, "p.html")
        png = os.path.join(d, "p.png")
        with open(svg, "rb") as f:
            head = f.read(4000).decode("utf-8", "ignore")
        # the SVG's own proportions: its viewBox, else its width and height
        vb = re.search(r'viewBox="([\d.\-\s,]+)"', head)
        if vb:
            w, h = (float(v) for v in re.split(r"[\s,]+", vb.group(1).strip())[2:4])
        else:
            w, h = (float(re.search(r'\s%s="([\d.]+)' % k, head).group(1)) for k in ("width", "height"))
        height = round(RENDER_W * h / w)
        with open(html, "w", encoding="utf-8") as f:
            f.write('<html><body style="margin:0;background:#fff"><img src="file://%s" width="%d" height="%d" style="display:block"></body></html>'
                    % (svg, RENDER_W, height))
        subprocess.run(["chromium", "--headless=new", "--no-sandbox", "--disable-gpu", "--hide-scrollbars", "--screenshot=" + png,
                        "--window-size=%d,%d" % (RENDER_W, height), "file://" + html],
                       check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=120)
        return Image.open(png).convert("L").copy()


def mask_of(source):
    path = os.path.join(SRC, source)
    if source.endswith(".svg"):
        grey = rasterise(path, source)
    else:
        rgba = Image.open(path).convert("RGBA")
        white = Image.new("RGBA", rgba.size, (255, 255, 255, 255))
        grey = Image.alpha_composite(white, rgba).convert("L")
    return grey.point(lambda v: 255 - v)       # black ink becomes opaque, white ground transparent


def clear(mask, boxes):
    w, h = mask.size
    for l, t, r, b in boxes:
        mask.paste(0, (int(l * w), int(t * h), int(r * w), int(b * h)))


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--height", type=int, default=112)
    ap.add_argument("--out", default=os.path.join(ROOT, "compositor", "assets", "ui", "icons", "audio"))
    a = ap.parse_args()
    os.makedirs(a.out, exist_ok=True)
    index = {}
    for key, (source, boxes) in LOGOS.items():
        mask = mask_of(source)
        clear(mask, boxes)
        box = mask.point(lambda v: 255 if v > 24 else 0).getbbox()
        if box is None:
            raise SystemExit("%s rendered empty" % key)
        mask = mask.crop(box)
        width = round(mask.width * a.height / mask.height)
        mask = mask.resize((width, a.height), Image.LANCZOS)
        out = Image.new("RGBA", mask.size, (255, 255, 255, 0))
        out.putalpha(mask)
        out.save(os.path.join(a.out, key + ".png"), optimize=True)
        index[key] = {"width": width, "height": a.height}
    with open(os.path.join(a.out, "index.json"), "w", encoding="utf-8") as f:
        json.dump(index, f, indent=1, sort_keys=True)
        f.write("\n")
    print(json.dumps(index))


if __name__ == "__main__":
    main()
