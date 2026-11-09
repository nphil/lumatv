#!/usr/bin/env python3
"""Renders Home with the status bar full width at the top right and the bell's unread strip on its free side, on the host harness
(`lumatv-ui --host`), 1920 by 1080, into DIR as `strip-<case>.png`: 0, 1, 3 and 9 sources unread, 3 at 200 % text, and 3 with the ticker off.

    python3 tests/render_statusbar_strip.py [--out DIR] [--bin PATH] [--modes dark,light]
"""
import argparse
import os
import shutil
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import rust_home_host as H  # noqa: E402

OUT = '/data/home/tmp/lumatv-statusbar-20261109'
CLOCK = '2026-11-09T16:05'
KINDS = 'osd,toast,home,settings,quick,picker,drawer,menu,confirm'
STILL = {"wallpaperMotion": {"enabled": False}, "weather": {"animated": False}}
#: who has said something: (plugin id, its name, the channel's icon, the newest title, how many notifications)
SOURCES = [
    ('home-assistant', 'Home Assistant', 'home', 'Washing machine finished', 2),
    ('ring', 'Front door', 'camera', 'Motion at the front door', 4),
    ('plex', 'Plex', 'tv', 'Dune: Part Two was added', 1),
    ('scrypted', 'Scrypted', 'camera', 'Garage camera went offline', 1),
    ('lock', 'Back door lock', 'lock', 'Battery low: replace it soon', 1),
    ('sonos', 'Sonos', 'speaker', 'Kitchen speaker is playing', 1),
    ('mail', 'Mail', 'message', 'Isabel: dinner at seven?', 3),
    ('nas', 'BeastNAS', 'notification', 'Parity check finished with no errors', 1),
    ('tuner', 'Live TV', 'tv', 'A recording is about to start', 1),
]


def feeds(n):
    plugins = [{"id": p, "name": name, "state": "connected"} for p, name, *_ in SOURCES[:n]]
    out = {"plugins": {"plugins": plugins, "firstParty": []}}
    for i, (p, _, icon, title, count) in enumerate(SOURCES[:n]):
        items = [{"id": "n%d" % k, "channel": "events", "channelIcon": icon, "revision": 1, "receivedAt": 1_800_000_000.0 - 60 * i - k,
                  "post": {"title": title if k == 0 else '%s (%d)' % (title, k), "body": "", "actions": []}} for k in range(count)]
        out["plugin.%s.notifications" % p] = {"items": items}
    if n == 0:
        out["plugin.home-assistant.notifications"] = {"items": []}
    return out


def render(binary, out_dir, mode, name, n, scale=None, ticker=True):
    H.BIN = binary
    tmp = tempfile.mkdtemp(prefix='strip-')
    shot = os.path.join(tmp, 'shot.png')
    bar = {"anchor": "top-right", "dx": 0, "dy": 0, "full": True}
    if not ticker:
        bar["ticker"] = False
    theme = dict(STILL, material=mode, layout={"overlays": {"statusbar": bar}})
    for attempt in range(3):
        H.RustHome().host(5000, [['--snap', '4500:' + shot]], feeds=feeds(n), mode=mode, scale=scale, theme=theme, kinds=KINDS, extra=['--clock', CLOCK])
        if os.path.exists(shot) and mean_of(shot) > 5:
            break
    shutil.copy(shot, os.path.join(out_dir, 'strip-%s-%s.png' % (name, mode)))
    shutil.rmtree(tmp, ignore_errors=True)
    print(name, mode)


def mean_of(path):
    from PIL import Image, ImageStat
    return ImageStat.Stat(Image.open(path).convert('L')).mean[0]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--out', default=OUT)
    ap.add_argument('--bin', default=os.path.join(os.path.dirname(HERE), 'rustui', 'target', 'debug', 'lumatv-ui'))
    ap.add_argument('--modes', default='dark')
    a = ap.parse_args()
    os.makedirs(a.out, exist_ok=True)
    for mode in a.modes.split(','):
        for name, n, scale, ticker in [('0', 0, None, True), ('1', 1, None, True), ('3', 3, None, True), ('9', 9, None, True), ('3-200', 3, 2.0, True), ('3-ticker-off', 3, None, False)]:
            render(a.bin, a.out, mode, name, n, scale, ticker)
    return 0


if __name__ == '__main__':
    sys.exit(main())
