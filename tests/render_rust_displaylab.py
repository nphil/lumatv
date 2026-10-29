#!/usr/bin/env python3
"""Renders the Rust shell's display lab (`lumatv-ui --host --kinds displaylab`: the engine in-process on Mesa, as the compositor runs it)
against the compositor's stand-in channel and a fake of the helper's `/v1/displaylab` door, dark and light, at 100 % and 200 % text, into
docs/mocks/host-rust/displaylab/ as 1280-wide JPEGs: the intro, each step with the option a person has switched to, the results.

    python3 tests/render_rust_displaylab.py [--out docs/mocks/host-rust/displaylab] [--only sharpness-strong,results] [--list]

A scene is the keys a person presses from the intro; the picture is taken at the end, with the host's layout and contrast lint
(`--eval lint`) and the lab's own state. `SCENES` and `play` are what tests/test_rust_displaylab.py uses.
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
import rust_settings_host as host  # noqa: E402
import lgsettings  # noqa: E402

OUT = os.path.join(ROOT, 'docs', 'mocks', 'host-rust', 'displaylab')
WIDTH = 1280
QUALITY = 80
OPEN_AT = 900
KEY_GAP = 500
SETTLE = 1300

SESSION = {'ok': True, 'displaylab': {'available': True, 'session': {'id': 1, 'held': True, 'usable': {'sharpness': True, 'dimming': True, 'window': True}, 'why': {}}}}


def routes():
    """The helper's door as it answers a TV where everything works."""
    return {
        'POST /v1/displaylab/begin': lambda b: json.loads(json.dumps(SESSION)),
        'POST /v1/displaylab/set': lambda b: json.loads(json.dumps(SESSION)),
        'POST /v1/displaylab/ping': lambda b: json.loads(json.dumps(SESSION)),
        'POST /v1/displaylab/end': lambda b: {'ok': True},
        'POST /v1/displaylab/results': lambda b: {'ok': True},
    }


def refused():
    """A TV that is not an LG set: the helper says no."""
    gone = {'_status': 503, 'ok': False, 'error': {'code': 'unavailable', 'message': 'not an LG set: /proc/lg/fbdev/chip/osd_sr_gain is missing'}}
    return {'POST /v1/displaylab/begin': lambda b: dict(gone)}


# the keys that go from a step to the next one: Skip is the last answer, and the focus starts on the first
SKIP_COMPARE = ['down', 'down', 'ok']
SKIP_QUESTION = ['down', 'down', 'down', 'ok']
# the steps of the plan, in order, each with the way past it
PAST = [SKIP_COMPARE, SKIP_COMPARE, SKIP_COMPARE, SKIP_QUESTION, SKIP_QUESTION, SKIP_QUESTION]


def to_step(n):
    """The keys from the intro to step `n` (0-based): Start, then past each step before it."""
    keys = ['ok']
    for i in range(n):
        keys += PAST[i]
    return keys


class Scene:
    def __init__(self, name, keys, routes=None, app='netflix', pointer=None):
        self.name, self.keys, self.routes, self.app, self.pointer = name, list(keys), routes, app, pointer

    def helper_routes(self):
        return dict(self.routes() if callable(self.routes) else routes() if self.routes is None else self.routes)


SCENES = [
    Scene('intro', []),
    Scene('refused', [], refused),
    Scene('sharpness-strong', to_step(0)),
    Scene('sharpness-strong-b', to_step(0) + ['right']),
    Scene('sharpness-soft', to_step(1)),
    Scene('sharpness-off', to_step(2)),
    Scene('sharpness-off-b', to_step(2) + ['right']),
    Scene('dimming-dimmed', to_step(3)),
    Scene('dimming-dimmed-b', to_step(3) + ['right']),
    Scene('dimming-grey', to_step(4)),
    Scene('window-a', to_step(5)),
    Scene('window-a-b', to_step(5) + ['right']),
    Scene('window-nothing-behind', to_step(5), app='com.webos.app.home'),
    # Yes on the first window setting leaves the second out: the results come next
    Scene('results', to_step(5) + ['ok']),
]
BY_NAME = {s.name: s for s in SCENES}

# what a scene is rendered as, besides the dark 100 % one: the steps that are the hard cases at 200 %, and light
EXTRA = {
    'intro': [('dark', 2.0), ('light', 1.0)],
    'sharpness-strong': [('dark', 2.0), ('light', 1.0)],
    'sharpness-off-b': [('dark', 2.0)],
    'dimming-dimmed': [('dark', 2.0), ('light', 1.0)],
    'window-a': [('dark', 2.0), ('light', 1.0)],
    'window-a-b': [('dark', 2.0)],
    'results': [('dark', 2.0), ('light', 1.0)],
}


def jpeg(png, path):
    from PIL import Image
    im = Image.open(png).convert('RGB')
    im = im.resize((WIDTH, round(im.height * WIDTH / im.width)), Image.LANCZOS)
    im.save(path, 'JPEG', quality=QUALITY, optimize=True, progressive=True)


def play(scene, mode='dark', scale=1.0, wait=None, shot=None, evals=('lint', 'state', 'geometry')):
    """Runs the scene; returns (rc, output, helper, evals by name, the png's path or None). The helper keeps every request it was sent."""
    shot = shot or os.path.join(tempfile.mkdtemp(prefix='dlrender-'), scene.name + '.png')
    t = OPEN_AT + KEY_GAP * (len(scene.keys) + 1) + SETTLE
    extra = []
    for i, k in enumerate(scene.keys):
        extra += ['--key', '%d:%s' % (OPEN_AT + 300 + KEY_GAP * (i + 1), k)]
    for e in evals:
        extra += ['--eval', '%d:%s' % (t - 200, e)]
    extra += ['--snap', '%d:%s' % (t, shot)]
    steps = [(OPEN_AT - 300, lambda c: c.push('foreground', {'app': scene.app, 'home': scene.app == 'com.webos.app.home'})),
             (OPEN_AT, lambda c: c.push('show', {'kind': 'displaylab'}))]
    helper = host.FakeHelper(scene.helper_routes())
    rc, text, chan, lg, helper = host.run(steps=steps, wait=wait or t + 800, extra=extra, lg=lgsettings.LgService(answers={}), helper=helper,
                                          kinds='displaylab', text_scale=scale, mode=mode)
    got = dict(zip(evals, host.evals(text)))
    return rc, text, helper, got, shot if os.path.exists(shot) else None


def render(scene, mode='dark', scale=1.0, out=None, wait=None):
    """Runs the scene; returns (rc, output, lint findings, the png's path or None). `out` is a path for a JPEG."""
    rc, text, _, got, png = play(scene, mode, scale, wait)
    if png and out:
        os.makedirs(os.path.dirname(out), exist_ok=True)
        jpeg(png, out)
    findings = got.get('lint') if isinstance(got.get('lint'), list) else []
    return rc, text, findings, png


def label(scene, mode, scale):
    return '%s%s%s' % (scene.name, '' if mode == 'dark' else '-' + mode, '' if scale == 1.0 else '-%dx' % int(scale))


def jobs_for(only=()):
    jobs = []
    for s in SCENES:
        if only and s.name not in only:
            continue
        for mode, scale in [('dark', 1.0)] + EXTRA.get(s.name, []):
            jobs.append((s, mode, scale))
    return jobs


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--out', default=OUT)
    ap.add_argument('--only', default='')
    ap.add_argument('--list', action='store_true')
    ap.add_argument('--jobs', type=int, default=2)
    a = ap.parse_args()
    only = [n for n in a.only.split(',') if n]
    jobs = jobs_for(only)
    if a.list:
        print('\n'.join(label(*j) for j in jobs))
        return 0
    if not host.available():
        print('needs rustui/target/debug/lumatv-ui and Mesa', file=sys.stderr)
        return 1
    bad = 0
    with concurrent.futures.ThreadPoolExecutor(a.jobs) as pool:
        for (s, mode, scale), (rc, text, findings, png) in zip(jobs, pool.map(lambda j: render(j[0], j[1], j[2], os.path.join(a.out, label(*j) + '.jpg')), jobs)):
            ok = rc == 0 and png is not None and not findings
            bad += not ok
            print('%s %s%s' % ('ok  ' if ok else 'FAIL', label(s, mode, scale), ''.join('\n     lint: %s %s %s' % (f['kind'], f['subject'], f['detail']) for f in findings)))
            if rc != 0 or png is None:
                print(text[-1500:])
    return 1 if bad else 0


if __name__ == '__main__':
    sys.exit(main())
