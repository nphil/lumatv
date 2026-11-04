#!/usr/bin/env python3
"""Renders the Rust shell's channel guide (`lumatv-ui --host --kinds guide`: the engine in-process on Mesa, as the compositor runs it) against
the make-believe set of channels and listings the host gives it (`--guide-fixture`), dark and light at 100 % and 200 % text: the grid, the grid
mid-scroll, a programme's sheet, a row of long titles and a channel with no listings. 1280-wide JPEGs, with the host's layout and contrast lint
(`--eval lint`) read for each.

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


def scenes():
    """name -> (mode, text scale, [(gap before the key in ms, key, how)], settle after the last one in ms)."""
    empty = first_without_listings(5)
    return {
        'dark-100-grid': ('dark', 1.0, [], SETTLE),
        'light-100-grid': ('light', 1.0, [], SETTLE),
        'dark-200-grid': ('dark', 2.0, [], SETTLE),
        'light-200-grid': ('light', 2.0, [], SETTLE),
        # the time moves by a held arrow: the picture is taken in the middle of the spring's flight
        'dark-100-scroll': ('dark', 1.0, [(0, 'right', ''), (300, 'right', 'repeat'), (100, 'right', 'repeat'), (100, 'right', 'repeat')], 120),
        'dark-100-sheet': ('dark', 1.0, [(0, 'right', ''), (400, 'info', '')], SETTLE),
        'light-100-sheet': ('light', 1.0, [(0, 'right', ''), (400, 'info', '')], SETTLE),
        # the long titles are on the rows of channels 7 and 8 of the fixture; the focus starts on 7
        'dark-100-long-titles': ('dark', 1.0, [(0, 'down', ''), (300, 'up', '')], SETTLE),
        'dark-200-long-titles': ('dark', 2.0, [(0, 'down', ''), (300, 'up', '')], SETTLE),
        'dark-100-empty': ('dark', 1.0, [(300 if i else 0, 'down', '') for i in range(empty - 5)], SETTLE),
    }


def render(name, out, binary):
    mode, scale, keys, settle = scenes()[name]
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
               '--clock', CLOCK, '--guide-fixture', logos, '--log', log, '--wait', str(snap + 400),
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
