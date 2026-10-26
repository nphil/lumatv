#!/usr/bin/env python3
"""The pictures of the TV slot (docs/research/hwjpeg-result.md): photo-like JPEGs (smooth colour under fine noise, so the files are
about 2.5 bits a pixel, a photograph's range) in the sizes the slot times, with an odd one that tells the buffer's alignment.

    jpeg/make_samples.py DIR

Writes DIR/1080.jpg (1920x1080 4:2:0), DIR/4k.jpg (3840x2160 4:2:0), DIR/odd.jpg (1003x707 4:2:0, no side a multiple of 16),
DIR/4k444.jpg (3840x2160 4:4:4), DIR/1080p.jpg (1920x1080 progressive).
"""
import os
import sys

from PIL import Image, ImageChops


def photo(w, h):
    r = Image.linear_gradient("L").rotate(90).resize((w, h))
    g = Image.linear_gradient("L").resize((w, h))
    b = Image.radial_gradient("L").resize((w, h))
    img = Image.merge("RGB", (r, g, b))
    noise = Image.effect_noise((w, h), 24).point(lambda v: v // 3)
    return ImageChops.add(img, Image.merge("RGB", (noise, noise, noise)), 1.0, -40)


def main():
    if len(sys.argv) != 2:
        print(__doc__)
        return 64
    out = sys.argv[1]
    os.makedirs(out, exist_ok=True)
    cases = (("1080.jpg", 1920, 1080, 2, False), ("4k.jpg", 3840, 2160, 2, False), ("odd.jpg", 1003, 707, 2, False),
             ("4k444.jpg", 3840, 2160, 0, False), ("1080p.jpg", 1920, 1080, 2, True))
    for name, w, h, sub, prog in cases:
        path = os.path.join(out, name)
        photo(w, h).save(path, "JPEG", quality=90, subsampling=sub, progressive=prog, optimize=True)
        print("%s %dx%d %d bytes" % (path, w, h, os.path.getsize(path)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
