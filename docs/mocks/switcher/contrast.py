#!/usr/bin/env python3
"""Text contrast of the switcher mocks, measured on the finished pictures (Lucent section 3: the background is the median of the text box, the ink
is the pixel farthest from it; body text needs 4.5:1). Run `node render.mjs` first (it writes the text boxes to the temp directory).
Usage: python3 contrast.py [name ...]   Exit status 1 when any text is under 4.5:1."""
import json
import os
import sys
import tempfile
from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
BOXES = os.path.join(tempfile.gettempdir(), 'switcher-boxes')
MIN = 4.5


def lin(c):
    c = c / 255.0
    return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4


def lum(p):
    return 0.2126 * lin(p[0]) + 0.7152 * lin(p[1]) + 0.0722 * lin(p[2])


def ratio(a, b):
    hi, lo = max(lum(a), lum(b)), min(lum(a), lum(b))
    return (hi + 0.05) / (lo + 0.05)


def measure(img, box):
    x0, y0 = max(0, int(box['x'])), max(0, int(box['y']))
    x1, y1 = min(img.width, int(box['x'] + box['w']) + 1), min(img.height, int(box['y'] + box['h']) + 1)
    if x1 <= x0 or y1 <= y0:
        return None
    px = list(img.crop((x0, y0, x1, y1)).convert('RGB').get_flattened_data())
    bg = tuple(sorted(c)[len(px) // 2] for c in zip(*px))
    ink = max(px, key=lambda p: abs(lum(p) - lum(bg)))
    return ratio(ink, bg)


def main(names):
    bad = 0
    files = sorted(f[:-5] for f in os.listdir(BOXES) if f.endswith('.json')) if os.path.isdir(BOXES) else []
    for name in files:
        if names and not any(name.startswith(n) for n in names):
            continue
        if name.startswith('shrink'):
            continue
        img = Image.open(os.path.join(HERE, name + '.png'))
        worst = (99, '')
        for b in json.load(open(os.path.join(BOXES, name + '.json'))):
            r = measure(img, b)
            if r is not None and r < worst[0]:
                worst = (r, b['t'])
        flag = 'ok ' if worst[0] >= MIN else 'LOW'
        bad += worst[0] < MIN
        print(f'{flag} {name:22s} weakest text {worst[0]:5.2f}:1  "{worst[1]}"')
    return 1 if bad else 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
