"""Run with: python3 -m unittest tests.test_cutover

The cutover tooling of compositor/tools, run without the set: the on-set cutover script (install, enable, trial, firmware)
under sh with a fake set, the gate's frame rule and firmware check (tv-gate.sh) on real result lines and a fake log, the
gate driver's verdict and its record of a passed build (tv_gate.py), the slot's trial rule (tv_next_slot.py), the virtual
remote (tv-uinput-key.py), the supervisor's own C tests (compositor/sm/build.sh) and the rule that nothing here ever
restarts, stops or kills SAM.
"""
import contextlib
import importlib.util
import fcntl
import io
import json
import os
import re
import shutil
import signal
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
    from test_compositor import RELEASE, Fixture
except ImportError:
    from tests.test_compositor import RELEASE, Fixture

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
  show) case "$*" in *MainPID*--value*) echo 77 ;; *) echo MainPID=77; echo ActiveEnterTimestampMonotonic=5 ;; esac ;;
  restart) echo "systemctl $*" >> "$FAKE/calls"; if grep -qF " $LUMATV_LG " "$LUMATV_MOUNTINFO" && [ ! -e "$FAKE/no-native" ]; then touch "$FAKE/compositor.up"; else rm -f "$FAKE/compositor.up"; fi ;;
  *) echo "systemctl $*" >> "$FAKE/calls" ;;
esac
'''
PGREP = '''#!/bin/sh
case "$*" in
  *"-x surface-manager"*) echo 1234; exit 0 ;;
  *"-P 77"*bin/lumatv-compositor*) [ -e "$FAKE/compositor.up" ] && exit 0; exit 1 ;;
  *bin/lumatv-compositor*) [ -e "$FAKE/compositor.up" ] && { echo 4321; exit 0; }; exit 1 ;;
esac
exit 1
'''
STAT = '''#!/bin/sh
case "$*" in
  "-Lc %d:%i /proc/77/exe")
    if [ -e "$FAKE/compositor.up" ]; then echo fake-supervisor; else echo fake-stock; fi; exit 0 ;;
  "-Lc %d:%i $FAKE/data/compositor/bin/lumatv-sm")
    echo fake-supervisor; exit 0 ;;
esac
exec /usr/bin/stat "$@"
'''
FAKE_SM = '''#!/bin/sh
echo "$*" >> "$FAKE/sm-args"
case "$*" in
  *--print-fingerprint*) echo abcdef0123456789 ;;
  *--would-run*|*--would-enable" session"|*--would-enable" persistent")
    [ -e "$FAKE/sm-broken" ] && exit 127
    [ -e "$FAKE/sm-says" ] && { cat "$FAKE/sm-says"; exit 3; }
    echo run ;;
  *--home-held*) [ -e "$FAKE/home-held" ] && exit 0; exit 1 ;;
  *) exit 2 ;;
esac
'''
NOOP = "#!/bin/sh\nexit 0\n"
# A sleep that waits a thousandth of what it is asked: the scripts' own waits cost nothing, and the safety timer's job,
# which sleeps against the clock, does not spin as it would on a sleep that returns at once.
FAST_SLEEP = '''#!/bin/sh
exec REAL_SLEEP "$(awk -v s="$1" 'BEGIN { printf "%.3f", s / 1000 }')"
'''.replace("REAL_SLEEP", shutil.which("sleep") or "/bin/sleep")
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


def alive(pid):
    """Whether the process runs: a zombie that has not been reaped yet does not."""
    try:
        with open("/proc/%d/stat" % pid) as f:
            return f.read().rsplit(")", 1)[1].split()[0] != "Z"
    except OSError:
        return False


def session_of(pid):
    with open("/proc/%d/stat" % pid) as f:
        return int(f.read().rsplit(")", 1)[1].split()[3])


def uptime():
    with open("/proc/uptime") as f:
        return float(f.read().split()[0])


def wait_until(cond, seconds=8.0):
    end = time.time() + seconds
    while time.time() < end:
        if cond():
            return True
        time.sleep(0.05)
    return cond()


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
        write(os.path.join(self.dep, "bin", "lumatv-sm"), FAKE_SM, 0o755)
        write(os.path.join(self.dep, "bin", "lumatv-compositor"), "the compositor\n", 0o755)
        write(os.path.join(self.dep, "recover-home.sh"), RECOVER)
        write(os.path.join(self.dep, "lib", "ld-linux-aarch64.so.1"), "loader\n")
        write(os.path.join(self.dep, "lib", "libmali-bifrost-g52-g24p0-wayland-gbm.so"), "blob\n")
        write(os.path.join(self.dep, "verified.list"), "# nothing listed\n")
        shutil.copy(LIB, os.path.join(self.dep, "compositor.sh"))
        shutil.copy(os.path.join(ROOT, "tools", "logrelay.py"), os.path.join(self.dep, "logrelay.py"))
        shutil.copytree(os.path.join(ROOT, "compositor", "assets", "ui"), os.path.join(self.dep, "assets", "ui"))
        app = os.path.join(d, "app")
        os.makedirs(os.path.join(app, "tools"))
        for name in ("brand.sh", "compositor.sh"):
            shutil.copy(os.path.join(ROOT, "tools", name), os.path.join(app, "tools", name))
        shutil.copy(os.path.join(ROOT, "brand.json"), os.path.join(app, "brand.json"))
        cli = read(os.path.join(ROOT, "tools", "lumatv"))
        cli = cli.replace('STATE="$BRAND_DATA_DIR"', 'STATE="$FAKE/data"')
        cli = cli.replace('/proc/mounts', '"$FAKE/mounts"')
        cli = cli.replace('MAINTENANCE_ACCESS=write\ncase', '''AUTOSTART_LINK="$FAKE/autostart/link"
ENV_FILE="$FAKE/surface.env"
HS_MODE="$FAKE/homemode"
HS_WD_PID="$FAKE/watchdog.pid"
LEGACY_STATE="$FAKE/legacy"
MIGRATING="$FAKE/data/.migrating"
NOTI_PERSIST="$FAKE/notification.json"
SURFACE_SVC=surface-manager-daemon
MAINTENANCE_ACCESS=write
case''')
        cli = cli.replace('case "$VERB" in\n  status)', '''install_early_hook() { return 0; }
native_start() { return 0; }
close_input_apps() { :; }
wam_stamp() { :; }
snapshot_reports() { :; }
disarm_restart_crashes() { :; }
case "$VERB" in
  status)''')
        cli_path = os.path.join(app, "tools", "lumatv")
        write(cli_path, cli, 0o755)
        os.makedirs(os.path.join(d, "data"))
        os.symlink(self.state, os.path.join(d, "data", "compositor"))
        write(os.path.join(d, "mounts"), "")
        write(os.path.join(d, "mountinfo"), "")
        self.fakebin = os.path.join(d, "fakebin")
        for name, body in (("mount", MOUNT), ("umount", UMOUNT), ("systemctl", SYSTEMCTL), ("pgrep", PGREP), ("stat", STAT), ("sleep", FAST_SLEEP), ("luna-send", "#!/bin/sh\necho '{\"returnValue\":true}'\n")):
            write(os.path.join(self.fakebin, name), body, 0o755)
        write(os.path.join(self.fakebin, "python3"), '''#!/bin/sh
case "$1" in
  */keyfilter.py) exit 0 ;;
  */systemui.py) [ "$2" = status ] && echo '{}'; exit 0 ;;
esac
exec "$REAL_PYTHON" "$@"
''', 0o755)
        self.env = dict(os.environ, PATH=self.fakebin + os.pathsep + os.environ["PATH"], FAKE=d,
                        LUMATV_STATE=self.state, LUMATV_D=self.dep, LUMATV_LG=self.lg, LUMATV_MOUNTINFO=os.path.join(d, "mountinfo"),
                        LUMATV_RECOVER_LOCK=os.path.join(d, "recover.lock"), LUMATV_OUT=self.out,
                        COMP_RELEASE=self.release, COMP_FW_LIBS="\n".join(self.libs), COMP_BOOT_ID=os.path.join(d, "boot_id"),
                        LUMATV_SAFETY_MINUTE="1", LUMATV_TOOL=cli_path, REAL_PYTHON=sys.executable,
                        COMP_LG=self.lg, COMP_MOUNTINFO=os.path.join(d, "mountinfo"), COMP_LOCK=os.path.join(d, "recover.lock"))

    def tearDown(self):
        # a safety timer a test left running: its record names it
        try:
            os.kill(int(read(self.st("safety-timer")).split()[0]), signal.SIGKILL)
        except (OSError, ValueError, IndexError):
            pass
        self.tmp.cleanup()

    def cutover(self, *args, env=None, timeout=120):
        """The script under sh; an env value of None removes the variable."""
        e = {k: v for k, v in dict(self.env, **(env or {})).items() if v is not None}
        r = subprocess.run(["sh", CUTOVER, *args], env=e, capture_output=True, text=True, timeout=timeout)
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
    def test_activation_rejects_low_capacity_before_config_or_migration(self):
        write(os.path.join(self.fakebin, "df"), '#!/bin/sh\necho "fs 1000000 1 1000 1% /"\n', 0o755)
        legacy = os.path.join(self.tmp.name, "legacy", "keep")
        write(legacy, "keep")
        for args in (("compositor", "on"), ("compositor-boot",)):
            result = subprocess.run([self.env["LUMATV_TOOL"], *args], env=self.env, capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertIn("insufficient storage", result.stdout)
            self.assertEqual(read(legacy), "keep")
            self.assertFalse(os.path.exists(os.path.join(self.tmp.name, "data", "config.json")))

    def test_emergency_commands_do_not_check_storage(self):
        marker = os.path.join(self.tmp.name, 'df-called')
        write(os.path.join(self.fakebin, 'df'), '#!/bin/sh\ntouch "$FAKE/df-called"\nexit 1\n', 0o755)
        for args in (("compositor", "off"), ("stock-ui-restore",)):
            subprocess.run([self.env["LUMATV_TOOL"], *args], env=self.env, capture_output=True, text=True)
            self.assertFalse(os.path.exists(marker))

    def test_install_refuses_low_storage_before_writing_bundle(self):
        write(os.path.join(self.fakebin, "df"), '#!/bin/sh\necho "fs 1000000 1 1000 1% /"\n', 0o755)
        rc, out = self.cutover("install")
        self.assertNotEqual(rc, 0)
        self.assertIn("insufficient storage", out)
        self.assertFalse(os.path.exists(self.st("bin/lumatv-sm")))

    def test_it_records_what_the_boot_checks_compare_with(self):
        rc, out = self.cutover("install")
        self.assertEqual(rc, 0, out)
        self.assertEqual(read(self.st("fingerprint")).strip(), "abcdef0123456789")
        st = os.stat(self.lg)
        self.assertEqual(read(self.st("lg-binary")).strip(), "%d %d" % (st.st_size, st.st_mtime))
        self.assertEqual(read(self.st("supervisor-api")).strip(), "3")
        self.assertEqual(read(self.st("boot-direct-api")).strip(), "1", "the tool places the Home stub only on a bundle that says it reads the flag")
        self.assertEqual(read(self.st("failures")).strip(), "0")
        manifest = read(self.st("manifest")).split("\n")
        self.assertEqual(manifest[0], "release " + RELEASE.strip())
        self.assertEqual([l.split()[3] for l in manifest if l.startswith("file ")], [self.release, self.lg] + self.libs)
        bundle = {l.split()[2]: l.split()[:2] for l in read(self.st("bundle.manifest")).strip().split("\n")}
        self.assertRegex(bundle["bin/lumatv-compositor"][1], r"^[0-9a-f]{64}$")        # bin/: a hash
        self.assertEqual(bundle["lib/ld-linux-aarch64.so.1"][1], "-")                    # lib/: the size alone
        self.assertEqual(int(bundle["bin/lumatv-compositor"][0]), len("the compositor\n"))
        self.assertFalse(os.path.exists(self.st("enabled")))

    def test_install_replaces_the_ui_assets_as_a_bundle(self):
        write(os.path.join(self.dep, "assets", "ui", "fixture-asset"), "new")
        write(self.st("assets/ui/stale-asset"), "old")
        rc, out = self.cutover("install")
        self.assertEqual(rc, 0, out)
        self.assertEqual(read(self.st("assets/ui/fixture-asset")), "new")
        self.assertFalse(os.path.exists(self.st("assets/ui/stale-asset")))

    def test_install_without_ui_assets_does_not_change_the_installed_bundle(self):
        rc, out = self.cutover("install")
        self.assertEqual(rc, 0, out)
        before = read(self.st("bundle.manifest"))
        shutil.rmtree(os.path.join(self.dep, "assets", "ui"))
        rc, out = self.cutover("install")
        self.assertNotEqual(rc, 0, out)
        self.assertEqual(read(self.st("bundle.manifest")), before)
        self.assertTrue(os.path.isdir(self.st("assets/ui")))

    def test_the_rust_shell_of_a_ui_bundle_is_installed_beside_the_compositor_and_a_stale_one_is_dropped(self):
        # the supervisor runs the compositor from the installed copy, and the compositor looks for lumatv-ui beside itself
        write(os.path.join(self.dep, "bin", "lumatv-ui"), "the ui\n", 0o755)
        rc, out = self.cutover("install")
        self.assertEqual(rc, 0, out)
        self.assertEqual(read(self.st("bin/lumatv-ui")), "the ui\n")
        self.assertTrue(os.access(self.st("bin/lumatv-ui"), os.X_OK))
        self.assertIn("bin/lumatv-ui", [l.split()[2] for l in read(self.st("bundle.manifest")).strip().split("\n")])
        os.remove(os.path.join(self.dep, "bin", "lumatv-ui"))
        rc, out = self.cutover("install")
        self.assertEqual(rc, 0, out)
        self.assertFalse(os.path.exists(self.st("bin/lumatv-ui")))

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
        self.assertFalse(os.path.exists(self.st("enabled")))
        self.assertFalse(self.mounted())
        self.assertEqual(self.calls(), [])

    def test_a_session_runs_on_any_build_and_names_this_boot(self):
        rc, out = self.cutover("enable", "--session")
        self.assertEqual(rc, 0, out)
        self.assertEqual(read(self.st("enabled")).strip(), "session BOOT-A")
        self.assertEqual(json.loads(next(line for line in out.splitlines() if line.startswith('{')))["compositor"]["flag"], "session")
        self.assertTrue(self.mounted())
        self.assertIn("systemctl restart surface-manager-daemon", self.calls())
        self.assertIn("enabled (session)", out)

    def test_a_persistent_enable_on_a_listed_build_is_persistent(self):
        self.list_this_build()
        self.cutover("install")
        rc, out = self.cutover("enable")
        self.assertEqual(rc, 0, out)
        self.assertEqual(read(self.st("enabled")).strip(), "persistent")
        self.assertTrue(os.path.exists(self.st("fw-verified")))
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
        self.assertEqual(json.loads(next(line for line in out.splitlines() if line.startswith('{')))["compositor"]["flag"], "session")
        self.assertIn("t=10s", out)
        self.assertFalse(os.path.exists(self.st("enabled")))
        self.assertFalse(self.mounted())
        c = self.calls()
        self.assertEqual(c.count("systemctl restart surface-manager-daemon"), 2)         # in, and the revert out
        self.assertIn("recover", c)

    def test_restart_without_native_child_restores_stock(self):
        self.fake("no-native")
        rc, out = self.cutover("enable", "--session")
        self.assertNotEqual(rc, 0, out)
        self.assertFalse(self.mounted())
        self.assertFalse(os.path.exists(self.st("enabled")))
        self.assertTrue(os.path.exists(self.st("stock-ui-held")))
        self.assertFalse(os.path.exists(self.st("safety-timer")))

    def test_switching_does_not_restart_or_stop_sam(self):
        rc, out = self.cutover("enable", "--session")
        self.assertEqual(rc, 0, out)
        rc, out = self.cutover("disable")
        self.assertEqual(rc, 0, out)
        self.assertFalse(any(re.search(r'\bsam(?:\.service)?\b', call, re.I) for call in self.calls()), self.calls())

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
        write(self.st("bin/lumatv-compositor"), "the compositor\n")
        os.remove(self.st("supervisor-api"))
        rc, out = self.cutover("enable", "--session")
        self.assertEqual(rc, 7, out)
        self.assertFalse(json.loads(next(line for line in out.splitlines() if line.startswith('{')))["ok"])
        self.assertFalse(self.mounted())
        self.assertFalse(os.path.exists(self.st("enabled")))
        self.assertEqual(self.calls(), [])

    def test_an_unknown_argument_is_refused(self):
        rc, out = self.cutover("enable", "--forever")
        self.assertEqual(rc, 1)
        self.assertIn("enable [--session [--max-minutes N]]", out)

    def test_disable_removes_the_flag_and_the_unsettled_marker_and_restores_lgs(self):
        self.cutover("enable", "--session")
        write(self.st("running"), "BOOT-A 1\n")
        rc, out = self.cutover("disable")
        self.assertEqual(rc, 0, out)
        self.assertFalse(os.path.exists(self.st("enabled")) or os.path.exists(self.st("running")))
        self.assertFalse(self.mounted())

    def reboot(self):
        """The next boot: another boot id and no mount."""
        write(os.path.join(self.tmp.name, "boot_id"), "BOOT-B\n")
        write(os.path.join(self.tmp.name, "mountinfo"), "")

    def boot_hook(self):
        """What tools/lumatv compositor-boot runs at the early hook, against this set: its one-word answer."""
        e = dict(self.env, COMP=self.state, COMP_LG=self.lg, COMP_MOUNTINFO=self.env["LUMATV_MOUNTINFO"],
                 COMP_LOCK=self.env["LUMATV_RECOVER_LOCK"])
        r = subprocess.run(["sh", "-c", 'log_line() { :; }; . "$LUMATV_D/compositor.sh"; comp_boot_mount'], env=e, capture_output=True,
                           text=True, timeout=60)
        return r.stdout.strip()

    def test_a_persistent_enable_is_mounted_again_by_the_boot_hook_after_a_reboot(self):
        self.list_this_build()
        self.cutover("install")
        rc, out = self.cutover("enable")
        self.assertEqual(rc, 0, out)
        self.reboot()
        rc, out = self.cutover("boot-check")
        j = json.loads(out)
        self.assertEqual((j["flag"], j["decision"], j["wouldMount"]), ("persistent", "ok", True), out)
        self.assertEqual(self.boot_hook(), "mounted")
        self.assertTrue(self.mounted())
        self.assertEqual(read(self.st("enabled")).strip(), "persistent")

    def test_a_persistent_enable_that_was_disabled_stays_off_after_a_reboot(self):
        self.list_this_build()
        self.cutover("install")
        self.cutover("enable")
        rc, out = self.cutover("disable")
        self.assertEqual(rc, 0, out)
        self.reboot()
        self.assertEqual(json.loads(self.cutover("boot-check")[1])["decision"], "held")
        self.assertEqual(self.boot_hook(), "held")
        self.assertFalse(self.mounted())

    def test_a_firmware_update_after_a_persistent_enable_boots_on_lgs_compositor(self):
        self.list_this_build()
        self.cutover("install")
        self.cutover("enable")
        self.reboot()
        with open(self.lg, "w") as f:
            f.write("LG's compositor, after an update")
        self.assertEqual(json.loads(self.cutover("boot-check")[1])["decision"], "firmware")
        self.assertEqual(self.boot_hook(), "firmware")
        self.assertFalse(self.mounted())

    def test_an_enable_the_supervisor_falls_back_from_is_a_failure_that_leaves_no_flag(self):
        # the unit's restart is where the supervisor decides: here it hands the screen back at once
        write(os.path.join(self.fakebin, "systemctl"), '''#!/bin/sh
case "$1" in
  is-active) echo active ;;
  show) case "$*" in *MainPID*--value*) echo 77 ;; *) echo MainPID=77; echo ActiveEnterTimestampMonotonic=5 ;; esac ;;
  restart)
    echo "systemctl $*" >> "$FAKE/calls"
    grep -vF " $LUMATV_LG " "$LUMATV_MOUNTINFO" > "$LUMATV_MOUNTINFO.new"; mv "$LUMATV_MOUNTINFO.new" "$LUMATV_MOUNTINFO"
    echo "heartbeat: two beats missed" > "$LUMATV_STATE/last-fallback" ;;
  *) echo "systemctl $*" >> "$FAKE/calls" ;;
esac
''', 0o755)
        self.list_this_build()
        self.cutover("install")
        for args in ((), ("--session",)):
            rc, out = self.cutover("enable", *args, timeout=30)
            self.assertEqual(rc, 9, out)
            self.assertFalse(json.loads(next(line for line in out.splitlines() if line.startswith('{')))["ok"])
            self.assertEqual(read(self.st("last-fallback")).strip(), "heartbeat: two beats missed")
            self.assertTrue(os.path.exists(self.st("stock-ui-held")))
            self.assertNotIn("stays live", out)
            self.assertFalse(os.path.exists(self.st("enabled")), args)
            self.assertFalse(self.mounted())
            self.assertEqual(json.loads(self.cutover("boot-check")[1])["decision"], "held")


class SafetyTimer(FakeSet):
    """`enable --session --max-minutes N`: the detached job that puts LG's compositor back when a session outlives its
    minutes (the workstation that drove it died). The job runs for real here, under setsid, as the script's own
    `safety-run`, at one second to the minute."""

    def setUp(self):
        super().setUp()
        self.cutover("install")

    def record(self):
        pid, end, mins = read(self.st("safety-timer")).split()
        return int(pid), int(end), int(mins)

    def restarts(self):
        return self.calls().count("systemctl restart surface-manager-daemon")

    def test_a_session_starts_a_detached_timer_of_15_minutes_unless_told_otherwise(self):
        t0 = time.time()
        rc, out = self.cutover("enable", "--session", env={"LUMATV_SAFETY_MINUTE": None}, timeout=20)   # the set's own minute
        self.assertEqual(rc, 0, out)
        self.assertLess(time.time() - t0, 10, "the job holds the caller's pipes")
        pid, end, mins = self.record()
        self.assertEqual(mins, 15)
        self.assertAlmostEqual(end - uptime(), 15 * 60, delta=5)
        self.assertIn("ends by itself after 15 min", out)
        self.assertTrue(alive(pid))
        with open("/proc/%d/cmdline" % pid) as f:
            self.assertIn("safety-run", f.read())
        self.assertNotEqual(session_of(pid), session_of(os.getpid()))
        for fd in (0, 1, 2):
            self.assertEqual(os.readlink("/proc/%d/fd/%d" % (pid, fd)), os.devnull)

    def test_max_minutes_sets_the_length(self):
        rc, out = self.cutover("enable", "--session", "--max-minutes", "3", env={"LUMATV_SAFETY_MINUTE": "60"}, timeout=20)
        self.assertEqual(rc, 0, out)
        pid, end, mins = self.record()
        self.assertEqual(mins, 3)
        self.assertAlmostEqual(end - uptime(), 180, delta=5)
        self.assertIn("ends by itself after 3 min", out)

    def test_a_length_that_is_not_a_whole_number_of_minutes_is_refused_before_anything_happens(self):
        for bad in ("0", "-1", "x", "1.5", "05", "1441", ""):
            rc, out = self.cutover("enable", "--session", "--max-minutes", bad)
            self.assertEqual(rc, 1, (bad, out))
            self.assertIn("--max-minutes N: whole minutes, 1 to 1440", out)
        rc, out = self.cutover("enable", "--session", "--max-minutes")
        self.assertEqual(rc, 1, out)
        self.assertEqual(self.calls(), [])
        self.assertFalse(os.path.exists(self.st("enabled")) or os.path.exists(self.st("safety-timer")))

    def test_a_persistent_enable_has_no_timer_and_the_option_needs_a_session(self):
        self.list_this_build()
        self.cutover("install")
        rc, out = self.cutover("enable", "--max-minutes", "5")
        self.assertEqual(rc, 1, out)
        self.assertIn("goes with --session", out)
        self.assertFalse(self.mounted())
        rc, out = self.cutover("enable")
        self.assertEqual(rc, 0, out)
        self.assertFalse(os.path.exists(self.st("safety-timer")))
        self.assertNotIn("safety timer", out)

    def test_a_refused_enable_starts_no_timer(self):
        self.fake("sm-says", "no escaped: escape recorded; re-enable from the tool\n")
        rc, out = self.cutover("enable", "--session")
        self.assertEqual(rc, 7, out)
        self.assertFalse(os.path.exists(self.st("safety-timer")))

    def test_a_session_whose_timer_cannot_start_is_not_mounted(self):
        os.makedirs(self.st("safety-timer"))             # the job cannot write its record there
        rc, out = self.cutover("enable", "--session", timeout=30)
        self.assertEqual(rc, 8, out)
        self.assertIn("the safety timer did not start; nothing was mounted", out)
        self.assertFalse(self.mounted())
        self.assertFalse(os.path.exists(self.st("enabled")))
        self.assertEqual(self.calls(), [])

    def test_the_job_ends_a_session_that_outlives_its_minutes(self):
        rc, out = self.cutover("enable", "--session", "--max-minutes", "2", timeout=20)
        self.assertEqual(rc, 0, out)
        pid = self.record()[0]
        self.assertTrue(self.mounted())
        self.assertTrue(wait_until(lambda: not alive(pid), 10))
        self.assertFalse(self.mounted())
        self.assertFalse(os.path.exists(self.st("enabled")))
        self.assertFalse(os.path.exists(self.st("safety-timer")))
        c = self.calls()
        self.assertEqual(c.count("systemctl restart surface-manager-daemon"), 2)         # the session's start and the job's disable
        self.assertIn("umount " + self.lg, c)
        self.assertEqual(c.count("recover"), 2)

    def test_disable_and_revert_cancel_it(self):
        for ending in ("disable", "revert"):
            with self.subTest(ending):
                self.cutover("enable", "--session", "--max-minutes", "2", timeout=20)
                pid = self.record()[0]
                rc, out = self.cutover(ending)
                self.assertEqual(rc, 0, out)
                self.assertIn("safety timer cancelled", out)
                self.assertTrue(wait_until(lambda: not alive(pid), 3))
                self.assertFalse(os.path.exists(self.st("safety-timer")))
                self.assertFalse(self.mounted())

    def test_a_cancelled_timer_never_runs_disable_when_its_time_comes(self):
        self.cutover("enable", "--session", "--max-minutes", "2", timeout=20)
        self.cutover("disable")
        self.assertEqual(self.restarts(), 2)
        time.sleep(2.6)                                  # past what its deadline would have been
        self.assertEqual(self.restarts(), 2)

    def test_a_second_session_replaces_the_timer_of_the_first(self):
        self.cutover("enable", "--session", "--max-minutes", "5", timeout=20)
        first = self.record()[0]
        write(os.path.join(self.tmp.name, "mountinfo"), "")        # the supervisor fell back: the session is over
        self.cutover("enable", "--session", "--max-minutes", "7", timeout=20)
        pid, _, mins = self.record()
        self.assertNotEqual(pid, first)
        self.assertEqual(mins, 7)
        self.assertTrue(wait_until(lambda: not alive(first), 3))
        self.assertTrue(alive(pid))

    def test_a_fallback_with_a_session_flag_is_disabled_by_the_timer(self):
        self.cutover("enable", "--session", "--max-minutes", "2", timeout=20)
        pid = self.record()[0]
        write(os.path.join(self.tmp.name, "mountinfo"), "")        # the supervisor fell back to LG's compositor
        self.assertTrue(wait_until(lambda: not alive(pid), 10))
        self.assertEqual(self.restarts(), 2)
        self.assertFalse(os.path.exists(self.st("enabled")))
        self.assertTrue(os.path.exists(self.st("stock-ui-held")))
        self.assertNotIn("umount " + self.lg, self.calls())

    def test_a_persistent_flag_is_not_the_timers_to_end(self):
        self.cutover("enable", "--session", "--max-minutes", "2", timeout=20)
        pid = self.record()[0]
        write(self.st("enabled"), "persistent\n")                   # `lumatv compositor on` while the session ran
        self.assertTrue(wait_until(lambda: not alive(pid), 10))
        self.assertTrue(self.mounted())
        self.assertEqual(self.restarts(), 1)
        self.assertEqual(read(self.st("enabled")).strip(), "persistent")

    def test_a_record_naming_another_process_is_not_a_timer(self):
        other = subprocess.Popen(["sleep", "30"])
        self.addCleanup(other.wait)
        self.addCleanup(other.kill)
        write(self.st("safety-timer"), "%d 0 15\n" % other.pid)
        rc, out = self.cutover("status")
        self.assertIn("safety timer: none", out)
        rc, out = self.cutover("disable")
        self.assertNotIn("cancelled", out)
        self.assertIsNone(other.poll())
        self.assertFalse(os.path.exists(self.st("safety-timer")))

    def test_status_says_how_long_the_timer_has_left(self):
        env = {"LUMATV_SAFETY_MINUTE": "60"}
        self.cutover("enable", "--session", "--max-minutes", "7", env=env, timeout=20)
        rc, out = self.cutover("status", env=env)
        self.assertRegex(out, r"safety timer: pid \d+, [67] of 7 min left")
        self.cutover("disable", env=env)
        rc, out = self.cutover("status", env=env)
        self.assertIn("safety timer: none", out)

    def test_a_trial_is_covered_for_its_watch_and_five_minutes_and_ends_with_it(self):
        rc, out = self.cutover("trial", "120", timeout=30)
        self.assertEqual(rc, 0, out)
        self.assertIn("ends by itself after 7 min", out)
        self.assertFalse(os.path.exists(self.st("safety-timer")))        # the trial's own revert cancelled it

    def test_a_deploy_over_the_script_while_the_job_sleeps_does_not_reach_the_job(self):
        script = os.path.join(self.tmp.name, "deployed.sh")
        shutil.copy(CUTOVER, script)
        r = subprocess.run(["sh", script, "enable", "--session", "--max-minutes", "2"], env=self.env, capture_output=True, text=True, timeout=20)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        pid = self.record()[0]
        # The next deploy, written in place as the copy to the set is: every line of it would show if the job read on into it.
        write(script, '[ "$1" = disable ] && { echo replaced-disable >> "$FAKE/calls"; exit 0; }\n' + 'echo read-on >> "$FAKE/calls"\n' * 400)
        self.assertTrue(wait_until(lambda: not alive(pid), 10))
        self.assertIn("replaced-disable", self.calls())
        self.assertNotIn("read-on", self.calls())


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
        self.assertIn("flag: off; firmware: unverified; supervisor-api: 3", out)
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
            # a refresh with nothing new to show: not a frame the panel missed
            "[srv  43297.600] info kms: page-flip interval 33.37 ms, nothing due at the refresh between (one refresh is 16.67)",
            "[srv  43300.000] warn frame gap 33.3 ms on surface 7 (x1) after 291 steady frames",           # over, inside
            "[srv  43300.100] warn frame gap 18.0 ms on surface 7 (x1) after 292 steady frames",
            "[srv  43357.053] info kms: page-flip stamp 33.37 ms after the last (one refresh is 16.67)",   # after the client left
            "[srv  43290.000] warn loop: 90.0 ms in one pass (accept)",                      # before it started
            "[srv  43298.000] info client connected: pid 1 uid 0 cmd x"]) + "\n")
        self.assertEqual(self.sh("comp_over_limit '%s' 43296.9 43356.976" % log), (0, "3"))
        self.assertEqual(self.sh("comp_over_limit '%s' 0 99999" % log), (0, "5"))
        self.assertEqual(self.sh("comp_over_limit /nonexistent 0 99999"), (0, "unreadable"))
        # `own`: the client's frame gaps (late commits) are not the compositor's; the loop's passes and the flip intervals are
        self.assertEqual(self.sh("comp_over_limit '%s' 43296.9 43356.976 own" % log), (0, "2"))
        self.assertEqual(self.sh("comp_over_limit '%s' 0 99999 own" % log), (0, "4"))

    ONE_GAP = GOOD.replace("20-33.5 0, >33.5 0", "20-33.5 1, >33.5 0")

    def attributed(self, result, att, own, comp=0, before="1", after="1", asked=60):
        return self.sh("x1_rule '%s' '%s' '%s' '%s' '%s' '%s' '%s'" % (result, asked, before, after, comp, att, own))

    def test_with_an_attribution_the_clients_own_gaps_are_reported_and_not_counted(self):
        rc, out = self.attributed(self.ONE_GAP, "0 1 0", "0", comp=1)
        self.assertEqual(rc, 0, out)
        self.assertTrue(out.startswith("PASS 60.00 fps over 60.00 s, no compositor-attributable gap over 20 ms, pacer late unchanged"), out)
        self.assertIn("1 client-only of 1 gap(s) over 20 ms (worst", out)
        self.assertIn("reported, not counted", out)
        self.assertIn("the compositor logged 1 client frame gap(s) (late commits)", out)
        rc, out = self.rule(self.ONE_GAP)
        self.assertEqual((rc, out.startswith("FAIL 1 gap(s) over 20 ms in the client's swaps")), (1, True), "without an attribution the PLAN's rule stands")

    def test_with_an_attribution_a_gap_that_is_the_compositors_a_late_flip_or_an_unexplained_gap_fails(self):
        rc, out = self.attributed(self.ONE_GAP, "1 0 0", "0")
        self.assertEqual(rc, 1)
        self.assertIn("1 gap(s) or late flip(s) are the compositor's", out)
        rc, out = self.attributed(self.ONE_GAP, "0 0 1", "0")
        self.assertEqual(rc, 1)
        self.assertIn("1 gap(s) over 20 ms in the client's swaps the trace cannot explain", out)
        rc, out = self.attributed(GOOD, "1 0 0", "0")
        self.assertEqual(rc, 1, "a late flip fails whether or not the client counted a gap with it")
        self.assertIn("are the compositor's", out)

    def test_with_an_attribution_the_length_the_rate_the_pacer_and_the_loops_own_log_still_count(self):
        self.assertEqual(self.attributed(GOOD, "0 0 0", "2"), (1, "FAIL the compositor logged 2 loop pass(es) or page-flip interval(s) over 20 ms during the run"))
        self.assertEqual(self.attributed(GOOD, "0 0 0", ""), (1, "FAIL the compositor's log of the run could not be read"))
        self.assertEqual(self.attributed(self.ONE_GAP, "0 1 0", "0", before="0", after="1"), (1, "FAIL pacer late 0 -> 1"))
        self.assertIn("55.10 fps is under 59.5", self.attributed(GOOD.replace("= 60.00 fps", "= 55.10 fps"), "0 0 0", "0")[1])
        self.assertIn("the run asked for 10 s", self.attributed(GOOD, "0 0 0", "0", asked=10)[1])
        rc, out = self.attributed(GOOD, "0 0 0", "0")
        self.assertEqual(rc, 0, out)
        self.assertNotIn("reported", out)

    def decision(self, result=GOOD, attr="0 0 0", coverage="COMPLETE", strict="", comp=0):
        return self.sh("x1_decision '%s' 60 1 1 %s '%s' 0 '%s' '%s'" % (result, comp, attr, coverage, strict))

    def test_attribution_never_hides_a_raw_strict_failure(self):
        rc, out = self.decision(self.ONE_GAP, attr="0 1 0")
        self.assertEqual(rc, 0, out)
        self.assertIn("RAW STRICT: FAIL 1 gap(s) over 20 ms", out)
        self.assertIn("RAW RESULT: " + self.ONE_GAP, out)
        self.assertIn("client-only", out)

    def test_missing_or_incomplete_capture_is_inconclusive_even_when_raw_strict_passes(self):
        for coverage, attr in [("", "0 0 0"), ("missing capture", ""), ("INCONCLUSIVE; lost events", "0 0 0"), ("COMPLETE", "")]:
            with self.subTest(coverage=coverage, attr=attr):
                rc, out = self.decision(coverage=coverage, attr=attr)
                self.assertEqual(rc, 2, out)
                self.assertTrue(out.startswith("INCONCLUSIVE attribution:"), out)
                self.assertIn("RAW STRICT: PASS", out)
        rc, out = self.decision(self.ONE_GAP, attr="0 1 0", coverage="INCONCLUSIVE; overwritten events")
        self.assertEqual(rc, 2)
        self.assertIn("RAW STRICT: FAIL", out)

    def test_explicit_strict_mode_remains_strict_without_a_trace(self):
        rc, out = self.decision(coverage="missing capture", strict="1")
        self.assertEqual(rc, 0, out)
        rc, out = self.decision(self.ONE_GAP, attr="0 1 0", coverage="missing capture", strict="1")
        self.assertEqual(rc, 1, out)
        self.assertTrue(out.startswith("FAIL 1 gap(s) over 20 ms"), out)


class GateRecovery(FakeSet):
    def shell(self, script):
        return subprocess.run(["sh", "-c", '. "$1"; ' + script, "sh", GATE],
                              env=dict(self.env, GATE_LIB="1"), capture_output=True, text=True, timeout=10)

    def test_waits_for_the_actual_lock_and_times_out_if_held(self):
        with open(self.env["LUMATV_RECOVER_LOCK"], "w") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            process = subprocess.Popen(["sh", "-c", '. "$1"; sleep() { /bin/sleep 0.05; }; wait_recovery', "sh", GATE],
                                       env=dict(self.env, GATE_LIB="1"), stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            try:
                time.sleep(0.15)
                self.assertIsNone(process.poll())
                fcntl.flock(lock, fcntl.LOCK_UN)
                self.assertEqual(process.wait(timeout=5), 0)
            finally:
                if process.poll() is None:
                    process.kill()
                process.communicate()
            fcntl.flock(lock, fcntl.LOCK_EX)
            result = self.shell('sleep() { :; }; wait_recovery')
            self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.shell('wait_recovery').returncode, 0)
        os.remove(os.path.join(self.dep, "compositor.sh"))
        self.assertNotEqual(self.shell('wait_recovery').returncode, 0)

    def test_fallback_reason_uses_persisted_reason_only_when_log_is_empty(self):
        write(self.st("last-fallback"), "hold Home 10 s (supervisor)")
        result = self.shell('comp_log() { :; }; fallback_evidence')
        self.assertEqual(result.stdout.strip(), "hold Home 10 s (supervisor)")
        result = self.shell('comp_log() { echo "fallback: other reason"; }; fallback_evidence')
        self.assertEqual(result.stdout.strip(), "fallback: other reason")


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


# The compositor's `ctl` and `luna-send` as the keys and ime checks see them: one small model of the journal, the remote's routing, the windows,
# maliit's keyboard and SAM's foreground. Its state is $FAKE/sim.json; $FAKE/sim-config.json says what it should get wrong. LG's Settings and
# Search are overlays here as on the set: their windows are mapped, and SAM never names them as the foreground app.
SIM_PRELUDE = r'''#!/usr/bin/env python3
import json, os, sys
F = os.environ["FAKE"]
OVERLAYS = ("com.palm.app.settings", "com.webos.app.voice")


def load(name, default):
    try:
        with open(os.path.join(F, name)) as f:
            return json.load(f)
    except (OSError, ValueError):
        return default


cfg = load("sim-config.json", {})
s = load("sim.json", {"journal": [], "windows": [["com.webos.app.home", "Card"]], "down": [], "panel": None, "ctx": False, "moved": False,
                      "oks": 0, "commits": 0, "launched": [], "closed": [], "keys": []})


def save():
    with open(os.path.join(F, "sim.json"), "w") as f:
        json.dump(s, f)


def out(o):
    print(json.dumps(o, sort_keys=True, separators=(",", ":")))


def open_app(app):
    s["launched"].append(app)
    s["windows"] = [w for w in s["windows"] if w[0] != app] + [[app, "Overlay" if app in OVERLAYS else "Card"]]


def drop_app(app):
    s["windows"] = [w for w in s["windows"] if w[0] != app]


def front():
    return [a for a, t in s["windows"] if t == "Card"][-1]


def pid_of(app):
    return 1000 + sum(map(ord, app)) % 1000


RUST_PANELS = ("quick", "picker", "drawer")


def rust_state():
    """Native UI state, declared kinds and rendering counters."""
    default_kinds = ["home", "osd", "settings", "toast", "cameras", "quick", "picker", "drawer", "menu", "confirm", "switcher", "entry", "keyboard", "alert", "saver", "capture", "cards", "popup", "layout", "ai", "banner", "displaylab"] if cfg.get("native") else []
    r = s.setdefault("rust", {"kinds": default_kinds, "toast_ttl": 0, "drawn": 100, "gl": 0,
                              "ui": {"osd": False, "toasts": 0, "alerts": 0, "saver": False, "panels": {}, "settings": {"open": False},
                                     "cameras": {"screen": {"mode": ""}}, "home": {"home": True, "focusLayer": "dock"}}})
    return r


def rust_built(r):
    return [k for k in r["kinds"] if k in cfg.get("rust_built", r["kinds"])]


def rust_drew():
    r = rust_state()
    if not cfg.get("rust_no_frames"):
        r["drawn"] += 12
    if cfg.get("rust_gl"):
        r["gl"] += 1


def rust_tap(code):
    r = rust_state()
    ui, kinds = r["ui"], r["kinds"]
    if code in (139, 241) and cfg.get("native"):
        kind = "quick" if code == 139 else "picker"
        if kind in rust_built(r) and kind not in cfg.get("rust_dead", []):
            ui["panels"][kind] = "open"
            r["wants"] = front() != "com.webos.app.home" and not cfg.get("native_no_wants")
            rust_drew()
    elif code == 115 and "osd" in kinds and "osd" not in cfg.get("rust_dead", []):
        ui["osd"] = True
        rust_drew()
    elif code == 114 and ui["osd"]:
        ui["osd"] = False
    elif code == 28 and ui["alerts"]:
        ui["alerts"] = 0
    elif code == 106 and ui["saver"]:
        ui["saver"] = False
    elif code == 103 and ui["home"]["focusLayer"] == "dock":
        ui["home"]["focusLayer"] = "widgets"
        rust_drew()
    elif code == 108 and ui["home"]["focusLayer"] == "widgets":
        ui["home"]["focusLayer"] = "dock"
        rust_drew()
    elif code == 412 and not cfg.get("native_back_stuck"):
        for k in RUST_PANELS:
            if ui["panels"].get(k) == "open":
                ui["panels"][k] = "parked"
        ui["settings"]["open"] = False
        ui["cameras"]["screen"]["mode"] = ""
        r["wants"] = False


'''
SIM_LOADER = '#!/bin/sh\nshift 2\nexec "$@"\n'
SIM_COMPOSITOR = SIM_PRELUDE + r'''
cmd, rest = sys.argv[4], sys.argv[5:]


def say(line):
    s["journal"].append(line)


def held():
    say("input: 5 held: Quick Access slot 5 has no window of its own here yet, launching com.webos.app.magicnum")
    open_app("com.webos.app.magicnum")


def ok():
    if s["panel"] is None:
        s["oks"] += 1
        if cfg.get("keyboard", True) and s["oks"] >= cfg.get("oks_needed", 1):
            s["panel"] = {"x": 0, "y": 720, "w": 1920, "h": 360}
            s["ctx"] = True
    elif not cfg.get("typing_dead") and (cfg.get("first_key_types", True) or s["moved"]):
        s["commits"] += 1


def tap(code):
    rust_tap(code)
    s["keys"].append(code)
    if cfg.get("native") and code in (139, 241):
        kind = "quick" if code == 139 else "picker"
        if rust_state()["ui"]["panels"].get(kind) == "open":
            say("input: gear tap: Quick Settings -> LumaTV's quick panel" if code == 139 else
                "input: Input tap: the input picker -> LumaTV's input picker")
        return
    focused = s["windows"][-1][0]
    if code == 139:
        say("input: gear tap: Quick Settings has no window of its own here yet, launching com.palm.app.settings")
        open_app("com.palm.app.settings")
    elif code == 241:
        say("input: Input tap: the input picker has no window of its own here yet, launching com.webos.app.homeconnect")
        open_app("com.webos.app.homeconnect")
    elif code == 1037 and not cfg.get("netflix_dead"):
        choice = "netflix" if cfg.get("ignore_choice") else cfg.get("app_keys", {}).get("netflix", "netflix")
        if choice:
            say("input: app button: %s, launching %s" % (choice, choice))
            open_app(choice)
    elif code == 6 and cfg.get("digit_tap_asks"):
        held()
    elif code == 174 and s["panel"]:
        s["panel"] = None
        say("input: Back or Exit: closing the keyboard")
    elif code == 174 and focused == "com.webos.app.home":
        say("input: Back or Exit on Home: nothing to close")
    elif code == 174:
        say("input: Back or Exit: closing " + focused)
        if not cfg.get("close_fails"):
            drop_app(focused)
    elif code == 412 and s["panel"] and cfg.get("back_closes", True):
        s["panel"] = None
    elif code in (106, 108) and s["panel"]:
        s["moved"] = True
    elif code == 28 and focused == "com.webos.app.voice":
        ok()


if cmd == "journal":
    out({"ok": True, "lines": s["journal"]})
elif cmd == "note":
    say("note: " + " ".join(rest))
    out({"ok": True})
elif cmd == "remote":
    code, how = int(rest[0]), rest[1]
    if how == "tap":
        tap(code)
    elif how == "press":
        s["down"].append(code)
    elif code in s["down"]:
        s["down"].remove(code)
        if code == 6:
            held()
    out({"ok": True})
elif cmd == "windows":
    ws = [{"appId": a, "id": 2 + i, "keyMask": 4286578682, "mapped": True, "position": [0, 0], "props": {}, "size": [1920, 1080],
           "state": "Fullscreen", "surface": 103 + i, "title": a, "type": t} for i, (a, t) in enumerate(s["windows"])]
    if s["panel"]:
        ws.append({"appId": "", "id": 90, "keyMask": 0, "mapped": True, "position": [0, 720], "props": {}, "size": [1920, 360],
                   "state": "Default", "surface": 221, "title": "MaliitServer", "type": "Keyboard"})
    out({"groups": [], "ok": True, "video": [], "windows": ws})
elif cmd == "keys":
    out({"ok": True, "table": [{"name": n, "code": c} for n, c in (("gear", 139), ("input", 241), ("netflix", 1037))], "buttons": {k: ("app:" + v if v else "nothing") for k, v in cfg.get("app_keys", {}).items()},
         "unnamed": cfg.get("unnamed", []), "foreground": s["windows"][-1][0], "keyboardUp": s["panel"] is not None, "delivered": 0, "dropped": 0})
elif cmd == "shell" and rest[:1] == ["rust"]:
    s.setdefault("kind_changes", []).append(rest[1:])
    out({"ok": False, "error": "shell [restart | show KIND [PAGE]]"})
elif cmd == "shell" and rest[:1] == ["show"]:
    r = rust_state()
    kind = rest[1]
    ui = r["ui"]
    if kind in rust_built(r) and kind not in cfg.get("rust_dead", []):
        if kind in RUST_PANELS:
            ui["panels"][kind] = "open"
        elif kind == "settings":
            ui["settings"]["open"] = True
        elif kind == "cameras":
            ui["cameras"]["screen"]["mode"] = "grid"
        rust_drew()
    out({"ok": True, "kind": kind, "taken": kind in rust_built(r)})
elif cmd == "shell":
    # the shell client's report as `ctl shell` carries it: the refresh it draws for and its per-frame slides (`Lu.slides`), and the Rust shell's
    r = rust_state()
    if r["toast_ttl"] > 0:
        r["toast_ttl"] -= 1
        if r["toast_ttl"] == 0:
            r["ui"]["toasts"] = 0
    built = [] if cfg.get("rust_no_ui") else rust_built(r)
    ui = dict(r["ui"], kinds=built)
    if cfg.get("native"):
        connected = not cfg.get("rust_no_ui")
        engine = cfg.get("native_engine", True) and connected
        frame_ready = cfg.get("native_frame_ready", True) and engine
        out({"ok": True, "kinds": r["kinds"], "connected": connected, "engine": engine,
             "frameReady": frame_ready, "ready": connected and engine and frame_ready and set(r["kinds"]).issubset(built),
             "error": None if connected else "no native UI", "can": built, "ui": ui,
             "wants": r.get("wants", False), "pid": 99999999,
             "stats": {"starts": 1, "exits": 0}})
    else:
        out({"ok": True, "connected": True, "ui": {"state": {"refreshHz": cfg.get("refresh_hz", 60), "slides": cfg.get("slides", [])}},
             "rust": {"kinds": r["kinds"], "connected": bool(built), "error": "no lumatv-ui" if cfg.get("rust_no_ui") else None, "can": built, "ui": ui}})
elif cmd == "callbacks":
    s.setdefault("pacing", {"clients": "commit"})[rest[0]] = rest[1]
    out({"ok": True, rest[0]: rest[1]})
elif cmd == "status":
    r = rust_state()
    out({"ok": True, "summary": "uptime 1.0s clients 2 gl errors %d render last 5 us" % r["gl"],
         "stats": {"drawn": r["drawn"], "callbacks": 16657, "callbacksAhead": 15682, "callbacksAtCommit": 3,
                   "clientCallbacks": s.get("pacing", {}).get("clients", "commit")}})
elif cmd == "ime":
    up = s["panel"] is not None
    out({"ok": True, "asked": 1, "methods": [{"id": 5, "display": 0}] * cfg.get("methods", 1), "models": 1, "panel": s["panel"],
         "keyboardUp": up, "grab": up, "grabbing": up,
         "active": {"app": s["windows"][-1][0], "context": True, "model": 1, "showWanted": False, "surface": 3} if s["ctx"] else None,
         "counts": {"activations": 1, "declined": 0, "commits": s["commits"] + cfg.get("commits_before", 0), "preedits": 0, "deletes": 0,
                    "keysyms": 0, "grabbedKeys": s["commits"]}})
else:
    out({"ok": False, "error": "the model knows no " + cmd})
save()
'''
SIM_LUNA = SIM_PRELUDE + r'''
def sam_log(app, status):
    """SAM's APP_CLOSED line for APP, in the log the gate reads."""
    path = os.environ.get("LUMATV_MESSAGES")
    if path:
        with open(path, "a") as f:
            f.write('2026-10-28T17:28:04.285632Z user.info sam [] SAM APP_CLOSED {"app_id":"%s","type":"native","pid":"%d","exit_status":%d} \n' % (app, pid_of(app), status))


def crash_report(app):
    """PlxNative's own crash report, as LG's SDL appends it to its log."""
    path = os.environ.get("LUMATV_PLX_CRASH_LOG")
    if path and app == "com.beb.plxnative":
        with open(path, "a") as f:
            f.write("\n*** SIGNAL 11 (SIGSEGV) addr=0x18 pc=0xf7c075c6 lr=0xf7bf24b5\ntime: 1793249407\n")


uri = [a for a in sys.argv[1:] if a.startswith("luna://")][0]
payload = sys.argv[-1] if not sys.argv[-1].startswith("luna://") else "{}"
method = uri.rsplit("/", 1)[1]
if method == "getForegroundAppInfo":
    out({"returnValue": True, "appId": front()})
elif method == "getAppInfo":
    out({"returnValue": json.loads(payload)["id"] in cfg.get("installed", ["com.geforcenow.play"])})
elif method == "launch":
    request = json.loads(payload)
    app = request["id"]
    s.setdefault("journal_at_launch", {})[app] = len(s["journal"])
    if app in cfg.get("unlaunchable", []):
        s["launched"].append(app)
        out({"returnValue": False, "errorText": "no such app"})
    elif app == "com.geforcenow.play":
        # NVIDIA's web app opens LG's browser and ends itself
        s["launched"].append(app)
        open_app("com.webos.app.browser")
        out({"returnValue": True})
    else:
        open_app(app)
        if app == "com.webos.app.home" and cfg.get("stray_browser") and any(w[0] == "com.webos.app.browser" for w in s["windows"]):
            # the browser asks for fullscreen again and takes the front back (2026-10-28)
            s["windows"] = [w for w in s["windows"] if w[0] != "com.webos.app.browser"] + [["com.webos.app.browser", "Card"]]
        if app != "com.webos.app.home":
            s["journal"].append("client connected: pid %d uid 5365 cmd \"/media/developer/apps/usr/palm/applications/%s/main {\\\"event\\\":\\\"launch\\\"}\"" % (pid_of(app), app))
        if app in cfg.get("dies", []):
            # an app that SIGSEGVs after its map, as YouTube and PlxNative did: its connection goes, SAM logs the signal
            s["journal"].append("client disconnected: pid %d uid 5365 after 599 ms (ConnectionClosed); bound [\"wl_compositor\"]" % pid_of(app))
            sam_log(app, 11)
            drop_app(app)
        if app == "com.webos.app.voice" and request.get("params", {}).get("launchMode") == "vkb" and cfg.get("vkb", True) and cfg.get("keyboard", True):
            s["panel"] = {"x": 0, "y": 720, "w": 1920, "h": 360}
            s["ctx"] = True
        out({"returnValue": True})
elif method == "closeByAppId":
    app = json.loads(payload)["id"]
    s["closed"].append(app)
    if any(w[0] == app for w in s["windows"]):
        if app in cfg.get("crashes_at_close", []) or app in cfg.get("crash_report_at_close", []):
            # PlxNative's SIGSEGV after its close: its report and SAM's line come after the app has left SAM's running list (round 16,
            # about a second); SAM's line was missing or read exit 0 for 4 of 9 crashes (round 17)
            if os.fork() == 0:
                import time
                null = os.open(os.devnull, os.O_RDWR)
                for fd in (0, 1, 2):
                    os.dup2(null, fd)
                time.sleep(0.3)
                crash_report(app)
                sam_log(app, 11 if app in cfg.get("crashes_at_close", []) else 0)
                os._exit(0)
        else:
            sam_log(app, 0)
    drop_app(app)
    if app == "com.webos.app.voice":
        s["panel"] = None
        s["ctx"] = False
    out({"returnValue": True})
elif method in ("createToast", "createAlert", "turnOnScreenSaver"):
    r = rust_state()
    kind = {"createToast": "toast", "createAlert": "alert", "turnOnScreenSaver": "saver"}[method]
    if kind in rust_built(r) and kind not in cfg.get("rust_dead", []):
        if kind == "toast":
            r["ui"]["toasts"] = 1
            r["toast_ttl"] = 4
        elif kind == "alert":
            r["ui"]["alerts"] = 1
        else:
            r["ui"]["saver"] = True
        rust_drew()
    out({"returnValue": True})
elif method == "executeOneShot":
    out({"returnValue": True, "writtenBytes": 4321})
elif method == "running":
    # `luna-send -f` prints the answer indented, with a space after each colon: the check must read it so
    print(json.dumps({"returnValue": True, "running": [{"appType": "native", "id": a, "processid": str(pid_of(a))} for a, t in s["windows"]]}, indent=4, sort_keys=True))
else:
    out({"returnValue": True})
save()
'''


class GateKeysAndIme(FakeSet):
    """tv-gate.sh keys and ime against a small model of the compositor's journal and routing, maliit's keyboard and SAM's foreground."""

    def setUp(self):
        super().setUp()
        write(os.path.join(self.dep, "lib", "ld-linux-aarch64.so.1"), SIM_LOADER, 0o755)
        write(os.path.join(self.dep, "bin", "lumatv-compositor"), SIM_COMPOSITOR, 0o755)
        write(os.path.join(self.fakebin, "luna-send"), SIM_LUNA, 0o755)
        shutil.copy(os.path.join(TOOLS, "tv-rustjson.py"), os.path.join(self.dep, "rustjson.py"))
        self.configure()

    def configure(self, **cfg):
        write(os.path.join(self.tmp.name, "sim-config.json"), json.dumps({"native": True, **cfg}))

    def sim(self):
        return json.loads(read(os.path.join(self.tmp.name, "sim.json")))

    def test_keys_that_do_what_the_journal_says_pass_and_leave_home_in_front(self):
        rc, out = self.gate("keys")
        self.assertIn("GATE keys: PASS gear-shell ok; input-shell ok; netflix-button ok; digit-tap ok; digit-hold ok; exit ok;", out)
        self.assertIn("key table: gear=139 input=241 netflix=1037; unnamed keys seen: []", out)
        sim = self.sim()
        self.assertEqual(sim["windows"], [["com.webos.app.home", "Card"]], "Home alone is left, every app a key opened is closed")
        self.assertEqual(sim["closed"], ["netflix", "com.webos.app.magicnum", "com.webos.app.browser", "com.palm.app.settings"])
        self.assertEqual(sim["keys"], [139, 412, 241, 412, 1037, 6, 174])
        self.assertIn("gear tap", read(os.path.join(self.out, "keys-gear.txt")))
        self.assertIn("Quick Access slot 5", read(os.path.join(self.out, "keys-digit.txt")))
        self.assertEqual(read(os.path.join(self.out, "keys-exit.txt")).strip(), "input: Back or Exit: closing com.palm.app.settings")

    def test_unready_native_ui_does_not_exercise_panels_or_fall_back(self):
        for cfg in ({"rust_no_ui": True}, {"native_engine": False}, {"native_frame_ready": False},
                    {"rust_built": ["quick", "picker"]}):
            with self.subTest(cfg=cfg):
                self.configure(**cfg)
                state = os.path.join(self.tmp.name, "sim.json")
                if os.path.exists(state):
                    os.remove(state)
                _, out = self.gate("keys")
                self.assertIn("GATE keys: FAIL", out)
                self.assertIn("gear-shell (native quick panel is not ready)", out)
                self.assertIn("input-shell (native input picker is not ready)", out)
                sim = self.sim()
                self.assertNotIn(139, sim["keys"])
                self.assertNotIn(241, sim["keys"])
                self.assertNotIn("com.webos.app.homeconnect", sim["launched"])
                self.assertEqual(sim["launched"].count("com.palm.app.settings"), 1)

    def test_ready_panel_that_does_not_open_fails_without_a_fallback(self):
        self.configure(rust_dead=["quick", "picker"])
        _, out = self.gate("keys")
        self.assertIn("GATE keys: FAIL", out)
        self.assertIn("gear-shell (quick panel is", out)
        self.assertIn("input-shell (picker is", out)
        sim = self.sim()
        self.assertNotIn("com.webos.app.homeconnect", sim["launched"])
        self.assertEqual(sim["launched"].count("com.palm.app.settings"), 1)

    def test_a_key_that_did_nothing_fails_the_check_by_name_and_the_others_still_run(self):
        self.configure(netflix_dead=True)
        rc, out = self.gate("keys")
        self.assertIn("GATE keys: FAIL gear-shell ok; input-shell ok; digit-tap ok; digit-hold ok; exit ok; NOT: netflix-button (foreground com.webos.app.home", out)

    def test_the_netflix_button_follows_what_settings_remote_chose(self):
        self.configure(app_keys={"netflix": "com.webos.app.hdmi2"})
        rc, out = self.gate("keys")
        self.assertIn("GATE keys: PASS", out, out)
        self.assertIn("netflix-button(chosen: com.webos.app.hdmi2) ok", out)
        sim = self.sim()
        self.assertIn("com.webos.app.hdmi2", sim["closed"])
        self.assertNotIn("netflix", sim["launched"])

    def test_a_button_set_to_nothing_passes_when_it_opens_nothing_and_fails_when_it_does(self):
        self.configure(app_keys={"netflix": ""})
        rc, out = self.gate("keys")
        self.assertIn("GATE keys: PASS", out, out)
        self.assertIn("netflix-button(chosen: nothing) ok", out)
        self.configure(app_keys={"netflix": ""}, ignore_choice=True)
        os.remove(os.path.join(self.tmp.name, "sim.json"))
        rc, out = self.gate("keys")
        self.assertIn("GATE keys: FAIL", out)

    def test_an_exit_that_journals_a_close_but_leaves_the_window_fails_the_check(self):
        self.configure(close_fails=True)
        rc, out = self.gate("keys")
        self.assertIn("GATE keys: FAIL", out)
        self.assertIn("NOT: exit (foreground com.webos.app.home, windows mapped: com.webos.app.home com.palm.app.settings; or no journal line)", out)

    def test_a_digit_tap_that_asks_for_quick_access_fails_it(self):
        self.configure(digit_tap_asks=True)
        rc, out = self.gate("keys")
        self.assertIn("GATE keys: FAIL", out)
        self.assertIn("NOT: digit-tap (the tap asked for Quick Access)", out)

    def test_keys_the_table_could_not_name_are_listed(self):
        self.configure(unnamed=[1040])
        rc, out = self.gate("keys")
        self.assertIn("unnamed keys seen: [1040]", out)

    def test_the_search_app_launched_for_its_keyboard_brings_it_up_takes_a_typed_key_and_closes_on_back(self):
        rc, out = self.gate("ime")
        self.assertIn("GATE ime: PASS", out, out)
        self.assertIn("before: maliit-server.service active, MaliitServer not running; after the keyboard came up: unit active, MaliitServer not running; "
                      "input methods bound 1; asked for one 1 time(s); OK pressed 0 time(s); keyboard up yes (panel y 720 h 360, capture 4321 bytes); "
                      "context yes; typed 1; closed by Back", out)
        sim = self.sim()
        self.assertEqual((sim["windows"], sim["closed"]), ([["com.webos.app.home", "Card"]], ["com.webos.app.voice"]))
        self.assertEqual(sim["keys"], [28, 412], "no OK was needed to open it: the launch asked for the keyboard mode")
        self.assertEqual(read(os.path.join(self.out, "ime-result.txt")).count("typed 1"), 1)
        self.assertIn('"panel":{"h":360,"w":1920,"x":0,"y":720}', read(os.path.join(self.out, "ime-up.json")))

    def test_the_marker_is_in_the_journal_before_the_app_is_launched(self):
        self.gate("ime")
        sim = self.sim()
        self.assertEqual(sim["journal"][0], "note: ime-open")
        self.assertEqual(sim["journal_at_launch"]["com.webos.app.voice"], 1)

    def test_a_search_app_that_ignores_the_keyboard_mode_gets_ok_on_its_field(self):
        self.configure(vkb=False)
        rc, out = self.gate("ime")
        self.assertIn("GATE ime: PASS", out, out)
        self.assertIn("OK pressed 1 time(s)", out)
        self.assertEqual(self.sim()["keys"], [28, 28, 412])

    def test_a_field_that_needs_the_second_ok_gets_it_at_the_retry(self):
        self.configure(vkb=False, oks_needed=2)
        rc, out = self.gate("ime")
        self.assertIn("GATE ime: PASS", out, out)
        self.assertIn("OK pressed 2 time(s)", out)

    def test_a_first_key_that_types_nothing_is_followed_by_the_arrows(self):
        self.configure(first_key_types=False)
        rc, out = self.gate("ime")
        self.assertIn("GATE ime: PASS", out, out)
        self.assertIn("typed 1", out)
        self.assertEqual(self.sim()["keys"], [28, 106, 28, 412])

    def test_a_back_the_keyboard_ignores_is_followed_by_exit_which_closes_it(self):
        self.configure(back_closes=False)
        rc, out = self.gate("ime")
        self.assertIn("GATE ime: PASS", out, out)
        self.assertIn("closed by Exit", out)

    def test_no_keyboard_fails_naming_the_window_and_the_context(self):
        self.configure(keyboard=False)
        rc, out = self.gate("ime")
        self.assertIn("GATE ime: FAIL", out)
        self.assertIn("the keyboard's window never came on the screen", out)
        self.assertIn("nothing typed on the keyboard reached the field", out)
        self.assertIn("OK pressed 2 time(s)", out)

    def test_commits_from_before_the_keyboard_came_up_are_not_typing(self):
        self.configure(typing_dead=True, commits_before=3)
        rc, out = self.gate("ime")
        self.assertIn("GATE ime: FAIL nothing typed on the keyboard reached the field", out)
        self.assertIn("typed 0", out)

    def test_no_input_method_fails_it_even_with_a_keyboard(self):
        self.configure(methods=0)
        rc, out = self.gate("ime")
        self.assertIn("GATE ime: FAIL no input method was bound", out)

    def test_without_the_search_app_it_says_so_and_stops(self):
        self.configure(unlaunchable=["com.webos.app.voice"])
        rc, out = self.gate("ime")
        self.assertIn("GATE ime: FAIL LG's Search app (com.webos.app.voice) mapped no window within 15 s (foreground com.webos.app.home, "
                      "windows mapped: com.webos.app.home; before: maliit-server.service active, MaliitServer not running)", out)
        self.assertNotIn(28, self.sim()["keys"])
        for name in ("ime-result.txt", "ime-journal.txt", "ime-windows.json"):
            self.assertTrue(os.path.exists(os.path.join(self.out, name)), "an early failure still leaves its evidence: " + name)

    def test_a_running_maliit_is_named_by_its_pid_before_and_after(self):
        write(os.path.join(self.fakebin, "pgrep"), '#!/bin/sh\ncase "$*" in *"-x MaliitServer"*) echo 4071 ;; *) exit 1 ;; esac\n', 0o755)
        rc, out = self.gate("ime")
        self.assertIn("before: maliit-server.service active, MaliitServer pid 4071; after the keyboard came up: unit active, MaliitServer pid 4071;", out)

    def test_the_windows_the_compositor_reports_say_which_apps_are_up_overlays_included(self):
        windows = ('{"groups":[],"ok":true,"video":[],"windows":['
                   '{"appId":"com.webos.app.home","id":2,"keyMask":4286578682,"mapped":true,"position":[0,0],"props":{"_WEBOS_WINDOW_TYPE":"_WEBOS_WINDOW_TYPE_CARD",'
                   '"appId":"com.webos.app.home","title":"Home"},"size":[1920,1080],"state":"Fullscreen","surface":103,"title":"Home","type":"Card"},'
                   '{"appId":"com.palm.app.settings","id":3,"keyMask":0,"mapped":true,"position":[0,0],"props":{"_WEBOS_WINDOW_TYPE":"_WEBOS_WINDOW_TYPE_OVERLAY",'
                   '"appId":"com.palm.app.settings"},"size":[1920,1080],"state":"Default","surface":185,"title":"Settings","type":"Overlay"},'
                   '{"appId":"com.webos.app.voice","id":9,"keyMask":0,"mapped":false,"position":[0,0],"props":{},"size":[1920,1080],"state":"Default",'
                   '"surface":206,"title":"x","type":"Overlay"}]}')
        script = ". %s\nctl() { printf '%%s' \"$WINDOWS\"; }\nmapped_apps; app_mapped com.palm.app.settings && echo settings-up; app_mapped com.webos.app.voice || echo voice-not-up" % GATE
        r = subprocess.run(["sh", "-c", script], env=dict(self.env, GATE_LIB="1", WINDOWS=windows), capture_output=True, text=True, timeout=60)
        self.assertEqual(r.stdout.split(), ["com.webos.app.home", "com.palm.app.settings", "settings-up", "voice-not-up"], r.stdout + r.stderr)

    def test_the_verdict_names_every_reason_at_once(self):
        r = subprocess.run(["sh", "-c", ". %s\nime_rule 0 no no 0 no no" % GATE], env=dict(self.env, GATE_LIB="1"), capture_output=True, text=True)
        self.assertEqual(r.returncode, 1)
        for part in ("no input method was bound", "window never came on the screen", "no context", "nothing typed", "did not close", "not at the bottom"):
            self.assertIn(part, r.stdout)
        r = subprocess.run(["sh", "-c", ". %s\nime_rule 1 yes yes 3 yes yes" % GATE], env=dict(self.env, GATE_LIB="1"), capture_output=True, text=True)
        self.assertEqual((r.returncode, r.stdout[:4]), (0, "PASS"))

    def apps(self, repeat=None, **cfg):
        messages = os.path.join(self.tmp.name, "messages")
        write(messages, "old lines\n")
        # One report from before the check, which it does not count.
        write(self.crash_log, "*** SIGNAL 11 (SIGSEGV) addr=0x0 pc=0xf7c24acc lr=0xf7c24f21\n")
        self.configure(**cfg)
        env = {"LUMATV_MESSAGES": messages, "LUMATV_PLX_CRASH_LOG": self.crash_log, "LUMATV_CRASH_SETTLE": "0.6"}
        if repeat:
            env["LUMATV_APPS_REPEAT"] = str(repeat)
        return self.gate("apps", env=env)

    @property
    def crash_log(self):
        return os.path.join(self.tmp.name, "plxnative-crash.log")

    def test_youtube_plxnative_and_home_over_geforce_nows_browser_pass_the_apps_check(self):
        rc, out = self.apps()
        self.assertIn("GATE apps: PASS ok: youtube.leanback.v4 pid %d foreground-in-ms 0 alive-at-10s yes connection-lost 0 closed-in-15s yes; "
                      "com.beb.plxnative pid " % (1000 + sum(map(ord, "youtube.leanback.v4")) % 1000), out)
        self.assertIn("closed-in-15s yes; com.geforcenow.play browser-in-front yes front-6s-after-home com.webos.app.home;", out)
        sim = self.sim()
        self.assertEqual([a for a in sim["launched"] if a != "com.webos.app.home"],
                         ["youtube.leanback.v4", "com.beb.plxnative", "com.geforcenow.play", "com.webos.app.browser"])
        self.assertEqual(sim["closed"], ["youtube.leanback.v4", "com.beb.plxnative", "com.webos.app.browser", "com.geforcenow.play"])
        self.assertEqual(sim["windows"], [["com.webos.app.home", "Card"]], "Home alone is left")
        self.assertTrue(os.path.exists(os.path.join(self.out, "apps-com.beb.plxnative.txt")))
        self.assertTrue(os.path.exists(os.path.join(self.out, "apps-geforcenow.txt")))

    def test_the_apps_check_repeats_the_pair_and_keeps_each_passes_evidence(self):
        rc, out = self.apps(repeat=3)
        self.assertIn("GATE apps: PASS", out)
        sim = self.sim()
        self.assertEqual([a for a in sim["launched"] if a == "com.beb.plxnative"], ["com.beb.plxnative"] * 3)
        self.assertEqual([a for a in sim["launched"] if a == "com.geforcenow.play"], ["com.geforcenow.play"], "the browser case runs once")
        for n in (1, 2, 3):
            self.assertTrue(os.path.exists(os.path.join(self.out, "apps-com.beb.plxnative-r%d.txt" % n)))
        self.assertIn("[pass 3 of 3] com.beb.plxnative pid ", out)

    def test_a_crashing_close_in_a_repeated_apps_check_is_named_with_its_pass(self):
        rc, out = self.apps(repeat=2, crashes_at_close=["com.beb.plxnative"])
        self.assertIn("GATE apps: FAIL failed: [pass 1 of 2] com.beb.plxnative pid ", out)
        self.assertIn("[pass 2 of 2] com.beb.plxnative pid ", out)

    def test_sams_list_is_read_as_luna_send_prints_it_with_spaces_and_lines(self):
        # the app is on SAM's list while it runs and off it after the close: an app_running that never matches passed every app as "not alive"
        script = ". %s\nls1() { cat \"$LIST\"; }\napp_running netflix && echo up; app_running youtube.leanback.v4 || echo not-listed" % GATE
        listing = os.path.join(self.tmp.name, "running.json")
        write(listing, '{\n    "returnValue": true,\n    "running": [\n        {\n            "appType": "native",\n            "id": "netflix",\n'
                       '            "processid": "812"\n        }\n    ]\n}\n')
        r = subprocess.run(["sh", "-c", script], env=dict(self.env, GATE_LIB="1", LIST=listing), capture_output=True, text=True, timeout=60)
        self.assertEqual(r.stdout.split(), ["up", "not-listed"], r.stdout + r.stderr)

    def test_an_exit_status_survives_the_log_rotating_under_a_long_check(self):
        # /var/log/messages rotates about every minute: the check copies the lines out from before the launch instead of reading the file at its end
        messages = os.path.join(self.tmp.name, "messages")
        write(messages, 'old APP_CLOSED {"app_id":"com.beb.plxnative","type":"native","pid":"100","exit_status":11}\n')
        saved = os.path.join(self.tmp.name, "exit")
        real_sleep = shutil.which("sleep")
        script = (". %s\nexit_watch_start com.beb.plxnative %s\n"
                  "echo 'x APP_CLOSED {\"app_id\":\"com.beb.plxnative\",\"type\":\"native\",\"pid\":\"200\",\"exit_status\":11} y' >> %s\n"
                  "echo 'x APP_CLOSED {\"app_id\":\"youtube.leanback.v4\",\"type\":\"native\",\"pid\":\"300\",\"exit_status\":9} y' >> %s\n"
                  "%s 0.6\n: > %s\nrm -f %s.1\n%s 0.3\nexit_watch_stop com.beb.plxnative %s\n"
                  "echo \"of 200: $(exit_status_of %s 200)\"\necho \"of 100: $(exit_status_of %s 100)\"\necho \"of any: $(exit_status_of %s)\"\n"
                  % (GATE, saved, messages, messages, real_sleep, messages, messages, real_sleep, saved, saved, saved, saved))
        r = subprocess.run(["sh", "-c", script], env=dict(self.env, GATE_LIB="1", LUMATV_MESSAGES=messages), capture_output=True, text=True, timeout=60)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(read(messages), "", "the log was emptied under the watcher")
        self.assertIn('of 200: APP_CLOSED {"app_id":"com.beb.plxnative","type":"native","pid":"200","exit_status":11', r.stdout)
        self.assertIn("of 100: \n", r.stdout, "a line that was there before the launch is not this run's")
        self.assertNotIn("youtube", r.stdout, "another app's exit is not copied")

    def test_an_app_that_dies_after_its_map_fails_the_apps_check_by_name_with_sams_exit_status(self):
        rc, out = self.apps(dies=["youtube.leanback.v4"])
        self.assertIn("GATE apps: FAIL failed: youtube.leanback.v4 pid ", out)
        self.assertIn('alive-at-10s no connection-lost 1 closed-in-15s yes APP_CLOSED {"app_id":"youtube.leanback.v4","type":"native","pid":"', out)
        self.assertIn('"exit_status":11; ok: com.beb.plxnative pid ', out)
        self.assertEqual(self.sim()["windows"], [["com.webos.app.home", "Card"]])

    def test_an_app_that_crashes_at_its_close_fails_the_apps_check(self):
        # PlxNative's SIGSEGV a second after SAM's close, with its window and connection kept until then (2026-10-28)
        rc, out = self.apps(crashes_at_close=["com.beb.plxnative"])
        self.assertIn("GATE apps: FAIL failed: com.beb.plxnative pid ", out)
        self.assertIn('alive-at-10s yes connection-lost 0 closed-in-15s yes APP_CLOSED {"app_id":"com.beb.plxnative","type":"native","pid":"', out)
        self.assertIn('"exit_status":11 crash-reports +1; ok: youtube.leanback.v4 pid ', out)

    def test_a_crash_report_after_the_close_fails_the_apps_check_when_sams_line_reads_a_clean_close(self):
        # Round 17: SAM's line for 4 of 9 of PlxNative's crashes at its close was missing or read exit 0; its own log had the report.
        rc, out = self.apps(crash_report_at_close=["com.beb.plxnative"])
        self.assertIn("GATE apps: FAIL failed: com.beb.plxnative pid ", out)
        self.assertIn("alive-at-10s yes connection-lost 0 closed-in-15s yes crash-reports +1; ok: youtube.leanback.v4 pid ", out)

    def test_a_browser_that_takes_the_front_back_from_home_fails_the_apps_check(self):
        rc, out = self.apps(stray_browser=True)
        self.assertIn("GATE apps: FAIL failed: com.geforcenow.play browser-in-front yes front-6s-after-home com.webos.app.browser; ok: youtube.leanback.v4 pid ", out)
        self.assertEqual(self.sim()["windows"], [["com.webos.app.home", "Card"]], "the browser is closed after the check")

    def test_without_geforce_now_the_apps_check_says_so_and_passes(self):
        rc, out = self.apps(installed=[])
        self.assertIn("closed-in-15s yes; com.geforcenow.play not installed;", out)
        self.assertIn("GATE apps: PASS", out)


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


class GateRustKinds(FakeSet):
    """Exercise native readiness, drawing and panel transitions without changing configured kinds."""

    def setUp(self):
        super().setUp()
        write(os.path.join(self.dep, "lib", "ld-linux-aarch64.so.1"), SIM_LOADER, 0o755)
        write(os.path.join(self.dep, "bin", "lumatv-compositor"), SIM_COMPOSITOR, 0o755)
        write(os.path.join(self.fakebin, "luna-send"), SIM_LUNA, 0o755)
        shutil.copy(os.path.join(TOOLS, "tv-rustjson.py"), os.path.join(self.dep, "rustjson.py"))

    def rust(self, kinds, **cfg):
        sim = os.path.join(self.tmp.name, "sim.json")
        if os.path.exists(sim):
            os.remove(sim)
        write(os.path.join(self.tmp.name, "sim-config.json"), json.dumps(dict(cfg, native=True)))
        rc, out = self.gate("rust", env={"LUMATV_RUST_KINDS": kinds, "LUMATV_RUST_WAIT": "1", "LUMATV_RUST_OPEN_WAIT": "1"})
        return out

    def found(self):
        return json.loads(read(os.path.join(self.tmp.name, "sim.json")))["rust"]["kinds"]

    def test_declared_kinds_are_shown_and_closed_without_changing_configuration(self):
        kinds = ["home", "osd", "toast", "quick", "settings"]
        out = self.rust(" ".join(kinds))
        self.assertIn("GATE rust: PASS", out)
        self.assertEqual(len(self.found()), 22)
        self.assertEqual(json.loads(read(os.path.join(self.tmp.name, "sim.json"))).get("kind_changes", []), [])
        for kind in kinds:
            self.assertIn(kind + " ok", out)
            self.assertTrue(os.path.exists(os.path.join(self.out, "rust-" + kind + ".json")))
        state = json.loads(read(os.path.join(self.tmp.name, "sim.json")))["rust"]["ui"]
        self.assertEqual(state["panels"]["quick"], "parked")
        self.assertFalse(state["settings"]["open"])

    def test_missing_kind_fails_without_altering_configuration(self):
        all_but_settings = ["home", "osd", "toast", "alert", "quick", "picker", "drawer", "cameras", "saver", "menu", "confirm", "switcher", "cards", "popup", "capture", "layout", "entry", "keyboard", "ai", "banner", "displaylab"]
        out = self.rust("osd settings", rust_built=all_but_settings)
        self.assertIn("GATE rust: FAIL", out)
        self.assertNotIn("osd ok", out)
        self.assertIn("settings", out)
        self.assertEqual(len(self.found()), 22)

    def test_undriven_kind_is_skipped_when_other_kind_runs(self):
        out = self.rust("menu quick")
        self.assertIn("GATE rust: PASS", out)
        self.assertIn("menu SKIPPED", out)
        self.assertIn("quick ok", out)
        self.assertEqual(len(self.found()), 22)

    def test_additional_native_kinds_are_explicitly_skipped(self):
        out = self.rust("quick entry keyboard ai banner displaylab")
        self.assertIn("GATE rust: PASS", out)
        for kind in ("entry", "keyboard", "ai", "banner", "displaylab"):
            self.assertIn(kind + " SKIPPED", out)
        self.assertEqual(json.loads(read(os.path.join(self.tmp.name, "sim.json"))).get("kind_changes", []), [])

    def test_unready_native_ui_fails_even_if_kind_is_declared(self):
        for cfg in ({"rust_no_ui": True}, {"native_engine": False}, {"native_frame_ready": False}):
            with self.subTest(cfg=cfg):
                out = self.rust("quick", **cfg)
                self.assertIn("GATE rust: FAIL", out)
                self.assertNotIn("GATE rust: PASS", out)
                self.assertEqual(len(self.found()), 22)

    def test_a_kind_that_never_shows_fails_and_other_kinds_still_run(self):
        out = self.rust("quick osd", rust_dead=["quick"])
        self.assertIn("GATE rust: FAIL", out)
        self.assertIn("quick", out)
        self.assertIn("osd ok", out)

    def test_no_frame_or_gl_error_fails(self):
        for cfg in ({"rust_no_frames": True}, {"rust_gl": True}):
            with self.subTest(cfg=cfg):
                self.assertIn("GATE rust: FAIL", self.rust("osd", **cfg))

    def test_a_panel_that_does_not_close_fails(self):
        out = self.rust("quick", native_back_stuck=True)
        self.assertIn("GATE rust: FAIL", out)
        self.assertEqual(json.loads(read(os.path.join(self.tmp.name, "sim.json")))["rust"]["ui"]["panels"]["quick"], "open")

    def test_the_gate_reads_the_set_through_one_small_json_reader(self):
        doc = json.dumps({"rust": {"kinds": ["osd", "toast"], "connected": True, "ui": {"toasts": 2, "x": None, "o": {"b": 1}}}})
        reader = os.path.join(TOOLS, "tv-rustjson.py")

        def get(path, text=doc):
            return subprocess.run(["python3", reader, path], input=text, capture_output=True, text=True).stdout.strip()
        self.assertEqual(get("rust.kinds"), "osd,toast")
        self.assertEqual(get("rust.connected"), "true")
        self.assertEqual(get("rust.ui.toasts"), "2")
        self.assertEqual(get("rust.ui.o"), '{"b":1}')
        self.assertEqual(get("rust.kinds.1"), "toast")
        self.assertEqual((get("rust.ui.x"), get("rust.nothing.here"), get("rust.kinds", "not json")), ("", "", ""))


class GateShell(FakeSet):
    """Exercise the native UI shell gate and its failure cases."""

    def setUp(self):
        super().setUp()
        write(os.path.join(self.dep, "lib", "ld-linux-aarch64.so.1"), SIM_LOADER, 0o755)
        write(os.path.join(self.dep, "bin", "lumatv-compositor"), SIM_COMPOSITOR, 0o755)
        write(os.path.join(self.fakebin, "luna-send"), SIM_LUNA, 0o755)
        shutil.copy(os.path.join(TOOLS, "tv-rustjson.py"), os.path.join(self.dep, "rustjson.py"))

    def shell(self, **cfg):
        """The check's GATE line on a set that starts at Home with nothing open."""
        sim = os.path.join(self.tmp.name, "sim.json")
        if os.path.exists(sim):
            os.remove(sim)
        write(os.path.join(self.tmp.name, "sim-config.json"), json.dumps(dict(cfg, native=True)))
        rc, out = self.gate("shell")
        return [ln for ln in out.splitlines() if ln.startswith("GATE shell:")][0]

    def sim(self):
        return json.loads(read(os.path.join(self.tmp.name, "sim.json")))

    def test_native_ui_passes_and_leaves_home_hub_closed(self):
        line = self.shell()
        self.assertTrue(line.startswith("GATE shell: PASS"), line)
        for row in ("ready ok", "quick-home ok", "picker-home ok", "osd ok", "toast ok", "quick-app ok", "saver ok"):
            self.assertIn(row, line)
        sim = self.sim()
        self.assertEqual(sim["windows"], [["com.webos.app.home", "Card"]])
        self.assertEqual(sim["closed"], ["com.webos.app.homeconnect"])
        report = json.loads(read(os.path.join(self.out, "shell-end.json")))
        self.assertTrue(report["ready"])
        self.assertFalse(report["wants"])

    def test_unready_native_ui_fails(self):
        for cfg in ({"rust_no_ui": True}, {"native_engine": False}, {"native_frame_ready": False}):
            with self.subTest(cfg=cfg):
                self.assertTrue(self.shell(**cfg).startswith("GATE shell: FAIL"))

    def test_missing_key_capture_fails(self):
        line = self.shell(native_no_wants=True)
        self.assertTrue(line.startswith("GATE shell: FAIL"), line)
        self.assertIn("quick-app", line)

    def test_a_panel_that_back_cannot_close_fails(self):
        line = self.shell(native_back_stuck=True)
        self.assertTrue(line.startswith("GATE shell: FAIL"), line)
        self.assertIn("quick-app", line)

    def test_the_reader_returns_paths_and_handles_invalid_input(self):
        reader = os.path.join(TOOLS, "tv-rustjson.py")

        def run(*args, text):
            return subprocess.run(["python3", reader, *args], input=text, capture_output=True, text=True).stdout.strip()
        doc = json.dumps({"b": {"wants": False}, "a": 1, "rust": {"wants": True}, "wants": None})
        self.assertEqual(run(text=doc), '{"a":1,"b":{"wants":false},"rust":{"wants":true},"wants":null}')
        self.assertEqual(run("b.wants", text=doc), "false")
        self.assertEqual(run("rust.missing", text=doc), "")
        self.assertEqual(run("rust.wants", text="not json"), "")
        self.assertEqual(run("rust.wants", text=doc), "true")


class GateCallbacks(FakeSet):
    """tv-gate.sh callbacks: the frame callbacks' pacing, set and read back."""

    def setUp(self):
        super().setUp()
        write(os.path.join(self.dep, "lib", "ld-linux-aarch64.so.1"), SIM_LOADER, 0o755)
        write(os.path.join(self.dep, "bin", "lumatv-compositor"), SIM_COMPOSITOR, 0o755)

    def test_it_says_how_many_went_ahead_of_their_flip_and_how_many_with_their_commit(self):
        rc, out = self.gate("callbacks")
        self.assertEqual(rc, 0)
        self.assertEqual(re.findall(r"\b\d+\b", out), ["15682", "16657", "3"])
        self.assertIn("commit", out)

    def test_client_pacing_is_set_by_the_environment(self):
        rc, out = self.gate("callbacks", env={"LUMATV_CLIENTS_CALLBACKS": "early"})
        self.assertEqual(rc, 0)
        self.assertIn("clients early", out)
        self.assertEqual(json.loads(read(os.path.join(self.tmp.name, "sim.json")))["pacing"]["clients"], "early")
        rc, out = self.gate("callbacks")
        self.assertIn("early", out)



class GateDriver(unittest.TestCase):
    """tv_gate.py: the verdict, the record of a passed build, and the order the checks run in."""

    def setUp(self):
        self.gate = load("tv_gate")
        self.tmp = tempfile.TemporaryDirectory()

    def tearDown(self):
        self.tmp.cleanup()

    @property
    def first(self):
        """The minutes of the first session before the frame gate's are added."""
        return self.gate.FIRST_SESSION_MINUTES

    @property
    def standby_wait(self):
        """The minutes the first session is longer by when the standby check runs: the wake that follows it waits for LG's panel refresh."""
        return self.gate.WAKE_GUARD_MINUTES

    @property
    def full(self):
        """The minutes the frame gate's session is armed for (the whole set of surfaces)."""
        return self.gate.FRAMES_SESSION_MINUTES

    @staticmethod
    def session(minutes):
        return "cutover enable --session --max-minutes %d" % minutes

    def test_the_apps_check_repeats_its_pair_and_the_first_session_is_long_enough_for_them(self):
        calls, code, _ = self.run_main(argv=["--only", "apps", "--apps-repeat", "15"])
        self.assertEqual(code, 0)
        self.assertIn("LUMATV_APPS_REPEAT=15 sh /tmp/lumatv/gate.sh apps", calls)
        # 14 more passes of about 45 s: ten and a half minutes more than the one pass the base session counts
        self.assertEqual([c for c in self.checks(calls) if c.startswith("cutover enable")][0], "cutover enable --session --max-minutes %d" % (self.first + 11))
        calls, _, _ = self.run_main(argv=["--only", "apps"])
        self.assertIn("sh /tmp/lumatv/gate.sh apps", calls)
        with mock.patch.object(sys, "argv", ["tv_gate.py", "--apps-repeat", "0"]), contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as ex:
            self.gate.main()
        self.assertNotEqual(ex.exception.code, 0)

    def test_the_rust_check_runs_only_when_asked_and_takes_its_kinds_and_its_minutes(self):
        calls, _, _ = self.run_main()
        self.assertNotIn("rust", self.checks(calls), "off in a regular gate")
        calls, code, _ = self.run_main(argv=["--only", "rust", "--rust-kinds", "home,quick"])
        self.assertEqual(code, 0)
        self.assertIn("LUMATV_RUST_KINDS=home,quick sh /tmp/lumatv/gate.sh rust", calls)
        self.assertEqual([c for c in self.checks(calls) if c.startswith("cutover enable")][0], "cutover enable --session --max-minutes %d" % (self.first + 2 * self.gate.RUST_KIND_MINUTES))
        self.assertTrue(any(c.args[1] == "/tmp/lumatv/rustjson.py" for c in self.gate.tv.put.call_args_list), "the JSON reader goes to the set with the gate")
        calls, _, _ = self.run_main(argv=["--only", "shell,rust"])
        self.assertIn("sh /tmp/lumatv/gate.sh rust", calls, "--only names it: every kind")
        self.assertEqual([c for c in self.checks(calls) if c.startswith("cutover enable")][0],
                         "cutover enable --session --max-minutes %d" % (self.first + self.gate.RUST_KIND_MINUTES * self.gate.RUST_KINDS_ALL))
        calls, _, _ = self.run_main(argv=["--rust-kinds", "all", "--only", "rust,shell"])
        self.assertIn("sh /tmp/lumatv/gate.sh rust", calls)
        for bad in ("home;reboot", "Home", "home,,quick"):
            with mock.patch.object(sys, "argv", ["tv_gate.py", "--rust-kinds", bad]), contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as ex:
                self.gate.main()
            self.assertNotEqual(ex.exception.code, 0, bad)

    def test_safety_checks_require_a_live_session_and_retry_independently(self):
        calls, code, _ = self.run_main(
            argv=["--only", "homekey,wd-hang,wd-crash"],
            pty_answers={"gate.sh live after-homekey": "GATE live-after-homekey: FAIL not mounted\n"})
        self.assertEqual(code, 1)
        checks = self.checks(calls)
        self.assertIn("homekey", checks)
        self.assertNotIn("wd-hang", checks)
        self.assertIn("wd-crash", checks)
        self.assertLess(checks.index("live after-wd-hang"), checks.index("wd-crash"))

    def test_requested_check_without_verdict_fails_and_escape_is_rejected(self):
        calls, code, _ = self.run_main(argv=["--only", "keys"],
                                       pty_answers={"gate.sh keys": "usage: gate.sh help\n"})
        self.assertEqual(code, 1)
        self.assertIn("keys", self.checks(calls))
        self.gate.tv.put.reset_mock()
        calls, code, _ = self.run_main(argv=["--only", "escape"])
        self.assertEqual(code, 2)
        self.assertEqual(calls, [])
        self.assertEqual(self.gate.tv.put.call_args_list, [])

    def test_a_missing_live_result_fails_and_does_not_run_homekey(self):
        calls, code, _ = self.run_main(argv=["--only", "homekey"],
                                       pty_answers={"gate.sh live": "usage: gate.sh help\n"})
        self.assertEqual(code, 1)
        self.assertNotIn("homekey", self.checks(calls))

    def test_a_line_that_is_no_verdict_does_not_fail_the_gate(self):
        # `GATE standby: requested ({` is only the power-off call going out; counted as a failure it made every gate fail
        lines = ["GATE live: PASS ok", "GATE standby: requested ({", "GATE standby: PASS rounds", "GATE homekey: PASS ok"]
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

    def test_every_entry_of_the_list_the_repo_ships_is_one_the_gate_would_have_written(self):
        # Whatever the list holds (nothing before a gate passed, a build after): an entry the gate would refuse to add
        # must not be in it, since the set trusts every entry it finds.
        text = read(os.path.join(ROOT, "compositor", "firmware", "verified.list"))
        entries = re.findall(r"^build .*?^end$", text, re.S | re.M)
        for entry in entries:
            self.assertIsNone(self.gate.entry_problem(entry + "\n"), entry.splitlines()[0])
        self.assertEqual(text.count("\nbuild ") + text.startswith("build "), len(entries))

    def run_main(self, pty_answers=None, argv=(), passed=True, stub_status="", crash_on=None):
        """main() with every call to the set recorded and answered: returns (calls, exit code, the repo list's text)."""
        g = self.gate
        calls = []
        base = tempfile.mkdtemp(dir=self.tmp.name)
        repo = os.path.join(base, "repo", "compositor", "tools")
        os.makedirs(os.path.join(base, "repo", "docs", "research", "compositor"))
        os.makedirs(repo)
        lst = os.path.join(base, "repo", "compositor", "firmware", "verified.list")
        write(lst, "# list\n")

        def pty(cmd, timeout=600):
            calls.append(cmd)
            if crash_on and crash_on in cmd:
                raise RuntimeError("the connection to the set dropped")
            if "homestub status" in cmd:
                return stub_status
            if (pty_answers or {}).get(cmd.split("/tmp/lumatv/")[-1]) is not None:
                return pty_answers[cmd.split("/tmp/lumatv/")[-1]]
            parts = cmd.split()
            if "gate.sh" in cmd:
                i = [j for j, p in enumerate(parts) if p.endswith("gate.sh")][0]
                check, label = parts[i + 1], parts[i + 2] if len(parts) > i + 2 and not parts[i + 2].isdigit() else ""
                if check == "standby":
                    return "GATE standby: requested ({\n"
                if check == "woke":
                    return "GATE standby: PASS rounds\n"
                name = check + ("-" + label if label else "")
                if check == "x1" and not passed:
                    return "GATE x1: FAIL 2 gaps\n"
                return "GATE %s: PASS fine\n" % name
            return ""
        g.pty = pty
        g.wake = lambda standby_since=None: True
        g.avr_mute = lambda on: None
        g.volume_check = lambda: "GATE volume: PASS fine"
        g.frames_calls = []
        g.frames_scanout = []
        g.frames_check = lambda cycles, load=False, refresh=60, quick=False, scanout=False: g.frames_scanout.append(scanout) or g.frames_calls.append((load, refresh, quick)) or (
            "GATE frames-load: measured fine" if load else ("GATE frames at %d Hz: PASS fine" % refresh if refresh != 60 else "GATE frames: PASS fine"))
        g.VERIFIED_LIST = lst
        g.here = repo
        with mock.patch.object(sys, "argv", ["tv_gate.py", *argv]), mock.patch.object(g.time, "sleep"), mock.patch.object(g.os, "system"), \
                mock.patch("builtins.print"):
            with self.assertRaises((SystemExit, RuntimeError)) as ex:
                g.main()
        return calls, getattr(ex.exception, "code", None), read(lst)

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

    def test_the_frame_gate_at_120_hz_switches_the_mode_on_before_the_surfaces_and_off_after(self):
        calls, code, _ = self.run_main(argv=("--only", "frames", "--refresh", "120"))
        self.assertEqual(code, 0)
        checks = [c for c in self.checks(calls) if not c.startswith("key ")]
        self.assertEqual(checks, [
            self.session(self.first + self.full), "live", "evidence",
            self.session(self.full), "live frames", "refresh 120 10", "wake reset", "refresh read 10", "refresh 60 5",
            "wake frames", "mem", "callbacks", "cutover disable"])
        divider = [c for c in calls if "fbdev_vsync_cnt_on_flip" in c]
        self.assertEqual(len(divider), 1, "the divider is put back to 2 in the clean-up, once: %s" % calls)
        self.assertIn("echo 2 > $f", divider[0])
        # at 60 nothing of the mode runs, and the divider is not touched
        calls, _, _ = self.run_main(argv=("--only", "frames"))
        self.assertNotIn("refresh", " ".join(self.checks(calls)))
        self.assertFalse([c for c in calls if "fbdev_vsync_cnt_on_flip" in c])

    def test_the_frame_gate_with_direct_scan_out_switches_it_on_before_the_surfaces_and_reads_it_after(self):
        calls, code, _ = self.run_main(argv=("--only", "frames", "--refresh", "120", "--scanout", "direct"))
        self.assertEqual(code, 0)
        checks = [c for c in self.checks(calls) if not c.startswith("key ")]
        self.assertEqual(checks, [
            self.session(self.first + self.full), "live", "evidence",
            self.session(self.full), "live frames", "scanout direct", "refresh 120 10", "wake reset", "scanout read", "refresh read 10", "refresh 60 5",
            "wake frames", "mem", "callbacks", "cutover disable"])
        self.assertEqual(self.gate.frames_scanout, [True], "the gate's files and trace tag carry -scanout")
        # without the switch the gate asks the compositor nothing about it
        calls, _, _ = self.run_main(argv=("--only", "frames", "--refresh", "120"))
        self.assertNotIn("scanout", " ".join(self.checks(calls)))
        self.assertEqual(self.gate.frames_scanout, [False])

    def test_the_order_of_a_full_gate_with_a_fresh_session_after_every_check_that_ends_one(self):
        calls, code, _ = self.run_main()
        self.assertEqual(code, 0)
        self.assertEqual([c for c in self.checks(calls) if not c.startswith("key ")], [
            "firmware", "homeheld",
            self.session(self.first + self.standby_wait + self.full), "live", "x1 60", "netflix", "hdmi", "apps", "keys", "ime", "shell", "evidence",
            self.session(self.full), "live frames", "wake reset", "wake frames", "mem", "callbacks", "standby", "woke",
            "homekey", "cutover enable --session --max-minutes 6", "live after-homekey",
            "wd-hang", "cutover enable --session --max-minutes 6", "live after-wd-hang",
            "wd-crash", "cutover enable --session --max-minutes 6", "live after-wd-crash",
            "cutover disable"])

    def test_force_wake_leaves_the_standby_waits_minutes_out_and_reaches_the_wake(self):
        calls, code, _ = self.run_main(argv=["--force-wake", "--skip", "frames,homekey,wd-hang,wd-crash"])
        enables = [c.split("cutover.sh ")[1] for c in calls if "cutover.sh enable" in c]
        self.assertEqual(enables, ["enable --session --max-minutes %d" % self.first])
        self.assertTrue(self.gate.FORCE_WAKE)
        self.gate.FORCE_WAKE = False

    def test_skips_leave_out_their_checks_and_their_sessions(self):
        calls, code, _ = self.run_main(argv=["--skip", "firmware,homeheld,netflix,hdmi,apps,keys,ime,shell,frames,standby,homekey,wd-hang,wd-crash"])
        self.assertEqual([c for c in self.checks(calls) if not c.startswith("key ")],
                         [self.session(self.first), "live", "x1 60", "evidence", "cutover disable"])

    def test_the_keys_and_the_keyboard_are_checks_of_their_own_in_the_first_session(self):
        calls, code, _ = self.run_main(argv=["--skip", "firmware,homeheld,x1,netflix,hdmi,apps,shell,frames,standby,homekey,wd-hang,wd-crash"])
        self.assertEqual([c for c in self.checks(calls) if not c.startswith("key ")],
                         [self.session(self.first), "live", "keys", "ime", "evidence", "cutover disable"])
        sent = [c for c in calls if "gate.sh keys" in c or "gate.sh ime" in c]
        self.assertEqual(len(sent), 2)

    def test_only_runs_the_named_checks_in_one_session_and_ends_on_lgs_compositor(self):
        calls, code, _ = self.run_main(argv=["--only", "keys,ime"])
        self.assertEqual(code, 0)
        self.assertEqual([c for c in self.checks(calls) if not c.startswith("key ")],
                         [self.session(self.first), "live", "keys", "ime", "evidence", "cutover disable"])
        self.assertEqual([c for c in calls if "cutover.sh enable" in c], ["sh /tmp/lumatv/cutover.sh enable --session --max-minutes %d" % self.first])

    def test_only_with_a_check_that_does_not_exist_changes_nothing_on_the_set(self):
        calls, code, _ = self.run_main(argv=["--only", "keys,bogus"])
        self.assertEqual(code, 2)
        self.assertEqual(calls, [])
        self.assertEqual(self.gate.tv.put.call_args_list, [])

    def test_only_and_skip_together_leave_what_both_allow(self):
        calls, code, _ = self.run_main(argv=["--only", "keys,ime", "--skip", "ime"])
        self.assertEqual([c for c in self.checks(calls) if not c.startswith("key ")],
                         [self.session(self.first), "live", "keys", "evidence", "cutover disable"])

    def test_the_keys_ime_and_home_checks_in_one_run_each_session_ends_with_the_check_that_ends_it(self):
        calls, code, _ = self.run_main(argv=["--only", "homeheld,keys,ime,homekey"])
        self.assertEqual(code, 0)
        self.assertEqual([c for c in self.checks(calls) if not c.startswith("key ")],
                         ["homeheld", self.session(self.first), "live", "keys", "ime", "evidence",
                          "homekey", "cutover enable --session --max-minutes 6", "live after-homekey", "cutover disable"])

    def test_the_shell_check_runs_in_the_first_session_and_the_frame_gate_in_one_of_its_own(self):
        calls, code, _ = self.run_main(argv=["--only", "shell,frames"])
        self.assertEqual(code, 0)
        self.assertEqual([c for c in self.checks(calls) if not c.startswith("key ")],
                         [self.session(self.first + self.full), "live", "shell", "evidence",
                          self.session(self.full), "live frames", "wake reset", "wake frames", "mem", "callbacks", "cutover disable"])

    def test_the_frame_gate_rejects_removed_shell_callback_option(self):
        calls, code, _ = self.run_main(argv=["--only", "frames", "--frames-callbacks", "landing"])
        self.assertEqual((calls, code), ([], 2))

    def test_the_frame_gate_can_run_with_the_apps_callbacks_at_the_draw_and_puts_the_default_back(self):
        # an A/B in one session: the apps' pacing is set before the surfaces, read after them, and the default is back before the next check
        calls, code, _ = self.run_main(argv=["--only", "frames", "--frames-clients", "early"])
        self.assertEqual(code, 0)
        paced = [c for c in calls if "gate.sh callbacks" in c]
        self.assertEqual(paced, ["LUMATV_CLIENTS_CALLBACKS=early sh /tmp/lumatv/gate.sh callbacks", "sh /tmp/lumatv/gate.sh callbacks",
                                 "LUMATV_CLIENTS_CALLBACKS=commit sh /tmp/lumatv/gate.sh callbacks"])
        self.assertLess(calls.index(paced[0]), calls.index("sh /tmp/lumatv/gate.sh wake reset"), "set before the surfaces")
        self.assertLess(calls.index("sh /tmp/lumatv/gate.sh wake frames"), calls.index(paced[1]), "read after them")
        calls, code, _ = self.run_main(argv=["--only", "frames", "--frames-clients", "landing"])
        self.assertEqual(code, 0)
        self.assertIn("LUMATV_CLIENTS_CALLBACKS=landing sh /tmp/lumatv/gate.sh callbacks", calls)
        calls, code, _ = self.run_main(argv=["--only", "frames"])
        self.assertNotIn("LUMATV_CLIENTS_CALLBACKS", " ".join(calls), "the compositor's default stands unless it is asked")
        calls, code, _ = self.run_main(argv=["--only", "x1", "--frames-clients", "early"])
        self.assertNotIn("LUMATV_CLIENTS_CALLBACKS", " ".join(calls), "no frame run, nothing to set")
        calls, code, _ = self.run_main(argv=["--only", "frames", "--frames-clients", "sometimes"])
        self.assertEqual((calls, code), ([], 2))

    def test_the_loaded_frame_run_is_opt_in_and_a_report_the_verdict_does_not_read(self):
        calls, code, _ = self.run_main()
        self.assertNotIn("wake loaded", self.checks(calls), "a full gate has no loaded run unless asked")
        calls, code, _ = self.run_main(argv=["--only", "frames", "--frames-load"])
        self.assertEqual(code, 0, "a report line is no PASS and no FAIL")
        self.assertEqual([c for c in self.checks(calls) if not c.startswith("key ")],
                         [self.session(self.first + 2 * self.full), "live", "evidence",
                          self.session(2 * self.full), "live frames", "wake reset", "wake frames", "mem", "callbacks",
                          "wake reset", "wake loaded", "cutover disable"])

    def test_only_frames_load_runs_the_loaded_run_alone_in_a_session_of_its_own(self):
        calls, code, _ = self.run_main(argv=["--only", "frames-load"])
        self.assertEqual(code, 0)
        self.assertEqual([c for c in self.checks(calls) if not c.startswith("key ")],
                         [self.session(self.first + self.full), "live", "evidence",
                          self.session(self.full), "live frames", "wake reset", "wake loaded", "cutover disable"])

    def test_the_frame_gate_runs_the_whole_set_and_a_quick_round_runs_the_walks_that_touch_nothing(self):
        g = self.gate
        quick, full = g.FRAMES_SURFACES_QUICK.split(","), g.FRAMES_SURFACES.split(",")
        self.assertEqual(full[:len(quick)], quick)
        self.assertEqual(full[len(quick):], ["switcher", "transitions-netflix", "transitions-settings", "keyboard"])
        sys.path.insert(0, os.path.join(ROOT, "docs", "research", "perf"))
        try:
            import gate as perf_gate
            table = {e["name"]: perf_gate.surface(e) for e in perf_gate.SURFACES}
        finally:
            sys.path.pop(0)
        for n in full:
            self.assertIn(n, table, n)
        for n in quick:                    # a regular round starts no app, and changes a setting only if the owner has the number keys off or Quick Access on
            self.assertFalse(table[n]["setup"], n)
            self.assertLessEqual(set(table[n]["switches"]), {"number-keys", "quick-access"}, n)
        for n in full[len(quick):]:
            self.assertTrue(table[n]["switches"] or table[n]["setup"], n)
        self.assertLess(g.FRAMES_QUICK_SESSION_MINUTES, g.FRAMES_SESSION_MINUTES)

    def test_a_quick_round_arms_the_shorter_session_and_asks_the_gate_for_the_quick_set(self):
        g = self.gate
        calls, code, _ = self.run_main(argv=["--only", "frames", "--frames-quick"])
        self.assertEqual(code, 0)
        self.assertEqual(g.frames_calls, [(False, 60, True)])
        self.assertEqual([c for c in calls if "cutover.sh enable" in c],
                         ["sh /tmp/lumatv/cutover.sh enable --session --max-minutes %d" % (g.FIRST_SESSION_MINUTES + g.FRAMES_QUICK_SESSION_MINUTES),
                          "sh /tmp/lumatv/cutover.sh enable --session --max-minutes %d" % g.FRAMES_QUICK_SESSION_MINUTES])
        calls, code, _ = self.run_main(argv=["--only", "frames"])
        self.assertEqual(g.frames_calls, [(False, 60, False)], "the whole set by default")
        calls, code, _ = self.run_main(argv=["--only", "frames", "--frames-load", "--frames-quick"])
        self.assertEqual(g.frames_calls, [(False, 60, True), (True, 60, True)], "the loaded run follows the same set")

    def test_the_frame_gate_is_started_with_the_set_it_was_asked_for_and_the_time_that_set_needs(self):
        g = self.gate
        for quick in (False, True):
            with mock.patch.object(g.subprocess, "run", return_value=mock.Mock(stdout="VERDICT PASS\n", stderr="")) as run, \
                    mock.patch.object(g.os, "makedirs"), mock.patch("builtins.print"):
                g.frames_check(3, quick=quick)
            cmd = run.call_args.args[0]
            self.assertEqual(cmd[cmd.index("--surfaces") + 1], g.FRAMES_SURFACES_QUICK if quick else g.FRAMES_SURFACES)
            self.assertEqual(run.call_args.kwargs["timeout"], 60 * (g.FRAMES_QUICK_SESSION_MINUTES if quick else g.FRAMES_SESSION_MINUTES))

    def test_the_x1_run_is_traced_by_default_and_the_strict_rule_is_untraced(self):
        calls, code, _ = self.run_main(argv=["--only", "x1"])
        self.assertEqual(code, 0)
        self.assertIn("LUMATV_GATE_KWAIT=1 sh /tmp/lumatv/gate.sh x1 60", calls, "the compositor's gaps are told from the client's by the trace")
        put = [c.args[1] for c in self.gate.tv.put.call_args_list]
        for tool in ("capture.sh", "kwait.py", "sample.py"):
            self.assertIn("/tmp/perf/" + tool, put, tool)
        self.gate.tv.put.reset_mock()
        calls, code, _ = self.run_main(argv=["--only", "x1", "--x1-strict"])
        self.assertIn("LUMATV_X1_STRICT=1 sh /tmp/lumatv/gate.sh x1 60", calls, "the PLAN's rule as it was: every gap counts, the run is not traced")
        self.assertNotIn("LUMATV_GATE_KWAIT", " ".join(calls))
        self.assertEqual([c.args[1] for c in self.gate.tv.put.call_args_list if c.args[1].startswith("/tmp/perf/")], [])
        calls, code, _ = self.run_main(argv=["--only", "x1", "--x1-strict", "--kwait"])
        self.assertIn("LUMATV_X1_STRICT=1 LUMATV_GATE_KWAIT=1 sh /tmp/lumatv/gate.sh x1 60", calls, "strict and traced: the trace is for the reader")

    def test_x1_rt_runs_the_x1_client_fifo_and_asks_nothing_else_of_the_check(self):
        calls, code, _ = self.run_main(argv=["--only", "x1", "--x1-rt", "10"])
        self.assertEqual(code, 0)
        self.assertIn("LUMATV_X1_RT=10 LUMATV_GATE_KWAIT=1 sh /tmp/lumatv/gate.sh x1 60", calls)
        calls, code, _ = self.run_main(argv=["--only", "x1"])
        self.assertNotIn("LUMATV_X1_RT", " ".join(calls), "the client is time-shared unless it is asked")
        calls, code, _ = self.run_main(argv=["--only", "x1", "--x1-rt", "100"])
        self.assertEqual((calls, code), ([], 2), "a priority past 99 is refused before anything runs on the set")

    def test_x1_callbacks_sets_when_the_clients_frame_callbacks_go_out_for_the_x1_run(self):
        calls, code, _ = self.run_main(argv=["--only", "x1", "--x1-callbacks", "landing"])
        self.assertEqual(code, 0)
        self.assertIn("LUMATV_X1_CALLBACKS=landing LUMATV_GATE_KWAIT=1 sh /tmp/lumatv/gate.sh x1 60", calls)
        calls, code, _ = self.run_main(argv=["--only", "x1", "--x1-rt", "10", "--x1-callbacks", "early"])
        self.assertIn("LUMATV_X1_RT=10 LUMATV_X1_CALLBACKS=early LUMATV_GATE_KWAIT=1 sh /tmp/lumatv/gate.sh x1 60", calls)
        calls, code, _ = self.run_main(argv=["--only", "x1"])
        self.assertNotIn("LUMATV_X1_CALLBACKS", " ".join(calls), "the compositor's default stands unless it is asked")
        calls, code, _ = self.run_main(argv=["--only", "x1", "--x1-callbacks", "sometimes"])
        self.assertEqual((calls, code), ([], 2))

    def test_after_the_frame_run_the_interrupts_and_wakeups_are_read_once_and_not_judged(self):
        calls, code, _ = self.run_main(argv=["--only", "frames"])
        self.assertEqual(code, 0)
        probes = [c for c in calls if "lpc-probe.sh" in c]
        self.assertEqual(probes, ["sh /tmp/lumatv/lpc-probe.sh irq 10 home", "sh /tmp/lumatv/lpc-probe.sh wakeups 10 home"])
        self.assertLess(calls.index("sh /tmp/lumatv/gate.sh mem"), calls.index(probes[0]), "after the frame run and its reports, while the session is live")
        self.assertIn("/tmp/lumatv/lpc-probe.sh", [c.args[1] for c in self.gate.tv.put.call_args_list])
        calls, code, _ = self.run_main(argv=["--only", "x1"])
        self.assertEqual([c for c in calls if "lpc-probe.sh" in c], [], "no frame run, no probe")

    STUB_ON = '{"config":true,"state":"stub","mounted":true,"sam":"web","flutter":false,"guard":true,"beat":true,"faults":0}'

    def stub_calls(self, calls):
        tool = self.gate.LUMATV_TOOL
        status = [i for i, c in enumerate(calls) if c.startswith(tool + " homestub status")]
        on = [i for i, c in enumerate(calls) if c == tool + " homestub on"]
        sessions = [i for i, c in enumerate(calls) if "cutover.sh enable" in c]
        disables = [i for i, c in enumerate(calls) if c.endswith("cutover.sh disable")]
        return status, on, sessions, disables

    def test_a_home_stub_that_was_on_is_put_back_after_the_last_disable_and_its_status_printed(self):
        calls, code, _ = self.run_main(argv=["--only", "homekey"], stub_status=self.STUB_ON)
        status, on, sessions, disables = self.stub_calls(calls)
        self.assertEqual((len(status), len(on), len(disables)), (2, 1, 1), calls)
        self.assertLess(status[0], sessions[0], "the stub's state is read before the first session")
        self.assertLess(disables[0], on[0], "LG's compositor is back before the stub is")
        self.assertLess(on[0], status[1], "its status is read after it is put back")
        self.assertEqual(calls[-2:], [self.gate.LUMATV_TOOL + " homestub on", self.gate.LUMATV_TOOL + " homestub status 2>&1 | head -3"])

    def test_the_stub_is_put_back_when_a_check_dies_too(self):
        calls, code, _ = self.run_main(argv=["--only", "keys"], stub_status=self.STUB_ON, crash_on="gate.sh keys")
        status, on, sessions, disables = self.stub_calls(calls)
        self.assertEqual((len(on), len(disables)), (1, 1), calls)
        self.assertLess(disables[0], on[0])

    def test_a_stub_that_was_off_or_had_already_given_lgs_home_back_is_left_as_found(self):
        for status_text in ('{"config":true,"state":"lg","mounted":false,"faults":1}', '{"config":false,"state":"off"}', ""):
            with self.subTest(status_text):
                calls, code, _ = self.run_main(argv=["--only", "keys"], stub_status=status_text)
                status, on, sessions, disables = self.stub_calls(calls)
                self.assertEqual((len(status), len(on)), (1, 0), calls)

    def test_every_session_the_gate_makes_has_the_sets_own_timer_and_a_longer_x1_run_lengthens_the_first(self):
        calls, code, _ = self.run_main(argv=["--x1-seconds", "180"])
        enables = [c.split("cutover.sh ")[1] for c in calls if "cutover.sh enable" in c]
        self.assertEqual(enables, ["enable --session --max-minutes %d" % (self.first + 2 + self.standby_wait + self.full), "enable --session --max-minutes %d" % self.full] + ["enable --session --max-minutes 6"] * 3)

    def test_the_persistent_enable_after_a_pass_has_no_timer(self):
        answers = {"cutover.sh firmware record": "noise\r\n" + BLOCK.replace("\n", "\r\n")}
        calls, code, _ = self.run_main(pty_answers=answers, argv=["--enable-after"])
        enables = [c.split("cutover.sh ")[1] for c in calls if "cutover.sh enable" in c]
        self.assertEqual(enables[-1], "enable")

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
        idle = "[srv 1.0] info kms: page-flip interval 33.37 ms, nothing due at the refresh between (one refresh is 16.67)"
        self.assertEqual(self.slot.over_limit(idle), [], "a refresh with nothing new to show is no frame missed")

    def run_slot(self, stub, guard="1"):
        """main() of the slot with the set answered: the commands it sent through pty, in order, and the summary it wrote."""
        slot, calls, tmp = self.slot, [], tempfile.mkdtemp()
        slot.repo = tmp
        slot.TOOL = "TOOL"
        slot.tv_gate.wake.return_value = True
        slot.tv.tv.return_value = "none\n0\n/usr/bin/surface-manager\n%s\n%s" % (guard, stub)
        slot.tv_gate.pty = lambda cmd, timeout=600: calls.append(cmd) or ""
        slot.tv_gate.homestub_status = lambda: "STATUS-AFTER"
        slot.sh = lambda *args, timeout=900: (0, "firmware: verified\nprobe: 12 allowed, 0 refused\ntrial evidence: ok\n")
        slot.fetch = lambda *args: None
        with mock.patch.object(sys, "argv", ["tv_next_slot.py", "--skip-gate", "--no-standby"]), mock.patch("builtins.print"):
            slot.main()
        root = os.path.join(tmp, "docs", "research", "compositor")
        summary = read(os.path.join(root, os.listdir(root)[0], "summary.txt"))
        shutil.rmtree(tmp)
        return calls, summary

    def test_a_home_stub_that_was_on_is_put_back_after_the_slot_whatever_the_gate_did_to_it(self):
        calls, summary = self.run_slot(GateDriver.STUB_ON)
        self.assertEqual([c for c in calls if "homestub" in c], ["TOOL homestub on"])
        self.assertLess(calls.index("sh /tmp/lumatv/cutover.sh status"), calls.index("TOOL homestub on"))
        self.assertIn("home stub back on: STATUS-AFTER", summary)

    def test_a_stub_the_slot_turned_off_for_the_session_is_turned_on_again_at_the_end(self):
        calls, summary = self.run_slot(GateDriver.STUB_ON, guard="0")
        self.assertEqual([c for c in calls if "homestub" in c], ["TOOL homestub off", "TOOL homestub on"])

    def test_a_stub_that_was_not_on_is_not_touched(self):
        calls, summary = self.run_slot('{"config":true,"state":"lg","mounted":false,"faults":1}')
        self.assertEqual([c for c in calls if "homestub" in c], [])
        self.assertNotIn("home stub back on", summary)


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

    def test_the_default_key_is_lgs_rf_home_773_and_another_can_be_named(self):
        seen = []
        with mock.patch.object(self.k, "hold", lambda seconds, settle, key, device: seen.append((seconds, settle, key, device))):
            self.assertEqual(self.k.main(["x", "hold", "2"]), 0)
            self.assertEqual(self.k.main(["x", "hold", "2", "--key", "172", "--settle", "0.5"]), 0)
        self.assertEqual(seen, [(2.0, 1.0, 773, "/dev/uinput"), (2.0, 0.5, 172, "/dev/uinput")])



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



if __name__ == "__main__":
    unittest.main()
