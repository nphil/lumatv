"""Boot direct's shell side (tools/lumatv): the Home stub's placement around the compositor restart, the strikes that keep a bad boot from
repeating, `boot-direct on|off|status` and `home lg|luma`. The real functions run under busybox ash, as on the set; the platform (SAM, the
mounts, WebAppMgr) is a stand-in that models what each call does to it."""
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
    echo "{\\"appInfo\\":{\\"id\\":\\"com.webos.app.home\\",\\"type\\":\\"$(cat "$FAKE/sam_type" 2>/dev/null || echo flutter)\\"},\\"returnValue\\":true}" ;;
  *applicationManager/getForegroundAppInfo)
    if [ -e "$FAKE/front" ]; then echo '{"appId":"com.webos.app.home","returnValue":true}'
    else echo '{"appId":"netflix","returnValue":true}'; fi ;;
  *applicationManager/blockSystemApp) echo block >> "$FAKE/calls"; echo '{"returnValue":true}' ;;
  *applicationManager/releaseBlockSystemApp)
    echo release >> "$FAKE/calls"
    # SAM reads the app directory again: the type is whatever is mounted there now
    if [ -e "$FAKE/mounted" ]; then echo web > "$FAKE/sam_type"; else echo flutter > "$FAKE/sam_type"; fi
    echo '{"returnValue":true}' ;;
  *applicationManager/launch)
    echo launch >> "$FAKE/calls"
    n=$(cat "$FAKE/launches" 2>/dev/null || echo 0); n=$((n+1)); echo $n > "$FAKE/launches"
    # LAUNCH_STARTS=N: the Nth launch is the one that starts Home; 0: none does
    if [ "${LAUNCH_STARTS:-1}" -gt 0 ] && [ "$n" -ge "${LAUNCH_STARTS:-1}" ]; then touch "$FAKE/running"; fi
    echo '{"returnValue":true}' ;;
  *webappmanager/killApp) echo killApp >> "$FAKE/calls"; rm -f "$FAKE/running"; echo '{"returnValue":true}' ;;
  *webappmanager/listRunningApps) echo '{"returnValue":true}' ;;
esac
exit 0
'''

FAKE_MOUNT = '''#!/bin/sh
# `mount --bind SRC DST` on Home's directory; with no arguments the table, of the assets bind only.
if [ "$1" = --bind ]; then
  echo "mount $2 $3" >> "$FAKE/calls"
  case "$3" in "$HOME_APP_DIR") touch "$FAKE/mounted" ;; esac
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
# The device and inode of the staged stub, which the app directory shows while the stub is mounted over it.
case "$3" in
  "$HOME_APP_DIR") if [ -e "$FAKE/mounted" ]; then echo "9:77"; else echo "9:11"; fi ;;
  "$STUB_STAGE") echo "9:77" ;;
  *) exec /usr/bin/stat "$@" ;;
esac
'''

FAKE_PKILL = '''#!/bin/sh
echo "pkill $*" >> "$FAKE/calls"
rm -f "$FAKE/flutter" "$FAKE/running"
exit 0
'''

FAKE_PGREP = '''#!/bin/sh
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
        for d in (self.comp, self.bin, self.app / "payload" / "homestub", self.tmp / "home", self.tmp / "assets" / "i18n"):
            d.mkdir(parents=True)
        for name in ("appinfo.json", "index.html", "icon.png"):
            (self.app / "payload" / "homestub" / name).write_text(name)
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
        path.write_text("1\n") if present else path.unlink(missing_ok=True)

    def flag(self, text="on\ntool /app/tools/lumatv\n"):
        (self.state / "boot-direct").write_text(text)

    def has(self, name):
        return (self.state / name).exists()

    def put(self, name, text=""):
        (self.tmp / name).write_text(text)

    def sam(self, type_="flutter", running=False, front=False, flutter=False, mounted=False):
        self.put("sam_type", type_ + "\n")
        for name, on in (("running", running), ("front", front), ("flutter", flutter), ("mounted", mounted)):
            (self.tmp / name).unlink(missing_ok=True)
            if on:
                self.put(name)

    def called(self):
        return self.calls.read_text().split("\n")[:-1] if self.calls.exists() else []

    def logged(self):
        return self.log.read_text() if self.log.exists() else ""

    # -- running the real functions

    def script(self, body, real_stub=False):
        names = ["bd_enabled", "bd_api", "bd_strikes", "bd_strike", "bd_finish", "bd_fail", "bd_abort", "bd_stub_up",
                 "bd_home_up", "bd_json", "verb_boot_direct", "verb_home", "home_running", "home_in_front",
                 "sam_home_type", "json_escape", "emit_error", "hs_stub_in_front", "hs_open_home"]
        if real_stub:
            names += ["homestub_up", "homestub_drop", "hs_mounted_any", "hs_mounted_ours", "hs_stage", "hs_mode_set",
                      "hs_sam_reread", "hs_wait_type", "hs_unmount", "hs_wd_alive", "hs_wd_stop", "kill_home_app", "is_mounted"]
        fakes = '''
sleep() { :; }
uptime_s() { echo 42; }
log_line() { echo "$1" >> "$FAKE/log"; }
take_mutation_lock() { [ "${LOCK:-ok}" = ok ]; }
comp_live() { [ "${LIVE:-yes}" = yes ]; }
panel_is_active() { [ "${PANEL:-active}" = active ]; }
wait_wam_restart() { echo wait_wam >> "$FAKE/calls"; return 1; }
'''
        if not real_stub:
            fakes += '''
hs_mounted_ours() { [ -e "$FAKE/mounted" ]; }
homestub_up() {
  echo homestub_up >> "$FAKE/calls"
  [ "${HS_UP:-ok}" = ok ] || return 1
  # the real one stops LG's own instance (kill_home_app): what runs after it is what the launch starts
  touch "$FAKE/mounted"; echo web > "$FAKE/sam_type"; rm -f "$FAKE/running" "$FAKE/flutter"
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
export HOME_APP_DIR STUB_STAGE
'''
        shared = constants("HOME_PKILL_PATTERN", "HOME_LAUNCH_URI", "HOME_LAUNCH_PARAMS", "HOME_APP_ID",
                           "BD_FLAG", "BD_PENDING", "BD_STRIKES", "BD_MAX_STRIKES", "BD_HOME_TRIES")
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

    def test_a_bundle_that_predates_boot_direct_gets_no_stub(self):
        self.flag()
        self.api(False)
        self.assertEqual(self.sh("bd_stub_up").returncode, 1)
        self.assertEqual(self.called(), [])
        self.assertFalse(self.has("boot-direct.pending"))
        self.assertIn("predates", self.logged())

    def test_a_clean_boot_places_the_stub_and_marks_itself_unfinished_until_home_is_up(self):
        self.flag()
        self.assertEqual(self.sh("bd_stub_up").returncode, 0)
        self.assertEqual(self.called(), ["homestub_up"])
        self.assertTrue(self.has("boot-direct.pending"), "a power cut now is found at the next boot")
        self.assertEqual(self.sh("bd_finish").returncode, 0)
        self.assertFalse(self.has("boot-direct.pending"))

    def test_a_stub_that_does_not_come_up_leaves_lgs_home_and_no_mark(self):
        self.flag()
        self.assertEqual(self.sh("bd_stub_up", HS_UP="no").returncode, 1)
        self.assertFalse(self.has("boot-direct.pending"))
        self.assertFalse(self.has("boot-direct.strikes"), "a mount that failed is not a boot that stopped half way")
        self.assertTrue(self.has("boot-direct"), "the flag stays: the next boot tries again")

    def test_an_unfinished_earlier_boot_costs_a_strike_and_skips_the_stub_once(self):
        self.flag()
        (self.state / "boot-direct.pending").write_text("")
        self.assertEqual(self.sh("bd_stub_up").returncode, 1)
        self.assertEqual(self.called(), [], "this boot goes without it")
        self.assertEqual((self.state / "boot-direct.strikes").read_text().strip(), "1")
        self.assertFalse(self.has("boot-direct.pending"))
        self.assertTrue(self.has("boot-direct"))
        self.assertEqual(self.sh("bd_stub_up").returncode, 0, "and the next boot tries again")

    def test_two_faults_in_a_row_switch_the_flag_off(self):
        self.flag()
        (self.state / "boot-direct.pending").write_text("")
        (self.state / "boot-direct.strikes").write_text("1\n")
        self.assertEqual(self.sh("bd_stub_up").returncode, 1)
        self.assertFalse(self.has("boot-direct"))
        self.assertIn("switched off", self.logged())

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

    def test_on_writes_the_flag_with_the_tool_and_waits_for_the_next_boot_under_lgs_compositor(self):
        (self.state / "boot-direct.strikes").write_text("1\n")
        (self.state / "boot-direct.pending").write_text("")
        data = self.verb("verb_boot_direct on", LIVE="no")
        self.assertTrue(data["ok"])
        self.assertEqual((self.state / "boot-direct").read_text(), "on\ntool %s/tools/lumatv\n" % (self.tmp / "app"))
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
        self.assertEqual(data["bootDirect"], {"flag": True, "api": True, "stub": True, "sam": "web", "flutter": False,
                                              "pending": False, "strikes": 1})
        self.assertEqual(self.called(), [])
        self.sam("flutter", running=True, flutter=True)
        data = self.verb("verb_boot_direct")
        self.assertEqual((data["bootDirect"]["stub"], data["bootDirect"]["sam"], data["bootDirect"]["flutter"]), (False, "flutter", True))

    def test_a_word_it_does_not_know_is_an_error(self):
        data = self.verb("verb_boot_direct sideways", ok=False)
        self.assertFalse(data["ok"])
        data = self.verb("verb_home sideways", ok=False)
        self.assertFalse(data["ok"])

    # --------------------------------------------------------------------------------------------- home lg / luma

    def test_home_lg_takes_the_stub_away_and_opens_lgs_home(self):
        self.flag()
        self.sam("web", running=True, front=True, mounted=True)
        data = self.verb("verb_home lg")
        self.assertEqual(data, {"ok": True, "home": "lg"})
        self.assertEqual(self.called(), ["homestub_drop lg", "hs_open_home"])
        self.assertTrue(self.has("boot-direct"), "the flag is the owner's: a hold does not switch boot direct off")

    def test_home_lg_with_no_stub_still_opens_lgs_home(self):
        self.sam("flutter", running=True)
        self.assertTrue(self.verb("verb_home lg")["ok"])
        self.assertEqual(self.called(), ["homestub_drop lg", "hs_open_home"])

    def test_home_lg_gives_up_on_a_lock_it_cannot_take(self):
        data = self.verb("verb_home lg", ok=False, LOCK="no")
        self.assertFalse(data["ok"])
        self.assertEqual(self.called(), [])

    def test_home_luma_puts_the_stub_back_and_launches_it_where_home_is_in_front(self):
        self.flag()
        self.sam("flutter", running=True, front=True, flutter=True)
        data = self.verb("verb_home luma", LAUNCH_STARTS="1")
        self.assertEqual(data, {"ok": True, "home": "luma"})
        self.assertEqual(self.called(), ["homestub_up", "wait_wam", "launch"])

    def test_home_luma_with_an_app_in_front_leaves_the_app_alone(self):
        self.flag()
        self.sam("flutter", running=True, front=False, flutter=True)
        data = self.verb("verb_home luma")
        self.assertEqual(data, {"ok": True, "home": "luma"})
        self.assertEqual(self.called(), ["homestub_up"], "Home is not launched over the app: the next Home key opens the stub")

    def test_home_luma_is_refused_without_the_flag_or_a_live_compositor_or_the_lock(self):
        self.sam("flutter", running=True, front=True)
        data = self.verb("verb_home luma", ok=False)
        self.assertIn("off", data["error"])
        self.flag()
        data = self.verb("verb_home luma", ok=False, LIVE="no")
        self.assertIn("not live", data["error"])
        data = self.verb("verb_home luma", ok=False, LOCK="no")
        self.assertIn("in progress", data["error"])
        self.assertEqual(self.called(), [])

    def test_home_luma_that_cannot_place_the_stub_leaves_lgs_home(self):
        self.flag()
        self.sam("flutter", running=True, front=True)
        data = self.verb("verb_home luma", ok=False, HS_UP="no")
        self.assertFalse(data["ok"])
        self.assertEqual(self.called(), ["homestub_up"])
        self.assertTrue(self.has("boot-direct"), "a stub that would not mount is not a reason to switch boot direct off")

    def test_home_luma_whose_stub_never_starts_gives_lgs_home_back_without_a_fault_of_the_boot(self):
        self.flag()
        self.sam("flutter", running=True, front=True)
        data = self.verb("verb_home luma", ok=False, LAUNCH_STARTS="0")
        self.assertFalse(data["ok"])
        self.assertEqual(self.called()[-2:], ["homestub_drop legacy", "hs_open_home"])
        self.assertFalse(self.has("boot-direct.strikes"))

    # ------------------------------------------------------ the real stub functions, against a model of SAM and the mounts

    def test_home_lg_and_luma_through_the_real_stub_functions(self):
        self.flag()
        self.sam("flutter", running=True, front=True, flutter=True)
        (self.tmp / "mounts").write_text("")
        # to the stub: LG's Flutter Home is stopped, SAM reads the stub's manifest, Home is launched as the stub
        data = self.verb("verb_home luma", real_stub=True, LAUNCH_STARTS="1")
        self.assertEqual(data, {"ok": True, "home": "luma"})
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
        # and to LG's Home: SAM reads Flutter again before the stub's page is stopped, and Home is launched
        self.calls.unlink()
        data = self.verb("verb_home lg", real_stub=True)
        self.assertEqual(data, {"ok": True, "home": "lg"})
        calls = self.called()
        self.assertEqual(calls[0], "umount %s" % (self.tmp / "home"))
        self.assertLess(calls.index("release"), calls.index("killApp"), "the type is back to Flutter before the stub's page goes")
        self.assertEqual(calls[-1], "launch")
        self.assertEqual((self.tmp / "sam_type").read_text().strip(), "flutter")
        self.assertFalse((self.tmp / "mounted").exists())

    def test_the_real_stub_functions_are_idempotent_for_a_second_luma(self):
        self.flag()
        self.sam("flutter", running=True, front=True)
        self.verb("verb_home luma", real_stub=True, LAUNCH_STARTS="1")
        self.calls.unlink()
        self.sam("web", running=True, front=True, mounted=True)
        data = self.verb("verb_home luma", real_stub=True, LAUNCH_STARTS="1")
        self.assertTrue(data["ok"])
        self.assertFalse(any(c.startswith("mount ") for c in self.called()), "the stub is not mounted twice")

    # -------------------------------------------------------------------------------------------------- the wiring

    def test_the_verbs_are_dispatched_and_classified_for_the_maintenance_guard(self):
        self.assertIn('boot-direct) verb_boot_direct "${2:-status}"; exit $? ;;', SOURCE)
        self.assertIn('home) verb_home "${2:-status}"; exit $? ;;', SOURCE)
        self.assertIn("boot-direct:status|boot-direct:|home:status) MAINTENANCE_ACCESS=read ;;", SOURCE)
        # the ways back to LG's Home are emergencies: they must work while a maintenance veto stands
        self.assertIn("boot-direct:off|home:lg) MAINTENANCE_ACCESS=emergency ;;", SOURCE)
        self.assertNotIn("home:luma", SOURCE, "putting the stub back is an ordinary write")

    def test_the_installer_writes_the_marker_the_tool_looks_for(self):
        text = (ROOT / "compositor/tools/tv-cutover.sh").read_text()
        self.assertIn("echo 1 > $STATE/boot-direct-api", text)
        self.assertIn('"$COMP/boot-direct-api"', SOURCE)


if __name__ == "__main__":
    unittest.main()
