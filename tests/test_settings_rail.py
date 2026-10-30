"""Run with: cd tests && python3 -m unittest test_settings_rail   (needs rustui/target/debug/lumatv-ui or LUMATV_UI_BIN, and Mesa; skipped without them)

Settings' rail walked fast makes the page it comes to and not each page on the way (`SettingsSurface::page_waits`). Round 22's 120 Hz ring: 60 of a
walk's 64 missed refreshes were the rail's steps, each a page of a hundred layers made, deleted and drawn new. A deliberate step still shows its page at once.
"""
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
OPEN_AT = 1000
LEFT_AT = 2500
FIRST_AT = 3000


@unittest.skipUnless(host.available() or (BIN and os.path.exists(BIN)), 'needs the host binary and Mesa')
class RailWalk(unittest.TestCase):
    def walk(self, gap, steps):
        """Opens Settings, moves to the rail and steps down `steps` times `gap` ms apart. Returns (the model's page, the rows built and the zone 100 ms after
        the first step and at the end, the frames of the walk). The first step's page at once is `SettingsSurface::page_waits`' unit test."""
        if BIN:
            host.BIN = BIN
        log = os.path.join(tempfile.mkdtemp(prefix='railwalk-'), 'frames.log')
        end = FIRST_AT + steps * gap
        extra = ['--log', log, '--key', '%d:left' % LEFT_AT]
        for i in range(steps):
            extra += ['--key', '%d:down' % (FIRST_AT + i * gap)]
        extra += ['--eval', '%d:state' % (FIRST_AT + 100), '--eval', '%d:state' % (end + 700)]
        run = [(300, lambda c: c.push('foreground', {'app': 'com.webos.app.home', 'home': True})),
               (OPEN_AT, lambda c: c.push('show', {'kind': 'settings', 'page': 'picture'}))]
        rc, out, _, _, _ = host.run(steps=run, wait=end + 1500, extra=extra, lg=lgsettings.LgService(), helper=host.FakeHelper({'topics': {}}), kinds='home,settings')
        self.assertEqual(rc, 0, out[-400:])
        states = [(e['settings'].get('page'), e['settings'].get('built'), e['settings'].get('zone')) for e in host.evals(out)]
        self.assertEqual(len(states), 2, out[-400:])
        with open(log) as f:
            frames = [fr for fr in hf.frames(f.read()) if FIRST_AT - 50 <= fr[0] <= end + 700]
        return states, frames

    def test_a_fast_walk_ends_on_the_page_a_slow_one_does_and_draws_less_on_the_way(self):
        fast, fast_frames = self.walk(60, 6)
        slow, slow_frames = self.walk(500, 6)
        self.assertEqual(fast[1], slow[1], 'the page the rail came to is the one on screen at the end (page, rows, zone)')
        self.assertEqual(fast[1][2], 'rail')
        self.assertGreater(fast[1][1], 0, 'and it is built')
        # six pages made for a slow walk, two for a fast one: it draws well under what a step apiece would
        drawn = lambda frames: sum(f[2] for f in frames)  # noqa: E731
        per_slow_step = drawn(slow_frames) / 6.0
        self.assertLess(drawn(fast_frames), 4 * per_slow_step, 'fast %d px, a slow step %d px' % (drawn(fast_frames), per_slow_step))


if __name__ == '__main__':
    unittest.main()
