import unittest

import analyse_blank as ab

S = 1_000_000_000


def csv(segments, marks=(), states=()):
    """Edges for each (seconds, hz) in turn, a sampler's bracket of 0.2 ms around each; `None` as hz is a stop of that many seconds."""
    lines = ["# clock=CLOCK_MONOTONIC irq=104", "kind,a_ns,b_ns_or_value,c,d,e"]
    t = 10 * S
    for seconds, hz in segments:
        if hz is None:
            t += int(seconds * S)
            continue
        end = t + int(seconds * S)
        while t < end:
            lines.append(f"E,{t - 200_000},{t + 200_000},1,,")
            t += int(S / hz)
    for m in marks:
        lines.append(f"K,{10 * S + int(m * S)},,,,")
    for at, lm, vf, bo, src in states:
        lines.append(f"S,{10 * S + int(at * S)},{lm},{vf},{bo},{src}")
    return "\n".join(lines)


class Blank(unittest.TestCase):
    def test_a_stop_of_the_vsync_and_the_return_to_the_band_are_found(self):
        text = csv([(2.0, 96.0), (0.060, None), (3.0, 120.0)], marks=[2.0])
        r = ab.analyse(text)
        self.assertAlmostEqual(r["start_hz"], 96.0, delta=1.5)
        self.assertAlmostEqual(r["end_hz"], 120.0, delta=1.5)
        self.assertEqual(len(r["intervals_over_limit"]), 1)
        self.assertAlmostEqual(r["intervals_over_limit"][0][1], 60.0, delta=11.0)
        self.assertLess(r["seconds_to_band_after_ref"], 0.9)
        self.assertEqual(r["ref"], "mark")

    def test_a_rate_that_stays_out_of_the_band_has_no_return_time(self):
        r = ab.analyse(csv([(2.0, 120.0), (3.0, 100.0)], marks=[1.0]))
        self.assertIsNone(r["seconds_to_band_after_ref"])
        self.assertEqual(r["intervals_over_limit"], [])

    def test_mute_and_black_out_episodes_are_the_time_between_a_change_and_its_undoing(self):
        states = [(0.0, "0", "600", "orignal_(0x0)", "NONE"), (1.0, "34", "600", "orignal_(0x0)", "VDEC"), (1.3, "0", "600", "orignal_(0x0)", "VDEC"),
                  (2.0, "0", "600", "on_(0x1)", "VDEC"), (2.2, "0", "600", "orignal_(0x0)", "VDEC")]
        r = ab.analyse(csv([(3.0, 120.0)], states=states))
        self.assertEqual(r["mute_episodes_s"], [(1.0, 1.3)])
        self.assertEqual(r["black_out_episodes_s"], [(2.0, 2.2)])


if __name__ == "__main__":
    unittest.main()
