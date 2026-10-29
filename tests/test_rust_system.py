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

SAVER_UP = {"on": True, "state": "Screen Saver"}
SAVER_DOWN = {"on": True, "state": "Active"}
# the first spot of the saver's block is fixed by the clock: a render is the same on every run
CLOCK = ['--clock', '2026-10-29T10:08:30']
WEATHER = {"state": "fresh", "doc": {"display": {"temp": "61\u00b0", "cond": "Clear", "nowKind": "moon"}}}
SAVER_TOPIC = {"items": [{"id": "door", "source": "api", "glyph": "lock", "text": "Front door", "value": "Locked", "priority": 2, "expiresAt": 4102444800000, "tone": "plain", "updatedAt": 1},
                         {"id": "garage", "source": "api", "glyph": "warehouse", "text": "Garage", "value": "Open", "priority": 1, "expiresAt": 4102444800000, "tone": "warning", "updatedAt": 2}],
               "rules": {}, "lines": [{"id": "camera.e1", "class": "camera.still", "glyph": "user", "title": "Driveway", "text": "Person seen", "at": 1793282670000, "expiresAt": 4102444800000}]}
VOTE_TAGS = (0x53590010, 0x53590011)


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

    # ---- the screensaver (test_saver_own.py)

    def saver_extra(self, *more):
        helper = ['--helper', '150:weather:' + json.dumps(WEATHER), '--helper', '200:saver:' + json.dumps(SAVER_TOPIC)]
        return [*CLOCK, *helper, *more]

    def test_the_saver_is_black_with_a_dim_clock_under_the_cap_has_the_keys_and_a_key_asks_the_tv_to_turn_the_screen_on(self):
        shot = os.path.join(tempfile.gettempdir(), 'rustsystem-saver-%d.png' % os.getpid())
        rc, out, chan, data = self.host([(300, lambda c: c.push('foreground', HOME))], wait=3600, kinds='saver', extra=self.saver_extra(
            '--eval', '1000:report', '--eval', '1000:state', '--snap', '1900:' + shot,
            '--key', '2200:ok', '--eval', '2600:report',
            '--helper', '2800:screen:' + json.dumps(SAVER_DOWN), '--eval', '3300:report', '--helper', '400:screen:' + json.dumps(SAVER_UP)))
        self.assertEqual(rc, 0, out[-3000:])
        evals = self.evals(out)
        self.assertEqual(len(evals), 4, out[-3000:])
        up = evals[0]
        self.assertTrue(up['wants'] and up['live'] and up['state']['saver'], "the saver is up, on screen and has the keys: %s" % up)
        self.assertEqual(evals[1]['saverState']['up'], True)
        calls = self.calls(out)
        self.assertIn(('luna://com.webos.service.tvpower/power/turnOnScreen', {}), calls, "a key asks the TV to turn the screen on (LG's own state is the saver's)")
        self.assertTrue(evals[2]['wants'], "LG's state is still the saver until the helper says Active")
        self.assertFalse(evals[3]['wants'] or evals[3]['state']['saver'], "Active again: the keys are Home's")
        for uri in ('registerScreenSaverRequest',):
            self.assertTrue(any(c['uri'].endswith(uri) for c in chan.luna), "Home in front: the saver is on tvpower's lists")
        try:
            from PIL import Image
        except ImportError:
            Image = None
        self.assertTrue(os.path.exists(shot), "the render was written")
        if Image:
            im = Image.open(shot).convert('RGB')
            self.assertEqual(im.size, (1920, 1080))
            px = im.getdata()
            top = max(max(p) for p in px)
            self.assertLessEqual(top, 97, "no pixel of the saver is brighter than 0.38 of white")
            self.assertGreater(top, 40, "and it shows something")
            mean = sum(sum(p) for p in px) / (3.0 * 255 * len(px))
            self.assertLess(mean, 0.005, "a fraction of a percent of the panel's light: %.4f" % mean)
            self.assertEqual(im.getpixel((10, 10)), (0, 0, 0), "black but for the block")
        os.remove(shot)

    def test_a_saver_that_tvpower_left_to_lumatv_ends_on_a_key_without_asking_the_tv_for_anything_and_a_volume_step_ends_it_too(self):
        def answer(c):
            c.push('luna', {'tag': VOTE_TAGS[0], 'payload': {"returnValue": True, "timestamp": 77}})
        audio = lambda v: {"volumeStatus": {"volume": v, "muteStatus": False, "soundOutput": "tv_speaker", "maxVolume": 100}}
        rc, out, chan, data = self.host([
            (300, lambda c: c.push('foreground', HOME)), (700, answer),
            (1500, lambda c: c.push('audio', audio(20))), (2300, lambda c: c.push('audio', audio(21))),
        ], wait=3300, kinds='saver', extra=['--eval', '1200:report', '--eval', '1800:report', '--eval', '2000:report', '--eval', '2700:report', '--key', '1900:right'])
        self.assertEqual(rc, 0, out[-3000:])
        evals = self.evals(out)
        self.assertEqual(len(evals), 4, out[-3000:])
        self.assertTrue(evals[0]['state']['saver'] and evals[0]['wants'], "tvpower asked, the saver said no and is up: %s" % evals[0])
        refusals = [c for c in self.calls(out) if c[0].endswith('/responseScreenSaverRequest')]
        self.assertEqual([c[1]['ack'] for c in refusals], [False], "no: LG's saver is not started")
        self.assertTrue(evals[1]['state']['saver'], "the first reading of the volume is only where it was")
        self.assertFalse(evals[2]['state']['saver'], "a key ends it")
        self.assertFalse([c for c in self.calls(out) if c[0].endswith('/turnOnScreen')], "nothing to ask: the state was never the saver's")
        # a saver that is up again ends on a step of the volume (the bar it would show is under it)
        rc, out, chan, data = self.host([
            (300, lambda c: c.push('foreground', HOME)), (700, answer), (1500, lambda c: c.push('audio', audio(20))), (1900, lambda c: c.push('audio', audio(21))),
        ], wait=2800, kinds='saver', extra=['--eval', '1700:report', '--eval', '2400:report'])
        self.assertEqual(rc, 0, out[-3000:])
        evals = self.evals(out)
        self.assertTrue(evals[0]['state']['saver'])
        self.assertFalse(evals[1]['state']['saver'], "a volume key ended the saver")

    def test_what_the_saver_draws_stays_under_the_cap_and_reads_clean_at_both_text_sizes_in_both_materials(self):
        try:
            from PIL import Image
        except ImportError:
            Image = None
        for scale in ('1', '2'):
            for mode in ('dark', 'light'):
                shot = os.path.join(tempfile.gettempdir(), 'rustsystem-saver-%s-%s-%d.png' % (scale, mode, os.getpid()))
                rc, out, chan, data = self.host([(300, lambda c: c.push('foreground', HOME))], wait=2600, kinds='saver', extra=self.saver_extra(
                    '--text-scale', scale, '--mode', mode, '--helper', '400:screen:' + json.dumps(SAVER_UP), '--eval', '1700:lint', '--eval', '1700:atlas', '--snap', '1800:' + shot))
                self.assertEqual(rc, 0, out[-3000:])
                evals = self.evals(out)
                self.assertEqual(evals[0], [], "%s at %s: %s" % (mode, scale, evals[0]))
                self.assertEqual(evals[1]['pictures'], 0, "every word is glyph quads of the atlas at %s" % scale)
                if Image:
                    im = Image.open(shot).convert('RGB')
                    self.assertLessEqual(max(max(p) for p in im.getdata()), 97, "%s at %s: under the cap" % (mode, scale))
                    path = render_dir()
                    if path:
                        import shutil
                        folder = os.path.join(path, 'system')
                        os.makedirs(folder, exist_ok=True)
                        shutil.copy(shot, os.path.join(folder, 'saver-%s%s.png' % (mode, '' if scale == '1' else '-text200')))
                os.remove(shot)


    # ---- the camera pop-up (test_camera_popup.py, cameras-ui-spec.md "The pop-up")

    def popup_run(self, kinds='popup', scale=1.0, mode='dark', foreground=APP, until_ms=120000, wait=3200, **record):
        import rust_cameras_scenes as sc
        now = int(time.time() * 1000)
        r = sc.Run(scale=scale, mode=mode, wait=wait, backdrop='game', kinds=kinds)
        r.channel(200, 'foreground', foreground).channel(220, 'screen', {'on': True})
        r.helper(250, 'cameras', sc.cameras_topic(3, per={0: {'name': 'Front door'}}))
        doc = {'id': 41, 'event': 'e41', 'camera': '300', 'name': 'Front door', 'kind': 'person', 'at': now, 'untilMs': now + until_ms, 'thumbnail': False, 'seq': 1}
        doc.update(record)
        # the helper holds the record as the real one does (a poll that found it gone would clear the card)
        r.setup(lambda chan, st: st.topics.__setitem__('cameras.popup', doc))
        r.helper(600, 'cameras.popup', doc)
        return sc, r

    def test_a_camera_event_is_a_card_in_the_corner_that_asks_for_stills_takes_ok_and_back_only_after_600_ms_and_tells_the_helper(self):
        sc, r = self.popup_run()
        r.eval(900, 'report').eval(1500, 'report').eval(1500, 'geometry')
        r.press(1800, 'back').eval(2200, 'report')
        r.press(2400, 'left').eval(2600, 'report')
        r.wait = 3000
        res = sc.run(r)
        self.assertEqual(res['rc'], 0, res['out'][-3000:])
        reports = [e for e in res['evals'] if isinstance(e, dict) and 'wants' in e]
        self.assertEqual(reports[0]['takes'], [], "just arrived: the keys are the app's for the first 600 ms")
        self.assertEqual(reports[1]['takes'], ['ok', 'back'], "then OK and Back are the card's, and nothing else")
        self.assertFalse(reports[1]['wants'], "the arrows stay the app's")
        geo = [e for e in res['evals'] if isinstance(e, dict) and 'popup' in e][0]['popup']
        self.assertEqual(geo['x'] + geo['w'], 1856, "flush where Home's status bar ends: %s" % geo)
        self.assertEqual(geo['y'], 134)
        asked = res['helper'].frames
        self.assertTrue(asked and all((c, w, h) == ('300', geo['picture']['w'], geo['picture']['h']) for c, w, h, _ in asked), "stills at the picture's size: %s" % asked[:3])
        self.assertIn(['/v1/cameras/popup/clear', {}], res['helper'].posts, "Back clears it and the helper is told")
        self.assertEqual(reports[2]['takes'], [], "gone: no keys")

    def test_ok_on_the_card_opens_the_cameras_full_view_when_the_app_is_the_rust_shells(self):
        sc, r = self.popup_run(kinds='popup,cameras')
        r.press(1700, 'ok').eval(2300, 'state')
        r.wait = 2800
        res = sc.run(r)
        self.assertEqual(res['rc'], 0, res['out'][-3000:])
        state = [e for e in res['evals'] if isinstance(e, dict) and 'cameras' in e and 'kinds' in e][-1]
        self.assertEqual(state['cameras']['screen']['mode'], 'full', "OK expands to the camera's full view: %s" % state['cameras'])
        self.assertIn(['/v1/cameras/popup/clear', {}], res['helper'].posts)

    def test_a_card_waits_for_an_alert_and_with_over_apps_off_stays_off_an_app_and_shows_on_home(self):
        sc, r = self.popup_run(kinds='popup', overApps=False)
        r.eval(1500, 'state')
        r.channel(1700, 'foreground', HOME).eval(2400, 'state')
        r.wait = 2800
        res = sc.run(r)
        self.assertEqual(res['rc'], 0, res['out'][-3000:])
        states = [e for e in res['evals'] if isinstance(e, dict) and 'kinds' in e]
        self.assertFalse(states[0]['popup']['up'], "an app is in front and the owner's switch says not over apps")
        self.assertTrue(states[1]['popup']['up'], "Home: it shows, still in date")

    def test_what_the_camera_card_draws_reads_clean_at_both_text_sizes_in_both_materials(self):
        try:
            from PIL import Image
        except ImportError:
            Image = None
        for scale in (1.0, 2.0):
            for mode in ('dark', 'light'):
                sc, r = self.popup_run(scale=scale, mode=mode, kind='doorbell', label='')
                shot = os.path.join(tempfile.gettempdir(), 'rustsystem-popup-%s-%s-%d.png' % (scale, mode, os.getpid()))
                r.eval(1800, 'lint').eval(1800, 'atlas').snap(1900, shot)
                r.wait = 2300
                res = sc.run(r)
                self.assertEqual(res['rc'], 0, res['out'][-3000:])
                self.assertEqual(res['evals'][0], [], "%s at %s: %s" % (mode, scale, res['evals'][0]))
                self.assertEqual(res['evals'][1]['pictures'], 0, "every word is glyph quads of the atlas")
                if Image and os.path.exists(shot):
                    im = Image.open(shot).convert('RGB')
                    folder = render_dir()
                    if folder:
                        import shutil
                        out = os.path.join(folder, 'system')
                        os.makedirs(out, exist_ok=True)
                        im.resize((1280, 720)).save(os.path.join(out, 'popup-%s%s.jpg' % (mode, '' if scale == 1.0 else '-text200')), 'JPEG', quality=80)
                if os.path.exists(shot):
                    os.remove(shot)

    # ---- the overlay cards (test_overlays.py)

    def cards_run(self, scale=1.0, mode='dark', foreground=APP, wait=4200, extra_kinds='', notes=None, backdrop='game'):
        import rust_cameras_scenes as sc
        r = sc.Run(scale=scale, mode=mode, wait=wait, backdrop=backdrop, kinds='cards' + extra_kinds)
        r.channel(200, 'foreground', foreground).channel(220, 'screen', {'on': True})
        r.helper(250, 'plugins', {'plugins': [{'id': 'kitchen', 'name': 'Kitchen Show', 'state': 'connected'}], 'firstParty': []})
        r.helper(300, 'plugin.kitchen.notifications', {'items': []})
        items = notes if notes is not None else [
            {'id': 'pasta', 'channel': 'timer', 'channelTitle': 'Kitchen Show', 'channelIcon': 'base.icons/timer', 'revision': 1, 'receivedAt': 1, 'expiresAt': 0,
             'post': {'title': 'Pasta', 'body': "Time's up", 'kind': 'timer', 'actions': [{'id': 'stop', 'label': 'Stop', 'action': 'stop'}]}},
            {'id': 'door', 'channel': 'doorbell', 'channelTitle': 'Kitchen Show', 'channelIcon': 'base.icons/doorbell', 'revision': 1, 'receivedAt': 2, 'expiresAt': 0,
             'post': {'title': 'Front door', 'body': 'Someone is at the door', 'kind': 'doorbell', 'actions': []}}]
        # the hub marks what is there 1.5 s after the helper came up: news arrives after that
        r.helper(2200, 'plugin.kitchen.notifications', {'items': items})
        return sc, r

    def test_a_notification_is_a_card_whose_keys_are_the_cards_after_600_ms_ok_runs_its_action_and_back_dismisses(self):
        sc, r = self.cards_run()
        r.eval(2400, 'report').eval(3000, 'report').eval(3000, 'geometry')
        r.press(3200, 'ok').eval(3600, 'report')
        r.press(3700, 'back').eval(4000, 'report')
        r.wait = 4400
        res = sc.run(r)
        self.assertEqual(res['rc'], 0, res['out'][-3000:])
        reports = [e for e in res['evals'] if isinstance(e, dict) and 'wants' in e]
        self.assertEqual(reports[0]['takes'], [], "it just arrived: a press made as it did was the app's")
        self.assertEqual(reports[1]['takes'], ['ok', 'back'])
        cards = [e for e in res['evals'] if isinstance(e, dict) and 'cards' in e][0]['cards']
        self.assertEqual(len(cards), 2, cards)
        self.assertEqual(cards[0]['x'] + cards[0]['w'], 1856)
        self.assertEqual(cards[1]['y'], cards[0]['y'] + cards[0]['h'] + 16)
        self.assertIn(['/v1/plugins/kitchen/actions/stop', {}], res['helper'].posts, "OK ran the record's first action")
        self.assertEqual(reports[3]['takes'], [], "both cards dismissed: the keys are the app's again")

    def test_what_the_cards_draw_reads_clean_at_both_text_sizes_in_both_materials_and_over_the_camera_card(self):
        try:
            from PIL import Image
        except ImportError:
            Image = None
        for scale in (1.0, 2.0):
            for mode in ('dark', 'light'):
                sc, r = self.cards_run(scale=scale, mode=mode, extra_kinds=',popup')
                now = int(time.time() * 1000)
                r.helper(2000, 'cameras', sc.cameras_topic(3, per={0: {'name': 'Front door'}}))
                doc = {'id': 41, 'event': 'e41', 'camera': '300', 'name': 'Front door', 'kind': 'person', 'at': now, 'untilMs': now + 120000, 'thumbnail': False}
                r.setup(lambda chan, st, doc=doc: st.topics.__setitem__('cameras.popup', doc))
                r.helper(2100, 'cameras.popup', doc)
                shot = os.path.join(tempfile.gettempdir(), 'rustsystem-cards-%s-%s-%d.png' % (scale, mode, os.getpid()))
                r.eval(3600, 'lint').eval(3600, 'geometry').snap(3700, shot)
                r.wait = 4100
                res = sc.run(r)
                self.assertEqual(res['rc'], 0, res['out'][-3000:])
                self.assertEqual(res['evals'][0], [], "%s at %s: %s" % (mode, scale, res['evals'][0]))
                geo = res['evals'][1]
                self.assertTrue(geo['popup'], "the camera card is up: %s" % geo)
                if scale == 1.0:
                    self.assertTrue(geo['cards'], "a card under it: %s" % geo)
                    self.assertGreaterEqual(geo['cards'][0]['y'], geo['popup']['y'] + geo['popup']['h'], "the cards go under the camera's card: %s" % geo)
                else:
                    self.assertEqual(geo['cards'], [], "at 200 %% the card would end below the limit beside the window, so it waits: %s" % geo)
                if Image and os.path.exists(shot):
                    im = Image.open(shot).convert('RGB')
                    folder = render_dir()
                    if folder:
                        out = os.path.join(folder, 'system')
                        os.makedirs(out, exist_ok=True)
                        im.resize((1280, 720)).save(os.path.join(out, 'cards-%s%s.jpg' % (mode, '' if scale == 1.0 else '-text200')), 'JPEG', quality=80)
                if os.path.exists(shot):
                    os.remove(shot)


if __name__ == "__main__":
    unittest.main()
