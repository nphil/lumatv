#!/usr/bin/env python3
"""Renders the Cameras screens of the Rust shell on the host (`lumatv-ui --host`: the engine in-process on Mesa, as the compositor runs it)
against the compositor's stand-in channel (tests/qmlhost/channel.py) and the fake helper (tests/qmlhost/fakehelper.py), dark and light,
at 100 % and 200 % text, beside the mocks of docs/mocks/cameras-scrypted/.

    python3 tests/rust_cameras_scenes.py [--out docs/mocks/host-rust/cameras] [--only grid,strip] [--modes dark,light] [--scales 1,2]

Each scene is the steps a person takes (the helper's topics, the remote's keys) and the picture at the end of them; the host's own checks
are returned with it: the layout and contrast lint (`--eval lint`) and where things are (`--eval geometry`). `run_scene` is what
tests/test_rust_cameras.py calls.
"""
import argparse
import concurrent.futures
import json
import os
import re
import socket
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, '..'))
sys.path.insert(0, os.path.join(HERE, 'qmlhost'))
BIN = os.path.join(ROOT, 'rustui', 'target', 'debug', 'lumatv-ui')
IMG = os.path.join(ROOT, 'docs', 'mocks', 'img')
NAMES = ['Front Door', 'Backyard', 'Tool Room', 'Back Door', 'Plant Room', 'Bird', 'Office', 'Cat Feeder', 'Garage', 'Driveway', 'Side Gate', 'Attic']
PICS = ['wall', 'game', 'home', 'still']
HOME = {'app': 'com.webos.app.home', 'home': True}
APP = {'app': 'netflix', 'home': False}
NOW = lambda: int(time.time() * 1000)


def have_binary():
    return os.path.exists(BIN) and os.path.exists('/usr/lib/x86_64-linux-gnu/libEGL.so.1')


def camera(i, name=None, **kw):
    doc = {'id': str(i), 'name': name or NAMES[(i - 300) % len(NAMES)], 'room': '', 'online': True, 'live': True, 'recordings': True, 'events': True,
           'clips': True, 'since': NOW() - 600 * 60000, 'native': {'codec': 'h265', 'width': 3840, 'height': 2160, 'fps': 15}, 'prebuffered': True}
    doc.update(kw)
    return doc


def cameras_topic(n=8, state='ready', per=None):
    """The helper's `cameras` topic: `per` maps an index to the fields of that camera that differ."""
    per = per or {}
    return {'state': state, 'reason': '', 'now_skew_ms': 0, 'rev': 1, 'cameras': [camera(300 + i, **per.get(i, {})) for i in range(n)]}


def live_topic(camera_id='300', mode='live', state='playing', **kw):
    doc = {'camera': camera_id, 'mode': mode, 'state': state, 'reason': '', 'codec': 'h265', 'width': 3840, 'height': 2160, 'fps': 15, 'startedAt': None,
           'positionMs': None, 'rate': 1, 'paused': False}
    doc.update(kw)
    return doc


def small(name):
    """A mock picture as a 480 by 270 JPEG: what a sub-stream's still is, and cheap to decode in a debug build."""
    from PIL import Image
    import io
    out = io.BytesIO()
    Image.open(os.path.join(IMG, name + '.jpg')).convert('RGB').resize((480, 270)).save(out, 'JPEG', quality=85)
    return out.getvalue()


def backdrop_png(name, tmp):
    """A mock picture as the PNG the harness draws behind everything: the video the full view is a hole over."""
    from PIL import Image
    path = os.path.join(tmp, name + '.png')
    if not os.path.exists(path):
        Image.open(os.path.join(IMG, name + '.jpg')).convert('RGB').save(path)
    return path


def free_port():
    with socket.socket() as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]


class Run:
    """One run of the client: its steps (on the channel's and the helper's side), its keys, what it printed."""

    def __init__(self, scale=1.0, mode='dark', wait=4500, backdrop=None, kinds='cameras', extra=()):
        self.scale, self.mode, self.wait, self.backdrop, self.kinds, self.extra = scale, mode, wait, backdrop, kinds, tuple(extra)
        self.setups = []        # fn(chan, helper_state): what the fake helper answers, set before the client starts
        self.pushes = []        # (ms, topic, doc): the compositor's topics, on the client's own clock
        self.helpers = []       # (ms, topic, doc): the helper's topics, on the client's own clock
        self.keys = []          # (ms, name, up?)
        self.pointers = []      # (ms, x, y, buttons)
        self.evals = []         # ms, what
        self.snaps = []         # ms, path

    def setup(self, fn):
        self.setups.append(fn)
        return self

    def helper(self, ms, topic, doc):
        self.helpers.append((ms, topic, doc))
        return self

    def channel(self, ms, topic, doc):
        self.pushes.append((ms, topic, doc))
        return self

    def press(self, ms, name, hold=0, repeat=False):
        self.keys.append((ms, name, ''))
        self.keys.append((ms + max(hold, 30), name, ':up'))
        return self

    def hold(self, ms, name):
        """The key goes down and stays down (the release is another step)."""
        self.keys.append((ms, name, ''))
        return self

    def release(self, ms, name):
        self.keys.append((ms, name, ':up'))
        return self

    def repeat(self, ms, name, times, every=40):
        """A held key: the press, then `times` repeats, then the release."""
        self.keys.append((ms, name, ''))
        for i in range(times):
            self.keys.append((ms + (i + 1) * every, name, ':repeat'))
        self.keys.append((ms + (times + 1) * every, name, ':up'))
        return self

    def pointer(self, ms, x, y, buttons=0):
        self.pointers.append((ms, x, y, buttons))
        return self

    def eval(self, ms, what):
        self.evals.append((ms, what))
        return self

    def snap(self, ms, path):
        self.snaps.append((ms, path))
        return self


def run(r, pictures=None, helper_setup=None):
    """Runs the client for `r`; returns a dict: the output lines, the `[eval]` documents by name and the channel's and the helper's state."""
    import channel as ch
    import fakehelper
    token = 'offline-token'
    data = tempfile.mkdtemp(prefix='rustcameras-')
    token_path = os.path.join(data, 'lumatv-ui.token')
    open(token_path, 'w').write(token + '\n')
    cport, hport = free_port(), free_port()
    chan = ch.Channel(token)
    lines = []
    srv = ch.serve(chan, cport, say=lines.append)
    hsrv, st = fakehelper.serve(hport)
    pics = {n: small(n) for n in PICS}
    for i in range(32):
        st.pictures[str(300 + i)] = pics[PICS[i % 4]]
    st.thumb_cycle = [pics[n] for n in PICS]
    for k, v in (pictures or {}).items():
        st.pictures[k] = v
    if helper_setup:
        helper_setup(st)

    for fn in r.setups:
        fn(chan, st)
    cmd = [BIN, '--host', '--port', str(cport), '--token-file', token_path, '--helper-port', str(hport), '--wait', str(r.wait), '--data-dir', data,
           '--shm', os.path.join(data, 'shm'), '--kinds', r.kinds, '--text-scale', str(r.scale), '--mode', r.mode]
    if r.backdrop:
        cmd += ['--backdrop', backdrop_png(r.backdrop, data)]
    for ms, topic, doc in r.pushes:
        cmd += ['--push', '%d:%s:%s' % (ms, topic, json.dumps(doc, separators=(',', ':')))]
    for ms, topic, doc in r.helpers:
        cmd += ['--helper', '%d:%s:%s' % (ms, topic, json.dumps(doc, separators=(',', ':')))]
    for ms, name, how in r.keys:
        cmd += ['--key', '%d:%s%s' % (ms, name, how)]
    for ms, x, y, b in r.pointers:
        cmd += ['--pointer', '%d:%s:%s:%d' % (ms, x, y, b)]
    for ms, what in r.evals:
        cmd += ['--eval', '%d:%s' % (ms, what)]
    for ms, path in r.snaps:
        cmd += ['--snap', '%d:%s' % (ms, path)]
    cmd += list(r.extra)
    env = dict(os.environ, LIBGL_ALWAYS_SOFTWARE='1')
    done = subprocess.run(cmd, env=env, capture_output=True, text=True, timeout=int(os.environ.get('HOST_TIMEOUT', '240')))
    srv.shutdown()
    hsrv.shutdown()
    out = done.stdout + done.stderr
    evals = [json.loads(m.group(1)) for m in re.finditer(r'^\[eval\] (.*)$', out, re.M)]
    return {'rc': done.returncode, 'out': out, 'evals': evals, 'chan': chan, 'helper': st, 'lines': lines}


# ----------------------------------------------------------------------------------------------------------------------------------------
# the scenes
# ----------------------------------------------------------------------------------------------------------------------------------------

def base(r, topic):
    """Home in front, the screen on, the helper's cameras: the start of every scene."""
    r.channel(200, 'foreground', HOME).channel(220, 'screen', {'on': True})
    r.helper(250, 'cameras', topic)
    return r


def open_grid(r, ms=900):
    return r.channel(ms, 'show', {'kind': 'cameras', 'warm': False, 'n': 1})


def timeline(r, camera='300', gap=False):
    """What the helper answers for the strip: a recording with a gap, `unknown` cells and events."""
    def setup(chan, st):
        now = NOW()
        m = 60000
        ranges = [{'from': now - 50 * m, 'to': now - 10 * m}, {'from': now - 8 * m, 'to': now}]
        if gap:
            ranges = [{'from': now - 25 * m, 'to': now}]
        st.camera_routes[(camera, 'recordings')] = {'camera': camera, 'from': now - 60 * m, 'to': now, 'step_ms': 60000, 'complete': False, 'probed': 5,
                                                   'ranges': ranges, 'unknown': [{'from': now - 10 * m, 'to': now - 8 * m}] if not gap else []}
        st.camera_routes[(camera, 'events')] = {'camera': camera, 'truncated': False, 'events': [
            {'at': now - 42 * m, 'kind': 'person', 'label': 'person'}, {'at': now - 25 * m, 'kind': 'doorbell', 'label': 'doorbell'},
            {'at': now - 6 * m, 'kind': 'person', 'label': 'person'}]}
    return r.setup(setup)


def sheet_data(r, camera='300'):
    def setup(chan, st):
        now = NOW()
        m = 60000
        st.camera_routes[(camera, 'events')] = {'camera': camera, 'truncated': False, 'events': [
            {'at': now - 6 * m, 'kind': 'person', 'label': 'person'}, {'at': now - 39 * m, 'kind': 'doorbell', 'label': 'doorbell'},
            {'at': now - 145 * m, 'kind': 'animal', 'label': 'Domestic Cat'}, {'at': now - 233 * m, 'kind': 'vehicle', 'label': 'vehicle'},
            {'at': now - 5 * 60 * m, 'kind': 'package', 'label': 'package'}]}
        st.camera_routes[(camera, 'clips')] = {'camera': camera, 'truncated': False, 'clips': [
            {'id': 'c1.a', 'at': now - 6 * m, 'duration': 30000, 'kind': 'person', 'label': 'person', 'description': 'Person at the front door', 'has_thumbnail': True, 'has_video': True},
            {'id': 'c2.b', 'at': now - 39 * m, 'duration': 42000, 'kind': 'doorbell', 'label': 'doorbell', 'description': 'Doorbell at the front door', 'has_thumbnail': True, 'has_video': True},
            {'id': 'c3.c', 'at': now - 145 * m, 'duration': 18000, 'kind': 'animal', 'label': 'Domestic Cat', 'description': 'Cat on the porch', 'has_thumbnail': True, 'has_video': True},
            {'id': 'c4.d', 'at': now - 17 * 3600000, 'duration': 25000, 'kind': 'vehicle', 'label': 'vehicle', 'description': 'Car in the driveway', 'has_thumbnail': True, 'has_video': True},
            {'id': 'c5.e', 'at': now - 22 * 3600000, 'duration': 31000, 'kind': 'person', 'label': 'person', 'description': 'Person at the front door', 'has_thumbnail': True, 'has_video': True}]}
    return r.setup(setup)


def scene_grid(r):
    base(r, cameras_topic(8, per={2: {'online': False}}))
    r.setup(lambda c, s: s.no_picture.add('306'))
    open_grid(r)
    r.press(2300, 'right')
    return 3600


def scene_grid_more(r):
    base(r, cameras_topic(12))
    open_grid(r)
    for ms, key in ((2300, 'right'), (2500, 'right'), (2700, 'down')):
        r.press(ms, key)
    return 3800


def scene_grid_offline(r):
    base(r, cameras_topic(8, state='offline'))
    r.setup(lambda c, s: s.no_picture.update(str(300 + i) for i in range(8)))
    open_grid(r)
    return 3000


def scene_grid_unlinked(r):
    base(r, {'state': 'unlinked', 'reason': '', 'now_skew_ms': 0, 'rev': 1, 'cameras': []})
    open_grid(r)
    return 2500


def scene_grid_refused(r):
    base(r, {'state': 'refused', 'reason': '', 'now_skew_ms': 0, 'rev': 1, 'cameras': []})
    open_grid(r)
    return 2500


def full(r, state='playing', **topic):
    base(r, cameras_topic(8, per=topic.pop('per', {})))
    open_grid(r)
    r.press(1800, 'ok')                                         # the grid's first tile: the full view
    if state:
        r.helper(2300, 'cameras.live', live_topic('300', state=state, **topic))
    return r


def scene_full(r):
    r.backdrop = 'game'
    full(r)
    return 3500


def scene_full_start(r):
    r.backdrop = 'game'
    full(r, state='starting', per={0: {'prebuffered': False}})
    return 3500


def scene_full_failed(r):
    r.backdrop = 'game'
    full(r, state='failed', reason='The camera is offline.')
    return 3500


VWIN_REFUSED = {'mode': 'apps', 'slots': 2, 'free': 0, 'apps': 1, 'wall': False, 'claims': [
    {'id': 'cameras-full', 'kind': 'camera-full', 'state': 'denied', 'reason': 'no-window', 'text': 'No video window is free', 'slot': '', 'placed': False}]}


def scene_full_refused(r):
    """The compositor gave the live picture no video window: the plane carries it still and a glyph beside the name says so."""
    r.backdrop = 'game'
    full(r)
    r.channel(2600, 'vwin', VWIN_REFUSED)
    return 3800


def scene_strip(r):
    r.backdrop = 'game'
    full(r)
    timeline(r)
    r.press(3000, 'up')
    return 4400


def scene_strip_scrub(r):
    r.backdrop = 'game'
    full(r)
    timeline(r)
    r.press(3000, 'up')
    r.press(3300, 'up')
    r.press(3500, 'ok')
    r.press(3700, 'ok')                                         # the span: 1 h, 6 h
    r.press(3900, 'down')
    r.repeat(4100, 'left', 24)
    return 6600


def scene_strip_play(r):
    r.backdrop = 'game'
    full(r)
    timeline(r)
    r.press(3000, 'up')
    r.press(3300, 'left')
    r.helper(3900, 'cameras.live', live_topic('300', mode='playback', startedAt=NOW() - 70000, positionMs=NOW() - 70000, rate=1))
    return 5200


def scene_strip_gap(r):
    r.backdrop = 'game'
    full(r)
    timeline(r, gap=True)
    r.press(3000, 'up')
    r.repeat(3300, 'left', 40)
    return 6200


def scene_events(r):
    r.backdrop = 'game'
    full(r)
    sheet_data(r)
    r.press(3000, 'down')
    return 4600


def scene_clips(r):
    r.backdrop = 'game'
    full(r)
    sheet_data(r)
    r.press(3000, 'down')
    r.press(3600, 'up')
    r.press(3800, 'right')
    r.press(4000, 'down')
    r.press(4200, 'right')
    return 5600


def scene_events_all(r):
    base(r, cameras_topic(8))
    events = {'events': [
        {'id': 'e1', 'camera': '300', 'name': 'Front Door', 'kind': 'person', 'label': 'person', 'at': NOW() - 6 * 60000, 'phase': 'new', 'thumbnail': True, 'recording': True},
        {'id': 'e2', 'camera': '301', 'name': 'Backyard', 'kind': 'animal', 'label': 'animal', 'at': NOW() - 11 * 60000, 'phase': 'new', 'thumbnail': True},
        {'id': 'e3', 'camera': '300', 'name': 'Front Door', 'kind': 'doorbell', 'label': 'doorbell', 'at': NOW() - 39 * 60000, 'phase': 'new', 'thumbnail': True, 'recording': True},
        {'id': 'e4', 'camera': '309', 'name': 'Driveway', 'kind': 'vehicle', 'label': 'vehicle', 'at': NOW() - 72 * 60000, 'phase': 'new', 'thumbnail': False},
        {'id': 'e5', 'camera': '306', 'name': 'Office', 'kind': 'person', 'label': 'person', 'at': NOW() - 140 * 60000, 'phase': 'new', 'thumbnail': True}]}
    r.helper(260, 'cameras.events', events)

    def setup(chan, st):
        pics = {n: small(n) for n in PICS}
        for i, k in enumerate(('e1', 'e2', 'e3', 'e5')):
            st.event_pictures[k] = pics[PICS[i % 4]]
    r.setup(setup)
    open_grid(r)
    r.press(2000, 'up')
    r.press(2200, 'ok')
    return 3800


def scene_options(r):
    base(r, cameras_topic(8))
    open_grid(r)
    r.press(2000, 'right')
    r.hold(2400, 'ok')
    r.release(3900, 'ok')
    r.press(4200, 'down')
    r.press(4400, 'down')
    return 5200


def scene_toast(r):
    """A playback the helper refuses: the picture goes on as it was and the words are a toast over the strip."""
    r.backdrop = 'game'
    full(r)
    timeline(r)
    r.setup(lambda c, s: s.refusals.update({'/v1/cameras/playback': (409, 'no_recording', 'There is no recording at that time')}))
    r.press(3000, 'up')
    r.press(3300, 'left')
    return 4600


# The scenes kept in docs/mocks/host-rust/cameras/ (dark and light, 100 % and 200 % text each); the others render the same way on demand.
COMMITTED = 'grid,grid-unlinked,full-failed,full-refused,strip-scrub,events,options'

SCENES = {
    'grid': scene_grid, 'grid-more': scene_grid_more, 'grid-offline': scene_grid_offline, 'grid-unlinked': scene_grid_unlinked, 'grid-refused': scene_grid_refused,
    'full': scene_full, 'full-start': scene_full_start, 'full-failed': scene_full_failed, 'full-refused': scene_full_refused,
    'strip': scene_strip, 'strip-scrub': scene_strip_scrub, 'strip-play': scene_strip_play, 'strip-gap': scene_strip_gap,
    'events': scene_events, 'clips': scene_clips, 'events-all': scene_events_all, 'options': scene_options, 'toast': scene_toast,
}


def run_scene(name, scale=1.0, mode='dark', out=None, wait=None):
    r = Run(scale=scale, mode=mode)
    end = SCENES[name](r)
    r.wait = (wait or end + 900)
    r.eval(end - 200, 'lint').eval(end - 200, 'geometry').eval(end - 200, 'state')
    if out:
        r.snap(end, out)
    res = run(r)
    res['end'] = end
    return res


def one(args):
    """One scene in one look at one text size, saved as a 1280 wide JPEG (about 3 MB for the committed set) (the pictures in them would make PNGs of 2 MB each)."""
    from PIL import Image
    name, scale, mode, outdir = args
    suffix = ('-light' if mode == 'light' else '') + ('-text200' if scale >= 2 else '')
    path = os.path.join(outdir, '%s%s.jpg' % (name, suffix))
    shot = tempfile.mktemp(prefix='rustcameras-shot-', suffix='.png')
    res = run_scene(name, scale, mode, shot)
    lint = next((e for e in res['evals'] if isinstance(e, list)), [])
    if os.path.exists(shot):
        Image.open(shot).convert('RGB').resize((1280, 720), Image.LANCZOS).save(path, 'JPEG', quality=68, optimize=True)
        os.remove(shot)
    return name, scale, mode, res['rc'], lint, path


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    ap.add_argument('--out', default=os.path.join(ROOT, 'docs', 'mocks', 'host-rust', 'cameras'))
    ap.add_argument('--only', default='', help='scenes, comma separated (default: all; the committed set is %s)' % COMMITTED)
    ap.add_argument('--modes', default='dark,light')
    ap.add_argument('--scales', default='1,2')
    ap.add_argument('--jobs', type=int, default=4)
    a = ap.parse_args()
    if not have_binary():
        sys.exit('needs rustui/target/debug/lumatv-ui and Mesa')
    os.makedirs(a.out, exist_ok=True)
    names = [n for n in a.only.split(',') if n] or list(SCENES)
    work = [(n, float(s), m, a.out) for n in names for m in a.modes.split(',') for s in a.scales.split(',')]
    bad = 0
    with concurrent.futures.ThreadPoolExecutor(a.jobs) as pool:
        for name, scale, mode, rc, lint, path in pool.map(one, work):
            status = 'ok' if rc == 0 and not lint else 'FAIL'
            if status != 'ok':
                bad += 1
            print('%-14s %s %s %s %s' % (name, mode, scale, status, json.dumps(lint) if lint else ''), flush=True)
    sys.exit(1 if bad else 0)


if __name__ == '__main__':
    main()
