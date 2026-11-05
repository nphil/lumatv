#!/usr/bin/env python3
"""Renders Home's top status bar on the host harness (`lumatv-ui --host`), full width with the top of the cards under it, dark and light, and with
the focus on each button in turn, into DIR as `statusbar-<tag>-<mode>[-<item>].png`; with `--crisp` also the zoomed crops that show whether its
glyphs, text and pill edges are on whole pixels, as `crisp-<tag>-<mode>-<text size>-<part>.png`.

    python3 tests/render_rust_statusbar.py [--out DIR] [--tag after] [--bin PATH] [--modes dark,light] [--focus] [--crisp] [--scales 1,2]

`--tag before --bin PATH` renders another build (the one the redesign started from) the same way, so the two can be set beside each other.
`--focus` adds a picture for each button with the D-pad's focus on it (Right from the first, as a person walks the bar).
`--crisp` crops the four glyphs, the two ends of the pill, the profile chip and a focused button from the page as the shell draws it (1920 by 1080,
before the display doubles it), each magnified with no smoothing: one page pixel is a block of pixels, so a grey ramp where a stroke or an edge should be
a step shows. It also prints each glyph stroke's edge rise (the width between 10 % and 90 % of an edge, in page pixels; 1.0 is a step).
"""
import argparse
import concurrent.futures
import json
import os
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, '..'))
sys.path.insert(0, HERE)
import rust_home_host as H  # noqa: E402

OUT = '/data/home/tmp/camera-popup'
CLOCK = '2026-11-04T20:05'
#: the band of the screen a picture keeps: the status bar's row and the first row of the cards
BAND = (0, 0, 1920, 340)
#: what the host's Home is made to hold still for: no wallpaper motion, no weather sprites
STILL = {"wallpaperMotion": {"enabled": False}, "weather": {"animated": False}}
#: the old bar's buttons (bell, gear, search) at 100 % text, for a build that does not report where its buttons are
OLD_ITEMS = [["notifications", 1694, 60, 48, 48], ["settings", 1748, 60, 48, 48], ["search", 1802, 60, 48, 48]]
OLD_PILL = [64, 54, 1792, 60]


def crop(src, dst):
    from PIL import Image
    im = Image.open(src).convert('RGB')
    im.crop((BAND[0], BAND[1], BAND[0] + BAND[2], BAND[1] + BAND[3])).save(dst)


def render(binary, mode, shots, tmp, scale=None):
    """`shots` is [(key presses before it, name)]: Home at rest, then each shot after its keys. Returns ([(name, page picture)], the bar's geometry)."""
    H.BIN = binary
    steps, names, t = [], [], 7500
    for presses, name in shots:
        if presses:
            steps.append(H.RustHome.k(t - 900, *presses))
        path = os.path.join(tmp, '%s-%s-%s.png' % (mode, scale or 1, name))
        steps.append(['--snap', '%d:%s' % (t, path)])
        names.append((name, path))
        t += 1500
    steps.append(['--eval', '7400:home'])
    # the material is in the theme file as well: Home reads that file again after it starts, and `--theme` alone does not outlast it
    for attempt in range(3):
        out, evals, _ = H.RustHome().host(t + 500, steps, mode=mode, scale=scale, theme=dict(STILL, material=mode), extra=['--clock', CLOCK])
        # a picture taken before Home had drawn (a host under load) is black: the run is made again
        if all(os.path.exists(p) and mean_of(p) > 5 for _, p in names):
            break
    else:
        raise RuntimeError('no picture for %s: %s' % ([n for n, p in names if not os.path.exists(p) or mean_of(p) <= 5], out[-1500:]))
    bar = (evals[0] if evals else {}).get('statusBar')
    return names, bar


def mean_of(path):
    from PIL import Image, ImageStat
    return ImageStat.Stat(Image.open(path).convert('L')).mean[0]


def zoomed(src, box, dst, zoom):
    """`box` of the page picture at `src` magnified `zoom` times with no smoothing."""
    from PIL import Image
    im = Image.open(src).convert('RGB')
    x0, y0, x1, y1 = [int(v) for v in box]
    im.crop((x0, y0, x1, y1)).resize(((x1 - x0) * zoom, (y1 - y0) * zoom), Image.NEAREST).save(dst)


def edge_rises(src, box):
    """The 10 to 90 % rise, in page pixels, of the glyph strokes crossed by the rows of `box`: for each row the ink is the stroke's brightness against the
    button's, and every edge (a step from the button to the ink or back) is measured; returns the median."""
    from PIL import Image
    im = Image.open(src).convert('L')
    x0, y0, x1, y1 = [int(v) for v in box]
    rises = []
    for y in range(y0, y1):
        row = [im.getpixel((x, y)) for x in range(x0, x1)]
        lo, hi = min(row), max(row)
        if hi - lo < 60:
            continue
        norm = [(v - lo) / float(hi - lo) for v in row]
        for i in range(1, len(norm) - 1):
            a, b = norm[i - 1], norm[i + 1]
            # a monotone run from below 0.1 to above 0.9: its length in pixels is the rise
            if (a < 0.1 and b > 0.9) or (a > 0.9 and b < 0.1):
                j, k = i, i
                while j > 0 and 0.1 < norm[j - 1] < 0.9:
                    j -= 1
                while k < len(norm) - 1 and 0.1 < norm[k + 1] < 0.9:
                    k += 1
                rises.append(1.0 + (k - j) * 0 + sum(1 for v in norm[j:k + 1] if 0.1 < v < 0.9) * 0.5)
    rises.sort()
    return rises[len(rises) // 2] if rises else None


def crisp_crops(path, bar, out_dir, tag, mode, scale):
    """The crops of one page picture. `bar` is the shell's report of where the pill and the buttons are."""
    zoom = 4 if (scale or 1) < 1.5 else 2
    pct = int(round((scale or 1) * 100))
    pill = bar['pill']
    items = bar['items']
    glyphs = [it for it in items if it[0] != 'profile']
    names = []
    y0, y1 = pill[1] - 8, pill[1] + pill[3] + 8
    if glyphs:
        box = (glyphs[0][1] - 10, y0, glyphs[-1][1] + glyphs[-1][3] + 10, y1)
        dst = os.path.join(out_dir, 'crisp-%s-%s-%d-icons.png' % (tag, mode, pct))
        zoomed(path, box, dst, zoom)
        names.append(dst)
    # the two ends of the pill, side by side
    from PIL import Image
    left_box = (pill[0] - 12, y0, pill[0] + 72, y1)
    right_box = (pill[0] + pill[2] - 150, y0, pill[0] + pill[2] + 12, y1)
    a = os.path.join(out_dir, '.edge-a.png')
    b = os.path.join(out_dir, '.edge-b.png')
    zoomed(path, left_box, a, zoom)
    zoomed(path, right_box, b, zoom)
    ia, ib = Image.open(a), Image.open(b)
    sheet = Image.new('RGB', (ia.width + ib.width + 16, max(ia.height, ib.height)), (20, 20, 24))
    sheet.paste(ia, (0, 0))
    sheet.paste(ib, (ia.width + 16, 0))
    dst = os.path.join(out_dir, 'crisp-%s-%s-%d-pill-ends.png' % (tag, mode, pct))
    sheet.save(dst)
    os.remove(a)
    os.remove(b)
    names.append(dst)
    chip = [it for it in items if it[0] == 'profile']
    if chip:
        c = chip[0]
        dst = os.path.join(out_dir, 'crisp-%s-%s-%d-chip.png' % (tag, mode, pct))
        zoomed(path, (c[1] - 8, y0, c[1] + c[3] + 8, y1), dst, zoom)
        names.append(dst)
    rises = {}
    for it in glyphs:
        rises[it[0]] = edge_rises(path, (it[1] + 4, it[2] + 4, it[1] + it[3] - 4, it[2] + it[4] - 4))
    return names, rises


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    ap.add_argument('--out', default=OUT)
    ap.add_argument('--tag', default='after')
    ap.add_argument('--bin', default=H.BIN)
    ap.add_argument('--modes', default='dark,light')
    ap.add_argument('--focus', action='store_true', help='also one picture per button with the focus on it')
    ap.add_argument('--items', type=int, default=5, help='how many buttons the bar has, for --focus')
    ap.add_argument('--crisp', action='store_true', help='also the zoomed crops of the glyphs, the pill ends and the chip')
    ap.add_argument('--scales', default='1', help='text sizes, for --crisp: 1,2')
    a = ap.parse_args(argv)
    os.makedirs(a.out, exist_ok=True)
    tmp = tempfile.mkdtemp(prefix='statusbar-')
    shots = [((), 'rest')]
    if a.focus:
        # the focus starts on the first button (reading order); each Right is one step along the bar
        shots += [(('right',), 'focus-%d' % (i + 1)) for i in range(a.items - 1)]
    if a.crisp:
        shots += [(('right',), 'focus-camera')] if not a.focus else []
    modes = a.modes.split(',')
    scales = [float(s) for s in a.scales.split(',')] if a.crisp else [None]
    jobs = {}
    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
        for m in modes:
            for sc in scales:
                jobs[(m, sc)] = pool.submit(render, a.bin, m, shots, tmp, None if sc in (None, 1.0) else sc)
        for (m, sc), job in jobs.items():
            names, bar = job.result()
            for name, path in names:
                if sc in (None, 1.0):
                    dst = os.path.join(a.out, 'statusbar-%s-%s%s.png' % (a.tag, m, '' if name == 'rest' else '-' + name))
                    crop(path, dst)
                    print(dst)
            if a.crisp:
                bar = bar or {'pill': OLD_PILL, 'items': OLD_ITEMS}
                page = dict(names)['rest']
                made, rises = crisp_crops(page, bar, a.out, a.tag, m, sc)
                for n in made:
                    print(n)
                print('edge rise of the glyph strokes (page pixels, median; 1.0 is a step), %s %d%%: %s' % (m, int(round((sc or 1) * 100)), json.dumps({k: v for k, v in rises.items()})))
                focus = dict(names).get('focus-camera') or dict(names).get('focus-1')
                if focus:
                    cam = [it for it in bar['items'] if it[0] == 'cameras']
                    if cam:
                        c = cam[0]
                        dst = os.path.join(a.out, 'crisp-%s-%s-%d-focus.png' % (a.tag, m, int(round((sc or 1) * 100))))
                        zoomed(focus, (c[1] - 12, c[2] - 12, c[1] + c[3] + 12, c[2] + c[4] + 12), dst, 4 if (sc or 1) < 1.5 else 2)
                        print(dst)
    return 0


if __name__ == '__main__':
    sys.exit(main())
