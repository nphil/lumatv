"""Run with: cd tests && python3 -m unittest test_frame_budget   (needs rustui/target/debug/lumatv-ui or LUMATV_UI_BIN, and Mesa; skipped without them)

At 120 Hz a frame of the set holds about 80 layers (its GPU time is about 1.5 ms and 0.085 ms for each layer its passes draw, round 23). On the
host harness drawing as the compositor does (`host_budget.py`: Home under its floor, a page copying what it owes from the last one, one refresh a
frame), the quick panel and the input picker open and close, and Settings is walked as the gate walks it, in frames of at most that many layers,
and every frame is the frame drawn whole.
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
    def check(self, surface):
        windows, mismatched, _ = hb.run(surface, 120, reference=True)
        frames = [(label, f) for label, fs in windows for f in fs]
        self.assertTrue(frames, surface)
        label, worst = max(frames, key=lambda lf: lf[1][3])
        self.assertLessEqual(worst[3], hb.LAYERS_120, '%s, %s: a frame of %d layers at %d ms' % (surface, label, worst[3], worst[0]))
        self.assertEqual(mismatched, 0, '%s: frames that differ from the frame drawn whole' % surface)

    def test_the_quick_panel_opens_and_closes_in_frames_a_120_hz_refresh_holds(self):
        self.check('quick')

    def test_the_input_picker_opens_and_closes_in_frames_a_120_hz_refresh_holds(self):
        self.check('picker')

    def test_settings_walked_as_the_gate_walks_it_draws_frames_a_120_hz_refresh_holds(self):
        self.check('settings-scroll')


if __name__ == '__main__':
    unittest.main()
