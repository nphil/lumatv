"""Run with: python3 -m unittest tests.test_logos   (needs Xvfb and PyQt5; skipped without them)

The power-on and power-off logos and their system-UI windows through the offline QML host
(tests/qmlhost): every option loads without a warning, the power-off window answers tvpowerd
exactly once whatever happens (finished, logo missing, logo broken), and the power-on window
exits on close. Stand-ins for LG's window and TextFile are in tests/qmlhost/stubs; timing is the
set's to say.
"""
import os
import re
import subprocess
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
HOST = os.path.join(HERE, "qmlhost", "qmlhost.py")


sys.path.insert(0, HERE)
from xdisplay import DISPLAY, HAVE_QT  # noqa: E402


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class Logos(unittest.TestCase):
    def host(self, qml, *args):
        r = subprocess.run([sys.executable, HOST, qml, *args], env=dict(os.environ, DISPLAY=DISPLAY),
                           capture_output=True, text=True, timeout=90)
        return r.returncode, r.stdout + r.stderr

    def test_every_option_loads_at_start_middle_and_end(self):
        for qml in ("LogoOn", "LogoOn2", "LogoOn3", "LogoOff", "LogoOff2", "LogoOff3"):
            for progress in ("0", "0.5", "1"):
                rc, out = self.host(f"logo/{qml}.qml", "--wait", "500", "--prop", f"progress={progress}")
                self.assertEqual(rc, 0, f"{qml} at {progress}: {out[-600:]}")

    def test_power_off_answers_once_when_the_logo_finishes(self):
        for choice in (1, 2, 3):
            rc, out = self.host("systemui/MasterBrand/MasterBrand.qml", "--wait", "2200", "--theme", f'{{"logo":{{"off":{choice}}}}}',
                                "--fire", '100:launch:{"reason":"powerOff"}')
            self.assertEqual(rc, 0, out[-600:])
            self.assertEqual(out.count("responseOffLogoRequest true"), 1, out[-600:])

    def test_power_off_answers_at_once_when_the_logo_is_missing_or_broken(self):
        for remove in ("logo", "logo/LogoBase.qml"):
            rc, out = self.host("systemui/MasterBrand/MasterBrand.qml", "--wait", "800", "--remove", remove,
                                "--fire", '100:launch:{"reason":"powerOff"}')
            self.assertEqual(out.count("responseOffLogoRequest true"), 1, out[-600:])
            self.assertIn("did not load", out)

    def test_power_off_preload_launch_only_hides_the_window(self):
        rc, out = self.host("systemui/MasterBrand/MasterBrand.qml", "--wait", "800", "--fire", '100:launch:{"reason":"preload"}')
        self.assertEqual(rc, 0, out[-600:])
        self.assertIn("[host] hide", out)
        self.assertNotIn("responseOffLogoRequest", out)

    def test_power_on_fades_out_and_exits_on_close_and_at_once_on_suspend(self):
        rc, out = self.host("systemui/BootLogo/BootLogo.qml", "--wait", "1500", "--fire", '100:launch:{"params":{"type":"image","fadeOutDuration":300}}',
                            "--fire", "600:close:{}")
        self.assertEqual(rc, 0, out[-600:])
        self.assertEqual(len(re.findall(r"\[host\] exit", out)), 1, out[-600:])
        rc, out = self.host("systemui/BootLogo/BootLogo.qml", "--wait", "600", "--fire", '100:launch:{"params":{"type":"image"}}',
                            "--fire", '300:close:{"reason":"powerSuspend"}')
        self.assertEqual(len(re.findall(r"\[host\] exit", out)), 1, out[-600:])

    def test_power_on_with_a_missing_logo_stays_black_and_still_exits(self):
        rc, out = self.host("systemui/BootLogo/BootLogo.qml", "--wait", "900", "--remove", "logo",
                            "--fire", '100:launch:{"params":{"type":"image","fadeOutDuration":200}}', "--fire", "300:close:{}")
        self.assertIn("did not load", out)
        self.assertEqual(len(re.findall(r"\[host\] exit", out)), 1, out[-600:])


if __name__ == "__main__":
    unittest.main()
