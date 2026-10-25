"""Run with: python3 -m unittest tests.test_cutover

The cutover tooling of compositor/tools, run without the set: the on-set cutover script (install, enable, trial, firmware)
under sh with a fake set, the gate's frame rule and firmware check (tv-gate.sh) on real result lines and a fake log, the
gate driver's verdict and its record of a passed build (tv_gate.py), the slot's trial rule (tv_next_slot.py), the virtual
remote (tv-uinput-key.py), the supervisor's own C tests (compositor/sm/build.sh) and the rule that nothing here ever
restarts, stops or kills SAM.
"""
import importlib.util
import json
import os
import re
import shutil
import stat
import struct
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
TOOLS = os.path.join(ROOT, "compositor", "tools")
CUTOVER = os.path.join(TOOLS, "tv-cutover.sh")
GATE = os.path.join(TOOLS, "tv-gate.sh")
LIB = os.path.join(ROOT, "tools", "compositor.sh")
SM_C = os.path.join(ROOT, "compositor", "sm", "lumatv-sm.c")

try:
    from test_compositor import SM, RELEASE, Fixture
except ImportError:
    from tests.test_compositor import SM, RELEASE, Fixture

MOUNT = '''#!/bin/sh
[ "$1" = "--bind" ] || exit 1
[ -e "$FAKE/mount-fails" ] && exit 32
echo "1 2 0:3 / $3 rw - overlay x rw" >> "$LUMATV_MOUNTINFO"
echo "mount $2 $3" >> "$FAKE/calls"
'''
UMOUNT = '''#!/bin/sh
t=$1; [ "$t" = "-l" ] && t=$2
grep -vF " $t " "$LUMATV_MOUNTINFO" > "$LUMATV_MOUNTINFO.new"; mv "$LUMATV_MOUNTINFO.new" "$LUMATV_MOUNTINFO"
echo "umount $t" >> "$FAKE/calls"
'''
SYSTEMCTL = '''#!/bin/sh
case "$1" in
  is-active) echo active ;;
  show) echo MainPID=77; echo ActiveEnterTimestampMonotonic=5 ;;
  *) echo "systemctl $*" >> "$FAKE/calls" ;;
esac
'''
PGREP = '''#!/bin/sh
case "$*" in
  *"-x surface-manager"*) echo 1234; exit 0 ;;
  *bin/lumatv-compositor*) [ -e "$FAKE/compositor.up" ] && { echo 4321; exit 0; }; exit 1 ;;
esac
exit 1
'''
NOOP = "#!/bin/sh\nexit 0\n"
RECOVER = '#!/bin/sh\necho recover >> "$FAKE/calls"\n'


def write(path, text, mode=None):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(text)
    if mode:
        os.chmod(path, mode)


def read(path):
    with open(path) as f:
        return f.read()


class FakeSet(unittest.TestCase):
    """A set as the scripts see it: LG's binary and libraries as files, the deployed bundle of /tmp/lumatv, the commands
    they call."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        d = self.tmp.name
        self.state = os.path.join(d, "state")
        self.dep = os.path.join(d, "lumatv")
        self.lg = os.path.join(d, "lg", "surface-manager")
        self.out = os.path.join(d, "out")
        write(self.lg, "LG's compositor")
        self.release = os.path.join(d, "fw", "starfish-release")
        write(self.release, RELEASE)
        self.libs = [os.path.join(d, "fw", "libmali.so"), os.path.join(d, "fw", "libwayland-webos-client.so")]
        for i, p in enumerate(self.libs):
            write(p, "library %d\n" % i * 50)
        write(os.path.join(d, "boot_id"), "BOOT-A\n")
        write(os.path.join(self.dep, "bin", "lumatv-sm"), SM, 0o755)
        write(os.path.join(self.dep, "bin", "lumatv-compositor"), "the compositor\n", 0o755)
        write(os.path.join(self.dep, "recover-home.sh"), RECOVER)
        write(os.path.join(self.dep, "lib", "ld-linux-aarch64.so.1"), "loader\n")
        write(os.path.join(self.dep, "lib", "libmali-bifrost-g52-g24p0-wayland-gbm.so"), "blob\n")
        write(os.path.join(self.dep, "verified.list"), "# nothing listed\n")
        shutil.copy(LIB, os.path.join(self.dep, "compositor.sh"))
        write(os.path.join(d, "mountinfo"), "")
        self.fakebin = os.path.join(d, "fakebin")
        for name, body in (("mount", MOUNT), ("umount", UMOUNT), ("systemctl", SYSTEMCTL), ("pgrep", PGREP), ("sleep", NOOP), ("luna-send", NOOP)):
            write(os.path.join(self.fakebin, name), body, 0o755)
        self.env = dict(os.environ, PATH=self.fakebin + os.pathsep + os.environ["PATH"], FAKE=d,
                        LUMATV_STATE=self.state, LUMATV_D=self.dep, LUMATV_LG=self.lg, LUMATV_MOUNTINFO=os.path.join(d, "mountinfo"),
                        LUMATV_RECOVER_LOCK=os.path.join(d, "recover.lock"), LUMATV_OUT=self.out,
                        COMP_RELEASE=self.release, COMP_FW_LIBS="\n".join(self.libs), COMP_BOOT_ID=os.path.join(d, "boot_id"))

    def tearDown(self):
        self.tmp.cleanup()

    def cutover(self, *args, env=None):
        r = subprocess.run(["sh", CUTOVER, *args], env=dict(self.env, **(env or {})), capture_output=True, text=True, timeout=120)
        return r.returncode, r.stdout + r.stderr

    def gate(self, *args, env=None):
        r = subprocess.run(["sh", GATE, *args], env=dict(self.env, **(env or {})), capture_output=True, text=True, timeout=120)
        return r.returncode, r.stdout + r.stderr

    def st(self, name):
        return os.path.join(self.state, name)

    def calls(self):
        p = os.path.join(self.tmp.name, "calls")
        return read(p).split("\n")[:-1] if os.path.exists(p) else []

    def mounted(self):
        return self.lg in read(os.path.join(self.tmp.name, "mountinfo"))

    def fake(self, name, text=""):
        write(os.path.join(self.tmp.name, name), text)

    def list_this_build(self):
        """What the gate does after a pass: the entry for this set goes on the list that the next install ships."""
        rc, out = self.cutover("firmware", "record")
        self.assertEqual(rc, 0, out)
        write(os.path.join(self.dep, "verified.list"), "# listed\n" + out + "\n")


class Install(FakeSet):
    def test_it_records_what_the_boot_checks_compare_with(self):
        rc, out = self.cutover("install")
        self.assertEqual(rc, 0, out)
        self.assertEqual(read(self.st("fingerprint")).strip(), "abcdef0123456789")
        st = os.stat(self.lg)
        self.assertEqual(read(self.st("lg-binary")).strip(), "%d %d" % (st.st_size, st.st_mtime))
        self.assertEqual(read(self.st("supervisor-api")).strip(), "2")
        self.assertEqual(read(self.st("failures")).strip(), "0")
        manifest = read(self.st("manifest")).split("\n")
        self.assertEqual(manifest[0], "release " + RELEASE.strip())
        self.assertEqual([l.split()[3] for l in manifest if l.startswith("file ")], [self.release, self.lg] + self.libs)
        bundle = {l.split()[2]: l.split()[:2] for l in read(self.st("bundle.manifest")).strip().split("\n")}
        self.assertEqual(sorted(bundle), ["bin/lumatv-compositor", "bin/lumatv-sm", "bin/recover-home.sh", "lib/ld-linux-aarch64.so.1",
                                          "lib/libmali-bifrost-g52-g24p0-wayland-gbm.so"])
        self.assertRegex(bundle["bin/lumatv-compositor"][1], r"^[0-9a-f]{64}$")        # bin/: a hash
        self.assertEqual(bundle["lib/ld-linux-aarch64.so.1"][1], "-")                    # lib/: the size alone
        self.assertEqual(int(bundle["bin/lumatv-compositor"][0]), len("the compositor\n"))
        self.assertFalse(os.path.exists(self.st("enabled")))

    def test_a_build_nobody_listed_is_said_so_and_cached_as_nothing(self):
        rc, out = self.cutover("install")
        self.assertIn("firmware: unverified: verified.list has no entry", out)
        self.assertFalse(os.path.exists(self.st("fw-verified")))

    def test_a_listed_build_is_verified_and_cached(self):
        self.cutover("install")
        self.list_this_build()
        rc, out = self.cutover("install")
        self.assertIn("firmware: verified 10.2.1-5202", out)
        self.assertTrue(os.path.exists(self.st("fw-verified")))
        self.assertIn("fingerprint abcdef0123456789", read(self.st("verified.list")))

    def test_it_clears_what_an_earlier_run_left(self):
        os.makedirs(self.state)
        for name in ("escaped", "last-fallback", "enabled", "running", "fw-verified"):
            write(self.st(name), "x")
        write(self.st("failures"), "2\n")
        self.cutover("install")
        for name in ("escaped", "last-fallback", "enabled", "running"):
            self.assertFalse(os.path.exists(self.st(name)), name)
        self.assertEqual(read(self.st("failures")).strip(), "0")

    def test_a_deploy_without_a_list_leaves_the_sets_own(self):
        os.remove(os.path.join(self.dep, "verified.list"))
        self.cutover("install")
        self.assertIn("no build has passed the gate yet", read(self.st("verified.list")))
        write(self.st("verified.list"), "# the set's own\n")
        self.cutover("install")
        self.assertEqual(read(self.st("verified.list")), "# the set's own\n")

    def test_it_refuses_while_the_supervisor_is_mounted_over_lgs_binary(self):
        write(os.path.join(self.tmp.name, "mountinfo"), "1 2 0:3 / %s rw - overlay x rw\n" % self.lg)
        rc, out = self.cutover("install")
        self.assertEqual(rc, 1)
        self.assertIn("run disable first", out)
        self.assertFalse(os.path.exists(self.st("lg-binary")))


class Enable(FakeSet):
    def setUp(self):
        super().setUp()
        self.cutover("install")

    def test_a_persistent_enable_on_a_build_nobody_listed_is_refused(self):
        rc, out = self.cutover("enable")
        self.assertEqual(rc, 5, out)
        self.assertIn("refusing a persistent enable", out)
        self.assertIn("enable --session", out)
        self.assertFalse(os.path.exists(self.st("enabled")))
        self.assertFalse(self.mounted())
        self.assertEqual(self.calls(), [])

    def test_a_session_runs_on_any_build_and_names_this_boot(self):
        rc, out = self.cutover("enable", "--session")
        self.assertEqual(rc, 0, out)
        self.assertEqual(read(self.st("enabled")).strip(), "session BOOT-A")
        self.assertIn("this boot only", out)
        self.assertTrue(self.mounted())
        self.assertIn("systemctl restart surface-manager-daemon", self.calls())
        self.assertIn("enabled (session)", out)

    def test_a_persistent_enable_on_a_listed_build_is_persistent(self):
        self.list_this_build()
        self.cutover("install")
        rc, out = self.cutover("enable")
        self.assertEqual(rc, 0, out)
        self.assertEqual(read(self.st("enabled")).strip(), "persistent")
        self.assertIn("firmware: verified 10.2.1-5202", out)
        self.assertTrue(self.mounted())

    def test_it_clears_the_record_of_an_earlier_run_before_it_starts(self):
        write(self.st("escaped"), "hold Home 650 ms")
        write(self.st("running"), "BOOT-A 1\n")
        write(self.st("failures"), "2\n")
        rc, out = self.cutover("enable", "--session")
        self.assertEqual(rc, 0, out)
        self.assertFalse(os.path.exists(self.st("escaped")) or os.path.exists(self.st("running")))
        self.assertEqual(read(self.st("failures")).strip(), "0")

    def test_a_trial_is_a_session_that_ends_with_the_flag_gone_and_LGs_compositor_back(self):
        rc, out = self.cutover("trial", "10")
        self.assertEqual(rc, 0, out)
        self.assertIn("this boot only", out)
        self.assertIn("t=10s", out)
        self.assertFalse(os.path.exists(self.st("enabled")))
        self.assertFalse(self.mounted())
        c = self.calls()
        self.assertEqual(c.count("systemctl restart surface-manager-daemon"), 2)         # in, and the revert out
        self.assertIn("recover", c)

    def test_a_flag_that_was_a_session_is_not_persistent_after_a_reboot(self):
        self.cutover("enable", "--session")
        write(os.path.join(self.tmp.name, "boot_id"), "BOOT-B\n")                        # the next boot: another id, and no mount
        write(os.path.join(self.tmp.name, "mountinfo"), "")
        rc, out = self.cutover("boot-check")
        j = json.loads(out)
        self.assertEqual((j["flag"], j["decision"]), ("stale", "off"))

    def test_nothing_is_mounted_for_what_the_supervisor_would_refuse(self):
        self.fake("sm-says", "no escaped: escape recorded; re-enable from the tool\n")
        rc, out = self.cutover("enable", "--session")
        self.assertEqual(rc, 7, out)
        self.assertIn("the supervisor would not start: escaped: escape recorded", out)
        self.assertFalse(os.path.exists(self.st("enabled")))
        self.assertFalse(self.mounted())
        self.assertEqual(self.calls(), [])

    def test_what_would_break_the_screen_is_refused_first(self):
        write(os.path.join(self.tmp.name, "mountinfo"), "1 2 0:3 / %s rw - overlay x rw\n" % self.lg)
        rc, out = self.cutover("enable", "--session")
        self.assertEqual(rc, 1)
        self.assertIn("already mounted", out)
        write(os.path.join(self.tmp.name, "mountinfo"), "")
        with open(self.lg, "w") as f:
            f.write("LG's compositor, after an update")
        rc, out = self.cutover("enable", "--session")
        self.assertEqual(rc, 4)
        self.assertIn("firmware changed", out)
        self.cutover("install")
        write(self.st("bin/lumatv-compositor"), "the compositoX\n")                       # the size of the first, not its content
        rc, out = self.cutover("enable", "--session")
        self.assertEqual(rc, 6)
        self.assertIn("bin/lumatv-compositor differs from the installed copy", out)
        os.remove(self.st("supervisor-api"))
        rc, out = self.cutover("enable", "--session")
        self.assertEqual(rc, 1)
        self.assertIn("predates supervisor-api", out)
        self.assertEqual(self.calls(), [])

    def test_an_unknown_argument_is_refused(self):
        rc, out = self.cutover("enable", "--forever")
        self.assertEqual(rc, 1)
        self.assertIn("enable [--session]", out)

    def test_disable_removes_the_flag_and_the_unsettled_marker_and_restores_lgs(self):
        self.cutover("enable", "--session")
        write(self.st("running"), "BOOT-A 1\n")
        rc, out = self.cutover("disable")
        self.assertEqual(rc, 0, out)
        self.assertFalse(os.path.exists(self.st("enabled")) or os.path.exists(self.st("running")))
        self.assertFalse(self.mounted())


class Firmware(FakeSet):
    def setUp(self):
        super().setUp()
        self.cutover("install")

    def test_check_says_what_the_gate_has_not_passed_on(self):
        rc, out = self.cutover("firmware", "check")
        self.assertEqual((rc, out.strip()), (5, "unverified: verified.list has no entry"))
        self.list_this_build()
        self.cutover("install")
        self.assertEqual(self.cutover("firmware", "check"), (0, "verified 10.2.1-5202\n"))

    def test_the_record_is_the_entry_the_next_install_verifies(self):
        rc, out = self.cutover("firmware", "record")
        lines = out.strip().split("\n")
        self.assertEqual((lines[0].split()[:2], lines[-1]), (["build", "10.2.1-5202"], "end"))
        self.assertEqual(len([l for l in lines if l.startswith("file ")]), 4)

    def test_an_update_changes_the_answer(self):
        self.list_this_build()
        self.cutover("install")
        with open(self.libs[0], "ab") as f:
            f.write(b"update")
        rc, out = self.cutover("firmware", "check")
        self.assertEqual(rc, 5)
        self.assertIn("is not the verified file", out)

    def test_the_boot_check_reads_like_the_hook(self):
        rc, out = self.cutover("boot-check")
        j = json.loads(out)
        self.assertEqual((j["decision"], j["wouldMount"], j["flag"], j["build"]), ("off", False, "off", "10.2.1-5202"))

    def test_status_and_help(self):
        rc, out = self.cutover("status")
        self.assertIn("flag: off; firmware: unverified; supervisor-api: 2", out)
        rc, out = self.cutover()
        for word in ("install", "probe", "trial SECS", "enable --session", "disable", "firmware check", "firmware record", "boot-check"):
            self.assertIn(word, out)
        self.assertNotIn("STATE=", out)

    def test_it_will_not_run_without_the_deployed_library(self):
        os.remove(os.path.join(self.dep, "compositor.sh"))
        rc, out = self.cutover("status")
        self.assertEqual(rc, 1)
        self.assertIn("deploy first", out)


GOOD = "[x1] RESULT frames=3600 in 60.00 s = 60.00 fps; gaps: <=17.5 ms 3551, 17.5-20 48, 20-33.5 0, >33.5 0; worst 19.8 ms; ended at monotonic 100.5"
FIFTH = "[x1] RESULT frames=3597 in 60.00 s = 59.95 fps; gaps: <=17.5 ms 3549, 17.5-20 45, 20-33.5 2, >33.5 0; worst 22.5 ms; ended at monotonic 43356.976"


class FrameRule(unittest.TestCase):
    """tv-gate.sh, the frame rule: zero gaps over 20 ms in 60 s, no warm-up second excused."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.env = dict(os.environ, GATE_LIB="1", LUMATV_D=self.tmp.name, LUMATV_OUT=os.path.join(self.tmp.name, "out"),
                        LUMATV_STATE=os.path.join(self.tmp.name, "state"))

    def tearDown(self):
        self.tmp.cleanup()

    def sh(self, script):
        r = subprocess.run(["sh", "-c", ". %s\n%s" % (GATE, script)], env=self.env, capture_output=True, text=True, timeout=60)
        return r.returncode, r.stdout.strip()

    def rule(self, result, asked=60, before="1", after="1", comp=0):
        return self.sh("x1_rule '%s' '%s' '%s' '%s' '%s'" % (result, asked, before, after, comp))

    def test_the_numbers_of_a_result_line(self):
        self.assertEqual(self.sh("x1_numbers '%s'" % FIFTH), (0, "3597 60.00 59.95 3549 45 2 0 22.5"))
        self.assertEqual(self.sh("x1_numbers 'no result here'"), (0, ""))

    def test_a_clean_run_passes(self):
        rc, out = self.rule(GOOD)
        self.assertEqual(rc, 0)
        self.assertTrue(out.startswith("PASS 60.00 fps over 60.00 s, no gap over 20 ms"), out)

    def test_the_fifth_gates_two_gaps_in_the_first_second_fail(self):
        rc, out = self.rule(FIFTH, before="1", after="2")
        self.assertEqual(rc, 1)
        self.assertEqual(out, "FAIL 2 gap(s) over 20 ms in the client's swaps (worst 22.5 ms); pacer late 1 -> 2")

    def test_one_gap_over_20_ms_fails_in_either_bucket(self):
        for bucket in ("20-33.5 1, >33.5 0", "20-33.5 0, >33.5 1"):
            rc, out = self.rule(GOOD.replace("20-33.5 0, >33.5 0", bucket))
            self.assertEqual(rc, 1, bucket)
            self.assertIn("1 gap(s) over 20 ms", out)

    def test_a_gap_of_exactly_the_limit_is_not_over_it(self):
        rc, out = self.rule(GOOD.replace("17.5-20 48", "17.5-20 49").replace("<=17.5 ms 3551", "<=17.5 ms 3550"))
        self.assertEqual(rc, 0, out)

    def test_a_short_run_cannot_pass_the_rule_of_60_s(self):
        rc, out = self.rule(GOOD, asked=10)
        self.assertEqual(rc, 1)
        self.assertIn("the run asked for 10 s, the rule is 60 s", out)
        rc, out = self.rule(GOOD.replace("in 60.00 s", "in 31.40 s"))
        self.assertEqual(rc, 1)
        self.assertIn("the run lasted 31.40 s", out)

    def test_a_run_that_does_not_hold_60_hz_fails_without_a_single_long_gap(self):
        rc, out = self.rule(GOOD.replace("= 60.00 fps", "= 55.10 fps"))
        self.assertEqual(rc, 1)
        self.assertIn("55.10 fps is under 59.5", out)

    def test_the_pacers_late_counter_must_have_stayed_put_and_been_read(self):
        rc, out = self.rule(GOOD, before="0", after="3")
        self.assertEqual((rc, out), (1, "FAIL pacer late 0 -> 3"))
        rc, out = self.rule(GOOD, before="", after="")
        self.assertEqual((rc, out), (1, "FAIL the pacer's late counter was not read"))

    def test_the_compositors_own_log_counts_against_a_clean_client(self):
        rc, out = self.rule(GOOD, comp=2)
        self.assertEqual(rc, 1)
        self.assertIn("the compositor logged 2 loop pass(es), client frame gap(s) or page-flip interval(s) over 20 ms", out)

    def test_a_compositor_log_that_could_not_be_read_cannot_pass_the_rule(self):
        for comp in ("unreadable", ""):
            rc, out = self.rule(GOOD, comp=comp)
            self.assertEqual((rc, out), (1, "FAIL the compositor's log of the run could not be read"), comp)

    def test_no_result_is_no_pass(self):
        self.assertEqual(self.rule("")[1], "FAIL the X1 client printed no result")

    def test_every_reason_is_named_at_once(self):
        rc, out = self.rule(FIFTH.replace("= 59.95", "= 58.00"), asked=30, before="1", after="9", comp=1)
        for part in ("2 gap(s) over 20 ms", "asked for 30 s", "58.00 fps", "pacer late 1 -> 9", "compositor logged 1"):
            self.assertIn(part, out)

    def test_the_compositor_side_counts_loop_passes_frame_gaps_and_flip_intervals_inside_the_run_only(self):
        log = os.path.join(self.tmp.name, "compositor.log")
        write(log, "\n".join([
            "[srv  43296.100] warn loop: 17.4 ms in one pass (tick)",                        # under the limit
            "[srv  43297.052] warn loop: 21.0 ms in one pass (tick)",                        # over, inside
            "[srv  43297.066] info kms: page-flip stamp 33.36 ms after the last (one refresh is 16.67)",   # over, inside
            "[srv  43297.500] info kms: page-flip stamp 16.70 ms after the last (one refresh is 16.67)",   # one refresh
            "[srv  43300.000] warn frame gap 33.3 ms on surface 7 (x1) after 291 steady frames",           # over, inside
            "[srv  43300.100] warn frame gap 18.0 ms on surface 7 (x1) after 292 steady frames",
            "[srv  43357.053] info kms: page-flip stamp 33.37 ms after the last (one refresh is 16.67)",   # after the client left
            "[srv  43290.000] warn loop: 90.0 ms in one pass (accept)",                      # before it started
            "[srv  43298.000] info client connected: pid 1 uid 0 cmd x"]) + "\n")
        self.assertEqual(self.sh("comp_over_limit '%s' 43296.9 43356.976" % log), (0, "3"))
        self.assertEqual(self.sh("comp_over_limit '%s' 0 99999" % log), (0, "5"))
        self.assertEqual(self.sh("comp_over_limit /nonexistent 0 99999"), (0, "unreadable"))


class GateFirmwareCheck(FakeSet):
    """tv-gate.sh firmware: the check run on the set's own files, against a scratch list."""

    def setUp(self):
        super().setUp()
        self.cutover("install")

    def test_it_passes_on_files_it_can_read_and_proves_the_refusals(self):
        rc, out = self.gate("firmware")
        self.assertIn("GATE firmware: PASS", out, out)
        self.assertIn("a flipped digit: 5; another release: 5; an empty list: 5", out)
        self.assertIn("unreadable: none", out)
        self.assertIn("fingerprint abcdef0123456789", out)
        self.assertIn("the supervisor: run (exit 0)", out)
        entry = read(os.path.join(self.out, "firmware-entry.txt"))
        self.assertTrue(entry.startswith("build 10.2.1-5202 ") and entry.strip().endswith("end"))

    def test_a_file_the_set_does_not_have_fails_it_by_name(self):
        os.remove(self.libs[1])
        rc, out = self.gate("firmware")
        self.assertIn("GATE firmware: FAIL", out, out)
        self.assertIn("unreadable: %s" % self.libs[1], out)

    def test_it_changes_nothing_the_set_holds(self):
        before = {n: read(self.st(n)) for n in ("verified.list", "manifest", "fingerprint", "lg-binary", "bundle.manifest")}
        self.gate("firmware")
        self.assertEqual(before, {n: read(self.st(n)) for n in before})
        self.assertFalse(os.path.exists(self.st("enabled")))
        self.assertEqual(self.calls(), [])

    def test_the_supervisor_that_cannot_answer_fails_it(self):
        self.fake("sm-broken")
        rc, out = self.gate("firmware")
        self.assertIn("GATE firmware: FAIL", out, out)

    def test_without_the_deployed_library_it_says_so(self):
        os.remove(os.path.join(self.dep, "compositor.sh"))
        rc, out = self.gate("firmware")
        self.assertIn("GATE firmware: FAIL tools/compositor.sh is not deployed", out)


def load(name, extra_modules=()):
    mocks = {"tv": mock.MagicMock(), "paramiko": mock.MagicMock()}
    mocks.update({m: mock.MagicMock() for m in extra_modules})
    with mock.patch.dict(sys.modules, mocks):
        spec = importlib.util.spec_from_file_location(name + "_under_test", os.path.join(TOOLS, name + ".py"))
        mod = importlib.util.module_from_spec(spec)
        sys.modules[name + "_under_test"] = mod
        spec.loader.exec_module(mod)
    return mod


BLOCK = ("build 10.2.1-5202 2026-10-26\nrelease Rockhopper release 10.2.1-5202 (ponytail-paparoa)\nfingerprint e34f8de94ef657a4\n"
         + "".join("file %064x %d /usr/lib/lib%d.so\n" % (i + 1, 100 + i, i) for i in range(10)) + "end\n")


class GateDriver(unittest.TestCase):
    """tv_gate.py: the verdict, the record of a passed build, and the order the checks run in."""

    def setUp(self):
        self.gate = load("tv_gate")
        self.tmp = tempfile.TemporaryDirectory()

    def tearDown(self):
        self.tmp.cleanup()

    def test_a_line_that_is_no_verdict_does_not_fail_the_gate(self):
        # `GATE standby: requested ({` is only the power-off call going out; counted as a failure it made every gate fail
        lines = ["GATE live: PASS ok", "GATE standby: requested ({", "GATE standby: PASS rounds", "GATE escape: PASS ok"]
        self.assertTrue(self.gate.all_passed(lines))
        self.assertFalse(self.gate.all_passed(lines + ["GATE x1: FAIL gaps"]))
        self.assertFalse(self.gate.all_passed(["GATE standby: requested ({"]))
        self.assertFalse(self.gate.all_passed([]))
        self.assertFalse(self.gate.all_passed(["GATE standby: FAIL no SSH after the wake"]))

    def test_the_entry_is_cut_out_of_what_a_pty_printed(self):
        text = "\r\nsome noise\r\n" + BLOCK.replace("\n", "\r\n") + "more noise\r\n"
        self.assertEqual(self.gate.entry_from(text), BLOCK)
        self.assertIsNone(self.gate.entry_from("build 10.2.1-5202\nrelease x\n"))              # no `end`: cut short
        self.assertIsNone(self.gate.entry_from("nothing"))

    def test_an_entry_that_would_verify_a_build_nobody_hashed_is_refused(self):
        self.assertIsNone(self.gate.entry_problem(BLOCK))
        self.assertIn("SHA-256", self.gate.entry_problem(BLOCK.replace("file %064x" % 3, "file -")) or "")
        self.assertIn("only 8 file lines", self.gate.entry_problem("".join(BLOCK.splitlines(True)[:-3]) + "end\n"))
        self.assertEqual(self.gate.entry_problem(BLOCK.replace("fingerprint e34f8de94ef657a4", "fingerprint unknown")), "no 16-digit fingerprint")
        self.assertEqual(self.gate.entry_problem(BLOCK.replace("release Rockhopper release 10.2.1-5202 (ponytail-paparoa)\n", "")), "no release line")

    def test_a_passed_build_is_added_once(self):
        path = os.path.join(self.tmp.name, "verified.list")
        write(path, "# the builds the gate passed on\n")
        self.assertEqual(self.gate.add_entry(path, BLOCK), "added")
        self.assertEqual(read(path), "# the builds the gate passed on\n\n" + BLOCK)
        self.assertEqual(self.gate.add_entry(path, BLOCK), "listed")
        self.assertEqual(read(path).count("build "), 1)
        other = BLOCK.replace("e34f8de94ef657a4", "0123456789abcdef").replace("10.2.1-5202", "10.2.2-5300")
        self.assertEqual(self.gate.add_entry(path, other), "added")
        self.assertEqual(read(path).count("\nbuild "), 2)
        self.assertTrue(self.gate.add_entry(path, "build x\nend\n").startswith("refused: "))
        self.assertEqual(read(path).count("\nbuild "), 2)

    def test_the_list_the_repo_ships_has_nothing_in_it_until_a_gate_passed(self):
        text = read(os.path.join(ROOT, "compositor", "firmware", "verified.list"))
        self.assertEqual([l for l in text.splitlines() if l and not l.startswith("#")], [])

    def run_main(self, pty_answers=None, argv=(), passed=True):
        """main() with every call to the set recorded and answered: returns (calls, exit code, the repo list's text)."""
        g = self.gate
        calls = []
        repo = os.path.join(self.tmp.name, "repo", "compositor", "tools")
        os.makedirs(os.path.join(self.tmp.name, "repo", "docs", "research", "compositor"))
        os.makedirs(repo)
        lst = os.path.join(self.tmp.name, "repo", "compositor", "firmware", "verified.list")
        write(lst, "# list\n")

        def pty(cmd, timeout=600):
            calls.append(cmd)
            if (pty_answers or {}).get(cmd.split("/tmp/lumatv/")[-1]) is not None:
                return pty_answers[cmd.split("/tmp/lumatv/")[-1]]
            parts = cmd.split()
            if "gate.sh" in cmd:
                i = [j for j, p in enumerate(parts) if p.endswith("gate.sh")][0]
                check, label = parts[i + 1], parts[i + 2] if len(parts) > i + 2 and not parts[i + 2].isdigit() else ""
                if check == "standby":
                    return "GATE standby: requested ({\n"
                name = check + ("-" + label if label else "")
                if check == "x1" and not passed:
                    return "GATE x1: FAIL 2 gaps\n"
                return "GATE %s: PASS fine\n" % name
            return ""
        g.pty = pty
        g.wake = lambda: True
        g.avr_mute = lambda on: None
        g.volume_check = lambda: "GATE volume: PASS fine"
        g.VERIFIED_LIST = lst
        g.here = repo
        with mock.patch.object(sys, "argv", ["tv_gate.py", *argv]), mock.patch.object(g.time, "sleep"), mock.patch.object(g.os, "system"), \
                mock.patch("builtins.print"):
            with self.assertRaises(SystemExit) as ex:
                g.main()
        return calls, ex.exception.code, read(lst)

    def checks(self, calls):
        out = []
        for c in calls:
            if "gate.sh" in c:
                p = c.split()
                i = [j for j, x in enumerate(p) if x.endswith("gate.sh")][0]
                out.append(" ".join(p[i + 1:]))
            elif "cutover.sh" in c:
                out.append("cutover " + " ".join(c.split()[c.split().index("sh") + 2:]))
        return out

    def test_the_order_of_a_full_gate_with_a_fresh_session_after_every_check_that_ends_one(self):
        calls, code, _ = self.run_main()
        self.assertEqual(code, 0)
        self.assertEqual([c for c in self.checks(calls) if not c.startswith("key")], [
            "firmware", "homeheld",
            "cutover enable --session", "live", "x1 60", "netflix", "hdmi", "evidence", "standby", "woke",
            "homekey", "cutover enable --session", "live after-homekey",
            "wd-hang", "cutover enable --session", "live after-wd-hang",
            "wd-crash", "cutover enable --session", "live after-wd-crash",
            "escape",
            "cutover disable"])

    def test_skips_leave_out_their_checks_and_their_sessions(self):
        calls, code, _ = self.run_main(argv=["--skip", "firmware,homeheld,netflix,hdmi,standby,homekey,wd-hang,wd-crash"])
        self.assertEqual([c for c in self.checks(calls) if not c.startswith("key")],
                         ["cutover enable --session", "live", "x1 60", "evidence", "escape", "cutover disable"])

    def test_a_failed_check_fails_the_gate_and_never_enables_anything_for_good(self):
        calls, code, lst = self.run_main(argv=["--enable-after"], passed=False)
        self.assertEqual(code, 1)
        self.assertNotIn("cutover enable", self.checks(calls))
        self.assertFalse(any("firmware record" in c for c in calls))
        self.assertEqual(lst, "# list\n")

    def test_a_pass_lists_the_build_installs_it_and_only_then_enables_for_good(self):
        answers = {"cutover.sh firmware record": "noise\r\n" + BLOCK.replace("\n", "\r\n")}
        calls, code, lst = self.run_main(pty_answers=answers, argv=["--enable-after"])
        self.assertEqual(code, 0)
        self.assertEqual(lst, "# list\n\n" + BLOCK)
        tail = [c for c in self.checks(calls) if c.startswith("cutover")][-4:]
        self.assertEqual(tail, ["cutover disable", "cutover firmware record", "cutover install", "cutover enable"])
        self.assertEqual(self.checks(calls)[-1], "live enabled")
        # the list went to the set between the record and the install
        puts = [c.args for c in self.gate.tv.put.call_args_list]
        self.assertIn((self.gate.VERIFIED_LIST, "/tmp/lumatv/verified.list"), puts)

    def test_an_entry_the_check_would_not_trust_is_not_listed_and_nothing_is_enabled(self):
        answers = {"cutover.sh firmware record": "build 10.2.1-5202 2026-10-26\nrelease x\nfingerprint unknown\nend\n"}
        calls, code, lst = self.run_main(pty_answers=answers, argv=["--enable-after"])
        self.assertEqual(lst, "# list\n")
        self.assertNotIn("cutover enable", self.checks(calls))
        self.assertNotIn("cutover install", self.checks(calls))

    def test_the_files_the_checks_need_go_to_the_set(self):
        self.run_main()
        sent = {c.args[1] for c in self.gate.tv.put.call_args_list}
        for target in ("/tmp/lumatv/gate.sh", "/tmp/lumatv/cutover.sh", "/tmp/lumatv/recover-home.sh", "/tmp/lumatv/uinput-key.py",
                       "/tmp/lumatv/compositor.sh", "/tmp/lumatv/verified.list"):
            self.assertIn(target, sent)


SM_LOG = """[16:09:30] start: enabled 1 fingerprint e34f8de94ef657a4 (verified e34f8de94ef657a4) failures 0 escaped 0 dry-run 0 flag 1 on-verified-list 0
[16:09:30] fallback: hold Home 650 ms
[16:12:00] start: enabled 1 fingerprint e34f8de94ef657a4 (verified e34f8de94ef657a4) failures 0 escaped 0 dry-run 0 flag 2 on-verified-list 0
[16:12:01] bridge: compositor connected
[16:12:03] watchdog: the set is Active
[16:12:03] watchdog: progress rules on (frozen output watched, empty panel watched)
"""
COMP_LOG = """[srv    10.100] info listening on /tmp/xdg/wayland-0
[srv    11.500] warn loop: 17.3 ms in one pass (clients draw-now)
[srv    12.250] warn loop: 8.9 ms in one pass (tick)
[srv    40.000] info client disconnected: pid 2961 uid 0 after 60057 ms (ConnectionClosed); bound []; cmd "x"
"""


class Slot(unittest.TestCase):
    """tv_next_slot.py: the trial's rule, read from the logs of the run."""

    def setUp(self):
        self.slot = load("tv_next_slot", extra_modules=("tv_gate",))

    def test_a_clean_trial_has_no_problem_and_old_runs_in_the_log_do_not_count(self):
        self.assertEqual(self.slot.trial_problems(SM_LOG, COMP_LOG), [])
        self.assertEqual(len(self.slot.run_lines(SM_LOG)), 4)
        self.assertEqual(self.slot.fallbacks_of_run(SM_LOG), [])

    def test_what_counts_against_a_trial(self):
        sm = SM_LOG + "[16:12:30] fallback: compositor exited with 139\n"
        self.assertEqual(self.slot.trial_problems(sm, COMP_LOG), ["1 fallback(s): fallback: compositor exited with 139"])
        problems = self.slot.trial_problems(SM_LOG, COMP_LOG + "[srv 20.0] warn loop: 21.0 ms in one pass (tick)\n"
                                            "[srv 21.0] info kms: page-flip stamp 33.36 ms after the last (one refresh is 16.67)\n"
                                            "[srv 22.0] warn frame gap 22.5 ms on surface 4 (app) after 36 steady frames\n"
                                            "[srv 23.0] warn error in client communication\n")
        self.assertEqual(len(problems), 2)
        self.assertIn("3 frame(s) over 20 ms", " ".join(problems))
        self.assertIn("1 protocol error(s)", " ".join(problems))

    def test_a_watchdog_that_never_read_its_inputs_and_an_unsettled_run_are_problems(self):
        sm = "\n".join(l for l in SM_LOG.splitlines() if "progress rules" not in l) + "\n"
        self.assertEqual(self.slot.trial_problems(sm, COMP_LOG), ["the watchdog never read the compositor's counters and windows"])
        sm = SM_LOG.replace("[16:12:01] bridge", "[16:12:00] the previous run never settled or ended cleanly (power cut, kill or hang): failures now 1\n[16:12:01] bridge")
        self.assertEqual(self.slot.trial_problems(sm, COMP_LOG), ["the supervisor counted a run that never settled"])

    def test_the_lines_the_limit_is_read_from(self):
        over = self.slot.over_limit("[srv 1.0] warn loop: 20.0 ms in one pass (x)\n[srv 2.0] warn loop: 20.1 ms in one pass (x)\n")
        self.assertEqual(len(over), 1)
        self.assertEqual(self.slot.over_limit("[srv 1.0] info kms: page-flip stamp 16.70 ms after the last (one refresh is 16.67)"), [])


class VirtualRemote(unittest.TestCase):
    def setUp(self):
        self.k = load("tv-uinput-key")

    def test_the_device_description_is_the_size_the_kernel_reads(self):
        self.assertEqual(len(self.k.user_dev()), 1116)
        self.assertTrue(self.k.user_dev().startswith(b"lumatv-gate-remote\0"))

    def test_an_event_is_the_size_of_this_process_input_event(self):
        self.assertEqual(len(self.k.event(1, 172, 1)), struct.calcsize("llHHi"))
        t, us, etype, code, value = struct.unpack("llHHi", self.k.event(1, 172, 1, now=100.25))
        self.assertEqual((t, us, etype, code, value), (100, 250000, 1, 172, 1))

    def test_it_makes_the_device_presses_holds_releases_and_destroys_it(self):
        path = os.path.join(tempfile.mkdtemp(), "uinput")
        write(path, "")
        ioctls, naps = [], []
        with mock.patch.object(self.k.fcntl, "ioctl", lambda fd, req, arg=0: ioctls.append((req, arg))):
            self.k.hold(1.5, 0.5, 172, path, sleep=naps.append)
        self.assertEqual(ioctls, [(self.k.UI_SET_EVBIT, 1), (self.k.UI_SET_KEYBIT, 172), (self.k.UI_DEV_CREATE, 0), (self.k.UI_DEV_DESTROY, 0)])
        self.assertEqual(naps, [0.5, 1.5, 0.2])
        data = open(path, "rb").read()
        n = struct.calcsize("llHHi")
        self.assertEqual(len(data), 1116 + 4 * n)
        events = [struct.unpack("llHHi", data[1116 + i * n: 1116 + (i + 1) * n])[2:] for i in range(4)]
        self.assertEqual(events, [(1, 172, 1), (0, 0, 0), (1, 172, 0), (0, 0, 0)])           # press, sync, release, sync
        shutil.rmtree(os.path.dirname(path))

    def test_the_device_is_destroyed_even_when_a_step_fails(self):
        path = os.path.join(tempfile.mkdtemp(), "uinput")
        write(path, "")
        seen = []

        def boom(seconds):
            raise OSError("no")
        with mock.patch.object(self.k.fcntl, "ioctl", lambda fd, req, arg=0: seen.append(req)):
            with self.assertRaises(OSError):
                self.k.hold(1.0, 0.1, 172, path, sleep=boom)
        self.assertEqual(seen[-1], self.k.UI_DEV_DESTROY)
        shutil.rmtree(os.path.dirname(path))

    def test_usage_errors_and_a_missing_device_are_exit_codes(self):
        with mock.patch("builtins.print"):
            self.assertEqual(self.k.main(["x"]), 2)
            self.assertEqual(self.k.main(["x", "hold"]), 2)
            self.assertEqual(self.k.main(["x", "hold", "soon"]), 2)
            self.assertEqual(self.k.main(["x", "hold", "1", "--bogus", "1"]), 2)
            self.assertEqual(self.k.main(["x", "hold", "1", "--settle"]), 2)
            self.assertEqual(self.k.main(["x", "hold", "1", "--device", "/nonexistent/uinput"]), 1)


class SupervisorBuild(unittest.TestCase):
    @unittest.skipUnless(shutil.which("zig"), "zig cc builds the supervisor")
    def test_the_supervisor_builds_for_the_set_and_its_own_tests_pass(self):
        r = subprocess.run(["sh", os.path.join(ROOT, "compositor", "sm", "build.sh")], capture_output=True, text=True, timeout=300)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("lumatv-sm json tests: ok", r.stdout + r.stderr)
        self.assertNotIn("warning:", r.stdout + r.stderr)



@unittest.skipUnless(shutil.which("zig"), "zig cc builds the supervisor")
class RealSupervisor(Fixture):
    """The shell library against the real supervisor (its host build: the same decide(), Home and state code, linked to stub
    libraries): the words and exit codes of --would-run, that it changes nothing, and the boot hook's answers from them."""

    @classmethod
    def setUpClass(cls):
        r = subprocess.run(["sh", os.path.join(ROOT, "compositor", "sm", "build.sh")], capture_output=True, text=True, timeout=300)
        cls.host = os.path.join(ROOT, "compositor", "deploy", "sm", "host")
        if r.returncode != 0 or not os.path.exists(os.path.join(cls.host, "lumatv-sm")):
            raise unittest.SkipTest("the supervisor did not build: " + (r.stdout + r.stderr)[-300:])

    def setUp(self):
        super().setUp()
        self.write_bin("lumatv-sm", '#!/bin/sh\nLD_LIBRARY_PATH="%s" exec "%s/lumatv-sm" "$@"\n' % (self.host, self.host))
        self.put("bin/recover-home.sh", "#!/bin/sh\n")
        self.put("lib/libmali-bifrost-g52-g24p0-wayland-gbm.so", "blob\n")
        self.install()
        rc, fp = self.sm("--print-fingerprint")
        self.assertRegex(fp, r"^[0-9a-f]{16}$")
        self.put("fingerprint", fp + "\n")
        self.fingerprint = fp

    def sm(self, *args):
        r = subprocess.run([os.path.join(self.comp, "bin", "lumatv-sm"), "--state", self.comp, *args], capture_output=True, text=True, timeout=30)
        return r.returncode, r.stdout.strip()

    def boot_id(self):
        with open("/proc/sys/kernel/random/boot_id") as f:
            return f.read().strip()

    def test_it_runs_when_everything_is_in_place(self):
        self.assertEqual(self.sm("--would-run"), (0, "run"))

    def test_every_refusal_has_its_word_its_reason_and_exit_3(self):
        os.remove(self.state("enabled"))
        self.assertEqual(self.sm("--would-run"), (3, "no off: not enabled"))
        self.put("enabled", "session NOT-THIS-BOOT\n")
        self.assertEqual(self.sm("--would-run"), (3, "no session: the enable was a session of an earlier boot"))
        self.put("enabled")
        self.put("escaped", "hold Home 650 ms")
        self.assertEqual(self.sm("--would-run"), (3, "no escaped: escape recorded; re-enable from the tool"))
        os.remove(self.state("escaped"))
        os.remove(self.state("lib/libmali-bifrost-g52-g24p0-wayland-gbm.so"))
        self.assertEqual(self.sm("--would-run"), (3, "no bundle: the bundle is incomplete: lib/libmali-bifrost-g52-g24p0-wayland-gbm.so is missing"))
        self.put("lib/libmali-bifrost-g52-g24p0-wayland-gbm.so", "blob\n")
        self.put("fingerprint", "0000000000000000\n")
        self.assertEqual(self.sm("--would-run"), (3, "no firmware: firmware fingerprint differs from the verified one"))
        self.put("fingerprint", self.fingerprint + "\n")
        self.put("verified.list", "build x\nfingerprint 1111111111111111\nend\n")
        self.assertEqual(self.sm("--would-run"), (3, "no firmware: firmware build is not on the verified list"))
        self.install()
        self.put("fingerprint", self.fingerprint + "\n")
        self.put("failures", "2\n")
        self.assertEqual(self.sm("--would-run"), (3, "no failures: two consecutive failures"))

    def test_a_run_that_never_settled_counts_as_a_failure_without_being_consumed(self):
        self.put("failures", "1\n")
        self.put("running", "X 1\n")
        self.assertEqual(self.sm("--would-run"), (3, "no failures: two consecutive failures"))
        self.assertEqual(open(self.state("failures")).read().strip(), "1")
        self.assertTrue(os.path.exists(self.state("running")))

    def test_a_session_of_this_boot_runs_on_a_build_that_is_not_listed(self):
        self.put("verified.list", "# nothing listed\n")
        self.assertEqual(self.sm("--would-run")[0], 3)                           # persistent: refused
        self.put("enabled", "session %s\n" % self.boot_id())
        self.assertEqual(self.sm("--would-run"), (0, "run"))

    def test_asking_changes_nothing_and_writes_no_log(self):
        before = sorted(os.listdir(self.comp))
        self.sm("--would-run")
        self.sm("--home-held", "100")
        self.assertEqual(sorted(os.listdir(self.comp)), before)
        self.assertFalse(os.path.exists(self.state("sm.log")))

    def test_the_boot_hook_reads_the_real_binarys_answers(self):
        self.assertEqual(self.sh("comp_sm_says; echo $COMP_WORD")[1], "run")
        self.assertEqual(self.sh("comp_boot_mount check"), (0, "would-mount"))
        self.put("escaped", "hold Home 650 ms")
        self.assertEqual(self.sh("comp_boot_mount check"), (0, "escaped"))
        os.remove(self.state("escaped"))
        self.put("failures", "2\n")
        self.assertEqual(self.sh("comp_boot_mount check"), (0, "failures"))
        self.put("failures", "0\n")
        self.put("fingerprint", "0000000000000000\n")
        self.assertEqual(self.sh("comp_boot_mount check"), (0, "firmware"))
        self.assertIn("log: compositor-boot: firmware: firmware fingerprint differs from the verified one", self.calls())

    def test_home_is_not_held_on_a_machine_with_no_remote(self):
        t0 = time.time()
        self.assertEqual(self.sm("--home-held")[0], 1)
        self.assertLess(time.time() - t0, 2)
        self.assertEqual(self.sm("--home-held", "abc")[0], 1)          # not a number: the default, and still not held

    def test_a_binary_that_runs_is_what_the_hook_mounts(self):
        self.assertEqual(self.sh("comp_boot_mount"), (0, "mounted"))
        self.assertTrue(self.mounted())


SAM = re.compile(
    r"systemctl\s+(?:--\S+\s+)*(?:restart|try-restart|reload-or-restart|stop|kill|reload)\s+[^\n;&|]*\bsam(?:\.service)?\b"
    r"|\b(?:pkill|killall)\b[^\n]*\bsam\b"
    r"|\bservice\s+sam\b"
    r"|\brestart\s+sam\b",
    re.IGNORECASE)
SAM_QUOTED = re.compile(r"""["']sam(?:\.service)?["']""")


class SamIsNeverTouched(unittest.TestCase):
    """The set rule of tests/test_tool_rules.py, held for what the cutover adds: the watchdog falls back by exec in
    place and the recovery restarts WebAppMgr, never SAM (eARC, CEC and the picture mode are lost with it)."""

    def files(self):
        out = [LIB, SM_C]
        for n in sorted(os.listdir(TOOLS)):
            if n.endswith((".sh", ".py")):
                out.append(os.path.join(TOOLS, n))
        return out

    def test_nothing_of_the_cutover_restarts_stops_or_kills_sam(self):
        found = []
        files = self.files()
        self.assertGreater(len(files), 8, "the scan found no files")
        for path in files:
            for n, line in enumerate(read(path).splitlines(), 1):
                code = line.strip()
                if code.startswith(("#", "//")):
                    continue
                if SAM.search(line) or (path.endswith((".c", ".py")) and SAM_QUOTED.search(line) and re.search(r"restart|stop|kill|exec", line)):
                    found.append("%s:%d: %s" % (os.path.relpath(path, ROOT), n, code))
        self.assertEqual(found, [])

    def test_the_gate_reads_sam_to_prove_it_was_left_alone(self):
        text = read(GATE)
        self.assertIn("systemctl show sam.service", text)
        self.assertGreaterEqual(text.count("sam_id"), 6)           # escape, homekey, wd-hang and wd-crash compare it before and after

    def test_the_recovery_restarts_the_compositors_neighbours_and_not_sam(self):
        text = read(os.path.join(TOOLS, "tv-recover-home.sh"))
        self.assertIn("systemctl restart $WAM", text)
        self.assertNotRegex(text, r"restart[^\n]*\bsam\b")


if __name__ == "__main__":
    unittest.main()
