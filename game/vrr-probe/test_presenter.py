"""Builds and runs presenter/presenter_test.c: the decoded-picture presenter against a real HEVC decoder (libavcodec).

Skips when the host FFmpeg build (gfn/feeder/third_party/build-ffmpeg.sh) or zig is missing. Run from this directory:
python3 -B -m unittest -v test_presenter
"""
from pathlib import Path
import random
import subprocess
import tempfile
import unittest

PRESENTER = Path(__file__).resolve().parent / 'presenter'
STREAM = PRESENTER / 'fixtures' / 'tagged-bframes.hevc'
ORDER = PRESENTER / 'fixtures' / 'tagged-bframes.order'
SKIPPED = 77  # build.sh's exit status for a missing toolchain, as in the feeder's checks

CASES = (
    'bframe_reordering', 'hold_until_retirement', 'repeat_holds_one_surface', 'duplicates',
    'stale_and_future_epoch', 'full_queue', 'flush', 'flush_holds_the_scanned_surface', 'selection_follows_presentation_order',
    'shutdown', 'retire_after_drop_and_bad_cookies', 'lost_cookie_is_a_reported_leak', 'oracle_catches_early_release',
    'ack_is_not_ownership', 'backpressure_never_recycles', 'trace_matrix',
)


class PresenterTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        tmp = tempfile.TemporaryDirectory()
        cls.addClassCleanup(tmp.cleanup)
        build = subprocess.run(['nice', '-n', '15', 'sh', str(PRESENTER / 'build.sh'), tmp.name],
                               capture_output=True, text=True, timeout=600)
        if build.returncode == SKIPPED:
            raise unittest.SkipTest(build.stdout.strip())
        if build.returncode:
            raise RuntimeError(build.stdout + build.stderr)
        cls.exe = Path(tmp.name) / 'presenter_test'

    def run_exe(self, *args):
        return subprocess.run([self.exe, STREAM, ORDER, *args], capture_output=True, text=True, timeout=120)

    def test_ownership_and_retirement_on_decoded_surfaces(self):
        done = self.run_exe()
        self.assertEqual(done.returncode, 0, done.stderr)
        reported = [line.split(' ', 1)[0] for line in done.stdout.splitlines() if ' ok' in line]
        self.assertEqual(reported, list(CASES))

    def test_shuffled_trace_is_the_stimulus_shuffle(self):
        shuffled = [10, 16] * 24
        random.Random(174).shuffle(shuffled)
        done = self.run_exe('--print-shuffled')
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertEqual([int(x) for x in done.stdout.split()], shuffled)
