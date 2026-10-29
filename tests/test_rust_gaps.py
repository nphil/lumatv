"""Run with: cd tests && python3 -m unittest test_rust_gaps   (needs rustui/target/debug/lumatv-ui and Mesa; skipped without them)

The surfaces the QML shell had that the Rust kinds did not (docs/research/rust-gaps-result.md), on the host harness (`lumatv-ui --host`: the shell's
logic against the engine in its own process on Mesa) against the stand-in of the compositor's UI channel and the fake helper:

- the AI access prompt (kind `ai`): the helper's `ai.prompt` topic drawn over an app with the keys swallowed and nothing that can answer it;
- the input banners (kind `banner`): the input pill, the one with its buttons, and the black no-signal screen;
- Home's own: the launch splash and the hold ring, Quick Access, the on-screen remote, the voice bar and the mic mark, and the mini frame.

Every screen is checked by the host's layout and contrast lint at 100 % and 200 % text in both looks, and `RUST_GAPS_RENDERS=<dir>` writes the renders
(1280-wide JPEGs) the docs keep beside the mocks (docs/mocks/host-rust/gaps/).
"""
import json
import os
import sys
import tempfile
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(HERE, 'qmlhost'))
import lgsettings  # noqa: E402
import render_rust_settings as rs  # noqa: E402
import rust_settings_host as host  # noqa: E402
from test_rust_panels import APP, HOME, OPEN, Scene, jpeg  # noqa: E402

RENDERS = os.environ.get('RUST_GAPS_RENDERS')
WIDE = {'app': 'com.webos.app.hdmi2', 'home': False}


def run(scene, mode='dark', scale=1.0, evals=('geometry', 'state', 'lint', 'report'), snap=None, extra=()):
    """`test_rust_panels.run`, keeping the fake helper too: what the shell asked of it is what some tests are about."""
    extra = list(extra)
    for ms, topic, doc in scene.pushes:
        extra += ['--push', '%d:%s:%s' % (ms, topic, json.dumps(doc, separators=(',', ':')))]
    for ms, name, hold in scene.keys:
        if hold:
            extra += ['--key', '%d:%s' % (ms, name), '--key', '%d:%s:repeat' % (ms + hold // 2, name), '--key', '%d:%s:up' % (ms + hold, name)]
        else:
            extra += ['--key', '%d:%s' % (ms, name), '--key', '%d:%s:up' % (ms + 80, name)]
    for what in evals:
        extra += ['--eval', '%d:%s' % (scene.end - 300, what)]
    if snap:
        os.makedirs(os.path.dirname(snap), exist_ok=True)
        extra += ['--snap', '%d:%s' % (scene.end, snap)]
    start = [(OPEN - 400, 'foreground', scene.helper.get('foreground', HOME)), (OPEN - 380, 'screen', {'on': True})]
    if scene.show:
        start.append((OPEN, 'show', scene.show))
    for ms, topic, doc in start:
        extra += ['--push', '%d:%s:%s' % (ms, topic, json.dumps(doc, separators=(',', ':')))]
    helper = host.FakeHelper({'topics': dict(scene.helper.get('topics', {}))})
    lg = scene.lg() if callable(scene.lg) else scene.lg or lgsettings.LgService(answers=rs.ONLINE)
    rc, out, chan, lg, helper = host.run(wait=scene.end + 500, extra=extra, lg=lg, helper=helper, kinds=scene.kinds, text_scale=scale, mode=mode, theme=scene.theme)
    by = {}
    for e in host.evals(out):
        if isinstance(e, list):
            by['lint'] = e
        elif isinstance(e, dict) and 'kinds' in e:
            by['state'] = e
        elif isinstance(e, dict) and 'can' in e:
            by['report'] = e
        elif isinstance(e, dict):
            by['geometry'] = e
    return {'rc': rc, 'out': out, 'chan': chan, 'helper': helper, 'ev': by}


def picture(light=False):
    """A still stand-in for what an app is playing: a dusk sky over a dark horizon, so the sheets over it are read against a picture."""
    from PIL import Image, ImageDraw
    path = os.path.join(tempfile.gettempdir(), 'rustgaps-picture-%s.png' % ('light' if light else 'dark'))
    if not os.path.exists(path):
        im = Image.new('RGB', (1920, 1080))
        d = ImageDraw.Draw(im)
        top, mid, low = ((0xF3, 0xD9, 0xB8), (0xE9, 0x9F, 0x6E), (0x57, 0x44, 0x5E)) if light else ((0x1D, 0x28, 0x4F), (0x7A, 0x4A, 0x6E), (0xD8, 0x8A, 0x5C))
        for y in range(700):
            t = y / 700.0
            a, b, u = (top, mid, t * 2) if t < 0.5 else (mid, low, (t - 0.5) * 2)
            d.line([(0, y), (1920, y)], fill=tuple(round(a[i] + (b[i] - a[i]) * u) for i in range(3)))
        d.rectangle([0, 700, 1920, 1080], fill=(0x14, 0x10, 0x1C) if not light else (0x3A, 0x30, 0x40))
        for x, h in ((140, 160), (420, 260), (900, 120), (1380, 300), (1700, 180)):
            d.rectangle([x, 700 - h, x + 90, 700], fill=(0x14, 0x10, 0x1C))
        im.save(path)
    return path


def ai_doc(kind='tv_launch_app', who='Cody', title='Open Netflix', detail='The TV opens Netflix and leaves what is on now.', reason='Nitin asked for a movie tonight.'):
    now = int(time.time() * 1000)
    return {'id': 'a1b2c3', 'kind': kind, 'assistant': {'id': 'cody', 'name': who} if who else None, 'title': title, 'detail': detail, 'reason': reason,
            'shownAt': now, 'expiresAt': now + 33000, 'allow': 'OK', 'deny': 'Back'}


def ai_scene(name, doc, keys=(), foreground=APP):
    return Scene(name, 'ai', keys=keys, helper={'topics': {'ai.prompt': doc}, 'foreground': foreground})


@unittest.skipUnless(host.available(), 'needs rustui/target/debug/lumatv-ui and Mesa')
class RustGaps(unittest.TestCase):
    def ok(self, r):
        self.assertEqual(r['rc'], 0, r['out'][-3000:])
        return r['ev']

    def shot(self, scene, name, mode='dark', scale=1.0, evals=('geometry', 'state', 'lint', 'report')):
        """Runs `scene` over a picture and, when asked, writes the render under `name`."""
        suffix = '%s-%d' % (mode, round(scale * 100))
        png = os.path.join(tempfile.mkdtemp(prefix='rustgaps-'), name + '.png')
        r = run(scene, mode=mode, scale=scale, evals=evals, snap=png if RENDERS else None, extra=['--backdrop', picture(mode == 'light')])
        if RENDERS and os.path.exists(png):
            jpeg(png, os.path.join(RENDERS, '%s-%s.jpg' % (name, suffix)))
        return r

    # ---- the AI access prompt
    def test_the_ai_prompt_is_drawn_over_an_app_takes_the_keys_and_nothing_the_remote_or_a_click_does_answers_it(self):
        scene = ai_scene('ai-keys', ai_doc(), keys=[(OPEN + 400, 'ok', 0), (OPEN + 800, 'back', 0), (OPEN + 1200, 'up', 0), (OPEN + 1600, '1', 0)])
        r = run(scene, extra=['--pointer', '%d:960:540:1' % (OPEN + 2000), '--pointer', '%d:960:540:0' % (OPEN + 2100)])
        ev = self.ok(r)
        self.assertTrue(ev['state']['ai']['up'], 'the helper said a prompt is open: %s' % ev['state'])
        self.assertTrue(ev['report']['wants'], 'the compositor sends every key to the Rust shell while the prompt is up')
        self.assertEqual(ev['lint'], [], 'the prompt reads clean: %s' % ev['lint'])
        card = ev['geometry']['ai']['card']
        self.assertEqual(card['x'] + card['w'] / 2, 960.0, 'in the middle of the screen')
        self.assertEqual(ev['geometry']['ai']['hints'], ['OK, Allow once', 'Back, Deny'], 'the keys as glyphs with what each would do, no instruction words')
        # the keys, the arrows and a click came; nothing went to the helper's AI routes or the luna bridge
        asked = [(m, p) for m, p, b in r['helper'].requests if '/ai' in p]
        self.assertFalse([a for a in asked if a[0] != 'GET'], 'no route of AI access was called: %s' % asked)
        self.assertEqual(r['chan'].luna, [], 'no luna call: %s' % r['chan'].luna)

    def test_without_a_prompt_the_ai_kind_wants_no_key(self):
        quiet = self.ok(run(Scene('ai-none', 'ai', helper={'foreground': APP}), evals=('state', 'report')))
        self.assertFalse(quiet['state']['ai']['up'])
        self.assertFalse(quiet['report']['wants'])

    def test_renders_of_the_ai_prompt(self):
        if not RENDERS:
            self.skipTest('RUST_GAPS_RENDERS is not set')
        long_detail = 'Cody wants to switch this TV to HDMI 2, which takes the picture away from what is playing now.'
        for mode in ('dark', 'light'):
            for scale in (1.0, 2.0):
                ev = self.ok(self.shot(ai_scene('ai-prompt', ai_doc(), foreground=APP), 'ai-prompt', mode, scale))
                self.assertEqual(ev['lint'], [], '%s at %s: %s' % (mode, scale, ev['lint']))
        ev = self.ok(self.shot(ai_scene('ai-owner', ai_doc(kind='pair', who=None, title='Pair a new assistant', detail='A pairing code shows on this TV for one assistant. Give it to the assistant once.', reason=''), foreground=HOME), 'ai-owner'))
        self.assertEqual(ev['lint'], [])
        ev = self.ok(self.shot(ai_scene('ai-input', ai_doc(kind='tv_switch_input', title='Switch to HDMI 2', detail=long_detail), foreground=APP), 'ai-input', scale=2.0))
        self.assertEqual(ev['lint'], [])


if __name__ == '__main__':
    unittest.main()
