"""Stock UI recovery against isolated files, mount state and stand-in platform commands."""
import fcntl
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import time
import unittest


ROOT = Path(__file__).resolve().parents[1]
CLI = ROOT / "tools" / "lumatv"
INPUT = "/usr/palm/applications/com.webos.app.inputcommon/qml"
VOICE = "/usr/palm/applications/com.webos.app.voice/qml"
WEB = "/usr/palm/applications/com.webos.app.settings"
SUPERVISOR = '''#!/bin/sh
[ "$1" = --state ] && [ "$2" = "$FAKE/state/compositor" ] &&
  [ "$3" = --would-enable ] && [ "$4" = persistent ] || exit 10
if [ "${FAIL_SUPERVISOR:-0}" = 1 ]; then echo "no failures: fixture"; exit 3; fi
echo run
'''


def function(source, name):
    match = re.search(r"^" + re.escape(name) + r"\(\) [\{\(].*?^[\}\)]", source, re.M | re.S)
    if match is None:
        raise AssertionError("missing shell function " + name)
    return match.group()


class StockUiRestore(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="stock-ui-restore-"))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.state = self.tmp / "state"
        self.comp = self.state / "compositor"
        self.bin = self.tmp / "bin"
        self.comp.mkdir(parents=True)
        (self.comp / "supervisor-api").write_text("3\n")
        self.bin.mkdir()
        self.mounts = self.tmp / "mounts"
        self.mounts.write_text("")
        self.calls = self.tmp / "calls"
        self.calls.write_text("")
        self.status = self.tmp / "status.json"
        self.status.write_text("{}")
        for directory in ("stub", "stubn", "home", "assets", "stock-assets"):
            (self.tmp / directory).mkdir()
        self.env_file = self.tmp / "surface.env"
        self.notification = self.tmp / "notification.json"
        self.prefs = self.state / "config.json"
        self.prefs.write_text('{"homestub":true,"theme":"kept"}\n')
        (self.comp / "stock-ui-held").touch()
        self.command("timeout", '#!/bin/sh\nshift\nexec "$@"\n')
        self.command("mount", '#!/bin/sh\necho mount >> "$FAKE/calls"\ntouch "$FAKE/supervisor-mounted"\n')
        self.command("sleep", "#!/bin/sh\nexit 0\n")
        # the native stub's process is a file: `pkill -x` ends it unless the fixture says it will not die
        self.command("pgrep", '''#!/bin/sh
case "$*" in *-x*lumatv-homestub*) [ -e "$FAKE/native-proc" ]; exit $? ;; esac
exit 0
''')
        self.command("pkill", '''#!/bin/sh
echo "pkill $*" >> "$FAKE/calls"
case "$*" in *lumatv-homestub*) [ "${STUB_WONT_DIE:-0}" = 1 ] || rm -f "$FAKE/native-proc" ;; esac
exit 0
''')
        self.command("systemctl", '#!/bin/sh\n[ "$1" != show ] || echo 42\nexit 0\n')
        self.command("umount", '''#!/usr/bin/env python3
import os, pathlib, sys
p = pathlib.Path(os.environ["FAKE"])
target = sys.argv[-1]
with (p / "calls").open("a") as f: f.write("umount " + target + "\\n")
if target == os.environ.get("FAIL_UMOUNT", ""): sys.exit(1)
if target == str(p / "supervisor"):
    (p / "supervisor-mounted").unlink(missing_ok=True)
    sys.exit(0)
if target == str(p / "home"):
    (p / "home-mounted").unlink(missing_ok=True)
    sys.exit(0)
if target == str(p / "stock-assets"):
    (p / "assets-mounted").unlink(missing_ok=True)
    sys.exit(0)
m = p / "mounts"
lines = m.read_text().splitlines(True)
for i in range(len(lines) - 1, -1, -1):
    if lines[i].split()[1] == target:
        del lines[i]
        break
m.write_text("".join(lines))
''')
        self.command("stat", '''#!/bin/sh
if [ "$1" = -Lc ]; then
  if [ "${STOCK_PROCESS:-0}" = 1 ] && [ "$3" = /proc/42/exe ]; then echo stock-inode; else echo supervisor-inode; fi
  exit 0
fi
case "$3" in
  "$FAKE/home") [ -f "$FAKE/home-mounted" ] && { [ "$(cat "$FAKE/home-mounted")" = native ] && echo native-bind || echo home-bind; exit; } ;;
  "$FAKE/stub") echo home-bind; exit ;;
  "$FAKE/stubn") echo native-bind; exit ;;
  "$FAKE/stock-assets") [ -f "$FAKE/assets-mounted" ] && { echo asset-bind; exit; } ;;
  "$FAKE/assets") echo asset-bind; exit ;;
esac
exec /usr/bin/stat "$@"
''')
        self.command("luna-send", '''#!/bin/sh
printf 'luna %s\\n' "$*" >> "$FAKE/calls"
case "$*" in
  *notificationView*) [ "${FAIL_NOTIFICATION:-0}" = 1 ] && exit 1
    printf '%s' "file://$NOTI_STOCK/NotificationView.qml" > "$NOTI_PERSIST" ;;
esac
case "$*" in
  *"${FALSE_LUNA_URI:-__no_failure__}"*) echo '{"returnValue":false}' ;;
  *) echo '{"returnValue":true}' ;;
esac
exit 0
''')
        real_python = shutil.which("python3")
        self.command("python3", '''#!/bin/sh
case "$1" in
  */keyfilter.py) printf 'keyfilter %%s\\n' "$*" >> "$FAKE/calls"; exit "${FAIL_KEYFILTER:-0}" ;;
  */systemui.py)
    printf 'systemui %%s\\n' "$*" >> "$FAKE/calls"
    if [ "$2" = status ]; then cat "$FAKE/status.json"; exit "${FAIL_STATUS:-0}"; fi
    exit "${FAIL_SYSTEMUI:-0}" ;;
  *) exec "%s" "$@" ;;
esac
''' % real_python)
        source = CLI.read_text()
        selected = "\n".join(function(source, name) for name in
                             ("take_mutation_lock", "hs_stage_dir", "hs_kind_mounted", "hs_mounted_ours", "hs_unmount", "hs_native_stop", "home_running",
                              "stock_ui_unmount_overlay", "stock_ui_luna_ok", "stock_ui_restore_env",
                              "stock_ui_restore", "verb_stock_ui_restore", "clear_stock_running",
                              "verb_compositor", "bd_enabled", "bd_api", "bd_manual_stub"))
        compositor = (ROOT / "tools/compositor.sh").read_text()
        selected += "\n" + "\n".join(function(compositor, name) for name in
                                      ("comp_recovering", "comp_sm_says", "comp_preflight", "comp_switch", "comp_ready"))
        selected = selected.replace("/proc/mounts", '"$FAKE/mounts"')
        dispatch = source[source.rfind('\ncase "$VERB" in'):]
        # The real functions run; platform boundaries are confined to this fixture.
        harness = '''#!/bin/sh
set -u
STATE="$FAKE/state"; COMP="$STATE/compositor"; LOCKFILE="$STATE/mutate.lock"; APPDIR="$FAKE/app"
set -- "${FIXTURE_VERB:-stock-ui-restore}" "${FIXTURE_ARG:-off}"
VERB=$1; MAINTENANCE_ACCESS=read; LOCK_WAIT=3; COMP_LG="$FAKE/supervisor"
AUTOSTART_LINK="$FAKE/autostart/link"; AUTOSTART_SRC="$FAKE/autostart-source"
COMP_LOCK="$FAKE/recovery.lock"; WEBGLASS_STAGE="$STATE/webglass"
SURFACE_SVC=surface-manager-daemon.service
ASSETS="$FAKE/assets"; STOCK_ASSETS="$FAKE/stock-assets"
ENV_FILE="$FAKE/surface.env"; SHADOW_ROOT="$STATE/shadow"
STUB_STAGE="$FAKE/stub"; STUB_NATIVE_STAGE="$FAKE/stubn"; STUB_NATIVE_COMM=lumatv-homestub
HOME_APP_DIR="$FAKE/home"; HOME_APP_ID=com.webos.app.home
KEYFILTER_TOOL="$FAKE/keyfilter.py"; KEYFILTER_HANDLER="$FAKE/handler.js"; KEYFILTER_DIR="$FAKE"; SHORT_ID=lumatv
BD_FLAG="$STATE/boot-direct"; BD_PENDING="$STATE/boot-direct.pending"
SYSTEMUI_TOOL="$FAKE/systemui.py"; SYSTEMUI_STAGE="$STATE/systemui"
QS_APP=quick; LOGO_BOOT_APP=boot; LOGO_OFF_APP=off
NOTI_STAGE="$STATE/notifications"; NOTI_STOCK="$FAKE/stock"; NOTI_PERSIST="$FAKE/notification.json"
NOTI_VIEW_URI=luna://notification/setView
export NOTI_STOCK NOTI_PERSIST
maintenance_guard() { return 0; }
maintenance_present() { return 1; }
comp_mounted() { [ -f "$FAKE/supervisor-mounted" ]; }
hs_wd_stop() { echo watchdog >> "$FAKE/calls"; }
hs_mode_set() { echo "mode $1" >> "$FAKE/calls"; }
webglass_apps() { echo com.webos.app.settings; }
restart_surface() {
  echo "restart ${1:-}" >> "$FAKE/calls"
  [ "${FAIL_RESTART:-0}" != 1 ]
}
recover_home() {
  echo recover >> "$FAKE/calls"
  if [ "${CHECK_HANDOFF_LOCK:-0}" = 1 ]; then
    flock -n "$LOCKFILE" sh -c 'echo recovery-lock-free >> "$FAKE/calls"' || return 1
  fi
  if [ "${FRESH_FALLBACK:-0}" = 1 ] && [ ! -e "$COMP/stock-ui-held" ]; then
    echo hold-cleared >> "$FAKE/calls"
    echo fresh > "$COMP/stock-ui-held"
  fi
  return 0
}
comp_lg_unchanged() { [ "${FAIL_BINARY:-0}" != 1 ]; }
comp_fw_manifest() { :; }
comp_fw_compare() {
  if [ "${FALLBACK_IN_PREFLIGHT:-0}" = 1 ]; then echo fresh > "$FAKE/state/compositor/stock-ui-held"; fi
  if [ "${FAIL_FIRMWARE:-0}" = 1 ]; then echo unverified; else echo "verified fixture"; fi
}
comp_bundle_ok() { COMP_WHY=fixture; [ "${FAIL_BUNDLE:-0}" != 1 ]; }
install_early_hook() { echo hook >> "$FAKE/calls"; }
native_start() { echo native-start >> "$FAKE/calls"; }
native_storage_guard() { STORAGE_WHY="insufficient storage"; [ "${FAIL_STORAGE:-0}" != 1 ]; }
comp_json() { echo '{}'; }
log_line() { :; }
emit_error() { printf '{"ok":false,"error":"%s"}\\n' "$1"; }
''' + selected + dispatch + '\n'
        self.harness = self.tmp / "restore.sh"
        self.harness.write_text(harness)
        self.env = dict(os.environ, FAKE=str(self.tmp), PATH=str(self.bin) + os.pathsep + os.environ["PATH"])

    def command(self, name, text):
        path = self.bin / name
        path.write_text(text)
        path.chmod(0o755)

    def mount(self, target, upper):
        with self.mounts.open("a") as f:
            f.write(f"overlay {target} overlay rw,upperdir={upper},workdir=/work 0 0\n")

    def restore(self, **env):
        return subprocess.run(["sh", str(self.harness)], env=dict(self.env, **env),
                              capture_output=True, text=True, timeout=15)

    def calls_list(self):
        return self.calls.read_text().splitlines()

    def test_removes_only_owned_visible_layers_and_preserves_preferences(self):
        self.mount(INPUT, self.state / "inputcommon/upper")
        self.mount(INPUT, self.state / "inputcommon/upper")
        self.mount(VOICE, "/foreign/upper")
        self.mount(WEB, self.state / "webglass/com.webos.app.settings/upper")
        self.env_file.write_text(f"QML2_IMPORT_PATH={self.state}/shadow\n")
        self.status.write_text(json.dumps({"quick": f"file://{self.state}/systemui/quick.qml",
                                           "boot": "file:///foreign/boot.qml", "off": ""}))
        self.notification.write_text(f"file://{self.state}/notifications/View.qml")
        (self.tmp / "home-mounted").touch()
        (self.tmp / "assets-mounted").touch()
        before = self.prefs.read_bytes()
        result = self.restore()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(json.loads(result.stdout), {"ok": True})
        calls = self.calls_list()
        self.assertEqual(calls.count("umount " + INPUT), 2)
        self.assertIn("umount " + WEB, calls)
        self.assertNotIn("umount " + VOICE, calls)
        self.assertIn("umount " + str(self.tmp / "home"), calls)
        self.assertIn("umount " + str(self.tmp / "stock-assets"), calls)
        self.assertTrue(any("blockSystemApp" in c for c in calls))
        self.assertTrue(any("releaseBlockSystemApp" in c for c in calls))
        self.assertTrue(any("killApp" in c for c in calls))
        self.assertTrue(any("systemui " in c and " off quick" in c for c in calls))
        self.assertFalse(any(" off boot" in c for c in calls))
        self.assertIn("file://" + str(self.tmp / "stock/NotificationView.qml"), self.notification.read_text())
        self.assertEqual(self.env_file.read_text(), "")
        self.assertEqual(self.prefs.read_bytes(), before)
        self.assertTrue((self.comp / "stock-ui-held").exists())
        self.assertIn("/foreign/upper", self.mounts.read_text())

    def test_an_escape_leaves_no_stub_no_unfinished_boot_and_the_flag_on(self):
        # what the escape's recovery finds: the stub mounted over LG's Home, boot direct on, a boot's stub unfinished and one fault on record
        (self.tmp / "home-mounted").touch()
        (self.state / "boot-direct").write_text("on\n")
        (self.state / "boot-direct.pending").write_text("")
        (self.state / "boot-direct.strikes").write_text("1\n")
        for attempt in (1, 2):
            result = self.restore()
            self.assertEqual(result.returncode, 0, "%d: %s" % (attempt, result.stdout + result.stderr))
            self.assertFalse((self.tmp / "home-mounted").exists(), "no stub left under LG's Home")
            self.assertFalse((self.state / "boot-direct.pending").exists(), "no boot left unfinished to be counted against the next one")
            self.assertEqual((self.state / "boot-direct").read_text(), "on\n", "the flag survives: the next boot comes up on LumaTV")
            self.assertEqual((self.state / "boot-direct.strikes").read_text(), "1\n", "the escape is neither a bad boot nor a good one")
            self.assertTrue((self.comp / "stock-ui-held").exists(), "the hold lapses at the next boot, not here")
        calls = self.calls_list()
        self.assertEqual(calls.count("umount " + str(self.tmp / "home")), 1, "pressed twice, the stub is taken out once")
        self.assertIn("mode lg", calls)

    def test_a_native_stub_is_dropped_by_its_identity_and_its_program_is_ended_not_a_page(self):
        # what is over LG's Home directory is the native stub's staged copy: the unmount, SAM reading Flutter again, and then the stub's
        # process, which SAM would otherwise keep for as long as the type changed under it. WebAppMgr has no page of it to kill.
        (self.tmp / "home-mounted").write_text("native")
        (self.tmp / "native-proc").touch()
        result = self.restore()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(json.loads(result.stdout), {"ok": True})
        calls = self.calls_list()
        umount = calls.index("umount " + str(self.tmp / "home"))
        block = next(i for i, c in enumerate(calls) if "blockSystemApp" in c and "releaseBlockSystemApp" not in c)
        release = next(i for i, c in enumerate(calls) if "releaseBlockSystemApp" in c)
        kill = calls.index("pkill -x lumatv-homestub")
        self.assertLess(umount, block)
        self.assertLess(block, release)
        self.assertLess(release, kill, "SAM reads Flutter's manifest before the stub's program is ended")
        self.assertFalse(any("killApp" in c for c in calls))
        self.assertFalse((self.tmp / "native-proc").exists())
        self.assertIn("mode lg", calls)

    def test_a_native_stub_that_will_not_die_is_an_incomplete_restore_and_is_killed_harder(self):
        (self.tmp / "home-mounted").write_text("native")
        (self.tmp / "native-proc").touch()
        result = self.restore(STUB_WONT_DIE="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(json.loads(result.stdout)["ok"])
        self.assertIn("pkill -9 -x lumatv-homestub", self.calls_list())
        self.assertIn("mode lg", self.calls_list(), "and the rest of the restore goes on")

    def test_a_web_stub_is_ended_through_webappmanager_and_no_process_is_signalled(self):
        (self.tmp / "home-mounted").write_text("web")
        result = self.restore()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        calls = self.calls_list()
        self.assertTrue(any("killApp" in c for c in calls))
        self.assertFalse(any(c.startswith("pkill") for c in calls))

    def test_with_no_stub_over_lgs_home_nothing_is_ended_but_a_program_that_outlived_its_mount(self):
        result = self.restore()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        calls = self.calls_list()
        self.assertFalse(any("killApp" in c or c.startswith("pkill") or c.startswith("umount " + str(self.tmp / "home")) for c in calls))
        # a restore that unmounted the stub and could not end its program leaves one: the next restore ends it
        self.calls.write_text("")
        (self.tmp / "native-proc").touch()
        result = self.restore()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("pkill -x lumatv-homestub", self.calls_list())
        self.assertFalse((self.tmp / "native-proc").exists())

    def test_foreign_top_layer_blocks_owned_layer_and_foreign_settings_are_untouched(self):
        self.mount(INPUT, self.state / "inputcommon/upper")
        self.mount(INPUT, "/foreign/upper")
        self.env_file.write_text("LD_LIBRARY_PATH=/foreign/lib\n")
        self.notification.write_text("file:///foreign/NotificationView.qml")
        result = self.restore()
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(json.loads(result.stdout)["ok"])
        self.assertNotIn("umount " + INPUT, self.calls_list())
        self.assertEqual(len(self.mounts.read_text().splitlines()), 2)
        self.assertEqual(self.env_file.read_text(), "LD_LIBRARY_PATH=/foreign/lib\n")
        self.assertEqual(self.notification.read_text(), "file:///foreign/NotificationView.qml")

    def test_partial_failure_is_reported_but_other_cleanup_continues(self):
        self.mount(INPUT, self.state / "inputcommon/upper")
        self.mount(VOICE, self.state / "voice/upper")
        self.status.write_text(json.dumps({"quick": f"file://{self.state}/systemui/quick.qml"}))
        before = self.prefs.read_bytes()
        result = self.restore(FAIL_UMOUNT=INPUT)
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(json.loads(result.stdout)["ok"])
        self.assertIn("umount " + INPUT, self.calls_list())
        self.assertIn("umount " + VOICE, self.calls_list())
        self.assertTrue(any(" off quick" in c for c in self.calls_list()))
        self.assertIn(INPUT, self.mounts.read_text())
        self.assertNotIn(VOICE, self.mounts.read_text())
        self.assertEqual(self.prefs.read_bytes(), before)

    def test_the_restore_takes_the_launcher_repair_off_and_an_unfinished_off_is_incomplete(self):
        tool = self.tmp / "app/tools/sysbus.sh"
        tool.parent.mkdir(parents=True)
        tool.write_text('#!/bin/sh\necho "sysbus $*" >> "$FAKE/calls"\n[ "${FAIL_SYSBUS:-0}" != 1 ]\n')
        tool.chmod(0o755)
        result = self.restore()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("sysbus off", self.calls_list())
        self.calls.write_text("")
        result = self.restore(FAIL_SYSBUS="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(json.loads(result.stdout)["ok"])
        self.assertIn("sysbus off", self.calls_list())
        self.assertIn("mode lg", self.calls_list(), "and the rest of the restore goes on")

    def test_compositor_off_leaves_the_launcher_repair_as_the_owner_set_it(self):
        tool = self.tmp / "app/tools/sysbus.sh"
        tool.parent.mkdir(parents=True)
        tool.write_text('#!/bin/sh\necho "sysbus $*" >> "$FAKE/calls"\n')
        tool.chmod(0o755)
        result = self.restore(FIXTURE_VERB="compositor")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("restart stock", self.calls_list())
        self.assertNotIn("sysbus off", self.calls_list())

    def test_requires_hold_and_unmounted_supervisor_before_cleanup(self):
        for held, mounted in ((False, False), (True, True)):
            with self.subTest(held=held, mounted=mounted):
                hold = self.comp / "stock-ui-held"
                if held: hold.touch()
                else: hold.unlink(missing_ok=True)
                supervisor = self.tmp / "supervisor-mounted"
                if mounted: supervisor.touch()
                else: supervisor.unlink(missing_ok=True)
                result = self.restore()
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(json.loads(result.stdout)["ok"])
                self.assertEqual(self.calls_list(), [])
                self.assertTrue(self.prefs.exists())

    def test_lock_contention_prevents_all_cleanup(self):
        with (self.state / "mutate.lock").open("w") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            result = self.restore()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("lock", json.loads(result.stdout)["error"])
        self.assertEqual(self.calls_list(), [])
        self.assertTrue((self.comp / "stock-ui-held").exists())
        self.assertTrue(self.prefs.exists())




    def test_compositor_off_restores_stock_before_restart_and_preserves_preferences(self):
        (self.comp / "stock-ui-held").unlink()
        (self.comp / "enabled").write_text("persistent\n")
        (self.tmp / "supervisor-mounted").touch()
        self.mount(INPUT, self.state / "inputcommon/upper")
        before = self.prefs.read_bytes()
        result = self.restore(FIXTURE_VERB="compositor")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        calls = self.calls_list()
        self.assertLess(calls.index("umount " + str(self.tmp / "supervisor")), calls.index("umount " + INPUT))
        self.assertLess(calls.index("umount " + INPUT), calls.index("restart stock"))
        self.assertLess(calls.index("restart stock"), calls.index("recover"))
        self.assertTrue((self.comp / "stock-ui-held").exists())
        self.assertFalse((self.comp / "enabled").exists())
        self.assertEqual(self.prefs.read_bytes(), before)

    def test_compositor_off_attempts_handoff_after_restore_failure(self):
        self.mount(INPUT, self.state / "inputcommon/upper")
        result = self.restore(FIXTURE_VERB="compositor", FAIL_UMOUNT=INPUT)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("umount " + INPUT, self.calls_list())
        self.assertIn("restart stock", self.calls_list())
        self.assertIn("recover", self.calls_list())
        self.assertTrue((self.comp / "stock-ui-held").exists())

    def test_running_marker_cleared_only_after_confirmed_stock_restart(self):
        marker = self.comp / "running"
        for stock, failed, cleared in (("1", "0", True), ("0", "0", False), ("1", "1", False)):
            with self.subTest(stock=stock, restart_failed=failed):
                marker.write_text("unsettled\n")
                self.calls.write_text("")
                result = self.restore(FIXTURE_VERB="compositor", STOCK_PROCESS=stock,
                                      FAIL_RESTART=failed, CHECK_HANDOFF_LOCK="1")
                self.assertEqual(result.returncode == 0, cleared, result.stdout + result.stderr)
                self.assertEqual(marker.exists(), not cleared)
                self.assertIn("restart stock", self.calls_list())
                self.assertIn("recover", self.calls_list())
                self.assertIn("recovery-lock-free", self.calls_list())

    def test_foreign_overlay_under_state_is_not_removed(self):
        self.mount(INPUT, self.state / "unrelated/upper")
        result = self.restore()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNotIn("umount " + INPUT, self.calls_list())
        self.assertIn(str(self.state / "unrelated/upper"), self.mounts.read_text())

    def test_mixed_environment_preserves_foreign_entries(self):
        self.env_file.write_text(
            f"QML2_IMPORT_PATH=/foreign/one:{self.state}/shadow:/foreign/two\n"
            "OTHER_SETTING=keep\n"
        )
        result = self.restore()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.env_file.read_text(),
                         'QML2_IMPORT_PATH="/foreign/one:/foreign/two"\nOTHER_SETTING=keep\n')

    def test_luna_false_return_value_reports_incomplete_restore(self):
        for uri in ("blockSystemApp", "notificationView"):
            with self.subTest(uri=uri):
                (self.tmp / "home-mounted").touch()
                self.notification.write_text(f"file://{self.state}/notifications/View.qml")
                self.calls.write_text("")
                result = self.restore(FALSE_LUNA_URI=uri)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(json.loads(result.stdout)["ok"])
                self.assertTrue(any(uri in call for call in self.calls_list()))

    def test_low_storage_refuses_enable_before_hold_or_native_writes(self):
        (self.comp / "bin").mkdir()
        supervisor = self.comp / "bin/lumatv-sm"
        supervisor.write_text(SUPERVISOR)
        supervisor.chmod(0o755)
        hold = self.comp / "stock-ui-held"
        hold.write_text("keep")
        result = self.restore(FIXTURE_VERB="compositor", FIXTURE_ARG="on", FAIL_STORAGE="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("insufficient storage", result.stdout)
        self.assertEqual(hold.read_text(), "keep")
        self.assertNotIn("native-start", self.calls.read_text())

    def test_enable_does_not_clear_a_fresh_fallback_hold(self):
        (self.comp / "bin").mkdir()
        supervisor = self.comp / "bin/lumatv-sm"
        supervisor.write_text(SUPERVISOR)
        supervisor.chmod(0o755)
        before = self.prefs.read_bytes()
        result = self.restore(FIXTURE_VERB="compositor", FIXTURE_ARG="on", FRESH_FALLBACK="1")
        self.assertEqual(result.returncode, 9, result.stdout + result.stderr)
        self.assertEqual((self.comp / "stock-ui-held").read_text(), "fresh\n")
        self.assertIn("recover", self.calls_list())
        self.assertIn("hold-cleared", self.calls_list())
        self.assertEqual(self.prefs.read_bytes(), before)

    def test_fallback_during_preflight_does_not_consume_new_hold(self):
        (self.comp / "bin").mkdir()
        supervisor = self.comp / "bin/lumatv-sm"
        supervisor.write_text(SUPERVISOR)
        supervisor.chmod(0o755)
        (self.comp / "stock-ui-held").write_text("old\n")
        result = self.restore(FIXTURE_VERB="compositor", FIXTURE_ARG="on", FALLBACK_IN_PREFLIGHT="1")
        self.assertEqual(result.returncode, 9, result.stdout + result.stderr)
        self.assertEqual((self.comp / "stock-ui-held").read_text(), "fresh\n")
        self.assertEqual(self.calls_list(), [])

    def test_preflight_refusals_leave_existing_state_and_ui_untouched(self):
        (self.comp / "bin").mkdir()
        supervisor = self.comp / "bin/lumatv-sm"
        supervisor.write_text(SUPERVISOR)
        supervisor.chmod(0o755)
        hold = self.comp / "stock-ui-held"
        enabled = self.comp / "enabled"
        for variable, code in (("FAIL_BINARY", 4), ("FAIL_FIRMWARE", 5),
                               ("FAIL_BUNDLE", 6), ("FAIL_SUPERVISOR", 7)):
            with self.subTest(code=code):
                hold.write_text("existing hold\n")
                enabled.write_text("existing configuration\n")
                self.calls.write_text("")
                result = self.restore(FIXTURE_VERB="compositor", FIXTURE_ARG="on", **{variable: "1"})
                self.assertEqual(result.returncode, code, result.stdout + result.stderr)
                self.assertEqual(hold.read_text(), "existing hold\n")
                self.assertEqual(enabled.read_text(), "existing configuration\n")
                self.assertEqual(self.calls_list(), [])

    def test_older_supervisor_refuses_enable_without_state_changes(self):
        (self.comp / "bin").mkdir()
        supervisor = self.comp / "bin/lumatv-sm"
        supervisor.write_text(SUPERVISOR)
        supervisor.chmod(0o755)
        (self.comp / "supervisor-api").write_text("2\n")
        hold = self.comp / "stock-ui-held"
        hold.write_text("existing hold\n")
        result = self.restore(FIXTURE_VERB="compositor", FIXTURE_ARG="on")
        self.assertEqual(result.returncode, 7, result.stdout + result.stderr)
        self.assertEqual(hold.read_text(), "existing hold\n")
        self.assertEqual(self.calls_list(), [])

    def test_enable_checks_live_recovery_lock_not_stale_file(self):
        (self.comp / "bin").mkdir()
        supervisor = self.comp / "bin/lumatv-sm"
        supervisor.write_text(SUPERVISOR)
        supervisor.chmod(0o755)
        lock_path = self.tmp / "recovery.lock"
        with lock_path.open("w") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            blocked = self.restore(FIXTURE_VERB="compositor", FIXTURE_ARG="on")
        self.assertNotEqual(blocked.returncode, 0)
        self.assertNotIn("mount", self.calls_list())
        self.assertTrue((self.comp / "stock-ui-held").exists())
        self.assertTrue(lock_path.exists())
        allowed = self.restore(FIXTURE_VERB="compositor", FIXTURE_ARG="on")
        self.assertEqual(allowed.returncode, 0, allowed.stdout + allowed.stderr)
        self.assertIn("mount", self.calls_list())
        self.assertFalse((self.comp / "stock-ui-held").exists())

    def test_recovery_handoff_does_not_hold_mutation_lock(self):
        (self.comp / "bin").mkdir()
        supervisor = self.comp / "bin/lumatv-sm"
        supervisor.write_text(SUPERVISOR)
        supervisor.chmod(0o755)
        for action in ("on", "off"):
            with self.subTest(action=action):
                self.calls.write_text("")
                result = self.restore(FIXTURE_VERB="compositor", FIXTURE_ARG=action,
                                      CHECK_HANDOFF_LOCK="1")
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertIn("recover", self.calls_list())
                self.assertIn("recovery-lock-free", self.calls_list())


class RecoverHome(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="recover-home-"))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.bin = self.tmp / "bin"
        self.bin.mkdir()
        self.calls = self.tmp / "calls"
        self.calls.write_text("")
        (self.tmp / "mountinfo").write_text("")
        (self.tmp / "uptime").write_text("0 0\n")
        source = (ROOT / "compositor/tools/tv-recover-home.sh").read_text()
        source = source.replace("/tmp/lumatv-recover.flock", str(self.tmp / "lock"))
        source = source.replace("/tmp/lumatv-recover.png", str(self.tmp / "capture.png"))
        source = source.replace("/proc/self/mountinfo", str(self.tmp / "mountinfo"))
        source = source.replace("/proc/uptime", str(self.tmp / "uptime"))
        source = source.replace("/var/lib/lumatv/compositor/stock-ui-held", str(self.tmp / "stock-ui-held"))
        source = source.replace("/media/developer/apps/usr/palm/applications/org.nphil.lumatv/tools/lumatv",
                                str(self.bin / "restore"))
        self.script = self.tmp / "recover-home.sh"
        self.script.write_text(source)
        self.command("restore", '''#!/bin/sh
echo restore >> "$FAKE/calls"
[ "${EXHAUST_BUDGET:-0}" = 1 ] && echo '200 0' > "$FAKE/uptime"
if [ "${RESTORE_STUCK:-0}" = 1 ]; then
  trap '' TERM
  echo $$ > "$FAKE/restore.pid"
  exec /bin/sleep 30
fi
exit "$RESTORE_RESULT"
''')
        self.command("timeout", '''#!/bin/sh
if [ "${RESTORE_STUCK:-0}" = 1 ] && [ "$1" = -k ] && [ "$4" = "$FAKE/bin/restore" ]; then
  shift 3
  exec /usr/bin/timeout -k 0.2 0.2 "$@"
fi
if [ "$1" = -k ]; then shift 2; fi
shift
exec "$@"
''')
        self.command("flock", '''#!/bin/sh
if [ "$1" = -x ] && [ "$2" = -w ]; then
  [ "${SECOND_RECOVERY:-0}" = 1 ] && : > "$FAKE/second-waiting"
  shift 3
  exec /usr/bin/flock -x -w "${FIXTURE_WAIT:-0.1}" "$@"
fi
exec /usr/bin/flock "$@"
''')
        self.command("sleep", "#!/bin/sh\nexit 0\n")
        self.command("pgrep", "#!/bin/sh\nexit 1\n")
        self.command("python3", "#!/bin/sh\nexit 0\n")
        self.command("systemctl", '''#!/bin/sh
printf 'systemctl %s\\n' "$*" >> "$FAKE/calls"
[ "$1" = restart ] && [ "${FAIL_RESTART:-0}" = 1 ] && exit 1
if [ "$1" = is-active ]; then echo active; fi
''')
        self.command("luna-send", '''#!/bin/sh
printf 'luna %s\\n' "$*" >> "$FAKE/calls"
case "$*" in
  *applicationManager/launch*)
    [ "${FAIL_LAUNCH:-0}" = 1 ] && exit 1
    [ "${FALSE_LAUNCH:-0}" = 1 ] && { echo '{"returnValue":false}'; exit 0; }
    echo '{"returnValue":true}' ;;
  *executeOneShot*)
    if [ "${BLOCK_CAPTURE:-0}" = 1 ]; then
      : > "$FAKE/first-at-capture"
      while [ ! -f "$FAKE/release-first" ]; do /bin/sleep 0.02; done
    fi
    echo '{"writtenBytes":80000}' ;;
  *listRunningApps*)
    if [ "${BLOCK_LIST:-0}" = 1 ]; then
      : > "$FAKE/first-before-home"
      while [ ! -f "$FAKE/release-first" ]; do /bin/sleep 0.02; done
    fi
    echo '{"returnValue":true,"running":[{"id":"com.webos.app.home"}]}' ;;
  *) echo '{"returnValue":true}' ;;
esac
''')
        self.env = dict(os.environ, FAKE=str(self.tmp),
                        PATH=str(self.bin) + os.pathsep + os.environ["PATH"])

    def command(self, name, text):
        path = self.bin / name
        path.write_text(text)
        path.chmod(0o755)

    def test_restore_failure_or_timeout_still_restarts_webappmgr_and_launches_home(self):
        for status in ("1", "124"):
            with self.subTest(status=status):
                self.calls.write_text("")
                result = subprocess.run(["sh", str(self.script), "--stock"],
                                        env=dict(self.env, RESTORE_RESULT=status),
                                        capture_output=True, text=True, timeout=15)
                self.assertNotEqual(result.returncode, 0)
                calls = self.calls.read_text().splitlines()
                restore = calls.index("restore")
                restart = calls.index("systemctl restart webapp-mgr.service")
                launch = next(i for i, call in enumerate(calls)
                              if "applicationManager/launch" in call and "com.webos.app.home" in call)
                self.assertLess(restore, restart)
                self.assertLess(restart, launch)
                self.assertTrue((self.tmp / "lock").is_file())

    def test_timeout_kills_restore_that_ignores_term_and_continues_home_handoff(self):
        result = subprocess.run(["sh", str(self.script), "--stock"],
                                env=dict(self.env, RESTORE_RESULT="0", RESTORE_STUCK="1"),
                                capture_output=True, text=True, timeout=10)
        self.assertNotEqual(result.returncode, 0)
        pid = int((self.tmp / "restore.pid").read_text())
        stat = Path(f"/proc/{pid}/stat")
        self.assertTrue(not stat.exists() or stat.read_text().rsplit(")", 1)[1].split()[0] == "Z")
        self.assertIn("applicationManager/launch", self.calls.read_text())

    def test_live_lock_owner_prevents_duplicate_recovery(self):
        lock_path = self.tmp / "lock"
        with lock_path.open("w") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            result = subprocess.run(["sh", str(self.script), "--stock"],
                                    env=dict(self.env, RESTORE_RESULT="1"),
                                    capture_output=True, text=True, timeout=15)
        self.assertNotEqual(result.returncode, 0)
        self.assertTrue(lock_path.is_file())
        self.assertEqual(self.calls.read_text(), "")

    def test_stock_request_waits_for_ordinary_handoff_then_restores(self):
        def wait_for(path):
            deadline = time.monotonic() + 5
            while not path.exists() and time.monotonic() < deadline:
                time.sleep(0.01)
            self.assertTrue(path.exists(), f"timed out waiting for {path.name}")

        first = subprocess.Popen(["sh", str(self.script)],
                                 env=dict(self.env, BLOCK_CAPTURE="1"),
                                 stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        second = None
        try:
            wait_for(self.tmp / "first-at-capture")
            (self.tmp / "stock-ui-held").write_text("new-generation")
            second = subprocess.Popen(["sh", str(self.script), "--stock"],
                                      env=dict(self.env, RESTORE_RESULT="0", SECOND_RECOVERY="1", FIXTURE_WAIT="5"),
                                      stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            wait_for(self.tmp / "second-waiting")
            self.assertIsNone(second.poll())
            before = self.calls.read_text().splitlines()
            self.assertNotIn("restore", before)
            self.assertEqual(sum("applicationManager/launch" in c for c in before), 1)
            (self.tmp / "release-first").touch()
            out1, err1 = first.communicate(timeout=10)
            out2, err2 = second.communicate(timeout=10)
            self.assertEqual(first.returncode, 0, out1 + err1)
            self.assertEqual(second.returncode, 0, out2 + err2)
            calls = self.calls.read_text().splitlines()
            launches = [i for i, c in enumerate(calls) if "applicationManager/launch" in c]
            self.assertEqual(len(launches), 2)
            self.assertLess(launches[0], calls.index("restore"))
            self.assertLess(calls.index("restore"), launches[1])
            restarts = [i for i, c in enumerate(calls) if c == "systemctl restart webapp-mgr.service"]
            self.assertEqual(len(restarts), 2)
            self.assertLess(calls.index("restore"), restarts[1])
            self.assertLess(restarts[1], launches[1])
        finally:
            (self.tmp / "release-first").touch()
            for proc in (first, second):
                if proc is not None and proc.poll() is None:
                    proc.kill()
                    proc.communicate()

    def assert_stock_hold_interrupts_ordinary_before_home_launch(self):
        def wait_for(path):
            deadline = time.monotonic() + 5
            while not path.exists() and time.monotonic() < deadline:
                time.sleep(0.01)
            self.assertTrue(path.exists(), f"timed out waiting for {path.name}")

        first = subprocess.Popen(["sh", str(self.script)], env=dict(self.env, BLOCK_LIST="1"),
                                 stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        second = None
        try:
            wait_for(self.tmp / "first-before-home")
            (self.tmp / "stock-ui-held").write_text("new-generation")
            second = subprocess.Popen(["sh", str(self.script), "--stock"],
                                      env=dict(self.env, RESTORE_RESULT="0", SECOND_RECOVERY="1", FIXTURE_WAIT="5"),
                                      stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            wait_for(self.tmp / "second-waiting")
            self.assertNotIn("applicationManager/launch", self.calls.read_text())
            self.assertNotIn("restore", self.calls.read_text().splitlines())
            (self.tmp / "release-first").touch()
            first.communicate(timeout=10)
            out2, err2 = second.communicate(timeout=10)
            self.assertEqual(second.returncode, 0, out2 + err2)
            calls = self.calls.read_text().splitlines()
            launches = [i for i, c in enumerate(calls) if "applicationManager/launch" in c]
            self.assertEqual(len(launches), 1)
            self.assertLess(calls.index("restore"), launches[0])
        finally:
            (self.tmp / "release-first").touch()
            for proc in (first, second):
                if proc is not None and proc.poll() is None:
                    proc.kill()
                    proc.communicate()

    def test_new_stock_hold_interrupts_ordinary_before_home_launch(self):
        self.assert_stock_hold_interrupts_ordinary_before_home_launch()

    def test_existing_hold_selects_stock_restore_without_explicit_flag(self):
        (self.tmp / "stock-ui-held").write_text("old-generation")
        result = subprocess.run(["sh", str(self.script)], env=dict(self.env, RESTORE_RESULT="0"),
                                capture_output=True, text=True, timeout=15)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        calls = self.calls.read_text().splitlines()
        restore = calls.index("restore")
        restart = calls.index("systemctl restart webapp-mgr.service")
        launch = next(i for i, c in enumerate(calls) if "applicationManager/launch" in c)
        self.assertLess(restore, restart)
        self.assertLess(restart, launch)

    def test_stale_lock_file_does_not_prevent_home_recovery(self):
        lock_path = self.tmp / "lock"
        lock_path.write_text("old owner\n")
        result = subprocess.run(["sh", str(self.script), "--stock"],
                                env=dict(self.env, RESTORE_RESULT="1"),
                                capture_output=True, text=True, timeout=15)
        self.assertNotEqual(result.returncode, 0)
        self.assertTrue(lock_path.is_file())
        self.assertIn("systemctl restart webapp-mgr.service", self.calls.read_text())
        self.assertIn("applicationManager/launch", self.calls.read_text())


    def test_failed_restart_still_attempts_home_and_reports_failure(self):
        result = subprocess.run(["sh", str(self.script), "--stock"],
                                env=dict(self.env, RESTORE_RESULT="0", FAIL_RESTART="1"),
                                capture_output=True, text=True, timeout=15)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("applicationManager/launch", self.calls.read_text())

    def test_home_launch_failure_or_false_reply_reports_failure(self):
        for failure in ("FAIL_LAUNCH", "FALSE_LAUNCH"):
            with self.subTest(failure=failure):
                self.calls.write_text("")
                result = subprocess.run(["sh", str(self.script), "--stock"],
                                        env=dict(self.env, RESTORE_RESULT="0", **{failure: "1"}),
                                        capture_output=True, text=True, timeout=15)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("applicationManager/launch", self.calls.read_text())

    def assert_new_generation_during_stock_recovery(self, exhaust_budget):
        hold = self.tmp / "stock-ui-held"
        hold.write_text("generation-a")
        proc = subprocess.Popen(["sh", str(self.script), "--stock"],
                                env=dict(self.env, RESTORE_RESULT="0", BLOCK_LIST="1"),
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        try:
            marker = self.tmp / "first-before-home"
            deadline = time.monotonic() + 5
            while not marker.exists() and time.monotonic() < deadline:
                time.sleep(0.01)
            self.assertTrue(marker.exists(), "stock recovery did not reach the Home readiness check")
            self.assertEqual(self.calls.read_text().splitlines().count("restore"), 1)
            hold.write_text("generation-b")
            if exhaust_budget:
                (self.tmp / "uptime").write_text("200 0\n")
            (self.tmp / "release-first").touch()
            out, err = proc.communicate(timeout=10)
            calls = self.calls.read_text().splitlines()
            restores = [i for i, c in enumerate(calls) if c == "restore"]
            launches = [i for i, c in enumerate(calls) if "applicationManager/launch" in c]
            self.assertTrue(launches, out + err)
            if exhaust_budget:
                self.assertNotEqual(proc.returncode, 0)
                self.assertEqual(len(restores), 1)
            else:
                self.assertEqual(proc.returncode, 0, out + err)
                self.assertEqual(len(restores), 2)
                restarts = [i for i, c in enumerate(calls) if c == "systemctl restart webapp-mgr.service"]
                self.assertGreaterEqual(len(restarts), 2)
                self.assertLess(restores[-1], restarts[-1])
                self.assertLess(restarts[-1], launches[-1])
        finally:
            (self.tmp / "release-first").touch()
            if proc.poll() is None:
                proc.kill()
                proc.communicate()

    def test_fresh_generation_restarts_stock_recovery(self):
        self.assert_new_generation_during_stock_recovery(False)

    def test_fresh_generation_with_no_budget_still_attempts_home_and_fails(self):
        self.assert_new_generation_during_stock_recovery(True)

    def test_exhausted_work_budget_still_attempts_home(self):
        result = subprocess.run(["sh", str(self.script), "--stock"],
                                env=dict(self.env, RESTORE_RESULT="0", EXHAUST_BUDGET="1"),
                                capture_output=True, text=True, timeout=15)
        self.assertNotEqual(result.returncode, 0)
        calls = self.calls.read_text().splitlines()
        self.assertIn("restore", calls)
        self.assertTrue(any("applicationManager/launch" in c for c in calls))


if __name__ == "__main__":
    unittest.main()
