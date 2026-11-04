"""Native startup staging and removal of retired key handlers."""
import os
import io
import tarfile
from pathlib import Path
import re
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
CLI = ROOT / 'tools' / 'lumatv'


def shell_function(name):
    source = CLI.read_text()
    match = re.search(r'^' + re.escape(name) + r'\(\) \{.*?^\}', source, re.M | re.S)
    if match is None:
        raise AssertionError('missing shell function ' + name)
    return match.group()


class NativeStartup(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.calls = self.root / 'calls'

    def run_script(self, script, **variables):
        env = dict(os.environ, **{k: str(v) for k, v in variables.items()})
        result = subprocess.run(['sh', '-eu', '-c', script], env=env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_app_storage_measures_uncompressed_contents_on_both_filesystems(self):
        data = self.root / 'data.tar.gz'
        with tarfile.open(data, 'w:gz') as archive:
            info = tarfile.TarInfo('large')
            info.size = 2 * 1024 * 1024
            archive.addfile(info, io.BytesIO(b'\0' * info.size))
        ipk = self.root / 'app.ipk'
        subprocess.run(['ar', 'rc', str(ipk), str(data)], check=True)
        script = '''
storage_guard() { printf '%s %s %s\n' "$1" "$STORAGE_EXTRA_KIB" "$STORAGE_EXTRA_INODES" >> "$CALLS"; }
''' + shell_function('app_storage_guard') + '\napp_storage_guard "$IPK" "$DEST"\n'
        self.run_script(script, CALLS=self.calls, IPK=ipk, DEST=self.root / 'apps')
        rows = self.calls.read_text().splitlines()
        self.assertEqual(rows, [f'{self.root}/apps 2052 1', '/tmp 2052 1'])
        self.assertLess(ipk.stat().st_size, 2052 * 1024)
    def test_boot_defers_sibling_updates_without_blocking_native_start(self):
        app = self.root / 'gfn.ipk'
        app.touch()
        app_dir = self.root / 'gfn'
        app_dir.mkdir()
        script = '''
take_mutation_lock() { :; }
comp_enabled() { return 0; }
native_storage_guard() { :; }
panel_is_active_standby() { return 1; }
comp_boot_mount() { echo mounted; }
native_start() { echo native >> "$CALLS"; }
install_frame_app() { echo frame >> "$CALLS"; }
gfn_current() { return 1; }
gfn_running() { return 1; }
gfn_update() { echo gfn >> "$CALLS"; }
feeder_installed() { return 0; }
feeder_update() { echo feeder >> "$CALLS"; }
app_storage_guard() { STORAGE_WHY=low; [ "$ALLOW" = yes ]; }
storage_guard() { STORAGE_WHY=low; [ "$ALLOW" = yes ]; }
log_line() { :; }
''' + shell_function('verb_compositor_boot') + '\nverb_compositor_boot\n'
        for allow, expected in (('no', ['native', 'frame']), ('yes', ['native', 'frame', 'gfn', 'feeder'])):
            self.calls.unlink(missing_ok=True)
            self.run_script(script, CALLS=self.calls, ALLOW=allow, GFN_IPK=app,
                            GFN_DIR=app_dir, STATE=self.root, FEEDER_SRC=self.root / 'feeder')
            self.assertEqual(self.calls.read_text().splitlines(), expected)



    def test_standby_requires_exact_successful_power_reply(self):
        fake = self.root / 'bin'
        fake.mkdir()
        sender = fake / 'luna-send'
        sender.write_text('#!/bin/sh\nprintf "%s" "$REPLY"\n')
        sender.chmod(0o755)
        script = shell_function('panel_is_active_standby') + '\npanel_is_active_standby\n'
        replies = (
            ('{"returnValue":true,"state":"Active Standby"}', 0),
            ('{"returnValue":true,"state":"Active"}', 1),
            ('{"returnValue":false,"state":"Active Standby"}', 1),
            ('{"state":"Active Standby"}', 1),
            ('{"returnValue":true,"state":"Unknown"}', 1),
            ('not json', 1),
        )
        for reply, expected in replies:
            with self.subTest(reply=reply):
                env = dict(os.environ, REPLY=reply, PATH=str(fake) + os.pathsep + os.environ['PATH'])
                result = subprocess.run(['sh', '-c', script], env=env, capture_output=True, timeout=8)
                self.assertEqual(result.returncode, expected)

    def test_standby_restart_rechecks_before_systemctl(self):
        script = '''
comp_recovering() { return 1; }
panel_is_active_standby() { [ ! -e "$CHANGED" ]; }
close_input_apps() { :; }
wam_stamp() { :; }
snapshot_reports() { touch "$CHANGED"; }
systemctl() { echo restart >> "$CALLS"; }
log_line() { :; }
''' + shell_function('restart_surface') + '''
restart_surface native standby
'''
        env = dict(os.environ, CHANGED=str(self.root / 'changed'), CALLS=str(self.calls),
                   COMP=str(self.root), SURFACE_SVC='fake')
        result = subprocess.run(['sh', '-c', script], env=env, capture_output=True, timeout=8)
        self.assertEqual(result.returncode, 2)
        self.assertFalse(self.calls.exists())

    def test_boot_worker_activates_mounted_compositor_and_preserves_intent(self):
        tools = self.root / 'tools'
        tools.mkdir()
        state = self.root / 'state'
        (state / 'compositor').mkdir(parents=True)
        (state / 'compositor/enabled').touch()
        worker_id = 'boot-test-' + str(os.getpid()) + '-' + self.root.name
        marker = Path('/tmp/.' + worker_id + '-worker')
        self.addCleanup(marker.unlink, missing_ok=True)
        (tools / 'autostart.sh').write_text((ROOT / 'tools/autostart.sh').read_text())
        (tools / 'brand.sh').write_text('brand_load() { BRAND_DATA_DIR="$FAKE/state"; BRAND_SHORT_ID="$WORKER_ID"; }\n')
        cli = tools / worker_id
        cli.write_text('''#!/bin/sh
COMP="$FAKE/state/compositor"
take_mutation_lock() { exec 9>"$FAKE/lock"; flock -x 9; [ ! -e "$COMP/stock-ui-held" ]; }
comp_enabled() { [ -f "$COMP/enabled" ]; }
comp_mounted() { [ "${MOUNTED:-yes}" = yes ]; }
comp_recovering() { [ "${RECOVERING:-no}" = yes ]; }
comp_live() { [ "${LIVE:-no}" = yes ]; }
comp_ready() { echo ready >> "$CALLS"; [ "${READY:-yes}" = yes ] && { [ "${INITIAL_NOT_READY:-no}" = no ] || [ -e "$FAKE/restarted" ]; }; }
panel_is_active() { [ "${PANEL:-active}" = active ]; }
panel_is_active_standby() { [ "${PANEL:-active}" = standby ] && [ "${STANDBY_CHANGED:-no}" = no ] && { [ "${WAKE_ON_FAILURE:-no}" = no ] || [ ! -e "$FAKE/restarted" ]; }; }
boot_landing_app() { echo landing >> "$CALLS"; if [ "${PREPARE_FAIL:-no}" = yes ]; then echo __wake_skip__; else echo '{"appId":"netflix","params":{}}'; fi; }
land_on_app() { echo "land $2 $1" >> "$CALLS"; }
restart_surface() {
  flock -n "$FAKE/lock" -c ':' || { echo locked >> "$CALLS"; return 1; }
  if [ "${2:-}" = standby ] && ! panel_is_active_standby; then echo "blocked $1" >> "$CALLS"; return 2; fi
  echo "restart $1" >> "$CALLS"
  [ "$1" != native ] || touch "$FAKE/restarted"
  [ "$1" = stock ] || [ "${RESTART_FAIL:-no}" = no ]
}
comp_switch() { echo "switch $1" >> "$CALLS"; [ "$1" != off ] || rm -f "$COMP/enabled"; }
clear_stock_running() { echo clear >> "$CALLS"; }
emit_error() { echo "error $*"; }
log_line() { :; }
''' + shell_function('verb_compositor_boot_activate') + '''
case "$*" in
  compositor-boot)
    echo mount >> "$CALLS"
    standby=${BOOT_STANDBY:-false}
    # The set was in standby at the first look and switched on by the next one.
    [ "${BOOT_STANDBY_ONCE:-no}" = no ] || [ ! -e "$FAKE/looked" ] || standby=false
    touch "$FAKE/looked"
    result=${MOUNT_RESULT:-mounted}
    [ -f "$COMP/enabled" ] || result=off
    printf '{"ok":true,"compositor":"%s","standby":%s}\n' "$result" "$standby" ;;
  'compositor-boot activate') verb_compositor_boot_activate ;;
  'compositor-boot activate standby') verb_compositor_boot_activate standby ;;
  'wake boot-check prepare '*) echo prepare >> "$CALLS"; [ "${PREPARE_FAIL:-no}" = no ] ;;
  'wake boot-check finalize '*) echo finalize >> "$CALLS"; [ "${FINALIZE_FAIL:-no}" = no ] ;;
esac
''')
        cli.chmod(0o755)
        landed = 'land restart {"appId":"netflix","params":{}}'
        scenarios = (
            ({}, ['mount', 'prepare', 'landing', 'restart native', 'ready', landed, 'finalize'], True),
            ({'PANEL': 'standby'}, ['mount', 'prepare', 'restart native', 'ready', 'finalize'], True),
            ({'PANEL': 'standby', 'BOOT_STANDBY': 'true', 'PREPARE_FAIL': 'yes'}, ['mount', 'restart native', 'ready'], True),
            # Switched on between compositor-boot's look at the panel and the standby activation: the next pass is an active boot.
            ({'PANEL': 'active', 'BOOT_STANDBY': 'true', 'BOOT_STANDBY_ONCE': 'yes'},
             ['mount', 'mount', 'prepare', 'landing', 'restart native', 'ready', landed, 'finalize'], True),
            ({'PANEL': 'active', 'BOOT_STANDBY': 'true', 'BOOT_STANDBY_ONCE': 'yes', 'LIVE': 'yes'},
             ['mount', 'ready', 'mount', 'prepare', 'ready', 'finalize'], True),
            # A standby that cannot be activated twice is left alone.
            ({'PANEL': 'active', 'BOOT_STANDBY': 'true'}, ['mount', 'mount'], False),
            ({'PANEL': 'active', 'BOOT_STANDBY': 'true', 'LIVE': 'yes'}, ['mount', 'ready', 'mount', 'ready'], False),
            # A failed standby activation that fell back to LG's compositor leaves nothing for the second pass to do.
            ({'PANEL': 'standby', 'BOOT_STANDBY': 'true', 'RESTART_FAIL': 'yes', 'WAKE_ON_FAILURE': 'yes'},
             ['mount', 'restart native', 'switch off', 'blocked stock', 'mount'], True),
            # The wake preparation only picks the landing: its failure never holds the compositor back.
            ({'PANEL': 'unknown', 'PREPARE_FAIL': 'yes'}, ['mount', 'prepare', 'restart native', 'ready'], True),
            ({'PREPARE_FAIL': 'yes'}, ['mount', 'prepare', 'landing', 'restart native', 'ready'], True),
            ({'PREPARE_FAIL': 'yes', 'LIVE': 'yes'}, ['mount', 'prepare', 'ready'], True),
            ({'PREPARE_FAIL': 'yes', 'RESTART_FAIL': 'yes'},
             ['mount', 'prepare', 'landing', 'restart native', 'switch off', 'restart stock', 'clear'], False),
            ({'LIVE': 'yes'}, ['mount', 'prepare', 'ready', 'finalize'], True),
            ({'LIVE': 'yes', 'INITIAL_NOT_READY': 'yes'}, ['mount', 'prepare', 'ready', 'landing', 'restart native', 'ready', landed, 'finalize'], True),
            ({'MOUNT_RESULT': 'already'}, ['mount', 'prepare', 'landing', 'restart native', 'ready', landed, 'finalize'], True),
            ({'MOUNT_RESULT': 'held'}, ['mount'], True),
            ({'MOUNT_RESULT': 'error'}, ['mount'], False),
            ({'MOUNTED': 'no'}, ['mount', 'prepare'], False),
            ({'RECOVERING': 'yes'}, ['mount', 'prepare'], False),
            ({'RESTART_FAIL': 'yes'}, ['mount', 'prepare', 'landing', 'restart native', 'switch off', 'restart stock', 'clear'], False),
            ({'READY': 'no'}, ['mount', 'prepare', 'landing', 'restart native', 'ready', 'switch off', 'restart stock', 'clear'], False),
            ({'FINALIZE_FAIL': 'yes'}, ['mount', 'prepare', 'landing', 'restart native', 'ready', landed, 'finalize'], False),
        )
        for variables, expected, done in scenarios:
            with self.subTest(variables=variables):
                self.calls.unlink(missing_ok=True)
                marker.unlink(missing_ok=True)
                (self.root / 'restarted').unlink(missing_ok=True)
                (self.root / 'looked').unlink(missing_ok=True)
                (state / 'compositor/enabled').touch()
                env = dict(os.environ, FAKE=str(self.root), CALLS=str(self.calls), WORKER_ID=worker_id, **variables)
                result = subprocess.run(['sh', str(tools / 'autostart.sh'), '--worker'], env=env,
                                        capture_output=True, text=True, timeout=15)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(self.calls.read_text().splitlines(), expected)
                self.assertEqual(marker.exists(), done)
                if done:
                    self.assertEqual(marker.read_text().strip(), 'done')

    def test_native_start_stages_runtime_without_touching_sibling_apps(self):
        script = '''
log_line() { printf "%s\n" "$*" >> "$CALLS"; }
stage_theme() { echo theme >> "$CALLS"; }
helper_start() { echo helper >> "$CALLS"; }
capture_start() { echo capture >> "$CALLS"; }
vplane_installed() { return 1; }
install_frame_app() { echo frame >> "$CALLS"; }
gfn_update() { echo gfn >> "$CALLS"; }
feeder_installed() { return 0; }
feeder_update() { echo feeder >> "$CALLS"; }
''' + shell_function('native_start') + '\nnative_start\n'
        self.run_script(script, CALLS=self.calls, PRELOAD_STATE=self.root / 'no-preload')
        self.assertEqual(self.calls.read_text().splitlines(), ['theme', 'helper', 'capture'])

    def test_migration_removes_both_handlers_without_adding_one(self):
        state = self.root / 'state'
        state.mkdir()
        (state / '.migrating').touch()
        script = '''
log_line() { :; }
log_stderr() { printf '%s\\n' "$*" >> "$CALLS"; }
rewrite_legacy_paths() { :; }
cfg_get_bool() { echo false; }
remove_legacy_app() { :; }
''' + shell_function('migrate_legacy') + '\nmigrate_legacy\n'
        self.run_script(script, CALLS=self.calls, STATE=state, MIGRATING=state / '.migrating',
                        LEGACY_STATE=self.root / 'old', LUMA_STAGE=state / 'old-stage',
                        KEYFILTER_DIR=state / 'keyfilter', LEGACY_KEYFILTER='legacy.js', VERIFIED=state / 'verified',
                        WEBGLASS_STAGE=state / 'webglass', LEGACY_CSS='legacy.css', ENV_FILE=state / 'env',
                        EARLY_ENV_FILE=state / 'early-env', EARLY_SHIM=state / 'shim',
                        LEGACY_AUTOSTART_LINK=state / 'autostart', KEYFILTER_TOOL='keyfilter.py',
                        KEYFILTER_HANDLER='/old/handler.js', SHORT_ID='lumatv',
                        LEGACY_FRAME_APP='old.frame', LEGACY_APP='old.app')
        self.assertEqual(self.calls.read_text().splitlines(), [
            'python3 keyfilter.py remove /old/handler.js %s' % (state / 'keyfilter' / 'lumatv.js')])
        self.assertFalse((state / '.migrating').exists())

    def test_migration_restores_only_owned_legacy_ui_and_retries_failures(self):
        for resumed, redirect, failure in (
            (False, 'legacy', ''), (True, 'current', ''),
            (False, 'foreign', ''), (False, 'legacy', 'unmount'),
            (False, 'legacy', 'status'), (False, 'legacy', 'off'),
        ):
            with self.subTest(resumed=resumed, redirect=redirect, failure=failure):
                with tempfile.TemporaryDirectory() as d:
                    root = Path(d)
                    old, state = root / 'old', root / 'state'
                    source = state if resumed else old
                    source.mkdir()
                    (source / '.webglass-on').touch()
                    (source / '.systemui-on').touch()
                    if resumed:
                        (source / '.migrating').touch()
                    css = source / 'webapps' / 'settings' / 'upper' / 'webOSUserScripts' / 'legacy.css'
                    css.parent.mkdir(parents=True)
                    css.write_text('legacy')
                    foreign = css.parent / 'foreign.css'
                    foreign.write_text('keep')
                    redirect_root = {'legacy': old / 'systemui', 'current': state / 'systemui',
                                     'foreign': root / 'foreign'}[redirect]
                    script = '''
log_line() { :; }
log_stderr() { :; }
rewrite_legacy_paths() { :; }
cfg_get_bool() { echo false; }
remove_legacy_app() { :; }
webglass_apps() { echo settings; }
stock_ui_unmount_overlay() {
  printf 'unmount %s %s\n' "$1" "$2" >> "$CALLS"
  [ "$FAILURE" != unmount ]
}
timeout() {
  case "$4" in
    status) [ "$FAILURE" != status ] || return 1; printf '{"qs":"file://%s/QuickSettings.qml"}\n' "$REDIRECT" ;;
    off) echo off >> "$CALLS"; [ "$FAILURE" != off ] ;;
  esac
}
''' + shell_function('migrate_legacy') + '''
if migrate_legacy; then echo ok > "$RESULT"; else echo failed > "$RESULT"; fi
'''
                    calls, result = root / 'calls', root / 'result'
                    self.run_script(script, CALLS=calls, RESULT=result, FAILURE=failure,
                                    REDIRECT=redirect_root, STATE=state, MIGRATING=state / '.migrating',
                                    LEGACY_STATE=old, LOCK_WAIT='1', SYSTEMUI_TOOL='systemui.py', QS_APP='qs',
                                    LUMA_STAGE=state / 'old-stage', KEYFILTER_DIR=state / 'keyfilter',
                                    LEGACY_KEYFILTER='legacy.js', VERIFIED=state / 'verified',
                                    WEBGLASS_STAGE=state / 'webapps', LEGACY_CSS='legacy.css',
                                    ENV_FILE=state / 'env', EARLY_ENV_FILE=state / 'early-env',
                                    EARLY_SHIM=state / 'shim', LEGACY_AUTOSTART_LINK=state / 'autostart',
                                    KEYFILTER_TOOL='keyfilter.py', KEYFILTER_HANDLER='/old/handler.js',
                                    SHORT_ID='lumatv', LEGACY_FRAME_APP='old.frame', LEGACY_APP='old.app')
                    self.assertEqual(result.read_text().strip(), 'failed' if failure else 'ok')
                    entries = calls.read_text().splitlines()
                    self.assertEqual(entries[0], f'unmount /usr/palm/applications/settings {source}/webapps/settings/upper')
                    self.assertEqual('off' in entries, redirect != 'foreign' and failure not in ('unmount', 'status'))
                    remaining = source if failure else state
                    self.assertEqual((remaining / '.migrating').exists(), bool(failure))
                    self.assertEqual((remaining / 'webapps/settings/upper/webOSUserScripts/legacy.css').exists(), bool(failure))
                    self.assertEqual((remaining / 'webapps/settings/upper/webOSUserScripts/foreign.css').read_text(), 'keep')
