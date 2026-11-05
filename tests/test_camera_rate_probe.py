"""Run with: python3 -m unittest tests.test_camera_rate_probe

compositor/tools/tv-camera-rate-probe.py runs on the set as root. Its readers (the panel's rate, the decoder's, the display's low delay), the picture-mode readings
and verdicts, and the loads it reads from the system log are plain functions of text: its `selftest` runs them here.
"""
import contextlib
import importlib.util
import io
import os
import unittest

SCRIPT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "compositor", "tools", "tv-camera-rate-probe.py")


class SelfTest(unittest.TestCase):
    def test_the_probes_readers_and_verdicts_read_the_set_s_text(self):
        spec = importlib.util.spec_from_file_location("camera_rate_probe", SCRIPT)
        probe = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(probe)
        with contextlib.redirect_stdout(io.StringIO()) as out:
            probe.selftest()
        self.assertEqual(out.getvalue().strip(), "selftest ok")


if __name__ == "__main__":
    unittest.main()
