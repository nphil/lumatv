"""Run with: python3 -m unittest tests.test_compositor

tools/compositor.sh, the functions `lumatv` uses to put LumaTV's own compositor in LG's place, run
under sh with a fake mount table and fake `mount`, `umount` and `pgrep`: the boot mount's every
refusal, the switch on and off, the live test the guards use, and the status object.
"""
import json
import os
import stat
import subprocess
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
LIB = os.path.join(ROOT, "tools", "compositor.sh")

MOUNT = '''#!/bin/sh
# mount --bind SRC DST: records DST in the fake mount table
[ "$1" = "--bind" ] || exit 1
[ -e "$FAKE/mount-fails" ] && exit 32
echo "1 2 0:3 / $3 rw - overlay x rw" >> "$FAKE/mountinfo"
echo "mount $2 $3" >> "$FAKE/calls"
'''
UMOUNT = '''#!/bin/sh
t=$1; [ "$t" = "-l" ] && t=$2
grep -vF " $t " "$FAKE/mountinfo" > "$FAKE/mountinfo.new"; mv "$FAKE/mountinfo.new" "$FAKE/mountinfo"
echo "umount $t" >> "$FAKE/calls"
'''
PGREP = '''#!/bin/sh
[ -e "$FAKE/compositor.up" ] && echo 4242 && exit 0
exit 1
'''


class Compositor(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        d = self.tmp.name
        self.comp = os.path.join(d, "comp")
        os.makedirs(os.path.join(self.comp, "bin"))
        self.lg = os.path.join(d, "surface-manager")
        with open(self.lg, "w") as f:
            f.write("LG's binary")
        self.bin = os.path.join(d, "bin")
        os.makedirs(self.bin)
        for name, body in (("mount", MOUNT), ("umount", UMOUNT), ("pgrep", PGREP)):
            path = os.path.join(self.bin, name)
            with open(path, "w") as f:
                f.write(body)
            os.chmod(path, os.stat(path).st_mode | stat.S_IEXEC)
        open(os.path.join(d, "mountinfo"), "w").close()
        self.env = dict(os.environ, PATH=self.bin + os.pathsep + os.environ["PATH"], FAKE=d, COMP=self.comp, COMP_LG=self.lg,
                        COMP_MOUNTINFO=os.path.join(d, "mountinfo"), COMP_LOCK=os.path.join(d, "recover.lock"))
        # the installed state: bundle, flag, LG's binary as recorded
        sm = os.path.join(self.comp, "bin", "lumatv-sm")
        with open(sm, "w") as f:
            f.write("#!/bin/sh\n")
        os.chmod(sm, 0o755)
        self.install()

    def tearDown(self):
        self.tmp.cleanup()

    def install(self):
        st = os.stat(self.lg)
        with open(os.path.join(self.comp, "lg-binary"), "w") as f:
            f.write(f"{st.st_size} {int(st.st_mtime)}\n")
        open(os.path.join(self.comp, "enabled"), "w").close()

    def sh(self, script):
        prelude = ('log_line() { echo "log: $1" >> "$FAKE/calls"; }\n'
                   'restart_surface() { echo restart >> "$FAKE/calls"; [ ! -e "$FAKE/restart-fails" ]; }\n'
                   'recover_home() { echo recover >> "$FAKE/calls"; }\n'
                   f'. {LIB}\n')
        r = subprocess.run(["sh", "-c", prelude + script], env=self.env, capture_output=True, text=True)
        return r.returncode, r.stdout.strip()

    def calls(self):
        p = os.path.join(self.tmp.name, "calls")
        return open(p).read().split("\n")[:-1] if os.path.exists(p) else []

    def mounted(self):
        return self.lg in open(os.path.join(self.tmp.name, "mountinfo")).read()

    def test_boot_mounts_when_everything_says_so(self):
        self.assertEqual(self.sh("comp_boot_mount"), (0, "mounted"))
        self.assertTrue(self.mounted())
        self.assertEqual(self.calls(), [f"mount {self.comp}/bin/lumatv-sm {self.lg}"])

    def test_boot_does_nothing_twice(self):
        self.sh("comp_boot_mount")
        self.assertEqual(self.sh("comp_boot_mount"), (0, "already"))
        self.assertEqual(len([c for c in self.calls() if c.startswith("mount")]), 1)

    def test_boot_refuses_without_the_flag_or_the_bundle(self):
        os.remove(os.path.join(self.comp, "enabled"))
        self.assertEqual(self.sh("comp_boot_mount"), (0, "off"))
        self.install()
        os.remove(os.path.join(self.comp, "bin", "lumatv-sm"))
        self.assertEqual(self.sh("comp_boot_mount"), (0, "off"))
        self.assertFalse(self.mounted())

    def test_boot_keeps_lgs_compositor_when_its_binary_changed(self):
        with open(self.lg, "w") as f:
            f.write("LG's binary, updated by a firmware update")
        self.assertEqual(self.sh("comp_boot_mount"), (0, "firmware"))
        self.assertFalse(self.mounted())
        # a recorded size and time that cannot be read count as changed
        os.remove(os.path.join(self.comp, "lg-binary"))
        self.assertEqual(self.sh("comp_boot_mount"), (0, "firmware"))

    def test_boot_keeps_lgs_compositor_after_a_boot_that_never_completed(self):
        pending = os.path.join(self.tmp.name, "pending")
        open(pending, "w").close()
        self.env["BOOT_PENDING"] = pending
        self.assertEqual(self.sh("comp_boot_mount"), (0, "failsafe"))
        self.assertFalse(self.mounted())

    def test_boot_never_fails_the_boot(self):
        open(os.path.join(self.tmp.name, "mount-fails"), "w").close()
        self.assertEqual(self.sh("comp_boot_mount"), (0, "error"))

    def test_live_needs_the_flag_the_mount_and_the_process(self):
        up = os.path.join(self.tmp.name, "compositor.up")
        self.assertEqual(self.sh("comp_live && echo live || echo not")[1], "not")
        self.sh("comp_boot_mount")
        self.assertEqual(self.sh("comp_live && echo live || echo not")[1], "not")          # mounted, no process yet
        open(up, "w").close()
        self.assertEqual(self.sh("comp_live && echo live || echo not")[1], "live")
        os.remove(os.path.join(self.comp, "enabled"))
        self.assertEqual(self.sh("comp_live && echo live || echo not")[1], "not")          # the flag is a builtin test: no pgrep
        self.install()
        os.remove(os.path.join(self.tmp.name, "mountinfo"))
        open(os.path.join(self.tmp.name, "mountinfo"), "w").close()
        self.assertEqual(self.sh("comp_live && echo live || echo not")[1], "not")          # process but no mount

    def test_a_path_that_only_contains_the_name_is_not_the_mount(self):
        with open(os.path.join(self.tmp.name, "mountinfo"), "w") as f:
            f.write(f"1 2 0:3 / {self.lg}.old rw - overlay x rw\n1 2 0:3 / /mnt{self.lg} rw - overlay x rw\n")
        self.assertEqual(self.sh("comp_mounted && echo yes || echo no")[1], "no")

    def test_on_mounts_clears_the_failure_state_restarts_and_recovers(self):
        open(os.path.join(self.comp, "escaped"), "w").close()
        open(os.path.join(self.comp, "last-fallback"), "w").write("hold-home")
        open(os.path.join(self.comp, "failures"), "w").write("2\n")
        rc, out = self.sh("comp_switch on")
        self.assertEqual((rc, out), (0, ""))
        self.assertTrue(self.mounted())
        self.assertEqual(self.calls()[1:], ["restart", "recover"])
        self.assertFalse(os.path.exists(os.path.join(self.comp, "escaped")))
        self.assertFalse(os.path.exists(os.path.join(self.comp, "last-fallback")))
        self.assertEqual(open(os.path.join(self.comp, "failures")).read().strip(), "0")

    def test_on_refuses_what_would_break_the_screen(self):
        os.remove(os.path.join(self.comp, "bin", "lumatv-sm"))
        self.assertEqual(self.sh("comp_switch on")[0], 2)
        with open(os.path.join(self.comp, "bin", "lumatv-sm"), "w") as f:
            f.write("#!/bin/sh\n")
        os.chmod(os.path.join(self.comp, "bin", "lumatv-sm"), 0o755)
        os.mkdir(os.path.join(self.tmp.name, "recover.lock"))
        self.assertEqual(self.sh("comp_switch on")[0], 3)
        os.rmdir(os.path.join(self.tmp.name, "recover.lock"))
        with open(self.lg, "w") as f:
            f.write("a different binary")
        self.assertEqual(self.sh("comp_switch on")[0], 4)
        self.assertFalse(self.mounted())
        self.assertNotIn("restart", self.calls())

    def test_on_reports_a_unit_that_did_not_come_back(self):
        open(os.path.join(self.tmp.name, "restart-fails"), "w").close()
        rc, out = self.sh("comp_switch on")
        self.assertEqual(rc, 1)
        self.assertIn("run 'off'", out)
        self.assertNotIn("recover", self.calls())

    def test_off_removes_the_flag_and_the_mount_and_brings_lgs_home_back(self):
        self.sh("comp_switch on")
        rc, out = self.sh("comp_switch off")
        self.assertEqual(rc, 0)
        self.assertFalse(self.mounted())
        self.assertFalse(os.path.exists(os.path.join(self.comp, "enabled")))
        self.assertEqual(self.calls()[-2:], ["restart", "recover"])

    def test_off_when_it_was_never_on_restarts_nothing(self):
        rc, out = self.sh("comp_switch off")
        self.assertEqual(rc, 0)
        self.assertNotIn("restart", self.calls())

    def test_release_unmounts_and_forgets_the_flag_without_restarting(self):
        self.sh("comp_boot_mount")
        self.sh("comp_release")
        self.assertFalse(self.mounted())
        self.assertFalse(os.path.exists(os.path.join(self.comp, "enabled")))
        self.assertNotIn("restart", self.calls())

    def test_a_recovery_in_progress_is_seen(self):
        self.assertEqual(self.sh("comp_recovering && echo yes || echo no")[1], "no")
        os.mkdir(os.path.join(self.tmp.name, "recover.lock"))
        self.assertEqual(self.sh("comp_recovering && echo yes || echo no")[1], "yes")

    def test_status_is_one_valid_json_object(self):
        open(os.path.join(self.comp, "sm.log"), "w").write('12:00:00 said "hello"\\ there\n')
        open(os.path.join(self.comp, "last-fallback"), "w").write('a "quoted" reason\n')
        open(os.path.join(self.comp, "failures"), "w").write("1\n")
        j = json.loads(self.sh("comp_json")[1])
        self.assertEqual((j["bundle"], j["enabled"], j["mounted"], j["live"], j["lgBinary"], j["failures"]), (True, True, False, False, "same", 1))
        self.sh("comp_boot_mount")
        j = json.loads(self.sh("comp_json")[1])
        self.assertEqual((j["mounted"], j["lgBinary"]), (True, "mounted"))
        with open(os.path.join(self.tmp.name, "mountinfo"), "w"):
            pass
        with open(self.lg, "w") as f:
            f.write("changed")
        self.assertEqual(json.loads(self.sh("comp_json")[1])["lgBinary"], "changed")

    def test_status_with_nothing_installed(self):
        for n in os.listdir(self.comp):
            p = os.path.join(self.comp, n)
            os.remove(p) if os.path.isfile(p) else None
        os.remove(os.path.join(self.comp, "bin", "lumatv-sm")) if os.path.exists(os.path.join(self.comp, "bin", "lumatv-sm")) else None
        j = json.loads(self.sh("comp_json")[1])
        self.assertEqual((j["bundle"], j["enabled"], j["live"], j["lgBinary"], j["failures"]), (False, False, False, "unknown", 0))


if __name__ == "__main__":
    unittest.main()
