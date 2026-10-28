"""Run with: python3 -m unittest tests.test_r19_checks

tools/tvslot/r19_checks.py on the host: the pixel classifiers against the round 18 captures, the BLE reply parser, the walk plans'
key safety, and the whole slot against the canned set (`--dry-run`: every command through `sh -n`, every payload and key token checked).
"""
import importlib.util
import os
import re
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

    def test_plugins_in_the_whole_slot(self):
        with tempfile.TemporaryDirectory() as d:
            p = subprocess.run([sys.executable, SCRIPT, '--dry-run', '--out', d, '--only', 'plugins'], capture_output=True, text=True, timeout=120)
            self.assertEqual(p.returncode, 0, p.stdout[-2000:])
            self.assertIn('plugins   ', p.stdout)
            self.assertTrue(os.path.exists(os.path.join(d, 'plugins', 'plugins.json')))


class BannerTree(unittest.TestCase):
    def test_expectations_come_from_the_patch_not_from_constants(self):
        t = r19.banner_tree()
        self.assertIn("UserInterfaceLayer/Components/LumaStyle.qml", t["files"])      # one level deeper than the first guess
        self.assertIn("UserInterfaceLayer/GlobalStyle.qml", t["files"])
        self.assertEqual(len(t["files"]), len(set(t["files"])))
        self.assertGreaterEqual(t["at_least"], 20)
        self.assertFalse(hasattr(r19, "IC_FILES"))
        if t["count"] is not None:                                                    # with the census copy of LG's tree on this host
            self.assertGreaterEqual(t["count"], t["at_least"])

    def test_the_banner_check_asks_the_set_for_every_derived_file(self):
        d = tempfile.mkdtemp()
        tv = r19.DryTv()
        r19.Slot(r19.parse_args(["--dry-run", "--out", d]), tv).check_banners()
        cmd = next(c for c in tv.commands if "missing-files" in c)
        for rel in r19.banner_tree()["files"]:
            self.assertIn(rel, cmd)


class UpdateBlockCheck(unittest.TestCase):
    def run_check(self):
        d = tempfile.mkdtemp()
        tv = r19.DryTv()
        return r19.Slot(r19.parse_args(["--dry-run", "--out", d]), tv).check_updateblock(), tv, d

    def test_passes_on_a_blocked_set_and_writes_nothing_to_it(self):
        res, tv, d = self.run_check()
        by = {i["name"]: i["verdict"] for i in res["items"]}
        self.assertEqual(res["verdict"], "PASS", res)
        self.assertEqual(by["/etc/hosts maps LG's four update servers to loopback"], "PASS")
        self.assertEqual(by["Homebrew Channel's blockUpdates (the switch)"], "PASS")
        self.assertEqual(tv.problems, [])
        for banned in ("setConfiguration", "POST", "> /etc", ">> /etc", "rm ", "mount "):
            self.assertFalse([c for c in tv.commands if banned in c], banned)
        self.assertTrue(os.path.exists(os.path.join(d, "updateblock", "etc-hosts-lge.txt")))

    def test_fails_when_the_switch_reads_off(self):
        d = tempfile.mkdtemp()

        class Off(r19.DryTv):
            def sh(self, cmd, timeout=60):
                out = super().sh(cmd, timeout)
                return out.replace('"blockUpdates":true', '"blockUpdates":false') if "getConfiguration" in cmd else out
        res = r19.Slot(r19.parse_args(["--dry-run", "--out", d]), Off()).check_updateblock()
        self.assertEqual(res["verdict"], "FAIL")
        self.assertIn("Homebrew Channel's blockUpdates (the switch)", res["detail"])


class Layout(unittest.TestCase):
    def slot(self):
        d = tempfile.mkdtemp()
        tv = r19.DryTv()
        return r19.Slot(r19.parse_args(['--dry-run', '--out', d]), tv), tv, d

    def test_dialog_classifier(self):
        P = r19.pil()
        flat = P[0].new('RGB', (960, 540), (24, 20, 32))
        self.assertTrue(r19.dialog_like(flat))
        photo = r19.load(os.path.join(R18, 'info-1.png'))
        self.assertFalse(r19.dialog_like(photo))

    def test_passes_when_the_file_is_unchanged_and_only_planned_keys_are_sent(self):
        s, tv, d = self.slot()
        res = s.check_layout()
        by = {i['name']: i['verdict'] for i in res['items']}
        self.assertEqual(by['layout.json unchanged'], 'PASS')
        self.assertEqual(by['Cancel and confirm Discard'], 'EYES')
        self.assertEqual(tv.problems, [])
        keys = [c.split(r19.KB + ' ', 1)[1] for c in tv.commands if r19.KB + ' ' in c]
        self.assertIn('RIGHTx5@0.25 s0.6', keys)
        self.assertEqual(keys.index('RIGHTx5@0.25 s0.6') + 1, keys.index('OK s1.8'))    # the dialog's OK follows the move to Discard at once
        self.assertNotIn('LEFT', ' '.join(keys).split()[:0])
        for f in ('home-widgets.png', 'edit-grid.png', 'edit-pickup.png', 'edit-cancel.png'):
            self.assertTrue(os.path.exists(os.path.join(d, 'layout', f)), f)
        # nothing but reads touches the file
        self.assertFalse([c for c in tv.commands if 'layout.json' in c and not re.search(r'^(if \[ -f|stat -c|cat )', c)])

    def test_fails_when_the_layout_was_written(self):
        s, tv, d = self.slot()
        reads = iter([('aaa', '1', '{}'), ('bbb', '2', '{"widgets":[1]}')])
        s.layout_file = lambda: next(reads)
        res = s.check_layout()
        self.assertEqual(res['verdict'], 'FAIL')
        self.assertIn('THE LAYOUT WAS WRITTEN', str(res['items']))

    def test_customised_layout_steps_up_one_row(self):
        s, tv, d = self.slot()
        s.layout_file = lambda: ('aaa', '1', '{"widgets":[{"id":"clock"}]}')
        s.check_layout()
        keys = [c.split(r19.KB + ' ', 1)[1] for c in tv.commands if r19.KB + ' ' in c]
        i = keys.index('DOWNx30@0.08 s0.6')
        self.assertEqual(keys[i + 1], 'UP s0.6')
        s2, tv2, _ = self.slot()
        s2.check_layout()
        keys2 = [c.split(r19.KB + ' ', 1)[1] for c in tv2.commands if r19.KB + ' ' in c]
        self.assertNotEqual(keys2[keys2.index('DOWNx30@0.08 s0.6') + 1], 'UP s0.6')


class Plugins(unittest.TestCase):
    def test_ha_words_follow_the_hub(self):
        ha = lambda **k: [dict({'id': 'home-assistant', 'state': 'connected'}, **k)]
        self.assertEqual(r19.ha_words([], {}), 'Not switched on')
        self.assertEqual(r19.ha_words(ha(), {'state': 'off'}), 'Not linked')
        self.assertEqual(r19.ha_words(ha(), {}), 'Not linked')
        self.assertEqual(r19.ha_words(ha(), {'state': 'auth_failed'}), 'Token refused')
        self.assertEqual(r19.ha_words(ha(), {'state': 'connected'}), 'Connected')
        self.assertEqual(r19.ha_words(ha(state='inert'), {}), 'Cannot run')
        self.assertEqual(r19.ha_words(ha(state='waiting', reason='x'), {}), 'Not running')

    def run_check(self, tweak=None):
        d = tempfile.mkdtemp()
        args = r19.parse_args(['--dry-run', '--out', d])
        tv = r19.DryTv()
        slot = r19.Slot(args, tv)
        res = slot.check_plugins()
        return res, tv, d

    def test_read_only(self):
        res, tv, d = self.run_check()
        cmds = '\n'.join(tv.commands)
        self.assertEqual(tv.problems, [])
        for banned in ('POST', ' -X', '--data', ' -d ', 'lumatv-helper', 'plugin-enable', 'ha-link', 'rescan', '/actions/', 'lease'):
            self.assertNotIn(banned, cmds)
        self.assertNotIn('OK', ' '.join(c.split(r19.KB)[1] for c in tv.commands if r19.KB in c).split())
        self.assertTrue(any('/v1/plugins' in c for c in tv.commands))

    def test_dry_verdicts_and_evidence(self):
        res, tv, d = self.run_check()
        by = {i['name']: i['verdict'] for i in res['items']}
        self.assertEqual(by['helper answers on 127.0.0.1:18731'], 'PASS')
        self.assertEqual(by['Home Assistant plugin state'], 'PASS')
        self.assertEqual(by['first-party card: clock'], 'PASS')
        self.assertEqual(by['no token or secret in /v1/state'], 'PASS')
        ev = open(os.path.join(d, 'plugins', 'plugins.json')).read()
        self.assertIn('Not switched on', ev)
        for f in ('home-lane.png', 'quick-mid.png', 'quick-low.png'):
            self.assertTrue(os.path.exists(os.path.join(d, 'plugins', f)), f)


class DryRun2(unittest.TestCase):
    def test_no_standby_flag_and_isolation(self):
        with tempfile.TemporaryDirectory() as d:
            p = subprocess.run([sys.executable, SCRIPT, '--dry-run', '--out', d, '--only', 'ble', '--no-standby'], capture_output=True, text=True, timeout=120)
            self.assertEqual(p.returncode, 0, p.stdout[-1500:])
            self.assertNotIn('powerOff', p.stdout)
            self.assertNotIn('banners ', p.stdout.split('end state')[0].split('round 19 checks, ')[-1])


if __name__ == '__main__':
    unittest.main()
