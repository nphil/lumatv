#!/usr/bin/env python3
"""Renders the power-on and power-off logo options through the offline QML host into docs/mocks/logo/.

    DISPLAY=:N python3 tests/qmlhost/logos.py [--out docs/mocks/logo]

Per option: a strip of stills at fixed progress values (a logo's frame is a pure function of its
`progress`, see logo/LogoBase.qml), one PNG per option, and `index.html` that shows all of them for
choosing. The stills are what the running animation shows at those moments; they say nothing about
timing, which only the set can.
"""
import argparse
import os
import subprocess
import sys

from PIL import Image, ImageDraw

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, '..', '..'))

OPTIONS = [
    ('on-1', 'logo/LogoOn.qml', 'Power-on 1 (default): three glass panes slide in and settle, then the name rises under them', 740),
    ('on-2', 'logo/LogoOn2.qml', 'Power-on 2: the mark fades up in place, one soft arc sweeps around it, the name fades in', 740),
    ('on-3', 'logo/LogoOn3.qml', 'Power-on 3: the name alone, its letters rising one after another', 700),
    ('off-1', 'logo/LogoOff.qml', 'Power-off 1 (default): the name sinks and fades, then the panes slide apart and fade', 640),
    ('off-2', 'logo/LogoOff2.qml', 'Power-off 2: the name fades, an arc sweeps round the mark against the way in, the mark fades', 530),
    ('off-3', 'logo/LogoOff3.qml', 'Power-off 3: the name alone, its letters leaving from the right', 510),
]
STEPS = [0.0, 0.2, 0.4, 0.6, 0.8, 1.0]
CELL = (480, 270)


def still(qml, progress, path):
    r = subprocess.run([sys.executable, os.path.join(HERE, 'qmlhost.py'), qml, '--shot', path, '--wait', '600',
                        '--prop', f'progress={progress}'], capture_output=True, text=True)
    if r.returncode != 0 or not os.path.exists(path):
        raise SystemExit(f'{qml} at {progress}: {r.stdout[-400:]}{r.stderr[-400:]}')


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--out', default=os.path.join(ROOT, 'docs', 'mocks', 'logo'))
    a = ap.parse_args()
    os.makedirs(a.out, exist_ok=True)
    tmp = os.path.join(a.out, '.frames')
    os.makedirs(tmp, exist_ok=True)
    for key, qml, title, ms in OPTIONS:
        cells = []
        for p in STEPS:
            f = os.path.join(tmp, f'{key}-{int(p * 100):03d}.png')
            still(qml, p, f)
            cells.append(Image.open(f).convert('RGB').resize(CELL, Image.LANCZOS))
        sheet = Image.new('RGB', (CELL[0] * 3 + 8, CELL[1] * 2 + 4), (28, 28, 34))
        d = ImageDraw.Draw(sheet)
        for i, c in enumerate(cells):
            x, y = (i % 3) * (CELL[0] + 4), (i // 3) * (CELL[1] + 4)
            sheet.paste(c, (x, y))
            d.text((x + 8, y + 6), f'{int(STEPS[i] * ms)} ms', fill=(150, 150, 160))
        sheet.save(os.path.join(a.out, f'{key}.png'))
        print(key, f'{ms} ms')
    for n in os.listdir(tmp):
        os.remove(os.path.join(tmp, n))
    os.rmdir(tmp)
    rows = ''.join(f'<section><h2>{t}</h2><p>{ms} ms</p><img src="{k}.png" alt="{k}"></section>\n' for k, _, t, ms in OPTIONS)
    with open(os.path.join(a.out, 'index.html'), 'w') as f:
        f.write('<!doctype html><meta charset="utf-8"><title>LumaTV boot and power-off logos</title>\n'
                '<style>body{background:#111;color:#ddd;font:16px/1.4 Manrope,sans-serif;margin:32px}'
                'section{margin:0 0 32px}h2{font-size:18px;margin:0}p{margin:2px 0 8px;color:#889}img{max-width:100%}</style>\n'
                '<h1>Power-on and power-off logos: pick one of each</h1>\n'
                '<p>Option 1 of each is the default. Stills at 0, 20, 40, 60, 80 and 100 % of the animation, left to right, top to bottom.</p>\n'
                + rows)


if __name__ == '__main__':
    main()
