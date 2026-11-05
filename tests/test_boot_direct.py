"""Boot direct's shell side (tools/lumatv): the Home stub's placement around the compositor restart, the strikes that keep a bad boot from
repeating, `boot-direct on|off|status` and the escape's part (Home held 10 s, the supervisor's own: what it leaves behind and what the next boot
does with it). The real functions run under busybox ash, as on the set; the platform (SAM, the mounts, WebAppMgr) is a stand-in that models what each
call does to it."""
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
CLI = ROOT / "tools" / "lumatv"
BUSYBOX = "/usr/bin/busybox"
SOURCE = CLI.read_text()


def function(name):
    """One function of the tool, as written: a one-liner, or up to the closing brace at the start of a line."""
    lines = SOURCE.split("\n")
    for i, line in enumerate(lines):
        if line.startswith(name + "() {"):
            if line.rstrip().endswith("}"):
                return line
            for j in range(i + 1, len(lines)):
                if lines[j] == "}":
                    return "\n".join(lines[i:j + 1])
    raise AssertionError("missing shell function " + name)


def constants(*names):
    found = []
    for name in names:
        match = re.search(r"^" + re.escape(name) + r"=.*$", SOURCE, re.M)
        if match is None:
            raise AssertionError("missing shell variable " + name)
        found.append(match.group())
    return "\n".join(found)


FAKE_LUNA = '''#!/bin/sh
# SAM, as far as Home goes: what is running, what the manifest says (the type it read when it was last asked to), what is in front.
uri=""
for a in "$@"; do case "$a" in luna://*) uri=$a ;; esac; done
case "$uri" in
  *applicationManager/running)
    if [ -e "$FAKE/running" ]; then echo '{"running":[{"id":"com.webos.app.home","processid":"7"}],"returnValue":true}'
    else echo '{"running":[],"returnValue":true}'; fi ;;
  *applicationManager/getAppInfo)
    # SAM_DOWN_FOR=N: SAM does not answer for the first N asks (early in a boot)
    n=$(cat "$FAKE/asked" 2>/dev/null || echo 0); n=$((n+1)); echo $n > "$FAKE/asked"
    if [ "$n" -le "${SAM_DOWN_FOR:-0}" ]; then exit 0; fi
    echo "{\\"appInfo\\":{\\"id\\":\\"com.webos.app.home\\",\\"type\\":\\"$(cat "$FAKE/sam_type" 2>/dev/null || echo flutter)\\"},\\"returnValue\\":true}" ;;
  *applicationManager/getForegroundAppInfo)
    if [ -e "$FAKE/front" ]; then echo '{"appId":"com.webos.app.home","returnValue":true}'
    else echo '{"appId":"netflix","returnValue":true}'; fi ;;
  *applicationManager/blockSystemApp) echo block >> "$FAKE/calls"; echo '{"returnValue":true}' ;;
  *applicationManager/releaseBlockSystemApp)
    echo release >> "$FAKE/calls"
    # SAM reads the app directory again: the type is the kind of stub that is mounted there now (its file says which, web when it says nothing)
    if [ -e "$FAKE/mounted" ]; then kind=$(cat "$FAKE/mounted"); echo "${kind:-web}" > "$FAKE/sam_type"; else echo flutter > "$FAKE/sam_type"; fi
    echo '{"returnValue":true}' ;;
  *applicationManager/launch)
    echo launch >> "$FAKE/calls"
    n=$(cat "$FAKE/launches" 2>/dev/null || echo 0); n=$((n+1)); echo $n > "$FAKE/launches"
    # LAUNCH_STARTS=N: the Nth launch is the one that starts Home; 0: none does. A native Home is a process of its own.
    if [ "${LAUNCH_STARTS:-1}" -gt 0 ] && [ "$n" -ge "${LAUNCH_STARTS:-1}" ]; then
      touch "$FAKE/running"
      [ "$(cat "$FAKE/sam_type" 2>/dev/null)" = native ] && touch "$FAKE/native-proc"
    fi
    echo '{"returnValue":true}' ;;
  *webappmanager/killApp) echo killApp >> "$FAKE/calls"; rm -f "$FAKE/running"; echo '{"returnValue":true}' ;;
  *webappmanager/listRunningApps) echo '{"returnValue":true}' ;;
esac
exit 0
'''

FAKE_MOUNT = '''#!/bin/sh
# `mount --bind SRC DST` on Home's directory; with no arguments the table, of the assets bind only. The kind of stub is what the mount holds.
if [ "$1" = --bind ]; then
  echo "mount $2 $3" >> "$FAKE/calls"
  case "$3" in "$HOME_APP_DIR") case "$2" in "$STUB_NATIVE_STAGE") echo native > "$FAKE/mounted" ;; *) echo web > "$FAKE/mounted" ;; esac ;; esac
  exit 0
fi
exit 0
'''

FAKE_UMOUNT = '''#!/bin/sh
echo "umount $*" >> "$FAKE/calls"
rm -f "$FAKE/mounted"
exit 0
'''

FAKE_STAT = '''#!/bin/sh
# The device and inode of a staged stub, which the app directory shows while that stub is mounted over it: one for each kind.
case "$3" in
  "$HOME_APP_DIR") if [ -e "$FAKE/mounted" ]; then [ "$(cat "$FAKE/mounted")" = native ] && echo "9:78" || echo "9:77"; else echo "9:11"; fi ;;
  "$STUB_STAGE") echo "9:77" ;;
  "$STUB_NATIVE_STAGE") echo "9:78" ;;
  *) exec /usr/bin/stat "$@" ;;
esac
'''

FAKE_PKILL = '''#!/bin/sh
echo "pkill $*" >> "$FAKE/calls"
case "$*" in
  *lumatv-homestub*) [ "${STUB_WONT_DIE:-0}" = 1 ] || rm -f "$FAKE/native-proc" "$FAKE/running" ;;
  *) rm -f "$FAKE/flutter" "$FAKE/running" ;;
esac
exit 0
'''

FAKE_PGREP = '''#!/bin/sh
case "$*" in *lumatv-homestub*) [ -e "$FAKE/native-proc" ]; exit $? ;; esac
[ -e "$FAKE/flutter" ] && echo 812
[ -e "$FAKE/flutter" ]
'''


@unittest.skipUnless(os.access(BUSYBOX, os.X_OK), "busybox is the shell of the set")
class BootDirectShell(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="boot-direct-"))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.state = self.tmp / "state"
        self.comp = self.state / "compositor"
        self.bin = self.tmp / "bin"
        self.app = self.tmp / "app"
        for d in (self.comp, self.bin, self.app / "payload" / "homestub", self.app / "payload" / "homestub-native", self.tmp / "home",
                  self.tmp / "assets" / "i18n"):
            d.mkdir(parents=True)
        for name in ("appinfo.json", "index.html", "icon.png"):
            (self.app / "payload" / "homestub" / name).write_text(name)
        (self.app / "payload" / "homestub-native" / "appinfo.json").write_text("native appinfo.json")
        self.native_bin = self.app / "payload" / "homestub-native" / "lumatv-homestub"
        self.native_bin.write_text("native program")
        self.native_bin.chmod(0o755)
        self.calls = self.tmp / "calls"
        self.log = self.tmp / "log"
        for name, body in (("luna-send", FAKE_LUNA), ("mount", FAKE_MOUNT), ("umount", FAKE_UMOUNT), ("stat", FAKE_STAT),
                           ("pkill", FAKE_PKILL), ("pgrep", FAKE_PGREP)):
            (self.bin / name).write_text(body)
            (self.bin / name).chmod(0o755)
        (self.tmp / "mounts").write_text("")
        self.api()

    # -- the fixture's own files

    def api(self, present=True):
        path = self.comp / "boot-direct-api"
        path.write_text("2\n") if present else path.unlink(missing_ok=True)

    def flag(self, text="on\n"):
        (self.state / "boot-direct").write_text(text)

    def has(self, name):
        return (self.state / name).exists()

    def put(self, name, text=""):
        (self.tmp / name).write_text(text)

    def sam(self, type_="flutter", running=False, front=False, flutter=False, mounted=False, native_proc=False):
        """`mounted` is False, True (the web stub) or the kind that is over LG's Home directory."""
        self.put("sam_type", type_ + "\n")
        for name, on in (("running", running), ("front", front), ("flutter", flutter), ("mounted", mounted), ("native-proc", native_proc)):
            (self.tmp / name).unlink(missing_ok=True)
            if on:
                self.put(name, mounted if name == "mounted" and isinstance(mounted, str) else "")
        if mounted:
            # a stub that is mounted was staged: the stage directory is how the tool tells which one it is
            (self.state / ("homestub-native" if mounted == "native" else "homestub")).mkdir(exist_ok=True)

    def called(self):
        return self.calls.read_text().split("\n")[:-1] if self.calls.exists() else []

    def logged(self):
        return self.log.read_text() if self.log.exists() else ""

    # -- running the real functions

    def script(self, body, real_stub=False):
        names = ["bd_enabled", "bd_api", "bd_strikes", "bd_strike", "bd_finish", "bd_fail", "bd_abort", "bd_rec", "bd_now", "bd_worker_alive",
                 "bd_early_in_place", "bd_early_stub", "bd_early_undo", "bd_early_gone", "bd_early_boot", "bd_manual_stub", "bd_home_up", "bd_json", "verb_boot_direct",
                 "escape_lapse", "home_running", "home_in_front", "sam_home_type", "json_escape", "emit_error", "hs_stub_in_front", "hs_open_home",
                 "hs_stage_dir", "hs_kind_setting", "hs_kind_wanted", "hs_kind_set", "hs_kind_json", "verb_stub_kind"]
        if real_stub:
            names += ["homestub_up", "homestub_drop", "hs_mounted_any", "hs_mounted_ours", "hs_kind_mounted", "hs_stage", "hs_stage_web", "hs_stage_native",
                      "hs_mode_set", "hs_sam_reread", "hs_wait_type", "hs_unmount", "hs_wd_alive", "hs_wd_stop", "hs_native_stop", "hs_stop_instance",
                      "kill_home_app", "is_mounted"]
        fakes = '''
sleep() { :; }
uptime_s() { echo 42; }
log_line() { echo "$1" >> "$FAKE/log"; }
take_mutation_lock() { echo "$LOCK_WAIT" >> "$FAKE/lockwaits"; [ "${LOCK:-ok}" = ok ]; }
comp_live() { [ "${LIVE:-yes}" = yes ]; }
comp_enabled() { [ "${ENABLED:-yes}" = yes ]; }
panel_is_active() { [ "${PANEL:-active}" = active ]; }
wait_wam_restart() { echo wait_wam >> "$FAKE/calls"; return 1; }
bd_watch_start() { echo watch >> "$FAKE/calls"; }
'''
        if not real_stub:
            fakes += '''
hs_mounted_ours() { [ -e "$FAKE/mounted" ]; }
hs_kind_mounted() { [ -e "$FAKE/mounted" ] || return 1; hk=$(cat "$FAKE/mounted"); printf '%s' "${hk:-web}"; }
homestub_up() {
  echo homestub_up >> "$FAKE/calls"
  [ "${HS_UP:-ok}" = ok ] || return 1
  # the real one stops LG's own instance (kill_home_app): what runs after it is what the launch starts. The kind is the setting's.
  hk=$(hs_kind_wanted); echo "$hk" > "$FAKE/mounted"; echo "$hk" > "$FAKE/sam_type"; rm -f "$FAKE/running" "$FAKE/flutter" "$FAKE/native-proc"
}
homestub_drop() { echo "homestub_drop $1" >> "$FAKE/calls"; rm -f "$FAKE/mounted"; echo flutter > "$FAKE/sam_type"; }
'''
            names = [n for n in names if n != "hs_open_home"]
            fakes += 'hs_open_home() { echo hs_open_home >> "$FAKE/calls"; }\n'
        head = '''set -u
STATE="$FAKE/state"; COMP="$STATE/compositor"; APPDIR="$FAKE/app"; SHORT_ID=lumatv; LOCK_WAIT=1; VERB=test
HOME_APP_DIR="$FAKE/home"; STUB_SRC="$APPDIR/payload/homestub"; STUB_STAGE="$STATE/homestub"; STOCK_ASSETS="$FAKE/assets/stock"
ASSETS="$FAKE/assets"; HS_TRIAL="$FAKE/hs-trial"; HS_ESCAPE="$FAKE/hs-escape"; HS_CONFIRM="$FAKE/hs-confirm"
HS_WD_PID="$FAKE/hs-guard"; HS_MODE="$FAKE/hs-mode"; HS_STAGED=0
STUB_NATIVE_SRC="$APPDIR/payload/homestub-native"; STUB_NATIVE_BIN="$STUB_NATIVE_SRC/lumatv-homestub"; STUB_NATIVE_STAGE="$STATE/homestub-native"
STUB_NATIVE_COMM=lumatv-homestub; HS_KIND="$STATE/homestub-kind"
export HOME_APP_DIR STUB_STAGE STUB_NATIVE_STAGE
'''
        shared = constants("HOME_PKILL_PATTERN", "HOME_LAUNCH_URI", "HOME_LAUNCH_PARAMS", "HOME_APP_ID", "BD_FLAG", "BD_PENDING", "BD_STRIKES",
                           "BD_MAX_STRIKES", "BD_HOME_TRIES", "BD_WATCH_SECS", "BD_WATCH_MAX") + '\nBD_EARLY="$FAKE/early"\nBD_WORKER_MARK="$FAKE/worker"\nESCAPE_BOOT="$FAKE/escape-boot"'
        functions = "\n".join(function(n) for n in names)
        if real_stub:
            functions = functions.replace('mount 2>/dev/null | grep -qF -- " on $1 type "', 'grep -qF -- " on $1 type " "$FAKE/mounts"')
            functions = functions.replace('grep -qF " $HOME_APP_DIR " /proc/self/mountinfo', 'false')
        return head + shared + "\n" + fakes + functions + "\n" + body + "\n"

    def sh(self, body, real_stub=False, **env):
        env = dict(os.environ, FAKE=str(self.tmp), PATH=str(self.bin) + os.pathsep + os.environ["PATH"], **env)
        # the stand-ins read the fixture's own files as "$FAKE/NAME"; the state directory is one of them
        result = subprocess.run([BUSYBOX, "sh", "-c", self.script(body, real_stub)], env=env, capture_output=True, text=True, timeout=30)
        return result

    def verb(self, call, real_stub=False, ok=True, **env):
        result = self.sh(call, real_stub, **env)
        self.assertEqual(result.stderr, "", result.stdout)
        line = result.stdout.strip().split("\n")[-1] if result.stdout.strip() else ""
        data = json.loads(line) if line.startswith("{") else None
        if ok is not None:
            self.assertEqual(result.returncode == 0, ok, result.stdout)
        return data

    # ------------------------------------------------------------------------------------------------------ the flag

    def test_the_flag_is_on_only_for_on_on_the_first_line(self):
        # the compositor reads the same line trimmed (`boot_direct::Flag::parse`), and so does `read`
        for text, expected in (("on\ntool /x\n", True), ("on\n", True), ("on", True), (" on \n", True), ("off\n", False), ("", False),
                               ("\non\n", False), ("tool /x\non\n", False), ("onn\n", False), ("no\n", False)):
            with self.subTest(text=text):
                self.flag(text)
                self.assertEqual(self.sh("bd_enabled").returncode == 0, expected)
        (self.state / "boot-direct").unlink()
        self.assertNotEqual(self.sh("bd_enabled").returncode, 0, "no file")

    def test_the_bundle_marker_is_what_says_the_compositor_reads_the_flag(self):
        self.assertEqual(self.sh("bd_api").returncode, 0)
        self.api(False)
        self.assertNotEqual(self.sh("bd_api").returncode, 0)

    # ------------------------------------------------------------------------------------------- the stub at a boot

    def early(self, **env):
        """The boot worker's own run of the early step, under LG's compositor."""
        return self.sh("bd_early_stub", LIVE="no", LUMA_BOOT="1", **env)

    def record(self):
        path = self.tmp / "early"
        return dict(line.split("=", 1) for line in path.read_text().splitlines()) if path.exists() else {}

    def test_a_bundle_that_predates_boot_direct_gets_no_stub(self):
        self.flag()
        self.api(False)
        self.assertEqual(self.early().returncode, 1)
        self.assertEqual(self.called(), [])
        self.assertFalse(self.has("boot-direct.pending"))
        self.assertIn("predates", self.logged())

    def test_the_stub_goes_in_first_of_all_and_the_boot_is_marked_unfinished(self):
        self.flag()
        self.assertEqual(self.early().returncode, 0)
        # the stub, with the watch behind it; nothing else is asked of SAM first
        self.assertEqual(self.called(), ["homestub_up", "watch"])
        self.assertTrue(self.has("boot-direct.pending"), "a power cut now is found at the next boot")
        record = self.record()
        self.assertEqual(set(record), {"start", "flutter_before", "stub"})
        self.assertEqual(record["flutter_before"], "")
        for key in ("start", "stub"):
            self.assertRegex(record[key], r"^\d+\.\d+$", "uptime with the fraction, as LG's own log has it")
        self.assertLessEqual(float(record["start"]), float(record["stub"]))
        self.assertEqual(self.sh("bd_early_in_place", LIVE="no").returncode, 0)
        self.assertEqual(self.sh("bd_finish").returncode, 0)
        self.assertFalse(self.has("boot-direct.pending"))
        self.assertNotEqual(self.sh("bd_early_in_place", LIVE="no").returncode, 0, "a finished boot has nothing in doubt")

    def test_a_flutter_home_that_was_running_before_the_stub_is_on_the_record(self):
        self.flag()
        self.sam("flutter", running=True, front=True, flutter=True)
        self.assertEqual(self.early().returncode, 0)
        self.assertEqual(self.record()["flutter_before"].split(), ["812"], "what the check of the boot fails on")

    def test_a_stub_that_does_not_come_up_leaves_lgs_home_and_no_mark(self):
        self.flag()
        self.assertEqual(self.early(HS_UP="no").returncode, 1)
        self.assertEqual(self.called(), ["homestub_up"])
        self.assertFalse(self.has("boot-direct.pending"))
        self.assertFalse(self.has("boot-direct.strikes"), "a mount that failed is not a boot that stopped half way")
        self.assertTrue(self.has("boot-direct"), "the flag stays: the next boot tries again")
        self.assertEqual(self.record()["failed"], "homestub")

    def test_an_unfinished_earlier_boot_costs_a_strike_and_skips_the_stub_once(self):
        self.flag()
        (self.state / "boot-direct.pending").write_text("")
        self.assertEqual(self.early().returncode, 1)
        self.assertEqual(self.called(), [], "this boot goes without it")
        self.assertEqual((self.state / "boot-direct.strikes").read_text().strip(), "1")
        self.assertFalse(self.has("boot-direct.pending"))
        self.assertTrue(self.has("boot-direct"))
        self.assertEqual(self.record()["skipped"], "strike")
        self.assertNotEqual(self.sh("bd_early_in_place", LIVE="no").returncode, 0, "so the activation adopts nothing")
        self.assertEqual(self.early().returncode, 0, "and the next boot tries again")

    def test_two_faults_in_a_row_switch_the_flag_off(self):
        self.flag()
        (self.state / "boot-direct.pending").write_text("")
        (self.state / "boot-direct.strikes").write_text("1\n")
        self.assertEqual(self.early().returncode, 1)
        self.assertFalse(self.has("boot-direct"))
        self.assertIn("switched off", self.logged())

    def test_the_workers_second_pass_finds_the_stub_in_place_and_does_not_count_it_as_an_unfinished_boot(self):
        self.flag()
        self.assertEqual(self.early().returncode, 0)
        self.assertEqual(self.early().returncode, 0)
        self.assertEqual(self.called(), ["homestub_up", "watch"], "nothing is done twice")
        self.assertFalse(self.has("boot-direct.strikes"))

    def test_only_the_boot_worker_on_a_compositor_that_is_not_live_puts_it_in(self):
        self.flag()
        self.assertEqual(self.sh("bd_early_stub", LIVE="no").returncode, 1, "a hand-run compositor-boot is not a boot")
        self.assertEqual(self.sh("bd_early_stub", LIVE="yes", LUMA_BOOT="1").returncode, 1, "nor is a late call on a live compositor")
        self.assertEqual(self.called(), [])
        self.assertFalse(self.has("boot-direct.pending"))
        self.assertEqual(self.record(), {})

    def test_a_boot_that_does_not_start_lumatv_takes_the_stub_back_and_opens_lgs_home(self):
        self.flag()
        self.sam("web", mounted=True)
        (self.state / "boot-direct.pending").write_text("")
        self.assertEqual(self.sh("bd_early_undo").returncode, 0)
        self.assertEqual(self.called(), ["homestub_drop legacy", "hs_open_home"])
        self.assertFalse(self.has("boot-direct.pending"))
        self.assertIn("giving LG's Home back", self.logged())

    # ------------------------------------------------------------------------------------------ the early step of the dispatch

    def boot(self, **env):
        """What the dispatch runs first for the boot worker's `compositor-boot`: the stub, then what it says to the rest of that verb."""
        return self.sh('bd_early_boot; echo "on=${BD_EARLY_ON:-unset} lockwait=$LOCK_WAIT"', LIVE="no", LUMA_BOOT="1", **env)

    def test_the_early_step_of_the_dispatch_puts_the_stub_in_under_a_short_lock_wait_and_says_so(self):
        self.flag()
        result = self.boot()
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertEqual(result.stdout.split()[-2:], ["on=1", "lockwait=1"], "the caller's wait is back")
        self.assertEqual(self.called(), ["homestub_up", "watch"])
        self.assertEqual((self.tmp / "lockwaits").read_text().split(), ["10"], "a lock that is not free in 10 s is not waited for")

    def test_the_early_step_does_nothing_with_the_flag_off_or_the_compositor_not_enabled(self):
        self.assertEqual(self.boot().stdout.split()[-2:], ["on=0", "lockwait=1"], "no flag file")
        self.flag()
        self.assertEqual(self.boot(ENABLED="no").stdout.split()[-2:], ["on=0", "lockwait=1"])
        self.assertEqual(self.called(), [])
        self.assertFalse((self.tmp / "lockwaits").exists(), "not even the lock")

    def test_the_early_step_goes_without_the_stub_when_the_lock_is_not_free(self):
        self.flag()
        result = self.boot(LOCK="no")
        self.assertEqual(result.stdout.split()[-2:], ["on=0", "lockwait=1"], "and the wait is put back")
        self.assertEqual(self.called(), [])
        self.assertFalse(self.has("boot-direct.pending"))
        self.assertIn("no stub for this boot", self.logged())

    def test_the_early_step_reports_no_stub_when_the_stub_step_refuses(self):
        self.flag()
        (self.state / "boot-direct.pending").write_text("")
        self.assertEqual(self.boot().stdout.split()[-2:], ["on=0", "lockwait=1"], "an unfinished earlier boot: this one goes without")
        self.assertEqual(self.boot(HS_UP="no").stdout.split()[-2:], ["on=0", "lockwait=1"], "a stub that does not come up")

    def test_an_exit_before_the_verb_takes_the_stub_back_once(self):
        self.flag()
        self.sam("web", mounted=True)
        (self.state / "boot-direct.pending").write_text("")
        result = self.sh('BD_EARLY_ON=1; bd_early_gone; bd_early_gone; echo "on=$BD_EARLY_ON"')
        self.assertEqual(result.stdout.split()[-1], "on=0")
        self.assertEqual(self.called(), ["homestub_drop legacy", "hs_open_home"], "once")
        self.assertFalse(self.has("boot-direct.pending"))
        self.calls.unlink()
        self.sh("bd_early_gone")
        self.sh("BD_EARLY_ON=0; bd_early_gone")
        self.assertEqual(self.called(), [], "no stub of the early step, nothing to take back")

    def test_the_dispatch_runs_the_early_step_first_and_every_exit_before_the_verb_takes_the_stub_back(self):
        early = SOURCE.index("compositor-boot:) escape_lapse; bd_early_boot ;;")
        end = SOURCE.index("app_storage_guard() {", early)
        region = "\n".join(l for l in SOURCE[early:end].split("\n") if not l.lstrip().startswith("#"))
        self.assertLess(early, SOURCE.index("native_storage_guard || {", early), "before the storage guard")
        self.assertLess(early, SOURCE.index("migrate_legacy", early), "and before the migration")
        self.assertLess(early, SOURCE.index("ensure_config", early))
        self.assertEqual(region.count("exit 1"), 4, "the exits between the early step and the verb: the storage guard, the lock, the migration, the guard")
        self.assertEqual(region.count("bd_early_gone"), 4, "each of them gives the stub back first")
        for line in region.split("\n"):
            if "exit 1" in line and "bd_early_gone" not in line:
                self.assertIn(line.strip(), ("exit 1",), "the migration's own exit, whose block calls it on the line before it")
        self.assertIn("bd_early_gone\n      emit_error \"could not safely migrate", region)
        # the verb itself reads what the step did, and does not place a stub of its own
        verb = SOURCE[SOURCE.index("verb_compositor_boot() {"):SOURCE.index("\n}\n", SOURCE.index("verb_compositor_boot() {"))]
        self.assertIn("bd_early=${BD_EARLY_ON:-0}", verb)
        self.assertNotIn("bd_early_stub", verb)

    def test_a_boot_that_finishes_clears_the_strikes(self):
        self.flag()
        (self.state / "boot-direct.strikes").write_text("1\n")
        self.sh("bd_finish")
        self.assertFalse(self.has("boot-direct.strikes"))

    def test_a_failure_whose_outcome_is_known_counts_once(self):
        self.flag()
        (self.state / "boot-direct.pending").write_text("")
        self.sh('bd_fail "the stub did not start"')
        self.assertFalse(self.has("boot-direct.pending"), "or the next boot would count it again")
        self.assertEqual((self.state / "boot-direct.strikes").read_text().strip(), "1")
        self.assertTrue(self.has("boot-direct"))

    def test_an_abort_takes_the_stub_back_and_leaves_nothing_in_doubt(self):
        self.flag()
        self.sam("web", mounted=True)
        (self.state / "boot-direct.pending").write_text("")
        self.assertEqual(self.sh("bd_abort").returncode, 0)
        self.assertEqual(self.called(), ["homestub_drop legacy"])
        self.assertFalse(self.has("boot-direct.pending"))
        self.assertFalse(self.has("boot-direct.strikes"))

    # --------------------------------------------------------------------------- Home's launch after the restart

    def test_home_is_launched_after_webappmgr_answers_and_looked_for_in_sams_list(self):
        self.flag()
        self.sam("web")
        self.assertEqual(self.sh("bd_home_up", LAUNCH_STARTS="1").returncode, 0)
        self.assertEqual(self.called(), ["wait_wam", "launch"])

    def test_a_launch_lost_to_the_old_webappmgr_is_made_again(self):
        self.sam("web")
        self.assertEqual(self.sh("bd_home_up", LAUNCH_STARTS="3").returncode, 0)
        self.assertEqual(self.called(), ["wait_wam", "launch"] * 3)

    def test_a_stub_that_never_starts_gives_lgs_home_back(self):
        self.flag()
        self.sam("web", mounted=True)
        self.assertEqual(self.sh("bd_home_up", LAUNCH_STARTS="0").returncode, 1)
        self.assertEqual(self.called(), ["wait_wam", "launch"] * 3 + ["homestub_drop legacy", "hs_open_home"])
        self.assertFalse((self.tmp / "mounted").exists())
        self.assertIn("giving LG's Home back", self.logged())

    def test_a_home_that_runs_as_lgs_flutter_app_is_not_the_stub(self):
        # SAM still reads the old manifest: Home runs, and it is not what was asked for
        self.sam("flutter", running=True)
        self.assertEqual(self.sh("bd_home_up", LAUNCH_STARTS="1").returncode, 1)
        self.assertEqual(self.called().count("launch"), 3)

    # ------------------------------------------------------------------------------------------ boot-direct on/off

    def test_on_needs_the_bundle_and_the_stubs_files(self):
        self.api(False)
        data = self.verb("verb_boot_direct on", ok=False)
        self.assertFalse(data["ok"])
        self.assertIn("bundle predates", data["error"])
        self.assertFalse(self.has("boot-direct"))
        self.api()
        shutil.rmtree(self.app / "payload" / "homestub")
        data = self.verb("verb_boot_direct on", ok=False)
        self.assertIn("stub's files", data["error"])
        self.assertFalse(self.has("boot-direct"))

    def test_a_bundle_of_the_earlier_marker_is_refused(self):
        (self.comp / "boot-direct-api").write_text("1\n")
        data = self.verb("verb_boot_direct on", ok=False)
        self.assertIn("predates", data["error"])
        self.assertFalse(self.has("boot-direct"))
        self.flag()
        self.assertEqual(self.early().returncode, 1, "nor does the boot's early step put the stub in under it")
        self.assertEqual(self.called(), [])
        self.assertIn("predates", self.logged())

    def test_on_writes_the_bare_flag_and_waits_for_the_next_boot_under_lgs_compositor(self):
        (self.state / "boot-direct.strikes").write_text("1\n")
        (self.state / "boot-direct.pending").write_text("")
        data = self.verb("verb_boot_direct on", LIVE="no")
        self.assertTrue(data["ok"])
        self.assertEqual((self.state / "boot-direct").read_text(), "on\n", "no tool line: the compositor runs nothing")
        self.assertEqual(self.called(), [], "the stub would show nothing under LG's compositor")
        self.assertFalse(self.has("boot-direct.strikes") or self.has("boot-direct.pending"), "a fresh start")
        self.assertTrue(data["bootDirect"]["flag"])
        self.assertTrue(self.sh("bd_enabled").returncode == 0)

    def test_on_under_lumatvs_compositor_puts_the_stub_in_at_once(self):
        self.sam("flutter", running=True, front=True, flutter=True)
        data = self.verb("verb_boot_direct on", LIVE="yes", LAUNCH_STARTS="1")
        self.assertTrue(data["ok"])
        self.assertEqual(self.called(), ["homestub_up", "wait_wam", "launch"])
        self.assertTrue(data["bootDirect"]["stub"])
        self.assertEqual(data["bootDirect"]["sam"], "web")
        self.assertTrue(self.has("boot-direct"))
        self.assertFalse(self.has("boot-direct.pending"), "this is no boot's activation")

    def test_on_that_cannot_place_the_stub_is_off_again_and_lgs_home_is_back(self):
        self.sam("flutter", running=True, front=True)
        data = self.verb("verb_boot_direct on", ok=False, LIVE="yes", HS_UP="no")
        self.assertFalse(data["ok"])
        self.assertFalse(self.has("boot-direct"))
        data = self.verb("verb_boot_direct on", ok=False, LIVE="yes", LAUNCH_STARTS="0")
        self.assertFalse(self.has("boot-direct"))
        self.assertEqual(self.called()[-2:], ["homestub_drop legacy", "hs_open_home"])

    def test_on_does_not_wait_for_a_lock_it_cannot_take(self):
        data = self.verb("verb_boot_direct on", ok=False, LOCK="no")
        self.assertIn("in progress", data["error"])
        self.assertFalse(self.has("boot-direct"))

    def test_off_removes_everything_and_opens_lgs_home_only_where_the_stub_was_in_front(self):
        for front in (True, False):
            with self.subTest(front=front):
                self.calls.unlink(missing_ok=True)
                self.flag()
                (self.state / "boot-direct.strikes").write_text("1\n")
                self.sam("web", running=True, front=front, mounted=True)
                data = self.verb("verb_boot_direct off")
                self.assertTrue(data["ok"])
                self.assertEqual(self.called(), ["homestub_drop legacy"] + (["hs_open_home"] if front else []))
                self.assertFalse(self.has("boot-direct") or self.has("boot-direct.strikes"))
                self.assertFalse(data["bootDirect"]["flag"] or data["bootDirect"]["stub"])

    def test_off_with_nothing_on_is_quiet(self):
        data = self.verb("verb_boot_direct off")
        self.assertTrue(data["ok"])
        self.assertEqual(self.called(), ["homestub_drop legacy"])

    def test_status_says_what_is_in_place_and_changes_nothing(self):
        self.flag()
        self.sam("web", running=True, mounted=True)
        (self.state / "boot-direct.strikes").write_text("1\n")
        data = self.verb("verb_boot_direct status")
        self.assertEqual(data["bootDirect"], {"flag": True, "api": True, "stub": True, "kind": "web", "sam": "web", "flutter": False,
                                              "pending": False, "strikes": 1})
        self.assertEqual(self.called(), [])
        self.sam("flutter", running=True, flutter=True)
        data = self.verb("verb_boot_direct")
        self.assertEqual((data["bootDirect"]["stub"], data["bootDirect"]["kind"], data["bootDirect"]["sam"], data["bootDirect"]["flutter"]),
                         (False, "none", "flutter", True))
        self.sam("native", running=True, mounted="native")
        data = self.verb("verb_boot_direct status")
        self.assertEqual((data["bootDirect"]["stub"], data["bootDirect"]["kind"], data["bootDirect"]["sam"]), (True, "native", "native"))

    def test_a_word_it_does_not_know_is_an_error(self):
        data = self.verb("verb_boot_direct sideways", ok=False)
        self.assertFalse(data["ok"])

    # ------------------------------------------------------------------------------------- the dead man's switch of a boot

    def watch(self, secs="4", before="", **env):
        # `sleep` ends the wait at once; `before` can change what the loop finds
        return self.sh("%sBD_WATCH_SECS=%s; verb_boot_direct watch" % (before, secs), **env)

    def test_the_watch_ends_quietly_when_the_boot_finishes(self):
        self.flag()
        self.assertEqual(self.watch(LIVE="no").returncode, 0)
        self.assertEqual(self.called(), [], "no pending mark: nothing is in doubt")
        (self.state / "boot-direct.pending").write_text("")
        result = self.watch(before='sleep() { rm -f "$BD_PENDING"; }\n', LIVE="no")
        self.assertEqual(result.returncode, 0)
        self.assertEqual(self.called(), [], "the boot finished while it waited")

    def test_a_boot_that_does_not_finish_in_time_gives_the_stub_up_and_opens_lgs_home(self):
        self.flag()
        self.sam("web", mounted=True)
        (self.state / "boot-direct.pending").write_text("")
        self.assertEqual(self.watch(LIVE="no").returncode, 0)
        self.assertEqual(self.called(), ["homestub_drop legacy", "hs_open_home"])
        self.assertFalse(self.has("boot-direct.pending"))
        self.assertEqual((self.state / "boot-direct.strikes").read_text().strip(), "1", "a boot that stopped half way is a fault")
        self.assertIn("giving LG's Home back", self.logged())

    def test_a_compositor_that_is_live_is_not_taken_from_under_its_boot(self):
        self.flag()
        self.sam("web", mounted=True)
        (self.state / "boot-direct.pending").write_text("")
        self.assertEqual(self.watch(LIVE="yes").returncode, 0)
        self.assertEqual(self.called(), [])
        self.assertTrue(self.has("boot-direct.pending"))
        self.assertIn("the stub stays", self.logged())

    def test_a_watch_that_cannot_take_the_lock_changes_nothing(self):
        self.flag()
        (self.state / "boot-direct.pending").write_text("")
        self.assertEqual(self.watch(LOCK="no", LIVE="no").returncode, 1)
        self.assertEqual(self.called(), [])
        self.assertTrue(self.has("boot-direct.pending"))

    def test_the_watch_waits_on_while_the_boot_worker_is_alive_up_to_its_limit(self):
        self.flag()
        self.sam("web", mounted=True)
        (self.state / "boot-direct.pending").write_text("")
        count = 'sleep() { echo s >> "$FAKE/sleeps"; }\nBD_WATCH_MAX=12\n'
        self.assertEqual(self.watch(before=count, LIVE="no").returncode, 0)
        self.assertEqual((self.tmp / "sleeps").read_text().count("s"), 2, "no worker: the stub is given up at the first limit, 4 s of 2 s waits")
        self.assertEqual(self.called(), ["homestub_drop legacy", "hs_open_home"])

        (self.tmp / "sleeps").unlink()
        self.calls.unlink()
        self.sam("web", mounted=True)
        (self.state / "boot-direct.pending").write_text("")
        (self.tmp / "worker").write_text("%d\n" % os.getpid())       # a pid that is alive: the boot worker at its work
        self.assertEqual(self.watch(before=count, LIVE="no").returncode, 0)
        self.assertEqual((self.tmp / "sleeps").read_text().count("s"), 6, "a live worker: 12 s of waits, the second limit")
        self.assertEqual(self.called(), ["homestub_drop legacy", "hs_open_home"])

        # a worker that is done (its mark says so) is no reason to wait
        (self.tmp / "sleeps").unlink()
        self.calls.unlink()
        self.sam("web", mounted=True)
        (self.state / "boot-direct.pending").write_text("")
        (self.tmp / "worker").write_text("done\n")
        self.assertEqual(self.watch(before=count, LIVE="no").returncode, 0)
        self.assertEqual((self.tmp / "sleeps").read_text().count("s"), 2)

    # --------------------------------------------------------------------------------------- compositor on by hand

    def test_a_compositor_on_by_hand_puts_the_stub_in_before_its_restart_when_the_flag_is_on(self):
        self.flag()
        self.assertEqual(self.sh("bd_manual_stub").returncode, 0)
        self.assertEqual(self.called(), ["homestub_up"])
        self.assertFalse(self.has("boot-direct.pending"), "this is no boot: no mark, no watch")
        # flag off, a bundle that predates it, or a stub that does not come up: nothing is held back
        self.calls.unlink()
        (self.state / "boot-direct").unlink()
        self.assertEqual(self.sh("bd_manual_stub").returncode, 0)
        self.assertEqual(self.called(), [])
        self.flag()
        self.api(False)
        self.assertEqual(self.sh("bd_manual_stub").returncode, 0)
        self.assertEqual(self.called(), [])
        self.assertIn("predates", self.logged())
        self.api()
        self.assertEqual(self.sh("bd_manual_stub", HS_UP="no").returncode, 0)
        self.assertEqual(self.called(), ["homestub_up"])
        self.assertIn("LG's Home stays", self.logged())

    def test_the_stub_goes_in_before_the_restart_in_compositor_on(self):
        start = SOURCE.index("verb_compositor() {")
        body = SOURCE[start:SOURCE.index("\n}\n", start)]
        self.assertIn("(bd_manual_stub)\n          (restart_surface native)", body, "the stub first, then the restart")
        self.assertEqual(body.count("bd_manual_stub"), 1, "and only on the way to LumaTV's compositor, never on the way back")

    # ------------------------------------------------------ the real stub functions, against a model of SAM and the mounts

    def test_the_real_stub_functions_put_the_stub_in_and_take_it_out(self):
        self.flag()
        self.sam("flutter", running=True, front=True, flutter=True)
        (self.tmp / "mounts").write_text("")
        # in: LG's Flutter Home is stopped, SAM reads the stub's manifest, Home is launched as the stub
        self.assertEqual(self.sh("homestub_up && bd_home_up", real_stub=True, LAUNCH_STARTS="1").returncode, 0)
        calls = self.called()
        assets = "mount %s %s" % (self.tmp / "assets", self.tmp / "assets" / "stock")
        stub = "mount %s %s" % (self.state / "homestub", self.tmp / "home")
        kill = "pkill -f flutter-client -i com.webos.app.home"
        self.assertLess(calls.index(assets), calls.index(stub), "the assets bind goes first, so that it sits under the stub")
        self.assertLess(calls.index(stub), calls.index("block"), "SAM reads the manifest once the stub is in place")
        self.assertLess(calls.index("block"), calls.index("release"))
        self.assertLess(calls.index("release"), calls.index(kill), "LG's own instance is stopped")
        self.assertEqual(calls[-1], "launch")
        self.assertEqual((self.tmp / "sam_type").read_text().strip(), "web")
        self.assertEqual((self.state / "homestub" / "appinfo.json").read_text(), "appinfo.json", "staged from the payload")
        # out, as the escape's recovery does it: SAM reads Flutter again before the stub's page is stopped, and Home is launched
        self.calls.unlink()
        self.assertEqual(self.sh("homestub_drop legacy; hs_open_home", real_stub=True).returncode, 0)
        calls = self.called()
        self.assertEqual(calls[0], "umount %s" % (self.tmp / "home"))
        self.assertLess(calls.index("release"), calls.index("killApp"), "the type is back to Flutter before the stub's page goes")
        self.assertEqual(calls[-1], "launch")
        self.assertEqual((self.tmp / "sam_type").read_text().strip(), "flutter")
        self.assertFalse((self.tmp / "mounted").exists())

    def test_the_real_stub_functions_are_idempotent(self):
        self.flag()
        self.sam("flutter", running=True, front=True)
        self.sh("homestub_up && bd_home_up", real_stub=True, LAUNCH_STARTS="1")
        self.calls.unlink()
        self.sam("web", running=True, front=True, mounted=True)
        self.assertEqual(self.sh("homestub_up && bd_home_up", real_stub=True, LAUNCH_STARTS="1").returncode, 0)
        self.assertFalse(any(c.startswith("mount ") for c in self.called()), "the stub is not mounted twice")

    def test_homestub_up_waits_for_sam_only_when_asked_to(self):
        self.flag()
        self.sam("flutter")
        # SAM does not answer for the first twelve questions: more than the stub's own polling asks
        self.assertEqual(self.sh("homestub_up", real_stub=True, SAM_DOWN_FOR="12").returncode, 1, "not waited for: the stub is given up")
        self.assertFalse((self.tmp / "mounted").exists())
        self.calls.unlink()
        (self.tmp / "asked").unlink()
        self.sam("flutter")
        result = self.sh("HS_SAM_WAIT=20; homestub_up", real_stub=True, SAM_DOWN_FOR="12")
        self.assertEqual(result.returncode, 0, result.stdout)
        calls = self.called()
        self.assertTrue((self.tmp / "mounted").exists())
        self.assertLess(calls.index("mount %s %s" % (self.state / "homestub", self.tmp / "home")), calls.index("block"),
                        "the stub is bound at once, and SAM reads it once it answers")

    # -------------------------------------------------------------------------------------- the native kind of stub
    #
    # docs/research/boot-direct.md, "A native stub": the same placement with a program in place of the page. Which kind is over LG's Home
    # directory is read from the mount; the setting file says which one the next placement puts in.

    def native(self, text="native\n"):
        (self.state / "homestub-kind").write_text(text)

    def sam_type(self):
        return (self.tmp / "sam_type").read_text().strip()

    def test_the_kind_is_web_unless_the_setting_says_native_and_the_install_has_the_program(self):
        for text, expected in (("native\n", "native"), ("native", "native"), ("web\n", "web"), ("", "web"), ("Native\n", "web"),
                               ("nativ\n", "web"), ("\nnative\n", "web"), ("native stub\n", "web")):
            with self.subTest(text=text):
                self.native(text)
                self.assertEqual(self.sh("hs_kind_wanted").stdout, expected)
        (self.state / "homestub-kind").unlink()
        self.assertEqual(self.sh("hs_kind_wanted").stdout, "web", "no setting: the default")
        self.native()
        self.native_bin.chmod(0o644)
        self.assertEqual(self.sh("hs_kind_wanted").stdout, "web", "a program that cannot run")
        self.native_bin.chmod(0o755)
        (self.app / "payload" / "homestub-native" / "appinfo.json").unlink()
        self.assertEqual(self.sh("hs_kind_wanted").stdout, "web", "no manifest")
        (self.app / "payload" / "homestub-native" / "appinfo.json").write_text("native appinfo.json")
        self.native_bin.unlink()
        self.assertEqual(self.sh("hs_kind_wanted").stdout, "web", "an install without the program")

    def test_the_native_stub_is_staged_with_the_web_stubs_icon_and_a_program_that_is_renamed_into_place(self):
        stage = self.state / "homestub-native"
        result = self.sh('hs_stage native; echo "rc=$? staged=$HS_STAGED"', real_stub=True)
        self.assertEqual(result.stdout.split()[-2:], ["rc=0", "staged=1"])
        self.assertEqual((stage / "appinfo.json").read_text(), "native appinfo.json")
        self.assertEqual((stage / "icon.png").read_text(), "icon.png", "the web stub's: one file in the package, not two")
        self.assertEqual((stage / "lumatv-homestub").read_text(), "native program")
        self.assertTrue(os.access(stage / "lumatv-homestub", os.X_OK))
        self.assertFalse((stage / "lumatv-homestub.new").exists())
        self.assertEqual(self.sh('hs_stage native; echo "staged=$HS_STAGED"', real_stub=True).stdout.split()[-1], "staged=0", "nothing changed")
        # a new program: the running one is not written to (a program cannot be, while it runs), the new one is renamed over it
        inode = (stage / "lumatv-homestub").stat().st_ino
        self.native_bin.write_text("a newer program")
        self.assertEqual(self.sh('hs_stage native; echo "staged=$HS_STAGED"', real_stub=True).stdout.split()[-1], "staged=1", "a running copy has to be asked to go")
        self.assertEqual((stage / "lumatv-homestub").read_text(), "a newer program")
        self.assertNotEqual((stage / "lumatv-homestub").stat().st_ino, inode)
        self.assertEqual(sorted(os.listdir(stage)), ["appinfo.json", "icon.png", "lumatv-homestub"])
        self.native_bin.chmod(0o644)
        self.assertNotEqual(self.sh("hs_stage native", real_stub=True).returncode, 0, "a program that cannot run is not staged")

    def test_the_native_stub_goes_in_and_out_like_the_web_one(self):
        self.flag()
        self.native()
        self.sam("flutter", running=True, front=True, flutter=True)
        # in: LG's Flutter Home is stopped, SAM reads the native manifest, Home is launched, and the program is what SAM starts
        self.assertEqual(self.sh("homestub_up && bd_home_up", real_stub=True, LAUNCH_STARTS="1").returncode, 0)
        calls = self.called()
        assets = "mount %s %s" % (self.tmp / "assets", self.tmp / "assets" / "stock")
        stub = "mount %s %s" % (self.state / "homestub-native", self.tmp / "home")
        kill = "pkill -f flutter-client -i com.webos.app.home"
        self.assertLess(calls.index(assets), calls.index(stub), "the assets bind sits under the stub, whichever kind it is")
        self.assertLess(calls.index(stub), calls.index("block"))
        self.assertLess(calls.index("block"), calls.index("release"))
        self.assertLess(calls.index("release"), calls.index(kill))
        self.assertNotIn("wait_wam", calls, "a native stub has no WebAppMgr to wait for")
        self.assertEqual(calls[-1], "launch")
        self.assertEqual(self.sam_type(), "native")
        self.assertTrue((self.tmp / "native-proc").exists())
        status = self.verb("verb_boot_direct status")["bootDirect"]
        self.assertEqual((status["stub"], status["kind"], status["sam"]), (True, "native", "native"))
        # out, as the escape's recovery does it: SAM reads Flutter again, and then the stub's program is ended, not a page
        self.calls.unlink()
        self.assertEqual(self.sh("homestub_drop legacy; hs_open_home", real_stub=True).returncode, 0)
        calls = self.called()
        self.assertEqual(calls[0], "umount %s" % (self.tmp / "home"))
        self.assertLess(calls.index("release"), calls.index("pkill -x lumatv-homestub"), "the type is back to Flutter before the program goes")
        self.assertNotIn("killApp", calls)
        self.assertEqual(calls[-1], "launch")
        self.assertEqual(self.sam_type(), "flutter")
        self.assertFalse((self.tmp / "mounted").exists() or (self.tmp / "native-proc").exists())

    def test_a_stub_of_the_other_kind_is_taken_out_and_its_instance_ends_after_sam_has_read_the_new_one(self):
        for was, want, ended in (("web", "native", "killApp"), ("native", "web", "pkill -x lumatv-homestub")):
            with self.subTest(was=was, want=want):
                self.calls.unlink(missing_ok=True)
                self.flag()
                if want == "native":
                    self.native()
                else:
                    (self.state / "homestub-kind").unlink(missing_ok=True)
                self.sam(was, running=True, front=True, mounted=was, native_proc=was == "native")
                self.assertEqual(self.sh("homestub_up", real_stub=True).returncode, 0)
                calls = self.called()
                stage = "homestub-native" if want == "native" else "homestub"
                self.assertEqual(calls[0], "umount %s" % (self.tmp / "home"), "the other kind comes out first")
                self.assertLess(calls.index("mount %s %s" % (self.state / stage, self.tmp / "home")), calls.index("block"))
                self.assertEqual((calls.count("block"), calls.count("release")), (1, 1), "SAM reads once, with the new stub in")
                self.assertLess(calls.index("release"), calls.index(ended), "and the old instance goes after that")
                self.assertEqual(self.sh("hs_kind_mounted", real_stub=True).stdout, want)
                self.assertEqual(self.sam_type(), want)

    def test_the_same_kind_again_changes_nothing_and_a_new_program_asks_the_running_one_to_go(self):
        self.flag()
        self.native()
        self.sam("flutter", running=True, front=True, flutter=True)
        self.sh("homestub_up && bd_home_up", real_stub=True, LAUNCH_STARTS="1")
        self.calls.unlink()
        self.sam("native", running=True, front=True, mounted="native", native_proc=True)
        self.assertEqual(self.sh("homestub_up", real_stub=True).returncode, 0)
        self.assertEqual([c for c in self.called() if c.startswith(("mount", "umount", "block", "pkill -x"))], [], "nothing is mounted twice or ended")
        self.native_bin.write_text("a newer program")
        self.assertEqual(self.sh("homestub_up", real_stub=True).returncode, 0)
        calls = self.called()
        self.assertIn("pkill -x lumatv-homestub", calls, "the running copy keeps the old program until it ends")
        self.assertFalse(any(c.startswith("mount ") for c in calls))

    def test_a_native_stub_whose_files_cannot_be_staged_is_the_web_stub(self):
        self.flag()
        self.native()
        (self.state / "homestub-native").write_text("a file where the stage directory should be")
        self.sam("flutter", running=True, front=True, flutter=True)
        self.assertEqual(self.sh("homestub_up", real_stub=True).returncode, 0)
        self.assertEqual(self.sam_type(), "web")
        self.assertIn("mount %s %s" % (self.state / "homestub", self.tmp / "home"), self.called())
        self.assertIn("could not be staged", self.logged())

    def test_a_native_stub_that_never_starts_is_replaced_by_the_web_stub_and_the_setting_goes_back(self):
        self.flag()
        self.native()
        self.sam("flutter", running=True, front=True, flutter=True)
        self.assertEqual(self.sh("homestub_up", real_stub=True).returncode, 0)
        self.calls.unlink()
        # the fourth launch is the first that starts anything: three of the native stub's, none of them waiting for WebAppMgr, then the web stub's
        self.assertEqual(self.sh("bd_home_up", real_stub=True, LAUNCH_STARTS="4").returncode, 0)
        calls = self.called()
        self.assertEqual(calls[:3], ["launch"] * 3)
        self.assertLess(calls.index("umount %s" % (self.tmp / "home")), calls.index("mount %s %s" % (self.state / "homestub", self.tmp / "home")))
        self.assertLess(calls.index("release"), calls.index("pkill -x lumatv-homestub"), "the native program that did not take is ended")
        self.assertEqual(calls[-2:], ["wait_wam", "launch"], "the web stub waits for WebAppMgr as ever")
        self.assertEqual(self.sam_type(), "web")
        self.assertFalse(self.has("homestub-kind"), "the next boot does not try the native stub again")
        self.assertIn("the web stub takes its place", self.logged())

    def test_when_neither_kind_starts_lgs_home_is_given_back(self):
        self.flag()
        self.native()
        self.sam("flutter", running=True, front=True, flutter=True)
        self.sh("homestub_up", real_stub=True)
        self.calls.unlink()
        self.assertEqual(self.sh("bd_home_up", real_stub=True, LAUNCH_STARTS="0").returncode, 1)
        calls = self.called()
        self.assertEqual(calls.count("launch"), 3 + 3 + 1, "three of each kind's, and LG's own")
        self.assertEqual(calls[-1], "launch")
        self.assertEqual(self.sam_type(), "flutter")
        self.assertFalse((self.tmp / "mounted").exists() or (self.tmp / "native-proc").exists())
        self.assertFalse(self.has("homestub-kind"))

    # the verb: homestub-kind

    def test_homestub_kind_status_says_what_is_set_wanted_and_in_place(self):
        data = self.verb("verb_stub_kind status")["homestubKind"]
        self.assertEqual(data, {"setting": "web", "wanted": "web", "mounted": "none", "sam": "flutter", "nativeInstalled": True})
        self.native()
        self.sam("native", running=True, mounted="native")
        data = self.verb("verb_stub_kind")["homestubKind"]
        self.assertEqual(data, {"setting": "native", "wanted": "native", "mounted": "native", "sam": "native", "nativeInstalled": True})
        self.native_bin.unlink()
        data = self.verb("verb_stub_kind")["homestubKind"]
        self.assertEqual((data["setting"], data["wanted"], data["nativeInstalled"]), ("native", "web", False))
        self.assertEqual(self.called(), [])

    def test_homestub_kind_native_needs_the_program_and_with_no_stub_in_only_writes_the_setting(self):
        self.native_bin.unlink()
        data = self.verb("verb_stub_kind native", ok=False)
        self.assertIn("no native stub", data["error"])
        self.assertFalse(self.has("homestub-kind"))
        self.native_bin.write_text("native program")
        self.native_bin.chmod(0o755)
        data = self.verb("verb_stub_kind native")
        self.assertEqual((self.state / "homestub-kind").read_text(), "native\n")
        self.assertEqual((data["homestubKind"]["setting"], data["homestubKind"]["mounted"]), ("native", "none"))
        self.assertEqual(self.called(), [], "no stub is in: the next placement is the one that changes")
        data = self.verb("verb_stub_kind web")
        self.assertFalse(self.has("homestub-kind"), "web is the absence of the file")
        self.assertEqual(data["homestubKind"]["setting"], "web")

    def test_homestub_kind_swaps_a_stub_that_is_in_on_a_live_compositor(self):
        self.flag()
        self.sam("web", running=True, front=True, mounted="web")
        data = self.verb("verb_stub_kind native", LIVE="yes", LAUNCH_STARTS="1")
        self.assertEqual(self.called(), ["homestub_up", "launch"], "native: nothing to wait for")
        self.assertEqual((data["homestubKind"]["mounted"], data["homestubKind"]["sam"]), ("native", "native"))
        self.calls.unlink()
        self.sam("native", running=True, front=True, mounted="native", native_proc=True)
        data = self.verb("verb_stub_kind web", LIVE="yes", LAUNCH_STARTS="1")
        self.assertEqual(self.called(), ["homestub_up", "wait_wam", "launch"])
        self.assertEqual(data["homestubKind"]["mounted"], "web")
        self.assertFalse(self.has("homestub-kind"))

    def test_homestub_kind_waits_for_a_live_compositor_with_the_panel_on(self):
        self.flag()
        for env in ({"LIVE": "no"}, {"LIVE": "yes", "PANEL": "standby"}):
            with self.subTest(env=env):
                self.sam("web", running=True, front=True, mounted="web")
                data = self.verb("verb_stub_kind native", **env)
                self.assertEqual(self.called(), [], "the stub that is in stays; the setting is for the next placement")
                self.assertEqual((data["homestubKind"]["setting"], data["homestubKind"]["mounted"]), ("native", "web"))
                (self.state / "homestub-kind").unlink()

    def test_a_swap_that_does_not_take_puts_the_stub_that_was_in_back(self):
        self.flag()
        # the new stub cannot be put in: the old one was never taken out
        self.sam("web", running=True, front=True, mounted="web")
        data = self.verb("verb_stub_kind native", ok=False, LIVE="yes", HS_UP="no")
        self.assertIn("did not take", data["error"])
        self.assertFalse(self.has("homestub-kind"), "the setting is as it was")
        self.assertEqual(self.called(), ["homestub_up"])
        self.assertEqual(self.sh("hs_kind_mounted").stdout, "web")
        # the native stub goes in and never starts: bd_home_up has put the web stub back and the setting with it
        self.calls.unlink()
        self.sam("web", running=True, front=True, mounted="web")
        data = self.verb("verb_stub_kind native", ok=False, LIVE="yes", LAUNCH_STARTS="4")
        self.assertIn("did not take", data["error"])
        self.assertFalse(self.has("homestub-kind"))
        self.assertEqual(self.called(), ["homestub_up"] + ["launch"] * 3 + ["homestub_up", "wait_wam", "launch"])
        self.assertEqual(self.sh("hs_kind_mounted").stdout, "web")
        self.assertEqual(self.sam_type(), "web")

    def test_a_swap_that_cannot_even_put_the_old_stub_back_gives_lgs_home_back(self):
        self.flag()
        self.sam("web", running=True, front=True, mounted="web")
        # whatever is put in never starts: the native stub, then the web one that replaces it, then the web one that is put back
        data = self.verb("verb_stub_kind native", ok=False, LIVE="yes", LAUNCH_STARTS="0")
        self.assertIn("did not take", data["error"])
        calls = self.called()
        self.assertEqual(calls[-2:], ["homestub_drop legacy", "hs_open_home"], "the last resort")
        self.assertFalse(self.has("homestub-kind"))

    def test_the_verb_is_dispatched_and_the_way_back_works_under_a_maintenance_veto(self):
        self.assertIn('homestub-kind) verb_stub_kind "${2:-status}"; exit $? ;;', SOURCE)
        self.assertIn("homestub-kind:status|homestub-kind:) MAINTENANCE_ACCESS=read ;;", SOURCE)
        self.assertIn("homestub-kind:web) MAINTENANCE_ACCESS=emergency ;;", SOURCE)
        self.assertNotIn("homestub-kind:native) MAINTENANCE_ACCESS", SOURCE, "native is a change like any other")
        data = self.verb("verb_stub_kind sideways", ok=False)
        self.assertIn("status, web or native", data["error"])

    # ------------------------------------------------------------------------------------------------------ the escape
    #
    # Home held 10 s is the supervisor's (compositor/sm/lumatv-sm.c, its own tests); the tool's part is what it leaves behind: a hold and
    # `escaped` for the rest of that boot and no longer, no fault, and no way to LG's Home of its own.

    def dispatch(self, verb, *words):
        """The tool's last `case` on the verb, with only the error answer made real: a verb that is not there is an unknown one."""
        return self.sh("VERB=%s\nset -- %s\n%s" % (verb, " ".join((verb,) + words), SOURCE[SOURCE.rfind('\ncase "$VERB" in'):]))

    def test_there_is_no_verb_for_lgs_home_and_no_hold_of_five_seconds(self):
        for word in ("lg", "luma", "status"):
            result = self.dispatch("home", word)
            self.assertEqual(result.returncode, 1, word)
            self.assertEqual(json.loads(result.stdout.strip().split("\n")[-1]), {"ok": False, "error": "unknown verb: home"})
        self.assertNotIn("verb_home", SOURCE)
        self.assertNotIn("home lg", SOURCE)
        self.assertNotIn("home luma", SOURCE)

    def test_the_verbs_are_dispatched_and_classified_for_the_maintenance_guard(self):
        self.assertIn('boot-direct) verb_boot_direct "${2:-status}"; exit $? ;;', SOURCE)
        self.assertIn("boot-direct:status|boot-direct:) MAINTENANCE_ACCESS=read ;;", SOURCE)
        # the way back to LG's Home from a boot that did not work out is an emergency: it must work while a maintenance veto stands
        self.assertIn("boot-direct:off|boot-direct:watch) MAINTENANCE_ACCESS=emergency ;;", SOURCE)

    def test_the_installer_writes_the_marker_the_tool_looks_for(self):
        text = (ROOT / "compositor/tools/tv-cutover.sh").read_text()
        self.assertIn("echo 2 > $STATE/boot-direct-api", text)
        self.assertIn('"$COMP/boot-direct-api"', SOURCE)

    def escape(self, boot_mark=False, hold=True, escaped=True):
        if hold:
            (self.comp / "stock-ui-held").write_text("812:1234:5\n")
        if escaped:
            (self.comp / "escaped").write_text("hold Home 10 s (supervisor)")
        if boot_mark:
            self.put("escape-boot", "hold Home 10 s (supervisor)")

    def test_an_escape_lapses_at_the_next_boot_and_the_boot_starts_lumatv(self):
        self.escape()
        self.assertEqual(self.sh("escape_lapse").returncode, 0)
        self.assertFalse(self.has("compositor/stock-ui-held") or self.has("compositor/escaped"), "what refuses the boot (comp_boot_decide, decide()) is gone")
        self.assertIn("lapsed", self.logged())
        self.assertEqual(self.sh("escape_lapse").returncode, 0, "and doing it again is no harm")

    def test_an_escape_of_this_boot_holds_for_the_rest_of_it(self):
        self.escape(boot_mark=True)
        self.assertEqual(self.sh("escape_lapse").returncode, 0)
        self.assertTrue(self.has("compositor/stock-ui-held") and self.has("compositor/escaped"), "the boot worker's second pass does not start LumaTV again")
        self.assertEqual(self.logged(), "")

    def test_a_hold_that_a_fault_made_does_not_lapse(self):
        self.escape(escaped=False)
        self.assertEqual(self.sh("escape_lapse").returncode, 0)
        self.assertTrue(self.has("compositor/stock-ui-held"), "a crash holds LumaTV off until `compositor on`")
        self.assertEqual(self.logged(), "")
        self.escape(hold=False, escaped=False)
        self.assertEqual(self.sh("escape_lapse").returncode, 0, "no escape, nothing to do")

    def test_the_boot_worker_lets_the_escape_lapse_before_anything_looks_at_the_hold(self):
        self.assertIn("compositor-boot:) escape_lapse; bd_early_boot ;;", SOURCE)
        self.assertLess(SOURCE.index("compositor-boot:) escape_lapse; bd_early_boot ;;"), SOURCE.index("native_storage_guard() {"))

    def test_the_marks_of_the_tool_and_of_the_supervisor_are_the_same_file(self):
        sm = (ROOT / "compositor/sm/lumatv-sm.c").read_text()
        self.assertIn('static const char *escape_mark = "/tmp/.lumatv-escaped";', sm)
        self.assertIn('ESCAPE_BOOT="/tmp/.$SHORT_ID-escaped"', SOURCE, "with the short id lumatv, as brand.json has it")

    def test_a_boot_cut_short_by_an_escape_is_no_fault_while_the_stock_ui_is_held(self):
        self.flag()
        self.escape()
        (self.state / "boot-direct.pending").write_text("")
        self.sh('bd_fail "the stub did not start"')
        self.sh('bd_strike "the boot did not finish"')
        self.assertFalse(self.has("boot-direct.strikes"), "nothing counted")
        self.assertTrue(self.has("boot-direct"), "the flag stays on: the next boot comes up on LumaTV with it")
        self.assertIn("no fault counted", self.logged())
        # a hold gone (a boot that ended in doubt under LumaTV's compositor) counts as ever
        (self.comp / "stock-ui-held").unlink()
        self.sh('bd_strike "the boot did not finish"')
        self.assertEqual((self.state / "boot-direct.strikes").read_text().strip(), "1")

    def test_the_watch_of_an_escaped_boot_counts_nothing(self):
        self.flag()
        self.escape()
        self.sam("web", mounted=True)
        (self.state / "boot-direct.pending").write_text("")
        self.assertEqual(self.watch(LIVE="no").returncode, 0)
        self.assertFalse(self.has("boot-direct.strikes"))
        self.assertTrue(self.has("boot-direct"))


if __name__ == "__main__":
    unittest.main()
