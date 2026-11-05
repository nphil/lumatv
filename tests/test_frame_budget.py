"""Run with: cd tests && python3 -m unittest test_frame_budget   (needs rustui/target/debug/lumatv-ui or LUMATV_UI_BIN, and Mesa; skipped without them)

At 120 Hz a frame of the set holds about 80 layers (its GPU time is about 1.5 ms and 0.085 ms for each layer its passes draw, round 23). On the
host harness drawing as the compositor does (`host_budget.py`: Home under its floor, a page copying what it owes from the last one, one refresh a
frame), the quick panel and the input picker open and close, Settings is walked as the gate walks it, and the Smart home card is entered, toggled,
scrolled and opened into its sheets, in frames of at most that many layers, and every frame is the frame drawn whole. The gate's reading of a
burst (`host_budget.gate_bursts`) needs neither.
"""
import os
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import host_budget as hb  # noqa: E402

MESA = '/usr/lib/x86_64-linux-gnu/libEGL.so.1'


@unittest.skipUnless(os.path.exists(hb.BIN) and os.path.exists(MESA), 'needs the host binary and Mesa')
class FrameBudget(unittest.TestCase):
    def check(self, surface, opening=()):
        """`opening` names the windows that open a layer or bring the focus into a card: Home under them changes whole, as it does
        for the cards of `home-widgets` (115-127 layers on the host, `docs/research/frames-120-result.md`), so they are not held to
        the budget; every frame is still held to the frame drawn whole."""
        windows, mismatched, _ = hb.run(surface, 120, reference=True)
        frames = [(label, f) for label, fs in windows for f in fs]
        self.assertTrue(frames, surface)
        label, worst = max((lf for lf in frames if lf[0] not in opening), key=lambda lf: lf[1][3])
        self.assertLessEqual(worst[3], hb.LAYERS_120, '%s, %s: a frame of %d layers at %d ms' % (surface, label, worst[3], worst[0]))
        self.assertEqual(mismatched, 0, '%s: frames that differ from the frame drawn whole' % surface)
        return dict(windows)

    def test_the_quick_panel_opens_and_closes_in_frames_a_120_hz_refresh_holds(self):
        self.check('quick')

    def test_the_input_picker_opens_and_closes_in_frames_a_120_hz_refresh_holds(self):
        self.check('picker')

    def test_settings_walked_as_the_gate_walks_it_draws_frames_a_120_hz_refresh_holds(self):
        self.check('settings-scroll')

    def test_the_smart_home_card_draws_frames_a_120_hz_refresh_holds_and_none_once_it_rests(self):
        windows = self.check('onhome', opening=('enter the card', 'sensor history', 'colour sheet'))
        # (the shell commits once a second whether or not anything changed: those frames draw no pixel)
        self.assertEqual([f for f in windows['rest'] if f[1] > 0], [], 'frames drawn at rest')


class GateBursts(unittest.TestCase):
    def test_a_burst_chains_the_flips_within_50_ms_from_its_mark_and_counts_the_intervals_over_a_refresh_and_a_fifth(self):
        def frame(ms, kept=False):
            return (ms, 0, 0, 0, 0, 0, 0, 0, kept)
        frames = [frame(t) for t in (8, 16, 25, 41, 50)] + [frame(90, kept=True)] + [frame(t) for t in (130, 138, 180, 300)]
        got = hb.gate_bursts(frames, [(0, 'open'), (30, 'inside'), (125, 'next'), (250, 'end')], 120)
        # 25 -> 41 is over 10 ms; the kept frame at 90 is no flip, so 50 -> 130 ends the burst
        self.assertEqual(got['open'], (8, 50, [(25, 16)]))
        self.assertNotIn('inside', got, 'a mark inside the burst before is read with it')
        self.assertEqual(got['next'], (5, 55, [(13, 42)]))
        self.assertEqual(got['end'], (50, 50, []), 'a first flip 50 ms after the mark is in the burst')


if __name__ == '__main__':
    unittest.main()
