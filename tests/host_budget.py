#!/usr/bin/env python3
"""What each frame of the frame gate's surfaces asks of the set's GPU, measured on the host harness (`lumatv-ui --host`): Home with its
feeds, the gate's walks (`docs/research/perf/gate.py`), the engine drawing its pages as the compositor does (two stacks, Home from its
floor, a page copying from the last what it owes from the frames before), one refresh a frame (`--virtual`, so a slow host does not fold
refreshes into one frame), wallpaper motion and the weather card's sprites off as the gate has them.

A frame's GPU time on the set is about 1.5 ms and 0.085 ms for each layer its passes draw (round 23, 604 frames of an idle GPU, R2 0.47):
the 8.3 ms of a 120 Hz refresh is about 80 layers. For each window of a walk this prints the frames, the most layers a frame drew, and
that frame's estimate.

    python3 tests/host_budget.py [--refresh 120] [--surfaces quick,picker,settings-scroll,home-widgets,toast] [--frames] [--reference]
                                 [--bin PATH]

`--frames` lists every frame; `--reference` draws each frame whole as well and counts the frames that differ (the floor and the copies
must change nothing); `--bin` (or `LUMATV_UI_BIN`) names another `lumatv-ui`."""
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
import test_rust_home as home  # noqa: E402  (Home's make-believe feeds)

BIN = os.environ.get('LUMATV_UI_BIN') or os.path.join(ROOT, 'rustui', 'target', 'debug', 'lumatv-ui')
FRAME = re.compile(r"^\[frame\] (\d+) at (\d+) ms: damage (\d+) px, drew (\d+) px in (\d+) rects \([\d.]+ %\), layers (\d+) culled \d+ cpu (\d+) us gpu \d+ us"
                   r".*? floor (\d+) px apply (\d+) us", re.M)
SURFACES = ['quick', 'picker', 'settings-scroll', 'home-widgets', 'toast']
#: when a walk starts: Home has drawn its feeds and rests
START_MS = 6000
#: the layers a 120 Hz refresh holds (1.5 ms + 0.085 ms a layer <= 8.3 ms)
LAYERS_120 = 80


def gpu_ms(layers):
    return 1.5 + 0.085 * layers


class Walk:
    """A walk's keys and pushes on the harness's time, and the windows its frames are read in."""

    def __init__(self, at):
        self.at = at
        self.args = []
        self.marks = []

    def mark(self, label):
        self.marks.append((self.at, label))

    def key(self, name, how='', at=None):
        self.args += ['--key', '%d:%s%s' % (self.at if at is None else at, name, ':' + how if how else '')]

    def taps(self, name, n, gap):
        for _ in range(n):
            self.key(name)
            self.at += gap

    def hold(self, name, ms):
        """A held key as the remote sends it: the first repeat after 300 ms, then one every 100 ms."""
        self.key(name)
        for t in range(self.at + 300, self.at + ms, 100):
            self.key(name, 'repeat', at=t)
        self.key(name, 'up', at=self.at + ms)
        self.at += ms

    def push(self, topic, doc):
        self.args += ['--push', '%d:%s:%s' % (self.at, topic, json.dumps(doc))]

    def wait(self, ms):
        self.at += ms


def walk_of(surface):
    """The gate's walk of `surface` (its `open` sequence) from START_MS: a panel opened and closed twice, Settings walked as
    `settings-scroll` walks it, Home's rows as `home-widgets` does."""
    w = Walk(START_MS)
    if surface in ('quick', 'picker'):
        for n in (1, 2):
            w.mark('open %d' % n)
            w.push('show', {'kind': surface, 'n': n})
            w.wait(1500)
            w.mark('close %d' % n)
            w.key('back')
            w.wait(1500)
    elif surface == 'toast':
        for n in (1, 2):
            w.mark('toast %d' % n)
            w.push('show', {'kind': 'toast', 'n': n, 'page': 'Placement check|A toast for the gate'})
            w.wait(7000)
    elif surface == 'settings-scroll':
        # the quick panel's foot (QUICK_PANEL_FOOT), OK on its "All settings", then SETTINGS_PAGE, the rail down, SETTINGS_PAGE, the rail up, Back
        w.at = START_MS - 2200
        w.push('show', {'kind': 'quick', 'n': 1})
        w.wait(1300)
        w.taps('down', 20, 30)
        w.at = START_MS
        w.mark('open')
        w.key('ok')
        w.key('ok', 'up', at=w.at + 50)
        w.wait(600)
        for page in ('picture', 'system'):
            w.mark(page + ' list')
            w.key('right')
            w.wait(100)
            w.hold('down', 800)
            w.wait(150)
            w.hold('up', 800)
            w.wait(100)
            w.key('back')
            w.wait(150)
            if page == 'picture':
                w.mark('rail down x8')
                w.taps('down', 8, 60)
        w.mark('rail up x9')
        w.taps('up', 9, 60)
        w.mark('close')
        w.key('back')
        w.wait(1500)
    elif surface == 'home-widgets':
        # HOME_TOP first, then the walk: the status bar, the cards, the dock and back up
        w.at = START_MS - 1500
        w.taps('up', 8, 100)
        w.taps('left', 6, 60)
        w.at = START_MS
        w.mark('status bar')
        w.taps('right', 3, 120)
        w.taps('left', 3, 120)
        w.mark('into the cards')
        w.taps('down', 2, 150)
        w.mark('across the cards')
        w.taps('right', 2, 150)
        w.mark('to the dock')
        w.taps('down', 1, 150)
        w.mark('along the dock')
        w.hold('left', 900)
        w.wait(200)
        w.hold('right', 900)
        w.wait(200)
        w.mark('down and up the dock')
        w.taps('down', 2, 150)
        w.taps('up', 2, 150)
        w.mark('up to the status bar')
        w.taps('up', 6, 120)
        w.wait(600)
    else:
        raise ValueError('no walk for %s' % surface)
    w.mark('end')
    return w


def run(surface, refresh=120, binary=BIN, reference=False):
    """Runs the walk of `surface`; returns ([(label, [frame, ...]), ...], mismatched frames, the run's output). A frame is
    (ms, drawn px, rects, layers, floor px, CPU us, of it taking in commits us)."""
    data = tempfile.mkdtemp(prefix='hostbudget-')
    os.makedirs(os.path.join(data, 'theme'))
    shutil.copy(os.path.join(ROOT, 'compositor', 'assets', 'ui', 'wallpaper', 'wall_1080.png'), os.path.join(data, 'theme', 'wall_1080.png'))
    with open(os.path.join(ROOT, 'payload', 'theme', 'theme.json')) as f:
        theme = json.load(f)
    # the gate's `still`: the wallpaper's tick and the weather card's sprites off while a surface is measured
    theme.setdefault('wallpaperMotion', {})['enabled'] = False
    theme.setdefault('weather', {})['animated'] = False
    with open(os.path.join(data, 'theme', 'theme.json'), 'w') as f:
        json.dump(theme, f)
    w = walk_of(surface)
    log = os.path.join(data, 'frames.log')
    feeds = {"weather": home.weather(), "news": home.news(), "profiles": home.profiles(), "cameras": home.CAMERAS,
             "plugins": {"plugins": [{"id": "home-assistant", "name": "Home Assistant", "state": "connected"}], "firstParty": []},
             "plugin.home-assistant.cards": home.HOME_CARDS}
    cmd = [binary, '--host', '--virtual', '--refresh', str(refresh), '--kinds', 'osd,toast,home,settings,quick,picker,drawer,menu,confirm',
           '--data-dir', data, '--shm', os.path.join(data, 'shm'), '--wait', str(w.at + 500), '--log', log,
           '--module-dir', os.path.join(ROOT, 'compositor', 'assets', 'ui'),
           '--push', '40:refresh:' + json.dumps({"hz": refresh}), '--push', '50:foreground:' + json.dumps(home.HOME),
           '--luna', '80:%d:%s' % (home.LP_TAG, json.dumps({"launchPoints": home.launch_points()}))]
    for topic, doc in feeds.items():
        cmd += ['--helper', '100:%s:%s' % (topic, json.dumps(doc))]
    cmd += w.args + (['--reference'] if reference else [])
    r = subprocess.run(cmd, env=dict(os.environ, LIBGL_ALWAYS_SOFTWARE='1'), capture_output=True, text=True, timeout=1800)
    text = open(log).read() if os.path.exists(log) else ''
    shutil.rmtree(data, ignore_errors=True)
    if r.returncode != 0:
        raise RuntimeError('%s: lumatv-ui failed: %s' % (surface, (r.stdout + r.stderr)[-2000:]))
    fr = [(int(m[2]), int(m[4]), int(m[5]), int(m[6]), int(m[8]), int(m[7]), int(m[9])) for m in FRAME.finditer(text)]
    windows = []
    for (at, label), (nxt, _) in zip(w.marks, w.marks[1:]):
        windows.append((label, [f for f in fr if at <= f[0] < nxt]))
    return windows, len(re.findall(r"^\[mismatch\]", text, re.M)), text


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    ap.add_argument('--refresh', type=int, default=120)
    ap.add_argument('--surfaces', default=','.join(SURFACES))
    ap.add_argument('--frames', action='store_true')
    ap.add_argument('--reference', action='store_true')
    ap.add_argument('--bin', default=BIN)
    a = ap.parse_args(argv)
    budget = LAYERS_120 * 120 // a.refresh
    over = 0
    for surface in a.surfaces.split(','):
        windows, mismatched, _ = run(surface, a.refresh, a.bin, a.reference)
        worst = max((f[3] for _, fs in windows for f in fs), default=0)
        print('%s at %d Hz: most layers in a frame %d (%.1f ms)%s' % (surface, a.refresh, worst, gpu_ms(worst),
                                                                    ', %d frames differ from the frame drawn whole' % mismatched if a.reference else ''))
        for label, fs in windows:
            most = max((f[3] for f in fs), default=0)
            heavy = sum(1 for f in fs if f[3] > budget)
            over += heavy
            print('  %-22s %4d frames, most layers %3d (%.1f ms), %d over %d' % (label, len(fs), most, gpu_ms(most), heavy, budget))
            if a.frames:
                for f in fs:
                    print('      %6d ms  drew %8d px in %2d rects  layers %3d  floor %8d px  cpu %6d us (commits %6d us)' % f)
    return 0


if __name__ == '__main__':
    sys.exit(main())
