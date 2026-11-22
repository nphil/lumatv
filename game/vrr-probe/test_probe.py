import csv
from fractions import Fraction
from io import StringIO
import unittest

from compare_timer import BASE, channels
from stimulus import COUNT, PHASES, manifest, period_ns, rates


class StimulusTests(unittest.TestCase):
    def test_equal_time_not_arithmetic_mean_of_rates(self):
        shuffled = rates('shuffled')
        self.assertEqual(shuffled.count(Fraction(125, 2)), COUNT // 2)
        self.assertEqual(shuffled.count(Fraction(100)), COUNT // 2)
        self.assertTrue(any(a == b for a, b in zip(shuffled, shuffled[1:])))
        self.assertEqual(shuffled, rates('shuffled'))
        durations = [sum(map(period_ns, rates(p))) for p in PHASES]
        self.assertEqual(durations, [6_000_000_000, 9_360_000_000, 9_360_000_000])

    def test_exact_bounds(self):
        for r in (Fraction(24000, 1001), Fraction(120001, 1000), Fraction(0),
                  Fraction(65537, 1024)):
            with self.assertRaises(ValueError):
                period_ns(r)
        self.assertEqual(period_ns(Fraction(24)), Fraction(125000000, 3))
        self.assertEqual(period_ns(Fraction(120)), Fraction(25000000, 3))

    def test_frame_to_access_unit_and_deadline(self):
        stream = StringIO()
        manifest(stream, 'fixed120', [(100 * i, 100) for i in range(COUNT)])
        stream.seek(0)
        rows = list(csv.DictReader(stream))
        for i in (0, 3, 719):
            self.assertEqual(rows[i], dict(id=str(i), packet_offset=str(100*i),
                packet_bytes='100', patch_on=str(i % 2), rate_num='120', rate_den='1',
                period_ns='25000000/3', target_ns=str(round(Fraction(i * 10**9, 120)))))

    def test_register_receipt(self):
        def dump(w):
            return 'sync\n' + '\n'.join(f'{BASE+i*4:08X}: ' + ' '.join(
                f'{v:08X}' for v in w[i:i+4]) for i in range(0, 64, 4))
        words = [0] * 64
        words[0], words[1], words[14], words[15] = 1 << 29, 1 << 29, 210937, 210938
        before = dump(words)
        self.assertEqual(channels(before)[5], (210937, 210938, 1, 1))
        words[1] = 0
        self.assertNotEqual(channels(before), channels(dump(words)))
        for invalid in (before[:-10], before.replace('CE020710', 'CE020700')):
            with self.assertRaises(ValueError):
                channels(invalid)


if __name__ == '__main__':
    unittest.main()
