"""Run with: cd tests && python3 -m unittest test_rust_system   (needs rustui/target/debug/lumatv-ui and Mesa; skipped without them)

The Rust shell's system layer (rustui/ui/src/system/: alerts, overlay cards, the camera pop-up, the capture mark, the screensaver,
the layout editor) against the stand-in of the compositor's UI channel (tests/qmlhost/channel.py) and the helper's fake, on the cases
the QML tests state (test_notify_panel, test_overlays, test_camera_popup, test_capture_ui, test_saver_own, test_volume_osd), and
the layout and contrast lint of what is drawn at 100 % and 200 % text.
"""
import json
import os
import re
import socket
import subprocess
import sys
import tempfile
import threading
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, '..'))
sys.path.insert(0, os.path.join(HERE, 'qmlhost'))
BIN = os.path.join(ROOT, 'rustui', 'target', 'debug', 'lumatv-ui')
MESA = '/usr/lib/x86_64-linux-gnu/libEGL.so.1'
MODULE = os.path.join(ROOT, 'payload', 'compositor', 'lumatv')
HOME = {"app": "com.webos.app.home", "home": True}
APP = {"app": "netflix", "home": False}

ALERT_TAG = 0x53590001
INPUT_TAG = 0x53590002
PIN_TAG = 0x53590003
LAUNCH = {"serviceURI": "luna://com.webos.applicationManager", "serviceMethod": "/launch", "launchParams": {"id": "com.webos.app.hdmi3"}}


def render_dir():
    path = os.environ.get('RUST_SYSTEM_RENDERS')
    if path:
        os.makedirs(path, exist_ok=True)
    return path


@unittest.skipUnless(os.path.exists(BIN) and os.path.exists(MESA), "needs rustui/target/debug/lumatv-ui and Mesa")
class RustSystem(unittest.TestCase):
    def host(self, steps, wait, kinds, extra=(), helper=None):
        """Runs the client against the stand-in; `steps` are (ms, fn(channel)) done on the channel's side."""
        import channel as ch
        token = 'offline-token'
        data = tempfile.mkdtemp(prefix='rustsystem-')
        token_path = os.path.join(data, 'lumatv-ui.token')
        with open(token_path, 'w') as f:
            f.write(token + '\n')
        with socket.socket() as s:
            s.bind(('127.0.0.1', 0))
            port = s.getsockname()[1]
        chan = ch.Channel(token)
        srv = ch.serve(chan, port, say=lambda line: None)

        def run_steps():
            # the steps count from the client's first report, not from this thread's start: a loaded machine takes seconds to start the process
            give_up = time.time() + 120
            while not chan.reports and time.time() < give_up:
                time.sleep(0.01)
            t0 = time.time()
            for ms, fn in sorted(steps, key=lambda s: s[0]):
                time.sleep(max(0.0, t0 + ms / 1000.0 - time.time()))
                fn(chan)
        threading.Thread(target=run_steps, daemon=True).start()
        cmd = [BIN, '--host', '--port', str(port), '--token-file', token_path, '--helper-port', '1', '--wait', str(wait),
               '--data-dir', data, '--module-dir', MODULE, '--shm', os.path.join(data, 'shm'), '--kinds', kinds, *extra]
        env = dict(os.environ, LIBGL_ALWAYS_SOFTWARE='1')
        r = subprocess.run(cmd, env=env, capture_output=True, text=True, timeout=int(os.environ.get('HOST_TIMEOUT', '240')))
        srv.shutdown()
        return r.returncode, r.stdout + r.stderr, chan, data

    def reports(self, out):
        return [json.loads(m.group(1)) for m in re.finditer(r"^\[report\] (.*)$", out, re.M)]

    def evals(self, out):
        return [json.loads(m.group(1)) for m in re.finditer(r"^\[eval\] (.*)$", out, re.M)]

    def lunas(self, out):
        return [json.loads(m.group(1)) for m in re.finditer(r"^\[luna\] (.*)$", out, re.M)]

    def calls(self, out):
        """What the shell asked of the services besides subscribing to its feeds."""
        return [(c['uri'], c['payload']) for c in self.lunas(out) if not c['subscribe']]

    def feed(self, tag, doc):
        return lambda c: c.push('luna', {'tag': tag, 'payload': doc})

    def tag_of(self, chan, suffix):
        for call in chan.luna:
            if call['uri'].endswith(suffix):
                return call['tag']
        self.fail("the shell did not subscribe to %s: %s" % (suffix, [c['uri'] for c in chan.luna]))

    # ---- the alert, the input popup and the PIN prompt (test_notify_panel.py)

    def test_an_alert_takes_the_keys_its_button_runs_the_attached_action_and_back_runs_the_close_action(self):
        alert = {"alertAction": "open", "alertInfo": {"sourceId": "com.webos.app.home", "title": "Uninstall Crunchyroll?", "message": "The app is removed.",
                                                      "modal": True, "timeout": 0, "timestamp": 1,
                                                      "buttons": [{"label": "Keep it", "focus": True}, {"label": "Uninstall", "action": LAUNCH}],
                                                      "onCloseAction": {"serviceURI": "luna://com.webos.notification", "serviceMethod": "/closeAlert", "launchParams": {"timestamp": 1}}}}
        warning = json.loads(json.dumps(alert))
        warning["alertInfo"].update({"timestamp": 2, "buttons": [], "type": "warning"})
        shot = os.path.join(tempfile.gettempdir(), 'rustsystem-alert-%d.png' % os.getpid())
        rc, out, chan, data = self.host([
            (300, lambda c: (c.push('foreground', APP), c.push('screen', {"on": True}))),
            (700, self.feed(ALERT_TAG, alert)),
        ], wait=3600, kinds='alert', extra=[
            '--eval', '1500:report', '--eval', '1500:geometry', '--snap', '1600:' + shot,
            '--key', '1700:right', '--key', '1800:ok',
            '--eval', '2000:report',
            '--luna', '2200:%d:%s' % (ALERT_TAG, json.dumps(warning)), '--eval', '2500:geometry', '--key', '2700:back', '--eval', '3200:report'])
        self.assertEqual(rc, 0, out[-3000:])
        evals = self.evals(out)
        self.assertEqual(len(evals), 5, out[-3000:])
        self.assertTrue(evals[0]['wants'] and evals[0]['state']['alerts'] == 1, "the alert has the keys: %s" % evals[0])
        self.assertEqual(evals[0]['takes'], [])
        g = evals[1]['alert']
        self.assertEqual((g['kind'], g['focus']), ('alert', 0), "the button with focus: true starts focused")
        self.assertAlmostEqual(g['sheet']['x'] + g['sheet']['w'] / 2, 960, delta=1)
        self.assertFalse(evals[2]['wants'], "answered: the keys are the app's again")
        self.assertIn(['luna://com.webos.applicationManager/launch', {"id": "com.webos.app.hdmi3"}], [list(c) for c in self.calls(out)], "OK on Uninstall ran its action")
        self.assertEqual(len(evals[3]['alert']['buttons']), 1, "a warning without buttons is drawn with an OK, never filtered")
        self.assertIn(['luna://com.webos.notification/closeAlert', {"timestamp": 1}], [list(c) for c in self.calls(out)], "Back ran the close action")
        self.assertFalse(evals[4]['wants'])
        uris = {c['uri'] for c in chan.luna}
        for feed in ('getAlertNotification', 'getInputAlertNotification', 'getPincodePromptNotification'):
            self.assertIn('luna://com.webos.notification/' + feed, uris, "the feed is subscribed through the channel")
        self.assertTrue(os.path.exists(shot), "the render was written")
        try:
            from PIL import Image
        except ImportError:
            Image = None
        if Image:
            im = Image.open(shot).convert('RGB')
            self.assertNotEqual(im.getpixel((700, 440)), im.getpixel((20, 20)), "the sheet is drawn over the scrim")
        os.remove(shot)

    def test_the_input_popup_switches_or_stays_and_the_pin_prompt_relays_four_digits_or_cancels(self):
        popup = {"alertAction": "open", "alertInfo": {"sourceId": "com.webos.app.hdmi3", "timeout": 15, "timestamp": 2, "portName": "HDMI 3", "action": LAUNCH, "appId": "com.webos.app.hdmi3"}}
        pin = {"pincodePromptAction": "open", "pincodePromptInfo": {"appId": "com.webos.app.hdmi3", "title": "Enter the TV PIN", "message": "HDMI 3 is locked.",
                                                                    "promptType": "parental", "retry": 0, "timestamp": 3}}
        keys = []
        for ms, k in [(1000, 'right'), (1100, 'ok')]:
            keys += ['--key', '%d:%s' % (ms, k)]
        for ms, k in [(2400, '1'), (2500, '2'), (2600, '3'), (2700, '4'), (2800, 'down'), (2900, 'down'), (3000, 'down'), (3100, 'down'), (3200, 'ok'), (3700, 'back')]:
            keys += ['--key', '%d:%s' % (ms, k)]
        rc, out, chan, data = self.host([
            (300, lambda c: (c.push('foreground', APP), c.push('screen', {"on": True}))),
            (700, self.feed(INPUT_TAG, popup)),
            (1700, self.feed(PIN_TAG, pin)),
        ], wait=4300, kinds='alert', extra=['--eval', '900:geometry', '--eval', '1500:report', '--eval', '2300:geometry', '--eval', '3400:report', '--eval', '3900:report', *keys])
        self.assertEqual(rc, 0, out[-3000:])
        evals = self.evals(out)
        self.assertEqual(evals[0]['alert']['kind'], 'input')
        right_edge = evals[0]['alert']['sheet']['x'] + evals[0]['alert']['sheet']['w']
        self.assertEqual(right_edge, 1920 - 24, "at the toasts' corner")
        self.assertFalse(evals[1]['wants'], "Stay here closed it and launched nothing")
        self.assertEqual(evals[2]['alert']['kind'], 'pin')
        self.assertEqual(len(evals[2]['alert']['buttons']), 13)
        calls = self.calls(out)
        self.assertEqual(calls, [('luna://com.webos.notification/closePincodePrompt', {"pincode": "1234", "closeType": "relay", "mode": "", "appId": "com.webos.app.hdmi3"}),
                                 ('luna://com.webos.notification/closePincodePrompt', {"closeType": "close"})], "OK with four digits relays them, Back with none typed cancels")
        self.assertTrue(evals[3]['wants'], "the prompt keeps the keys until it is answered")
        self.assertFalse(evals[4]['wants'])

    def test_what_an_alert_draws_reads_clean_at_both_text_sizes(self):
        alert = {"alertAction": "open", "alertInfo": {"sourceId": "s", "title": "Uninstall Crunchyroll?", "message": "The app and its sign-in on this TV are removed. Downloads in the app are lost. It can be installed again from the LG Content Store.",
                                                      "modal": True, "timeout": 0, "timestamp": 1, "buttons": [{"label": "Keep it", "focus": True}, {"label": "Uninstall", "danger": True}]}}
        for scale in ('1', '2'):
            for mode in ('dark', 'light'):
                rc, out, chan, data = self.host([(300, lambda c: (c.push('foreground', APP), c.push('screen', {"on": True}))), (500, self.feed(ALERT_TAG, alert))],
                                                wait=1800, kinds='alert', extra=['--text-scale', scale, '--mode', mode, '--eval', '1400:lint'])
                self.assertEqual(rc, 0, out[-3000:])
                self.assertEqual(self.evals(out)[-1], [], "%s at %s: %s" % (mode, scale, self.evals(out)[-1]))


if __name__ == "__main__":
    unittest.main()
