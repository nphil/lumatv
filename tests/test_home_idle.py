"""Run with: cd tests && python3 -m unittest test_home_idle   (needs rustui/target/debug/lumatv-ui or LUMATV_UI_BIN, and Mesa; skipped without them)

Home at rest costs the engine what changes and no more. Round 22's rings (60 and 120 Hz, wallpaper motion off, the gate's own setting): an idle Home
redrew 93 % of the screen, three frames of 87 layers, one to four times a second. The toolkit says the look of the dock's glass and the pose of its cards
again each time Home draws (the clock's second draws it), and the engine took each for a change (`Op::Restyle`, `Op::Set`): on the host the idle second
drew 1.13 Mpx a frame, where the clock's own digits are 31 kpx.
"""
import json
import os
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(HERE, 'qmlhost'))
import host_frames as hf  # noqa: E402
import lgsettings  # noqa: E402
import rust_settings_host as host  # noqa: E402

BIN = os.environ.get('LUMATV_UI_BIN')
SHIPPED = os.path.join(os.path.dirname(HERE), 'payload', 'theme', 'theme.json')
#: the start-up draws everything once; after this the screen is at rest
SETTLED_MS = 3500
#: the clock's digits are 31 kpx; a frame of more than this has drawn something that did not change
MOST_PX = 100_000


@unittest.skipUnless(host.available() or (BIN and os.path.exists(BIN)), 'needs the host binary and Mesa')
class HomeAtRest(unittest.TestCase):
    def frames(self, motion):
        if BIN:
            host.BIN = BIN
        with open(SHIPPED) as f:
            theme = json.load(f)
        theme['wallpaperMotion']['enabled'] = motion
        log = os.path.join(tempfile.mkdtemp(prefix='homeidle-'), 'frames.log')
        steps = [(300, lambda c: c.push('foreground', {'app': 'com.webos.app.home', 'home': True}))]
        rc, out, _, _, _ = host.run(steps=steps, wait=9000, extra=['--log', log], lg=lgsettings.LgService(), helper=host.FakeHelper({'topics': {}}),
                                    kinds='home', theme=theme)
        self.assertEqual(rc, 0, out[-400:])
        if os.path.exists(log):
            with open(log) as f:
                text = f.read()
        else:
            text = out
        return [f for f in hf.frames(text) if f[0] >= SETTLED_MS]

    def test_a_home_with_its_wallpaper_still_draws_its_clock_and_nothing_else(self):
        late = self.frames(motion=False)
        self.assertGreaterEqual(len(late), 3, 'the clock ticks: a frame a second')
        worst = max(f[1] for f in late)
        self.assertLessEqual(worst, MOST_PX, 'a frame at rest drew %d px: %s' % (worst, late))


if __name__ == '__main__':
    unittest.main()
