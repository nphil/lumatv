"""Run with: python3 -m unittest tests.test_r19_checks

tools/tvslot/r19_checks.py on the host: the pixel classifiers against the round 18 captures, the BLE reply parser, the walk plans'
key safety, and the whole slot against the canned set (`--dry-run`: every command through `sh -n`, every payload and key token checked).
"""
import importlib.util
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
SCRIPT = os.path.join(ROOT, 'tools', 'tvslot', 'r19_checks.py')
R18 = os.path.join(ROOT, 'docs', 'research', 'lg-restyles', 'tv', 'banners-r18')

spec = importlib.util.spec_from_file_location('r19_checks', SCRIPT)
r19 = importlib.util.module_from_spec(spec)
spec.loader.exec_module(r19)


class Pixels(unittest.TestCase):
    def test_round18_frames(self):
        self.assertEqual(r19.classify_pill(r19.load(os.path.join(R18, 'pill-03.png')))[0], 'glass')
        self.assertEqual(r19.classify_pill(r19.load(os.path.join(R18, 'pill-25.png')))[0], 'glass')
        self.assertEqual(r19.classify_pill(r19.load(os.path.join(R18, 'restored-03.png')))[0], 'grey')
        self.assertEqual(r19.classify_pill(r19.load(os.path.join(R18, 'ok-0.png')))[0], 'none')

    def test_info_card_colour(self):
        self.assertEqual(r19.classify_info(r19.load(os.path.join(R18, 'info-1.png')))[0], 'grey')
        self.assertEqual(r19.classify_info(r19.load(r19.synth_png('info')))[0], 'glass')


class Ble(unittest.TestCase):
    def test_stream_and_shapes(self):
        text = ('{"subscribed":true,"returnValue":true}\n{\n "returnValue": true,\n "device": {"address":"AA:BB","rssi":-50,'
                '"scanRecord":[2,1,6,5,9,97,98,99,100]}\n}\n{"device":{"address":"AA:BB","rssi":-50,"scanRecord":[2,1,6]}}\n'
                '{"devices":[{"address":"CC:DD","rssi":-70,"scanRecord":[]}]}')
        s = r19.scan_stats(r19.split_json(text))
        self.assertEqual((s['replies'], s['addresses'], s['device_replies'], s['list_replies']), (4, 2, 2, 1))
        self.assertEqual((s['same_address_again'], s['same_address_again_same_rssi']), (1, 1))
        self.assertEqual((s['scan_record']['with_record'], s['scan_record']['empty_or_unreadable'], s['scan_record']['records_with_name']), (2, 1, 1))
        self.assertNotIn('AA:BB', str(s))

    def test_error_reply_and_ping(self):
        s = r19.scan_stats(r19.split_json('{"errorCode":-1,"returnValue":false,"errorText":"Permission denied"}'))
        self.assertIs(s['first_reply']['returnValue'], False)
        self.assertEqual(r19.ping_summary('3 packets, 0% packet loss\nround-trip min/avg/max = 0.9/1.4/3.2 ms')['avg_ms'], 1.4)


class Plans(unittest.TestCase):
    ALL = [('W1', r19.W1_STEPS), ('W2', r19.W2_STEPS), ('W3', r19.W3_STEPS)]
    OK_STEPS = {'Support page', 'Delete My Personal Information (opened only)', 'App Permissions: Camera', 'Home Assistant page'}

    def test_tokens_valid_and_ok_only_where_a_page_opens(self):
        for _, steps in self.ALL:
            for name, toks, _, _ in steps:
                self.assertEqual(r19.kb_problems(toks.split()), [], name)
                if 'OK' in toks.split():
                    self.assertIn(name, self.OK_STEPS)
                self.assertFalse({'LEFT', 'RIGHT'} & set(toks.split()) and name.startswith('Just'), name)

    def test_rail_never_reaches_lgs_own_settings_row(self):
        # 9 sections (index 0-8) then LG's row: more than 8 Downs after the clamp to the top could land on it
        for name, toks, _, _ in r19.W1_STEPS + r19.W2_STEPS + r19.W3_STEPS:
            if toks.startswith('UPx12'):
                downs = [t for t in toks.split() if t.startswith('DOWNx')]
                self.assertTrue(not downs or int(downs[0][5:].split('@')[0]) <= 8, name)

    def test_no_left_past_the_first_tab(self):
        # Left on tab 0 would leave the page for the rail: every LEFTx is preceded by RIGHTx6 that clamps at the last tab (3 or 4 tabs)
        for _, steps in self.ALL:
            for name, toks, _, _ in steps:
                if 'LEFTx2' in toks:
                    self.assertIn('RIGHTx6', toks, name)


class DryRun(unittest.TestCase):
    def test_whole_slot_against_the_canned_set(self):
        with tempfile.TemporaryDirectory() as d:
            p = subprocess.run([sys.executable, SCRIPT, '--dry-run', '--out', d], capture_output=True, text=True, timeout=120)
            self.assertEqual(p.returncode, 0, p.stdout[-2000:] + p.stderr[-500:])
            self.assertIn('FINAL POWER STATE: Active Standby', p.stdout)
            self.assertNotIn('PROBLEMS', p.stdout)
            for f in ('log.txt', 'summary.txt', 'results.json', 'start-state.json', 'end-state.json', 'ble/scan-shapes.json', 'banners/pill-montage.png'):
                self.assertTrue(os.path.exists(os.path.join(d, f)), f)
            self.assertNotIn('AA:BB', open(os.path.join(d, 'ble', 'scan-shapes.json')).read())

    def test_no_standby_flag_and_isolation(self):
        with tempfile.TemporaryDirectory() as d:
            p = subprocess.run([sys.executable, SCRIPT, '--dry-run', '--out', d, '--only', 'ble', '--no-standby'], capture_output=True, text=True, timeout=120)
            self.assertEqual(p.returncode, 0, p.stdout[-1500:])
            self.assertNotIn('powerOff', p.stdout)
            self.assertNotIn('banners ', p.stdout.split('end state')[0].split('round 19 checks, ')[-1])


if __name__ == '__main__':
    unittest.main()
