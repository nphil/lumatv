#!/usr/bin/env python3
"""Reduces the format marks to one-colour PNGs: the volume bar's audio marks and the format badge's video marks.

    python3 generators/gen-audio-logos.py [--set audio|video] [--height 112] [--out DIR]

The audio set's sources sit in `generators/audio-logos/` (Wikimedia Commons, for this private build; the marks are their owners' trademarks):
    atmos.svg   File:Logo Dolby Atmos.svg   public domain
    dolby.svg   File:DolbyAudio.svg         CC BY-SA 4.0 (the Dolby Audio lockup: the double-D and the words)
    dts.svg     File:DTS logo Invert.svg    CC0
    dtsx.png    File:DTS X B&W.png          CC BY-SA 4.0
Each is drawn black on white, turned into an alpha mask (white where the ink was, transparent elsewhere: the shell multiplies it by the
colour it is given, so a mark follows the ink of the look), cleared of its (R) and TM signs, which are specks at the bar's size, cropped
to the ink and scaled to `--height` px tall: 4x the 28 px the bar draws it at, so a 1080p surface scaled to 4K and text at 200 % stay
crisp. Rendering of the SVGs is Chromium's, headless. `index.json` lists each key with its width and height in px.

The video set's sources sit in `generators/video-logos/`: LumaTV's own simplified marks in `currentColor`, the words as outlines of
the pinned Manrope (`generators/fonts/`), on a frame 112 units tall (caps from 20 to the baseline at 92, the double-D from 8 to 104).
They keep the frame's height, so the marks of a badge share caps and baseline, and are drawn at exactly 8x and box-filtered down, so
the caps' edges fall on whole pixels at 112, 56 and 28 px; their widths are whole multiples of 4 px, so at 56 and 28 px every column
of the result is two or four of the master's.
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
RENDER_W = 2400          # px the audio SVGs are rasterised at before they are cropped and scaled down
OVERSAMPLE = 8           # the video marks are drawn at this many px a unit of their frame

# key -> (source, boxes (left, top, right, bottom as fractions of the source) holding the (R) or TM sign to clear)
AUDIO = {
    "atmos": ("atmos.svg", [(0.972, 0.38, 1.0, 0.56), (0.86, 0.58, 0.92, 0.75)]),
    "dolby": ("dolby.svg", [(0.97, 0.0, 1.0, 0.40)]),
    "dts": ("dts.svg", [(0.88, 0.0, 1.0, 0.40)]),
    "dtsx": ("dtsx.png", []),
}
VIDEO = {key: (key + ".svg", []) for key in
         ["uhd", "fhd", "hd", "dolby-vision", "hdr10plus", "hdr10", "hlg", "hdr", "sdr", "atmos"]}
# set -> (source directory, marks, whether they keep their frame's height)
SETS = {
    "audio": (os.path.join(HERE, "audio-logos"), AUDIO, False),
    "video": (os.path.join(HERE, "video-logos"), VIDEO, True),
}


def svg_size(svg):
    """The SVG's own proportions: its viewBox, else its width and height."""
    with open(svg, "rb") as f:
        head = f.read(4000).decode("utf-8", "ignore")
    vb = re.search(r'viewBox="([\d.\-\s,]+)"', head)
    if vb:
        return tuple(float(v) for v in re.split(r"[\s,]+", vb.group(1).strip())[2:4])
    return tuple(float(re.search(r'\s%s="([\d.]+)' % k, head).group(1)) for k in ("width", "height"))


def rasterise(svg, width, height):
    """The SVG on white at `width` x `height` px, as a greyscale image."""
    with tempfile.TemporaryDirectory() as d:
        html = os.path.join(d, "p.html")
        png = os.path.join(d, "p.png")
        with open(html, "w", encoding="utf-8") as f:
            f.write('<html><body style="margin:0;background:#fff"><img src="file://%s" width="%d" height="%d" style="display:block"></body></html>'
                    % (svg, width, height))
        subprocess.run(["chromium", "--headless=new", "--no-sandbox", "--disable-gpu", "--hide-scrollbars", "--screenshot=" + png,
                        "--window-size=%d,%d" % (width, height), "file://" + html],
                       check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=120)
        return Image.open(png).convert("L").copy()


def mask_of(src, source, framed, height):
    path = os.path.join(src, source)
    if source.endswith(".svg"):
        w, h = svg_size(path)
        if framed:
            grey = rasterise(path, round(w * OVERSAMPLE * height / h), OVERSAMPLE * height)
        else:
            grey = rasterise(path, RENDER_W, round(RENDER_W * h / w))
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
    ap.add_argument("--set", choices=sorted(SETS), default="audio")
    ap.add_argument("--height", type=int, default=112)
    ap.add_argument("--out")
    a = ap.parse_args()
    src, logos, framed = SETS[a.set]
    out_dir = a.out or os.path.join(ROOT, "compositor", "assets", "ui", "icons", a.set)
    os.makedirs(out_dir, exist_ok=True)
    index = {}
    for key, (source, boxes) in logos.items():
        mask = mask_of(src, source, framed, a.height)
        clear(mask, boxes)
        box = mask.point(lambda v: 255 if v > 24 else 0).getbbox()
        if box is None:
            raise SystemExit("%s rendered empty" % key)
        if framed:
            # the ink's width, out to a multiple of 4 px of the result; the frame's whole height
            step = 4 * OVERSAMPLE
            left, right = box[0] // step * step, -(-box[2] // step) * step
            mask = mask.crop((left, 0, right, mask.height)).resize(((right - left) // OVERSAMPLE, a.height), Image.BOX)
        else:
            mask = mask.crop(box)
            width = round(mask.width * a.height / mask.height)
            mask = mask.resize((width, a.height), Image.LANCZOS)
        out = Image.new("RGBA", mask.size, (255, 255, 255, 0))
        out.putalpha(mask)
        out.save(os.path.join(out_dir, key + ".png"), optimize=True)
        index[key] = {"width": mask.width, "height": a.height}
    with open(os.path.join(out_dir, "index.json"), "w", encoding="utf-8") as f:
        json.dump(index, f, indent=1, sort_keys=True)
        f.write("\n")
    print(json.dumps(index))


if __name__ == "__main__":
    main()
