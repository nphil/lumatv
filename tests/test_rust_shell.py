"""Run with: cd tests && python3 -m unittest test_rust_shell   (needs rustui/target/debug/lumatv-ui and Mesa; skipped without them)

The Rust shell client (rustui/ui, `lumatv-ui --host`: the shell's logic against the engine in its own process on Mesa, as the
compositor runs it) against the stand-in of the compositor's UI channel (tests/qmlhost/channel.py), on the cases
test_shell_client.py states for the system layer: it reads the state and polls, subscribes to the toast feed through the
channel, draws the volume display from the compositor's audio topic over an app and reports it, draws a toast from the feed,
and its frames are damage-only with the text as glyph quads.
"""
import json
import os
import re
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
HOME = {"app": "com.webos.app.home", "home": True}
APP = {"app": "netflix", "home": False}
AUDIO0 = {"volumeStatus": {"volume": 31, "muteStatus": False, "soundOutput": "tv_speaker", "maxVolume": 100, "cause": ""}}
AUDIO = {"volumeStatus": {"volume": 31, "muteStatus": False, "soundOutput": "tv_speaker", "maxVolume": 100, "cause": "volumeUp"}}
TOAST = {"sourceId": "lg", "message": "Remote battery low", "timeout": 8}


@unittest.skipUnless(os.path.exists(BIN) and os.path.exists(MESA), "needs rustui/target/debug/lumatv-ui and Mesa")
class RustShell(unittest.TestCase):
    def host(self, steps, wait, extra=()):
        """Runs the client against the stand-in; `steps` are (ms, fn(channel)) done on the channel's side."""
        import channel as ch
        token = 'offline-token'
        data = tempfile.mkdtemp(prefix='rustshell-')
        token_path = os.path.join(data, 'lumatv-ui.token')
        with open(token_path, 'w') as f:
            f.write(token + '\n')
        import socket
        with socket.socket() as s:
            s.bind(('127.0.0.1', 0))
            port = s.getsockname()[1]
        chan = ch.Channel(token)
        srv = ch.serve(chan, port, say=lambda line: None)

        def run_steps():
            t0 = time.time()
            for ms, fn in sorted(steps, key=lambda s: s[0]):
                time.sleep(max(0.0, t0 + ms / 1000.0 - time.time()))
                fn(chan)
        threading.Thread(target=run_steps, daemon=True).start()
        cmd = [BIN, '--host', '--port', str(port), '--token-file', token_path, '--helper-port', '1', '--wait', str(wait),
               '--data-dir', data, '--shm', os.path.join(data, 'shm'), *extra]
        env = dict(os.environ, LIBGL_ALWAYS_SOFTWARE='1')
        r = subprocess.run(cmd, env=env, capture_output=True, text=True, timeout=int(os.environ.get('HOST_TIMEOUT', '240')))
        srv.shutdown()
        return r.returncode, r.stdout + r.stderr, chan, data

    def reports(self, out):
        return [json.loads(m.group(1)) for m in re.finditer(r"^\[report\] (.*)$", out, re.M)]

    def evals(self, out):
        return [json.loads(m.group(1)) for m in re.finditer(r"^\[eval\] (.*)$", out, re.M)]

    def answer_toast(self, chan):
        for call in chan.luna:
            if call['uri'].endswith('/getToastNotification'):
                chan.push('luna', {'tag': call['tag'], 'payload': TOAST})

    def test_over_an_app_the_system_layer_draws_the_osd_and_a_toast_and_reports_them(self):
        shot = os.path.join(tempfile.gettempdir(), 'rustshell-osd-toast-%d.png' % os.getpid())
        rc, out, chan, data = self.host([
            (300, lambda c: (c.push('foreground', HOME), c.push('screen', {"on": True}))),
            (800, lambda c: c.push('foreground', APP)),
            (1200, lambda c: c.push('audio', AUDIO0)),
            (1500, lambda c: c.push('audio', AUDIO)),
            (2500, self.answer_toast),
        ], wait=4700, extra=['--eval', '2200:state', '--eval', '2300:geometry', '--eval', '4200:state', '--eval', '4300:geometry',
                              '--snap', '3800:' + shot, '--frames', '1100:before', '--frames', '2200:osd'])
        self.assertEqual(rc, 0, out[-3000:])
        evals = self.evals(out)
        self.assertEqual(len(evals), 4, out[-3000:])
        self.assertTrue(evals[0]['osd'], "the audio topic put the volume display up: %s" % evals[0])
        self.assertEqual(evals[0]['source'], 'tv')
        self.assertEqual(evals[0]['level'], '31')
        self.assertEqual(evals[0]['toasts'], 0)
        osd = evals[1]['osd']
        self.assertIsNotNone(osd, "the pane is placed")
        self.assertEqual(osd['y'] + osd['h'], 1080 - 32 - 60, "over an app the bar rides 60 px higher than its rest above the bottom edge: %s" % osd)
        self.assertAlmostEqual(osd['x'] + osd['w'] / 2, 960, delta=1, msg="centred")
        self.assertEqual(evals[2]['toasts'], 1, "the toast came through the feed: %s" % evals[2])
        self.assertFalse(evals[2]['osd'], "the display went after its hold")
        toasts = evals[3]['toasts']
        self.assertEqual(len(toasts), 1)
        self.assertEqual((toasts[0]['x'], toasts[0]['y'] + toasts[0]['h'], toasts[0]['w']), (680, 905 - 16, 560), "centred, over an app 16 px above the subtitles (at rest 54 px above the bottom edge): %s" % toasts)
        reports = self.reports(out)
        self.assertTrue(any(r['live'] and r['state']['osd'] for r in reports), "the display was reported live: %s" % [r['state'] for r in reports])
        self.assertTrue(any(r['state']['toasts'] == 1 for r in reports), "the toast was reported")
        self.assertTrue(all(r['wants'] is False for r in reports), "a display and a toast never want the keys")
        self.assertTrue(all(r['can'] == ['toast'] for r in reports), "this client shows no panel yet; a toast may be asked for by name (`ctl shell show toast`)")
        self.assertEqual(reports[0]['state']['kinds'], ['osd', 'toast'])
        uris = {c['uri'] for c in chan.luna}
        self.assertIn('luna://com.webos.notification/getToastNotification', uris, "the feed is subscribed through the channel")
        self.assertTrue(all(c['subscribe'] for c in chan.luna if c['uri'].endswith('Notification')))
        frames = dict(re.findall(r"^\[frames\] (\S+) (\d+)$", out, re.M))
        self.assertGreater(int(frames['osd']), int(frames['before']), "the display's frame was drawn")
        self.assertTrue(os.path.exists(shot), "the render was written")
        try:
            from PIL import Image
        except ImportError:
            Image = None
        if Image:
            im = Image.open(shot).convert('RGB')
            self.assertNotEqual(im.getpixel((960, 860)), (0, 0, 0), "the toast's card is drawn above the subtitles")
            self.assertEqual(im.getpixel((960, 540)), (0, 0, 0), "nothing else is")
        os.remove(shot)

    def test_frames_are_damage_only_and_the_text_is_glyph_quads(self):
        rc, out, chan, data = self.host([
            (300, lambda c: (c.push('foreground', APP), c.push('screen', {"on": True}))),
            (800, lambda c: c.push('audio', AUDIO0)),
            (1000, lambda c: c.push('audio', AUDIO)),
            (1600, self.answer_toast),
        ], wait=3000, extra=['--log', os.devnull])
        self.assertEqual(rc, 0, out[-3000:])
        m = re.search(r"^RESULT frames (\d+) .* drew ([\d.]+) % of the panel per frame, text pictures (\d+), glyph fallbacks (\d+)", out, re.M)
        self.assertIsNotNone(m, out[-2000:])
        frames, drew, pictures, fallbacks = int(m.group(1)), float(m.group(2)), int(m.group(3)), int(m.group(4))
        self.assertGreater(frames, 10, "the toast's arrival is animated")
        self.assertLess(drew, 40.0, "frames redraw the display's and the toast's tiles, not the panel: %.1f %%" % drew)
        self.assertEqual((pictures, fallbacks), (0, 0), "every label is glyph quads of the atlas")
        small = [int(x) for x in re.findall(r"^\[frame\] \d+ at \d+ ms: damage \d+ px, drew (\d+) px", out, re.M)]
        self.assertTrue(small, out[-2000:])
        self.assertTrue(any(px < 1920 * 1080 // 4 for px in small[1:]), "a frame of the display alone is a quarter of the panel or less")


if __name__ == "__main__":
    unittest.main()
