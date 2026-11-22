import hashlib
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from recovery_check import CLIPS, IMAGE, KERNEL, fresh

HERE = Path(__file__).resolve().parent
BOOT1 = '00000000-0000-4000-8000-000000000001'
BOOT2 = '00000000-0000-4000-8000-000000000002'
LOAD = '\n'.join(['load(from:normal):start', *[f'ioload fw{i} cfg:0 -> header' for i in (2,1,0)], 'ioload fw0 done(ret:0)', 'load(from:normal):done (ret:0)'])


def receipt(boot):
    slots = '\n'.join(f'[{i}] id:{i}, flag:0, fw:   0kb, alloc:   0kb, p:0000000000000000' for i in range(3))
    return f'''@@VRR boot
{boot}
@@VRR image
{IMAGE}  /boot/Image.lz4-{KERNEL}
@@VRR cmdline
pqe.year=24
@@VRR overrides
none
@@VRR status
fw_opt:none, usr_opt:none, usr_num:0, max_num:3, year:24
{slots}
fwload handler:0000000000000000
@@VRR foreground
{{"returnValue":true,"appId":"com.webos.app.home"}}
@@VRR loader
{LOAD}
@@VRR end
{boot}
'''


class Rehearsal(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)

    def put(self, name, data):
        (self.root/name).write_text(data if isinstance(data, str) else json.dumps(data))

    def cli(self, *args, stdin=None):
        return subprocess.run([sys.executable, '-B', str(HERE/'recovery_check.py'), *args], input=stdin, text=True, capture_output=True, cwd=self.root)

    def stock(self):
        for name, boot in [('before', BOOT1), ('after', BOOT2)]:
            self.put(name, receipt(boot))
            observed = self.cli('observe', name, stdin='yes\n' + ''.join(f'clip{m}\nyes\n' for m in CLIPS))
            self.assertEqual(observed.returncode, 0, observed.stderr)
            self.put(name+'.json', observed.stdout)

    def test_shell_collection_fails_closed_on_override_or_unreadable_log(self):
        for section in receipt(BOOT1).split('@@VRR ')[1:]:
            name, body = section.split('\n', 1)
            self.put(name, body)
        # Supply the TV's read results to the unmodified POSIX collector on the host.
        harness = '''
r=$PWD
cd() { if [ "$2" = /var ]; then command cd "$r"; else command cd "$@"; fi; }
uname() { echo 5.4.268-329.ptl4tv.2; }
sha256sum() { command cat "$r/image"; }
cat() {
 case "$1" in
  /proc/sys/kernel/random/boot_id) command cat "$r/boot";;
  /proc/cmdline) command cat "$r/cmdline";;
  /proc/lg/de/fw-status) command cat "$r/status";;
  *) return 1;;
 esac
}
timeout() { command cat "$r/foreground"; }
dmesg() { command cat "$r/loader"; }
. "$0"
'''
        cmd = ['sh', '-c', harness, str(HERE/'recovery_snapshot.sh')]
        r = subprocess.run(cmd, cwd=self.root, text=True, capture_output=True)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.put('captured', r.stdout)
        self.assertEqual(self.cli('baseline', 'captured').returncode, 0)
        (self.root/'firmware').mkdir()
        (self.root/'firmware/pqefw2.bin').symlink_to('absent')
        r = subprocess.run(cmd, cwd=self.root, capture_output=True)
        self.assertNotEqual(r.returncode, 0); self.assertNotIn(b'@@VRR end', r.stdout)
        (self.root/'firmware/pqefw2.bin').unlink()
        (self.root/'loader').unlink()
        r = subprocess.run(cmd, cwd=self.root, capture_output=True)
        self.assertNotEqual(r.returncode, 0); self.assertNotIn(b'@@VRR end', r.stdout)

    def test_functional_handback_cannot_pass_exact_restoration(self):
        self.stock()
        args = ['compare', 'before','after','before.json','after.json']
        r = self.cli(*args)
        self.assertEqual(r.returncode, 2, r.stderr)
        self.assertIn('PASS fresh normal', r.stdout)
        self.assertIn('NOT OBSERVED selector', r.stdout)
        self.assertIn('NOT OBSERVED active MCU', r.stdout)
        for name, boot in [('before', BOOT1), ('after', BOOT2)]:
            self.put(name+'.state', dict(boot=boot, receipt_sha256=hashlib.sha256(receipt(boot).encode()).hexdigest(), owner='main', cache=32, pulse=1 if name=='before' else 3, pixel=1, timer=5000000))
        r = self.cli(*args, '--states','before.state','after.state')
        self.assertEqual(r.returncode, 1); self.assertIn('pulse differs', r.stderr)

    def test_borrowed_notes_same_boot_and_unwitnessed_recovery_fail(self):
        self.stock()
        args = ['compare','before','after','before.json','after.json']
        n = json.loads((self.root/'after.json').read_text()); n['cold_start'] = 'unknown'
        self.put('after.json', n)
        self.assertEqual(self.cli(*args).returncode, 1)
        self.put('after.json', (self.root/'before.json').read_text()); self.assertEqual(self.cli(*args).returncode, 1)
        self.put('after', receipt(BOOT1)); self.assertIn('same boot', self.cli(*args).stderr)

    def test_missing_or_ambiguous_capture_stops(self):
        raw = receipt(BOOT1)
        for bad in (raw.split('@@VRR end')[0], raw.replace('pqe.year=24','pqe.year=24 dispopts=skipfw'), raw.replace('com.webos.app.home','org.nphil.lumagame'), raw.replace('alloc:   0kb','alloc: 100kb'), raw.replace('@@VRR loader','[0] new len:3, path(alt)\n@@VRR loader'), raw.replace('@@VRR end\n'+BOOT1,'@@VRR end\n'+BOOT2)):
            self.put('s', bad)
            self.assertEqual(self.cli('baseline','s').returncode, 1)

    def test_summary_alone_and_subsequent_resume_do_not_establish_source(self):
        self.assertTrue(fresh(LOAD))
        for log in (LOAD.splitlines()[-1], LOAD.replace('fw2 cfg:0 -> header','fw2 cfg:4'), LOAD+'\nload(from:resume):start', LOAD.replace('fw1 cfg:0 -> header','fw0 cfg:0 -> header'), LOAD.replace('done(ret:0)','done(ret:-1)')):
            self.assertFalse(fresh(log))

    def test_split_checks_full_hardware_not_cached_success(self):
        b = dict(boot=BOOT1, owner='main', time_ns=1, phase='original', cache=32, pulse=0xa5c5b009, pixel=0x4001, timer=4_000_000, timer_num=100, timer_den=1)
        self.put('b', b)
        r = self.cli('split','selected80','b')
        self.assertEqual(r.returncode, 2); self.assertIn('pulse=0xa5c5b00b', r.stdout)
        for phase, cache, pulse, period in [('selected64',80,0xa5c5b00b,6_250_000),('selected80',80,0xa5c5b00b,5_000_000),('disconnected64',32,0xa5c5b009,6_250_000),('disconnected80',32,0xa5c5b009,5_000_000),('restored',32,0xa5c5b009,4_000_000)]:
            a = dict(b, phase=phase, cache=cache, pulse=pulse, timer=period, time_ns=2)
            self.put('a', a); self.assertEqual(self.cli('split',phase,'b','a').returncode, 0)
        for diff in (dict(pulse=0xa5c5b00b), dict(pulse=b['pulse'] ^ (1<<30)), dict(pixel=3), dict(timer=5_000_000), dict(cache=80), dict(time_ns=0), dict(boot=BOOT2)):
            self.put('a', {**a, **diff}); self.assertEqual(self.cli('split','restored','b','a').returncode, 1)


if __name__ == '__main__':
    unittest.main()
