"""Run with: cd tests && python3 -m unittest test_shell_client   (needs Xvfb and PyQt5; skipped without them)

The shell client (compositor/shell/Shell.qml, LumaTV's QML as LumaTV's compositor's shell) against a stand-in of the
compositor's UI channel (tests/qmlhost/shellhost.py): it reads the state and polls, says what it can show once Home is
in front, opens and closes the quick panel on Home from a `show` and a Back key, shows the overlay's panel over an app
and wants the keys while it is up, draws the volume display from the compositor's audio topic, draws a toast and an
alert from the notification feeds it subscribed to through the channel, and makes its luna calls through the channel.
"""
import json
import os
import re
import subprocess
import sys
import unittest

from xdisplay import DISPLAY, HAVE_QT

HERE = os.path.dirname(os.path.abspath(__file__))
HOST = os.path.join(HERE, "qmlhost", "shellhost.py")
HOME = '{"app":"com.webos.app.home","home":true}'
APP = '{"app":"netflix","home":false}'
AUDIO = '{"volumeStatus":{"volume":31,"muteStatus":false,"soundOutput":"tv_speaker","maxVolume":100,"cause":"volumeUp"}}'
TOAST = '{"sourceId":"lg","message":"Remote battery low","timeout":8}'
ALERT = '{"alertAction":"open","alertInfo":{"sourceId":"x","title":"Thermal","message":"The TV is hot","timestamp":5,"buttons":[{"label":"OK"}]}}'


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class ShellClient(unittest.TestCase):
    def host(self, *args, wait):
        # The offscreen platform draws with GLX on the test display: the xcb plugin's libraries are not on every workstation.
        env = dict(os.environ, DISPLAY=DISPLAY, QT_QPA_PLATFORM="offscreen")
        cmd = [sys.executable, HOST, "--wait", str(wait), *args]
        r = subprocess.run(cmd, env=env, capture_output=True, text=True, timeout=int(os.environ.get("HOST_TIMEOUT", "240")))
        return r.returncode, r.stdout + r.stderr

    def evals(self, out):
        return [json.loads(m.group(1)) for m in re.finditer(r"\[eval\] \('(.*)', (?:True|False)\)$", out, re.M)]

    def reports(self, out):
        return [json.loads(m.group(1)) for m in re.finditer(r"^\[report\] (.*)$", out, re.M)]

    def luna(self, out):
        return [json.loads(m.group(1)) for m in re.finditer(r"^\[luna\] (.*)$", out, re.M)]

    def test_home_in_front_quick_panel_from_a_show_and_closed_by_back(self):
        rc, out = self.host(
            "--push", "500:foreground:" + HOME, "--push", "500:screen:{\"on\":true}",
            "--show", "4000:quick:warm", "--show", "4200:quick", "--eval", "5000:JSON.stringify(home.state())",
            "--key", "5200:home:412", "--eval", "5800:JSON.stringify(home.state())",
            "--key", "6000:home:106", "--eval", "6500:JSON.stringify(home.state())", wait=7000)
        evals = self.evals(out)
        self.assertEqual(len(evals), 3, out[-3000:])
        self.assertEqual(evals[0]["layer"], "quick", "the show opened the quick panel on Home")
        self.assertTrue(evals[0]["hostActive"])
        self.assertEqual(evals[1]["layer"], "", "Back closed it")
        self.assertEqual(evals[2]["keys"], 4, "two presses and two releases reached the shell")
        self.assertTrue(evals[2]["link"]["up"])
        reports = self.reports(out)
        self.assertTrue(any(set(r["can"]) >= {"quick", "picker", "drawer"} for r in reports), "the client said what it can show: %s" % [r["can"] for r in reports])
        self.assertTrue(all(r["wants"] is False for r in reports), "nothing in the system layer wanted the keys on Home")
        calls = self.luna(out)
        uris = {c["uri"] for c in calls}
        for u in ("luna://com.webos.notification/getToastNotification", "luna://com.webos.notification/getAlertNotification",
                  "luna://com.webos.notification/getPincodePromptNotification", "luna://com.webos.notification/getInputAlertNotification",
                  "luna://com.webos.service.applicationmanager/listLaunchPoints"):
            self.assertIn(u, uris)
        self.assertTrue(all(c["subscribe"] for c in calls if c["uri"].endswith("Notification")), "the feeds are subscriptions")
        self.assertEqual(rc, 0, "no warning of the shell's own:\n" + "\n".join(l for l in out.splitlines() if l.startswith("[warning]") or l.startswith("[critical]"))[-2000:])

    def test_over_an_app_the_system_layer_draws_the_panel_the_osd_a_toast_and_an_alert(self):
        rc, out = self.host(
            "--push", "500:foreground:" + HOME, "--push", "500:screen:{\"on\":true}",
            "--push", "7000:foreground:" + APP,
            # The overlay builds its panels 8 to 11.5 s after it started: the gear's panel is asked for after that.
            "--show", "13000:quick", "--eval", "13800:JSON.stringify([home.state().overlay, home.sysWants, sys.visible])",
            "--key", "14000:sys:412", "--eval", "14800:JSON.stringify([home.state().overlay, home.sysWants])",
            "--push", "15000:audio:" + AUDIO, "--eval", "15800:JSON.stringify([home.state().osd, sys.visible])",
            "--luna", "16000:/getToastNotification:" + TOAST, "--eval", "16800:JSON.stringify([home.state().toasts, sys.visible])",
            "--luna", "17000:/getAlertNotification:" + ALERT, "--eval", "17800:JSON.stringify([home.state().alert, home.sysWants])",
            "--key", "18000:sys:28", "--eval", "18800:JSON.stringify([home.state().alert, home.sysWants])", wait=19500)
        evals = self.evals(out)
        self.assertEqual(len(evals), 6, out[-3000:])
        self.assertEqual(evals[0], ["quick", True, True], "the overlay's quick panel is up over the app, the layer wants the keys and is mapped")
        self.assertEqual(evals[1], ["", False], "Back on the layer closed it")
        self.assertEqual(evals[2], [True, True], "the audio topic put the volume display up")
        self.assertEqual(evals[3], [1, True], "the toast came through the feed")
        self.assertEqual(evals[4], [True, True], "the alert is up and takes the keys")
        self.assertEqual(evals[5], [False, False], "OK answered the alert")
        reports = self.reports(out)
        self.assertTrue(any(r["wants"] for r in reports), "the layer's wanting the keys was reported")
        self.assertEqual(rc, 0, "no warning of the shell's own:\n" + "\n".join(l for l in out.splitlines() if l.startswith("[warning]") or l.startswith("[critical]"))[-2000:])

    def test_an_app_taking_the_front_ends_the_overlays_picker(self):
        # The Input key held opens the picker at its press and the Home Hub a second later: the picker over the app goes with the switch.
        hub = '{"app":"com.webos.app.homeconnect","home":false}'
        rc, out = self.host(
            "--push", "500:foreground:" + HOME, "--push", "500:screen:{\"on\":true}",
            "--push", "7000:foreground:" + APP,
            "--show", "13000:picker", "--eval", "13800:JSON.stringify([home.state().overlay, home.sysWants, sys.visible])",
            "--push", "14000:foreground:" + hub, "--eval", "14800:JSON.stringify([home.state().overlay, home.sysWants])", wait=15500)
        evals = self.evals(out)
        self.assertEqual(len(evals), 2, out[-3000:])
        self.assertEqual(evals[0], ["picker", True, True], "the picker is up over the app and wants the keys")
        self.assertEqual(evals[1], ["", False], "the Hub taking the front ended it")
        self.assertEqual(rc, 0, "no warning of the shell's own:\n" + "\n".join(l for l in out.splitlines() if l.startswith("[warning]") or l.startswith("[critical]"))[-2000:])


if __name__ == "__main__":
    unittest.main()
