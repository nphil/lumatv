"""Run with: cd tests && python3 -m unittest test_rust_gaps   (needs rustui/target/debug/lumatv-ui and Mesa; skipped without them)

The surfaces the QML shell had that the Rust kinds did not (docs/research/rust-gaps-result.md), on the host harness (`lumatv-ui --host`: the shell's
logic against the engine in its own process on Mesa) against the stand-in of the compositor's UI channel and the fake helper:

- the AI access prompt (kind `ai`): the helper's `ai.prompt` topic drawn over an app with the keys swallowed and nothing that can answer it;
- the input banners (kind `banner`): the input pill, the one with its buttons, and the black no-signal screen;
- Home's own, on the Home host (kind `home`): the launch screen, Quick Access, the on-screen remote, the voice bar and the mic mark, the mini frame, the
  moving wallpaper's controller, and the two ways into Settings (a Search answer that is a page, the status bar's popover).

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
import test_rust_home as home_host  # noqa: E402  (the module: importing its test class would run its tests here too)
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

    # ---- the input banners
    def banner_scene(self, name, page=None, spec=None, keys=(), theme=None, foreground=None):
        pushes = [(OPEN, 'banner.show', spec)] if spec else []
        return Scene(name, 'banner', pushes=pushes, keys=keys, theme=theme, helper={'foreground': foreground or WIDE},
                     show={'kind': 'banner', 'page': page} if page else None)

    def test_the_input_pill_is_shown_for_a_moment_takes_no_key_and_goes_by_itself(self):
        ev = self.ok(run(self.banner_scene('banner-pill', 'pill'), evals=('state', 'report', 'geometry', 'lint')))
        self.assertEqual(ev['state']['banner']['kind'], 'input')
        self.assertFalse(ev['report']['wants'], 'a pill takes no key: the keys are the app\'s')
        self.assertIn('banner', ev['report']['can'], 'the compositor may ask for it by name')
        self.assertEqual(ev['lint'], [])
        self.assertEqual(ev['geometry']['banner']['pill']['x'], 64.0)

    def test_the_bar_with_buttons_takes_the_keys_moves_picks_and_the_inputs_button_asks_for_the_input_picker(self):
        r = run(self.banner_scene('banner-bar', 'bar', keys=[(OPEN + 600, 'right', 0), (OPEN + 1100, 'ok', 0)]), evals=('state', 'report', 'geometry', 'lint'))
        ev = self.ok(r)
        self.assertFalse(ev['state']['banner']['up'], 'OK on Inputs ended the banner: %s' % ev['state']['banner'])
        self.assertFalse(ev['report']['wants'], 'the keys are given back')
        launched = [c for c in r['chan'].luna if 'launch' in str(c)]
        self.assertTrue(any('homeconnect' in str(c) for c in launched), 'without the Rust input picker it is LG\'s Home Hub that opens, as the compositor does for the Input key: %s' % r['chan'].luna)
        held = self.ok(run(self.banner_scene('banner-bar-held', 'bar', keys=[(OPEN + 600, 'right', 0)]), evals=('state', 'report', 'geometry', 'lint')))
        self.assertTrue(held['state']['banner']['up'] and held['state']['banner']['keys'])
        self.assertTrue(held['report']['wants'])
        self.assertEqual(held['state']['banner']['index'], 1, 'Right moved the focus to Inputs')
        self.assertEqual(held['geometry']['banner']['focus'], 1)
        self.assertEqual(held['lint'], [])

    def test_no_signal_takes_the_keys_and_home_goes_home(self):
        r = run(self.banner_scene('banner-nosignal', 'nosignal', keys=[(OPEN + 600, 'right', 0), (OPEN + 1100, 'ok', 0)]), evals=('state', 'report', 'lint'))
        ev = self.ok(r)
        self.assertFalse(ev['state']['banner']['up'])
        self.assertTrue(any('launchDefaultApp' in str(c) for c in r['chan'].luna), 'Home: %s' % r['chan'].luna)
        up = self.ok(run(self.banner_scene('banner-nosignal-up', 'nosignal'), evals=('state', 'report', 'lint', 'geometry')))
        self.assertEqual((up['state']['banner']['kind'], up['state']['banner']['buttons']), ('nosignal', ['inputs', 'home']))
        self.assertTrue(up['report']['wants'])
        self.assertEqual(up['lint'], [])
        self.assertEqual(up['geometry']['banner']['spot'], 0)

    def test_an_input_that_comes_to_the_front_shows_its_pill_when_the_owner_switched_the_banners_on_and_not_otherwise(self):
        on = Scene('banner-switch', 'banner', theme={'banners': {'own': True}}, helper={'foreground': HOME}, end=OPEN + 3600,
                   pushes=[(OPEN, 'foreground', {'app': 'com.webos.app.hdmi2', 'home': False, 'covered': True})])
        ev = self.ok(run(on, evals=('state', 'geometry')))
        self.assertEqual(ev['state']['banner']['kind'], 'input')
        self.assertEqual(ev['geometry']['banner']['name'], 'HDMI 2', 'the list gave no answer in the stand-in: the port alone')
        off = Scene('banner-switch-off', 'banner', helper={'foreground': HOME}, end=OPEN + 3600, pushes=[(OPEN, 'foreground', {'app': 'com.webos.app.hdmi2', 'home': False, 'covered': True})])
        self.assertFalse(self.ok(run(off, evals=('state',)))['state']['banner']['up'], 'LG\'s own banners draw the moment unless the owner asks for these')

    def test_renders_of_the_banners(self):
        if not RENDERS:
            self.skipTest('RUST_GAPS_RENDERS is not set')
        channel = {'kind': 'channel', 'number': '7', 'call': 'KABC', 'title': 'Evening News', 'sub': '6:00 PM - 6:30 PM', 'progress': 0.4, 'next': 'Jeopardy!', 'chips': ['1080i', 'Stereo', 'CC']}
        scenes = [('banner-pill', self.banner_scene('banner-pill', 'pill')), ('banner-bar', self.banner_scene('banner-bar', 'bar')),
                  ('banner-nosignal', self.banner_scene('banner-nosignal', 'nosignal')), ('banner-channel', self.banner_scene('banner-channel', spec=channel))]
        for mode in ('dark', 'light'):
            for scale in (1.0, 2.0):
                for name, scene in scenes:
                    ev = self.ok(self.shot(scene, name, mode, scale))
                    self.assertEqual(ev['lint'], [], '%s %s at %s: %s' % (name, mode, scale, ev['lint']))


def looks():
    """The looks the Home renders are made in: all four with `RUST_GAPS_RENDERS`, else the two that differ the most."""
    return [(m, c) for m in ('dark', 'light') for c in (1, 2)] if RENDERS else [('dark', 1), ('light', 2)]


@unittest.skipUnless(host.available(), 'needs rustui/target/debug/lumatv-ui and Mesa')
class RustGapsHome(unittest.TestCase):
    """Home's own extras, on the Home host (`test_rust_home.RustHome.host`: Home in front, the launch points answered, the feeds published)."""

    def home(self, wait, steps, render=None, mode='dark', scale=1, **kw):
        """Runs Home; `render` is (ms, name): a snapshot at that time kept under RUST_GAPS_RENDERS as `name-mode-scale`."""
        steps = list(steps)
        png = None
        if render and RENDERS:
            png = os.path.join(tempfile.mkdtemp(prefix='rustgaps-'), render[1] + '.png')
            steps.append(['--snap', '%d:%s' % (render[0], png)])
        if kw.get('theme') is not None:
            # a theme file in the data directory is read again by Home, which would drop the material the harness forced
            kw['theme'] = {**kw['theme'], 'material': mode}
        out, ev, data = home_host.RustHome.host(self, wait, steps, mode=mode, scale=scale, **kw)
        if png and os.path.exists(png):
            jpeg(png, os.path.join(RENDERS, '%s-%s-%d.jpg' % (render[1], mode, scale * 100)))
        return out, ev, data

    k = staticmethod(home_host.RustHome.k)

    # ---- the launch screen
    def test_the_launch_screen_covers_home_with_the_apps_icon_name_and_ring_while_it_opens_and_reads_clean(self):
        for mode, scale in looks():
            steps = [self.k(500, '2'), ['--eval', '3300:geometry'], ['--eval', '3300:lint'], ['--eval', '3300:state']]
            out, ev, _ = self.home(4000, steps, (3500, 'launch-screen'), mode, scale)
            geometry, lint, state = ev[0], ev[1], ev[2]
            splash = geometry['extras']['splash']
            self.assertEqual(splash['icon']['w'], 180.0, '%s %s: %s' % (mode, scale, splash))
            self.assertEqual(splash['caption'], 'Opening')
            self.assertEqual(state['extras']['splash']['name'], splash['name'], 'the model and the surface say the same app')
            self.assertEqual(lint, [], '%s at %s: %s' % (mode, scale, lint))
            self.assertIn('launch', out)

    # ---- Quick Access
    ACCESS = {'remote': {'quickAccess': {'on': True, 'slots': {}}}}

    def few_on_the_dock(self, keep=4):
        """A theme with Quick Access on and the dock cut to its first `keep` tiles, so a number past them has nothing behind it."""
        launch_points = home_host.launch_points
        ids = [p['id'] for p in launch_points()]
        return {**self.ACCESS, 'apps': {'dockHide': ids[keep:]}}

    def test_a_number_that_opens_a_tile_says_so_in_a_toast_at_the_top_right(self):
        for mode, scale in looks():
            steps = [self.k(2600, '3'), ['--eval', '3400:geometry'], ['--eval', '3400:lint'], ['--eval', '3400:state']]
            _, ev, _ = self.home(4400, steps, (3600, 'access-toast'), mode, scale, theme=self.ACCESS)
            toast = ev[0]['extras']['toast']
            self.assertEqual((toast['y'], toast['x'] + toast['w']), (137.0, 1856.0))
            self.assertTrue(toast['title'].startswith('3 \u00b7 '), toast)
            self.assertEqual(toast['line'], 'Opening.')
            self.assertEqual(ev[1], [], '%s at %s: %s' % (mode, scale, ev[1]))
            self.assertEqual(ev[2]['extras']['toast']['digit'], '3')

    def test_a_number_with_nothing_behind_it_asks_what_to_give_it_and_Back_leaves_it(self):
        for mode, scale in looks():
            steps = [self.k(900, '9'), ['--eval', '1800:geometry'], ['--eval', '1800:lint'], ['--eval', '1800:report'],
                     self.k(2000, 'right'), ['--eval', '2500:geometry'], self.k(4000, 'back'), ['--eval', '4600:state']]
            _, ev, _ = self.home(5000, steps, (3500, 'access-question'), mode, scale, theme=self.few_on_the_dock())
            ask = ev[0]['extras']['ask']
            self.assertEqual(ask['card']['w'], 820.0 if scale == 1 else 1496.0, 'the cards grow with the text so no name is broken')
            self.assertEqual(len(ask['picks']), 5, 'four apps and More')
            self.assertEqual(ask['hints'][-2:], ['OK, confirm', 'Back, close'])
            self.assertEqual(ask['hints'][:-2], ['Left and Right, move'] if scale == 1 else [], 'the D-pad glyph is the first to give way at a large text size')
            self.assertEqual(ev[1], [], '%s at %s: %s' % (mode, scale, ev[1]))
            self.assertTrue(ev[2]['wants'])
            self.assertEqual(ev[3]['extras']['ask']['focus'], 1, 'Right moved the choice')
            self.assertIsNone(ev[4]['extras']['ask'], 'Back leaves it and opens nothing')
            self.assertIsNone(ev[4]['home']['launching'])

    def test_the_choice_is_kept_in_the_owners_theme_and_the_app_opens(self):
        out, ev, data = self.home(3200, [self.k(900, '9'), self.k(1400, 'ok'), ['--eval', '2400:state']], theme=self.few_on_the_dock())
        with open(os.path.join(data, 'theme', 'theme.json')) as f:
            kept = json.load(f)
        slots = kept['remote']['quickAccess']['slots']
        self.assertEqual(list(slots), ['9'], kept['remote'])
        self.assertTrue(kept['remote']['quickAccess']['on'])
        self.assertIn('launch %s' % slots['9'], out)
        self.assertEqual(ev[0]['extras']['toast']['line'], 'Assigned. Opening.')

    # ---- the on-screen remote
    def test_the_remote_key_opens_the_on_screen_remote_over_home_and_it_reads_clean(self):
        for mode, scale in looks():
            steps = [self.k(900, 'screenremote'), self.k(1300, 'right', 'down'), ['--eval', '2000:geometry'], ['--eval', '2000:lint'], ['--eval', '2000:report']]
            out, ev, _ = self.home(4000, steps, (3500, 'on-screen-remote'), mode, scale)
            remote = ev[0]['extras']['remote']
            self.assertEqual(remote['sheet']['x'], 64.0)
            self.assertEqual(len(remote['keys']), 21)
            self.assertNotIn('ok', [k['name'] for k in remote['keys']])
            self.assertEqual(remote['focus'], '4', 'Right to Play, Down to the number pad')
            self.assertEqual(ev[1], [], '%s at %s: %s' % (mode, scale, ev[1]))
            self.assertTrue(ev[2]['wants'])

    # ---- the voice bar
    def voice_run(self, mode, scale, steps, routes, wait, snap=None, theme=None):
        """Home on the host harness with a fake helper that answers the voice routes (`routes`): returns (the evals, the helper)."""
        launch_points, profiles = home_host.launch_points, home_host.profiles
        extra = ['--push', '50:foreground:' + json.dumps(HOME), '--luna', '80:300:' + json.dumps({'launchPoints': launch_points()}),
                 '--helper', '100:profiles:' + json.dumps(profiles())]
        for st in steps:
            extra += st
        png = None
        if snap and RENDERS:
            png = os.path.join(tempfile.mkdtemp(prefix='rustgaps-'), snap[1] + '.png')
            extra += ['--snap', '%d:%s' % (snap[0], png)]
        helper = host.FakeHelper({'topics': {'voice': {'ready': True, 'state': 'idle', 'reason': ''}}, **routes})
        rc, out, chan, lg, helper = host.run(wait=wait, extra=extra, lg=lgsettings.LgService(answers=rs.ONLINE), helper=helper, kinds='home', text_scale=scale, mode=mode, theme=theme)
        self.assertEqual(rc, 0, out[-2000:])
        if png and os.path.exists(png):
            jpeg(png, os.path.join(RENDERS, '%s-%s-%d.jpg' % (snap[1], mode, scale * 100)))
        return host.evals(out), helper

    @staticmethod
    def voice_key(ms, hold):
        return ['--key', '%d:voice' % ms, '--key', '%d:voice:up' % (ms + hold)]

    def test_holding_the_voice_key_listens_with_a_bar_and_a_mark_and_letting_go_works_it_out(self):
        listen = {'POST /v1/voice/listen': {'ok': True, 'session': 7}, 'POST /v1/voice/stop': {'ok': True}}
        for mode, scale in looks():
            held = [['--key', '900:voice'], ['--eval', '3000:geometry'], ['--eval', '3000:lint'], ['--eval', '3000:report'], ['--eval', '3000:state']]
            ev, helper = self.voice_run(mode, scale, held, listen, 4000, (3400, 'voice-bar-listening'))
            voice = ev[0]['extras']['voice']
            self.assertEqual(voice['bar']['caption'], 'Listening', voice)
            self.assertEqual(voice['bar']['frame']['x'], 64.0)
            self.assertIsNotNone(voice['mark'], 'the microphone is open')
            self.assertEqual(ev[1], [], '%s at %s: %s' % (mode, scale, ev[1]))
            self.assertIn('voice', ev[2]['takes'], 'the helper says it can listen: the key is the shell\'s')
            self.assertEqual(ev[3]['extras']['mic'], True)
            self.assertIn(('POST', '/v1/voice/listen', {}), helper.requests)
            working = [['--key', '900:voice'], ['--key', '1500:voice:up'], ['--helper', '1700:voice:' + json.dumps({'ready': True, 'state': 'thinking', 'session': 7})],
                       ['--eval', '3000:geometry'], ['--eval', '3000:lint']]
            ev, helper = self.voice_run(mode, scale, working, listen, 4000, (3400, 'voice-bar-working'))
            self.assertEqual(ev[0]['extras']['voice']['bar']['caption'], 'Working it out')
            self.assertEqual(ev[1], [], '%s at %s: %s' % (mode, scale, ev[1]))
            self.assertIn(('POST', '/v1/voice/stop', {}), helper.requests)

    def test_a_voice_search_that_cannot_listen_says_why_in_the_bar(self):
        refused = {'POST /v1/voice/listen': {'ok': False, 'error': {'message': 'Voice search is not linked to Home Assistant'}}}
        for mode, scale in looks():
            ev, _ = self.voice_run(mode, scale, [['--key', '3000:voice'], ['--eval', '3500:geometry'], ['--eval', '3500:lint']], refused, 4200, (3600, 'voice-bar-failed'))
            bar = ev[0]['extras']['voice']['bar']
            self.assertEqual((bar['caption'], ' '.join(bar['title'])), ('Voice search', 'Voice search is not linked to Home Assistant'), bar)
            self.assertEqual(ev[1], [], '%s at %s: %s' % (mode, scale, ev[1]))

    # ---- the mini frame
    def test_a_story_opens_in_the_mini_frame_which_grows_out_of_its_card_and_reads_clean(self):
        for mode, scale in looks():
            steps = [self.k(900, 'down', 'right', 'ok', 'ok'), ['--eval', '3300:geometry'], ['--eval', '3300:lint'], ['--eval', '3300:state']]
            out, ev, _ = self.home(4200, steps, (3800, 'mini-frame'), mode, scale)
            frame = ev[0]['extras']['frame']
            self.assertEqual((frame['x'], frame['w']), (200.0, 1520.0))
            if scale == 1:
                self.assertEqual((frame['y'], frame['h']), (150.0, 860.0))
                self.assertEqual(frame['interior'], {'x': 232.0, 'y': 214.0, 'w': 1456.0, 'h': 764.0})
            else:
                self.assertLessEqual(frame['y'] + frame['h'], 1080 - 54, 'inside the safe area under the taller bar: %s' % frame)
            self.assertTrue(frame['title'].startswith('Denmark says'), frame)
            self.assertEqual(ev[1], [], '%s at %s: %s' % (mode, scale, ev[1]))
            self.assertTrue(ev[2]['extras']['frame']['landed'] and ev[2]['extras']['frame']['wanted'], 'the web half was asked for')


if __name__ == '__main__':
    unittest.main()
