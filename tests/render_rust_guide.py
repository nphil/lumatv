#!/usr/bin/env python3
"""Renders the Rust shell's channel guide (`lumatv-ui --host --kinds guide`: the engine in-process on Mesa, as the compositor runs it) against
the make-believe set of channels and listings the host gives it (`--guide-fixture`), dark and light at 100, 150 and 200 % text: the grid, rows of
long titles, the grid mid-scroll, a programme's sheet (over a long title, over no information), a channel with no listings, the edge channels
of the fixture (a ticker of programmes of 5 to 30 minutes, a long name, no logo; the first and last rows), the chips, a row walked along, and
a time just after a tick of the ruler. 1280-wide JPEGs and the full-size PNGs, with the host's layout and contrast lint (`--eval lint`) read
for each.

    python3 tests/render_rust_guide.py [--out DIR] [--only dark-100-grid,...] [--list] [--bin PATH]

A scene is the keys a person presses from the guide opening over Live TV; the picture is taken after them.
"""
import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path.insert(0, HERE)
import host_budget as hb  # noqa: E402  (the guide's logos, the binary)

OUT = os.path.join(tempfile.gettempdir(), 'lumatv-guide-renders')
WIDTH = 1280
#: the guide opens at OPEN_AT, its keys start at KEYS_AT, and the picture is taken SETTLE after the last of them
OPEN_AT = 400
KEYS_AT = 2600
SETTLE = 1200
#: a Tuesday evening on the TV's clock (the host runs on New York time)
CLOCK = '2026-11-03T20:42'


def fnv(text):
    h = 0xcbf29ce484222325
    for b in text.encode():
        h = ((h ^ b) * 0x100000001b3) & 0xFFFFFFFFFFFFFFFF
    return h


def first_without_listings(after):
    """The first antenna channel below row `after` that the fixture gives no listings (a fifth of them): its index."""
    return next(i for i in range(after + 1, 57) if fnv('1_32_%d_1_0_0_0' % (2 + i)) % 5 == 4)


def taps(key, n, gap=40):
    """`n` presses of `key`, `gap` ms apart, the first at once."""
    return [(gap if i else 0, key, '') for i in range(n)]


def scenes():
    """name -> (mode, text scale, [(gap before the key in ms, key, how)], settle after the last one in ms[, the clock])."""
    empty = first_without_listings(5)
    out = {}
    # the screen as it opens, in both looks at the three text sizes the settings offer
    for mode in ('dark', 'light'):
        for pct, scale in ((100, 1.0), (150, 1.5), (200, 2.0)):
            out['%s-%d-grid' % (mode, pct)] = (mode, scale, [], SETTLE)
    # the long titles are on the rows of channels 7 and 8 of the fixture; the focus starts on 7
    for mode, pct, scale in (('dark', 100, 1.0), ('dark', 150, 1.5), ('dark', 200, 2.0), ('light', 100, 1.0), ('light', 200, 2.0)):
        out['%s-%d-long-titles' % (mode, pct)] = (mode, scale, [(0, 'down', ''), (300, 'up', '')], SETTLE)
    # the time moves by a held arrow: the picture is taken in the middle of the spring's flight
    for mode, pct, scale in (('dark', 100, 1.0), ('light', 200, 2.0)):
        out['%s-%d-scroll' % (mode, pct)] = (mode, scale, [(0, 'right', ''), (300, 'right', 'repeat'), (100, 'right', 'repeat'), (100, 'right', 'repeat')], 120)
    # a programme's sheet: over the grid, over a row with long titles, over a row with no information
    for mode, pct, scale in (('dark', 100, 1.0), ('light', 100, 1.0), ('dark', 200, 2.0), ('light', 200, 2.0)):
        out['%s-%d-sheet' % (mode, pct)] = (mode, scale, [(0, 'right', ''), (400, 'info', '')], SETTLE)
    for pct, scale in ((100, 1.0), (200, 2.0)):
        out['dark-%d-sheet-long' % pct] = ('dark', scale, [(0, 'down', ''), (400, 'info', '')], SETTLE)
    out['dark-100-sheet-empty'] = ('dark', 1.0, taps('down', empty - 5, 300) + [(400, 'info', '')], SETTLE)
    # a row with no listings
    for pct, scale in ((100, 1.0), (200, 2.0)):
        out['dark-%d-empty' % pct] = ('dark', scale, taps('down', empty - 5, 300), SETTLE)
    # the edge channels (fixture.rs): 57 a ticker of programmes of 5 to 30 minutes, 58 a long name, 59 no logo, 60 a wide name and no
    # listings, 61 (the last row) a ticker with no logo
    for mode, pct, scale in (('dark', 100, 1.0), ('dark', 200, 2.0), ('light', 100, 1.0)):
        out['%s-%d-short' % (mode, pct)] = (mode, scale, taps('down', 52) + [(300, 'right', '')], SETTLE)
    for mode, pct, scale in (('dark', 100, 1.0), ('dark', 150, 1.5), ('dark', 200, 2.0), ('light', 100, 1.0)):
        out['%s-%d-last' % (mode, pct)] = (mode, scale, taps('down', 60), SETTLE)
    # the first row, and the chips above it
    for mode in ('dark', 'light'):
        out['%s-100-first' % mode] = (mode, 1.0, taps('up', 5), SETTLE)
    out['dark-100-chips'] = ('dark', 1.0, taps('up', 6), SETTLE)
    # cells longer than the screen at 200 % text, the focus walked along a row, and a time just after a tick of the ruler
    out['dark-200-along'] = ('dark', 2.0, taps('right', 3, 300), SETTLE)
    out['dark-100-along'] = ('dark', 1.0, taps('right', 3, 300), SETTLE)
    out['dark-100-tick'] = ('dark', 1.0, [], SETTLE, '2026-11-03T21:03')
    out['dark-100-tick-half'] = ('dark', 1.0, [], SETTLE, '2026-11-03T20:31')
    return out


def render(name, out, binary):
    mode, scale, keys, settle, *clock = scenes()[name]
    data = tempfile.mkdtemp(prefix='guiderender-')
    try:
        os.makedirs(os.path.join(data, 'theme'))
        shutil.copy(os.path.join(ROOT, 'payload', 'theme', 'theme.json'), os.path.join(data, 'theme', 'theme.json'))
        logos = hb.write_logos(os.path.join(data, 'logos'))
        log = os.path.join(data, 'render.log')
        png = os.path.join(data, name + '.png')
        args, at = [], KEYS_AT
        for gap, key, how in keys:
            at += gap
            args += ['--key', '%d:%s%s' % (at, key, ':' + how if how else '')]
        if any(how == 'repeat' for _, _, how in keys):
            args += ['--key', '%d:right:up' % (at + 60)]
        snap = at + settle
        cmd = [binary, '--host', '--virtual', '--refresh', '60', '--kinds', 'osd,toast,guide', '--mode', mode, '--text-scale', str(scale),
               '--data-dir', data, '--shm', os.path.join(data, 'shm'), '--module-dir', os.path.join(ROOT, 'compositor', 'assets', 'ui'),
               '--clock', clock[0] if clock else CLOCK, '--guide-fixture', logos, '--log', log, '--wait', str(snap + 400),
               '--push', '50:foreground:' + json.dumps({'app': 'com.webos.app.livetv', 'home': False, 'covered': True}),
               '--push', '%d:show:%s' % (OPEN_AT, json.dumps({'kind': 'guide'})),
               '--eval', '%d:lint' % (snap - 50), '--eval', '%d:guide' % (snap - 50), '--snap', '%d:%s' % (snap, png)] + args
        env = dict(os.environ, LIBGL_ALWAYS_SOFTWARE='1', TZ='America/New_York')
        r = subprocess.run(cmd, env=env, capture_output=True, text=True, timeout=900)
        text = open(log).read() if os.path.exists(log) else ''
        if r.returncode != 0 or not os.path.exists(png):
            raise RuntimeError('%s: lumatv-ui failed: %s' % (name, (r.stdout + r.stderr)[-1500:]))
        from PIL import Image
        os.makedirs(out, exist_ok=True)
        image = Image.open(png).convert('RGB')
        image.resize((WIDTH, round(image.height * WIDTH / image.width)), Image.LANCZOS).save(os.path.join(out, name + '.jpg'), quality=84)
        image.save(os.path.join(out, name + '.png'))
        lint = re.findall(r'^\[eval\] (\[.*\])\s*$', text, re.M)
        findings = json.loads(lint[-1]) if lint else None
        print('%-24s %s  lint: %s' % (name, os.path.join(out, name + '.png'), 'no report' if findings is None else ('clean' if not findings else findings)))
    finally:
        shutil.rmtree(data, ignore_errors=True)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    ap.add_argument('--out', default=OUT)
    ap.add_argument('--only', default='')
    ap.add_argument('--list', action='store_true')
    ap.add_argument('--bin', default=hb.BIN)
    a = ap.parse_args(argv)
    names = [n for n in a.only.split(',') if n] or list(scenes())
    if a.list:
        print('\n'.join(scenes()))
        return 0
    for n in names:
        render(n, a.out, a.bin)
    return 0


if __name__ == '__main__':
    sys.exit(main())
