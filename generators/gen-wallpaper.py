#!/usr/bin/env python3
"""Regenerate the Luma wallpaper set.

Runs on a workstation, not on the TV (the TV has no Python imaging stack).

Outputs four files into --out:

  wall_1080.png   1920x1080  the actual artwork, painted by the compositor
  bg_hd.png       1264x580   pure black Home hero-banner tier
  bg_2k.png       1920x880   pure black Home hero-banner tier
  bg_4k.png       3840x1760  pure black Home hero-banner tier

The three bg_* tiers are deliberately black: Luma blanks the Home app's
own hero banner so that the compositor-painted wallpaper shows through it.
Only wall_1080.png carries image content.

Pipeline, in the order that matters:

  1. cover-fit the source to exactly 1920x1080 (crop, never letterbox)
  2. lift the floor so near-black does not crush on an OLED panel
  3. apply a triangular-PDF blue-noise dither LAST

Step 3 is not optional. The graphics plane on this SoC is 8-bit ("abgr8888"),
and a low-saturation plum-to-plum gradient spanning ~20 levels over 1000 px
quantises into 50-px bands that an OLED renders perfectly, and therefore
visibly. Dithering before any later resample would let that resample average
the noise away, so it is applied to the final pixels only.
"""

import argparse
import os
import sys

import numpy as np
from PIL import Image

MASTER = (1920, 1080)
TIERS = {"bg_hd.png": (1264, 580), "bg_2k.png": (1920, 880), "bg_4k.png": (3840, 1760)}

# Rose Pine, the palette the rest of the mod is built from.
BASE = "#191724"
SURFACE = "#1f1d2e"
IRIS = "#c4a7e7"
LOVE = "#eb6f92"
PINE = "#31748f"
FOAM = "#9ccfd8"

FLOOR = 44.0 / 255.0  # black point; below this an OLED crushes detail


def hex_to_rgb(h):
    h = h.lstrip("#")
    return np.array([int(h[i : i + 2], 16) for i in (0, 2, 4)], dtype=np.float32) / 255.0


def cover_fit(im, size):
    """Scale to fully cover `size`, then centre-crop. Never distorts, never letterboxes."""
    tw, th = size
    sw, sh = im.size
    scale = max(tw / sw, th / sh)
    nw, nh = max(tw, int(round(sw * scale))), max(th, int(round(sh * scale)))
    im = im.resize((nw, nh), Image.LANCZOS)
    left, top = (nw - tw) // 2, (nh - th) // 2
    return im.crop((left, top, left + tw, top + th))


def procedural(size):
    """Built-in abstract gradient: soft diagonal ribbons in the Rose Pine palette."""
    w, h = size
    yy, xx = np.mgrid[0:h, 0:w].astype(np.float32)
    u, v = xx / w, yy / h

    base, surface = hex_to_rgb(BASE), hex_to_rgb(SURFACE)
    out = base[None, None, :] + (surface - base)[None, None, :] * v[..., None]

    def ribbon(colour, cx, cy, rot, width, amp):
        """A soft, flowing band of colour; `rot` shears it, `amp` waves it."""
        d = (u - cx) * rot + (v - cy) + amp * np.sin((u + cy) * np.pi * 2.0)
        falloff = np.exp(-(d * d) / (2.0 * width * width))
        return hex_to_rgb(colour)[None, None, :] * falloff[..., None]

    out += ribbon(IRIS, 0.55, 0.30, 0.75, 0.115, 0.10) * 0.85
    out += ribbon(LOVE, 0.35, 0.58, -0.55, 0.095, 0.13) * 0.60
    out += ribbon(PINE, 0.70, 0.72, 0.40, 0.130, 0.08) * 0.55
    out += ribbon(FOAM, 0.85, 0.88, -0.30, 0.070, 0.06) * 0.35

    # Vignette the corners slightly so the rail and dock read against it.
    r = np.sqrt((u - 0.5) ** 2 + (v - 0.5) ** 2)
    out *= (1.0 - 0.22 * np.clip(r / 0.78, 0.0, 1.0))[..., None]

    return np.clip(out, 0.0, 1.0)


def lift_floor(arr, floor=FLOOR):
    """Compress the range upward so the darkest pixel sits at `floor`, not at 0."""
    return floor + arr * (1.0 - floor)


def dither(arr, amp):
    """Triangular-PDF noise at +/- `amp` LSB, applied immediately before quantisation.

    Triangular (sum of two uniforms) rather than uniform: it decorrelates the
    quantisation error across the whole ramp instead of only at the step edges,
    which is what turns a hard 1-level staircase into grain.
    """
    rng = np.random.default_rng(7)  # fixed seed: regeneration is reproducible
    tri = rng.random(arr.shape, dtype=np.float32) - rng.random(arr.shape, dtype=np.float32)
    return np.clip(arr * 255.0 + tri * amp, 0.0, 255.0)


def main():
    ap = argparse.ArgumentParser(description="Regenerate the Luma wallpaper set.")
    ap.add_argument("--source", help="source image; omit to use the built-in procedural gradient")
    ap.add_argument("--out", default=".", help="output directory")
    ap.add_argument("--grain", type=float, default=1.5, help="dither amplitude in LSB (default 1.5)")
    ap.add_argument("--no-lift", action="store_true", help="skip the black-point lift")
    args = ap.parse_args()

    os.makedirs(args.out, exist_ok=True)

    if args.source:
        if not os.path.isfile(args.source):
            sys.exit(f"source not found: {args.source}")
        arr = np.asarray(cover_fit(Image.open(args.source).convert("RGB"), MASTER), dtype=np.float32) / 255.0
    else:
        arr = procedural(MASTER)

    if not args.no_lift:
        arr = lift_floor(arr)

    master = Image.fromarray(dither(arr, args.grain).round().astype(np.uint8), "RGB")
    master_path = os.path.join(args.out, "wall_1080.png")
    master.save(master_path, optimize=True)
    print(f"wall_1080.png  {master.size[0]}x{master.size[1]}  darkest={np.asarray(master).min()}")

    for name, size in TIERS.items():
        path = os.path.join(args.out, name)
        Image.new("RGB", size, (0, 0, 0)).save(path, optimize=True)
        print(f"{name:14s} {size[0]}x{size[1]}  black")


if __name__ == "__main__":
    main()
