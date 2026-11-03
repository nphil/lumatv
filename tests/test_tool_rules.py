"""Run with: python3 -m unittest tests.test_tool_rules

Rules the tool keeps on the set, checked in its source and, where one is cheap to run, in its
behaviour. The first: SAM (`sam.service`, the application manager) is never restarted, stopped or
killed by anything this package ships. It is a set rule: eARC, CEC and the picture mode are at risk
when it goes, and the one place that did it (the tile-icon revert) is gone.
"""
import os
import re
import shutil
import subprocess
import tempfile
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
TOOLS = os.path.join(ROOT, "tools")

# A line that stops or restarts a unit called sam, kills a process called sam, or says `restart sam`.
SAM = re.compile(
    r"systemctl\s+(?:--\S+\s+)*(?:restart|try-restart|reload-or-restart|stop|kill|reload)\s+[^\n;&|]*\bsam(?:\.service)?\b"
    r"|\b(?:pkill|killall)\b[^\n]*\bsam\b"
    r"|\bservice\s+sam\b"
    r"|\brestart\s+sam\b",
    re.IGNORECASE)


def shell_tools():
    for name in sorted(os.listdir(TOOLS)):
        p = os.path.join(TOOLS, name)
        if not os.path.isfile(p):
            continue
        with open(p, encoding="utf-8", errors="replace") as f:
            first = f.readline()
        if name.endswith(".sh") or first.startswith("#!/bin/sh") or first.startswith("#!/bin/bash"):
            yield name, p


def code_lines(path):
    with open(path, encoding="utf-8", errors="replace") as f:
        for n, line in enumerate(f, 1):
            if not line.lstrip().startswith("#"):
                yield n, line


class SamIsNeverRestarted(unittest.TestCase):
    def test_no_shell_tool_restarts_stops_or_kills_sam(self):
        found = []
        count = 0
        for name, path in shell_tools():
            count += 1
            for n, line in code_lines(path):
                if SAM.search(line):
                    found.append("%s:%d: %s" % (name, n, line.strip()))
        self.assertGreater(count, 3, "the scan found no tools")
        self.assertEqual(found, [])

    def test_the_pattern_does_catch_what_it_is_for(self):
        for bad in ("systemctl restart sam.service >/dev/null 2>&1", "systemctl restart sam", "  systemctl --no-block stop sam.service",
                    "pkill -f sam", "service sam restart", "restart sam", "systemctl kill sam.service"):
            self.assertTrue(SAM.search(bad), bad)
        for fine in ("systemctl restart surface-manager-daemon.service", "systemctl restart webapp-mgr", "luna-send blockSystemApp",
                     "systemctl is-active sam.service", "# restart sam would", "log_line \"same\"", "systemctl restart samba"):
            self.assertFalse(SAM.search(fine) and not fine.lstrip().startswith("#"), fine)

    def test_the_tile_icon_revert_restarts_nothing(self):
        with open(os.path.join(TOOLS, "lumatv"), encoding="utf-8") as f:
            text = f.read()
        m = re.search(r"^revert_tile_icons\(\) \{\n.*?^\}\n", text, re.S | re.M)
        self.assertIsNotNone(m, "revert_tile_icons is gone")
        with tempfile.TemporaryDirectory() as d:
            calls = os.path.join(d, "calls")
            bindir = os.path.join(d, "bin")
            os.mkdir(bindir)
            for name in ("systemctl", "pkill", "killall", "service"):
                with open(os.path.join(bindir, name), "w") as f:
                    f.write('#!/bin/sh\necho "%s $*" >> "%s"\n' % (name, calls))
                os.chmod(os.path.join(bindir, name), 0o755)
            node = os.path.join(d, "node")
            with open(node, "w") as f:
                f.write('#!/bin/sh\necho \'{"ok":true,"restored":3}\'\n')
            os.chmod(node, 0o755)
            tile = os.path.join(d, "tileicons.js")
            open(tile, "w").close()
            script = os.path.join(d, "run.sh")
            with open(script, "w") as f:
                f.write('NODE="%s"\nTILEICONS="%s"\nSTATE="%s"\nLOG_TOOL="%s/logrelay.py"\nREVERT_CHANGED=0\n' % (node, tile, d, TOOLS))
                f.write('log_line() { echo "$1" >> "%s/loglines"; }\n' % d)
                f.write(function("log_stderr"))
                f.write(m.group(0))
                f.write('revert_tile_icons\necho "rc=$? changed=$REVERT_CHANGED"\n')
            r = subprocess.run(["sh", script], env=dict(os.environ, PATH=bindir + ":" + os.environ["PATH"]), capture_output=True, text=True, timeout=30)
            self.assertEqual(r.returncode, 0, r.stderr)
            self.assertIn("rc=0 changed=0", r.stdout)       # restoring files is not a reason to restart the compositor either
            self.assertFalse(os.path.exists(calls), "the revert called a service tool: " + (open(calls).read() if os.path.exists(calls) else ""))
            with open(os.path.join(d, "loglines")) as f:
                self.assertIn("restored 3 file(s)", f.read())


def function(name):
    """One shell function of tools/lumatv, from its `name() {` line to the closing brace in column 0."""
    out, on = [], False
    with open(os.path.join(TOOLS, "lumatv"), encoding="utf-8") as f:
        for line in f:
            if line.startswith(name + "() {"):
                on = True
            if on:
                out.append(line)
                if line.rstrip() == "}":
                    return "".join(out)
    raise AssertionError(name + " not found")


@unittest.skipUnless(shutil.which("bash"), "the guard reads with `read -t`, which dash lacks")
class GuardWaitsForThePanel(unittest.TestCase):
    """verb_homestub_watch against a stub with no beat: a set in standby is not yet, a set that is on is a fault.

    The guard gave LG's Home back at 30 s on a boot into standby (2026-10-25 16:34): after the
    compositor restart the panel stayed off, LumaHome never beat, and nothing was wrong.
    """

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.d = self.tmp.name
        self.proc = None

    def tearDown(self):
        if self.proc and self.proc.poll() is None:
            self.proc.kill()
            self.proc.wait()
        self.tmp.cleanup()

    def p(self, name):
        return os.path.join(self.d, name)

    def start(self, beat=None, grace=1):
        script = (
            'HS_WD_PID="{d}/pid"; HS_TICK="{d}/tick"; HS_ESCAPE="{d}/esc"; HS_CONFIRM="{d}/conf"; HS_TRIAL="{d}/trial"\n'
            'HS_BEAT="{d}/beat"; HS_STALE=1; HS_GRACE={grace}\n'
            'log_line() {{ echo "$1" >> "{d}/log"; }}\n'
            'hs_wd_alive() {{ return 1; }}\ncomp_live() {{ return 1; }}\ncompositor_has_shadow() {{ return 0; }}\n'
            'hs_mounted_any() {{ return 0; }}\nmtime_epoch() {{ echo 0; }}\npgrep() {{ echo $$; }}\n'
            'panel_is_active() {{ [ -e "{d}/active" ]; }}\n'
            'homestub_fault() {{ echo "fault: $1" >> "{d}/log"; }}\n'
        ).format(d=self.d, grace=grace)
        script += function("hs_guard_exit") + function("verb_homestub_watch") + "verb_homestub_watch\n"
        self.proc = subprocess.Popen(["bash", "-c", script], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        # The guard removes a beat left from before it started, so a first beat is written after.
        self.assertTrue(self.until("guard started"), self.log())
        if beat is not None:
            with open(self.p("beat"), "w") as f:
                f.write(beat)

    def log(self):
        try:
            with open(self.p("log")) as f:
                return f.read()
        except FileNotFoundError:
            return ""

    def until(self, text, seconds=12):
        end = time.time() + seconds
        while time.time() < end:
            if text in self.log():
                return True
            time.sleep(0.1)
        return False

    def screen(self, on):
        if on:
            open(self.p("active"), "w").close()
        elif os.path.exists(self.p("active")):
            os.remove(self.p("active"))

    def test_no_beat_with_the_panel_off_is_not_a_fault(self):
        self.screen(False)
        self.start()
        self.assertTrue(self.until("the panel is off, waiting for the screen"), self.log())
        time.sleep(4)
        self.assertNotIn("fault", self.log())
        self.assertIsNone(self.proc.poll(), "the guard stopped")

    def test_the_beat_is_checked_again_from_the_first_screen_on_with_a_fresh_grace(self):
        self.screen(False)
        self.start(grace=5)
        self.assertTrue(self.until("waiting for the screen"), self.log())
        self.screen(True)
        self.assertTrue(self.until("screen on, waiting for the beat"), self.log())
        woke = time.time()    # whole seconds of uptime make the guard's own count up to 1 s short
        self.assertNotIn("fault", self.log())
        self.assertTrue(self.until("fault: LumaHome's heartbeat stopped"), self.log())
        self.assertGreaterEqual(time.time() - woke, 3.5, "the grace after the screen-on was not given")

    def test_a_beat_after_the_screen_on_keeps_the_stub(self):
        self.screen(False)
        self.start(grace=1)
        self.assertTrue(self.until("waiting for the screen"), self.log())
        self.screen(True)
        for n in range(1, 9):
            with open(self.p("beat"), "w") as f:
                f.write(str(n))
            time.sleep(0.5)
        self.assertNotIn("fault", self.log())

    def test_a_stopped_beat_on_a_set_that_is_on_is_still_a_fault(self):
        self.screen(True)
        self.start(beat="1", grace=1)
        self.assertTrue(self.until("fault: LumaHome's heartbeat stopped"), self.log())
        self.assertNotIn("panel is off", self.log())

    def test_a_load_error_is_a_fault_with_the_panel_off_too(self):
        self.screen(False)
        self.start(beat="error", grace=30)
        self.assertTrue(self.until("fault: LumaHome could not be loaded"), self.log())


class WaitForPixelsOnTheBootPath(unittest.TestCase):
    """wait_presenting ends early on a boot whose set is still in standby; everywhere else it waits READY_WAIT."""

    def run_wait(self, boot, active):
        with tempfile.TemporaryDirectory() as d:
            if active:
                open(os.path.join(d, "active"), "w").close()
            script = (
                'READY_WAIT=30; STANDBY_WAIT=5\n%s\n'
                'panel_presenting() {{ return 1; }}\npanel_is_active() {{ [ -e "{d}/active" ]; }}\n'
                'sleep() {{ echo . >> "{d}/slept"; }}\n'
            ).format(d=d) % ('LUMA_BOOT=1' if boot else 'unset LUMA_BOOT')
            script += function("wait_presenting") + "wait_presenting; echo rc=$?\n"
            r = subprocess.run(["sh", "-c", script], capture_output=True, text=True, timeout=30)
            slept = 0
            if os.path.exists(os.path.join(d, "slept")):
                with open(os.path.join(d, "slept")) as f:
                    slept = len(f.read().split())
            return r.stdout.strip(), slept

    def test_a_boot_into_standby_stops_waiting_after_standby_wait(self):
        self.assertEqual(self.run_wait(boot=True, active=False), ("rc=1", 5))

    def test_a_boot_with_the_panel_on_waits_the_whole_ready_wait(self):
        self.assertEqual(self.run_wait(boot=True, active=True), ("rc=1", 30))

    def test_outside_the_boot_path_the_panel_state_does_not_shorten_the_wait(self):
        self.assertEqual(self.run_wait(boot=False, active=False), ("rc=1", 30))



class BootCapture(unittest.TestCase):
    """tools/bootcap.sh, the one-off boot capture of the logo check, under sh with a fake capture service."""

    def run_cap(self, secs):
        with tempfile.TemporaryDirectory() as d:
            bindir = os.path.join(d, "bin")
            os.mkdir(bindir)
            with open(os.path.join(bindir, "luna-send"), "w") as f:
                f.write('#!/bin/sh\necho "$4" | sed -n \'s/.*"path":"\\([^"]*\\)".*/\\1/p\' | { read -r p; echo png > "$p"; }\n')
            os.chmod(os.path.join(bindir, "luna-send"), 0o755)
            out = os.path.join(d, "cap")
            r = subprocess.run(["sh", os.path.join(TOOLS, "bootcap.sh"), secs, out], capture_output=True, text=True, timeout=30,
                               env=dict(os.environ, PATH=bindir + ":" + os.environ["PATH"]))
            files = sorted(os.listdir(out)) if os.path.isdir(out) else []
            frames = open(os.path.join(out, "frames.txt")).read().split("\n") if "frames.txt" in files else []
            return r.returncode, files, [l for l in frames if l]

    def test_it_writes_frames_an_index_with_the_uptime_and_a_done_mark(self):
        rc, files, frames = self.run_cap("2")
        self.assertEqual(rc, 0)
        self.assertIn("done", files)
        self.assertGreaterEqual(len(frames), 2)
        self.assertEqual(sorted(f for f in files if f.endswith(".png")), sorted("%d.png" % i for i in range(len(frames))))
        index, uptime, size = frames[0].split()
        self.assertEqual((index, size), ("0", "4"))
        self.assertRegex(uptime, r"^\d+\.\d+$")

    def test_a_bad_length_captures_nothing(self):
        for bad in ("", "x", "-3"):
            rc, files, frames = self.run_cap(bad)
            self.assertEqual((rc, files, frames), (0, [], []), bad)

    def test_the_boot_hook_starts_it_once_per_flag_and_never_blocks(self):
        with open(os.path.join(TOOLS, "autostart.sh"), encoding="utf-8") as f:
            text = f.read()
        m = re.search(r'if \[ -f "\$STATE/bootcap" \].*?\n  fi\n', text, re.S)
        self.assertIsNotNone(m, "the boot capture hook is gone")
        hook = m.group(0)
        self.assertLess(hook.index('rm -f "$STATE/bootcap"'), hook.index("setsid"))
        self.assertIn("</dev/null >/dev/null 2>&1 &", hook)
        self.assertLess(text.index("bootcap"), text.index('"$CLI" homestub-boot'), "the capture starts before the boot work")

    def test_it_samples_what_is_in_front_beside_the_frames(self):
        with tempfile.TemporaryDirectory() as d:
            bindir = os.path.join(d, "bin")
            os.mkdir(bindir)
            with open(os.path.join(bindir, "luna-send"), "w") as f:
                f.write('#!/bin/sh\ncase "$3" in\n *getForegroundAppInfo) echo \'{"returnValue":true,\\n"appId":"com.webos.app.hdmi2"}\' ;;\n'
                        ' *getPowerState) echo \'{"state":"Active"}\' ;;\nesac\n')
            os.chmod(os.path.join(bindir, "luna-send"), 0o755)
            out = os.path.join(d, "cap")
            subprocess.run(["sh", os.path.join(TOOLS, "bootcap.sh"), "2", out], capture_output=True, timeout=30,
                           env=dict(os.environ, PATH=bindir + ":" + os.environ["PATH"]))
            with open(os.path.join(out, "state.txt")) as f:
                lines = [l for l in f.read().split("\n") if l]
            self.assertGreaterEqual(len(lines), 1)
            self.assertRegex(lines[0], r'^\d+\.\d+ fg=\{"returnValue":true,"appId":"com\.webos\.app\.hdmi2"\} ui=\S* cover=\S* power=\{"state":"Active"\}$')
            self.assertIn("done", os.listdir(out))

    def test_events_keep_the_logo_lines_of_every_log_file_oldest_first_and_the_last_boots_copy(self):
        with tempfile.TemporaryDirectory(prefix="bc") as d:
            logs = os.path.join(d, "log")
            os.mkdir(logs)
            with open(os.path.join(logs, "messages"), "w") as f:
                f.write("[  40.1] surface-manager: FRESH_LAUNCH_SYSTEMUI quicksettings\n[  41.0] unrelated line\n")
            import gzip
            with gzip.open(os.path.join(logs, "messages.4.gz"), "wt") as f:
                f.write("[  13.2] bootd: launch com.webos.app.bootlogo\n")
            with gzip.open(os.path.join(logs, "messages.1.gz"), "wt") as f:
                f.write("[  20.0] qml: [Logo] power-on logo 2\n")
            out = os.path.join(d, "state", "boot-events")
            env = dict(os.environ, BOOTCAP_LOGDIR=logs)
            for _ in range(2):
                self.assertEqual(subprocess.run(["sh", os.path.join(TOOLS, "bootcap.sh"), "events", out], env=env, timeout=30).returncode, 0)
            with open(out) as f:
                body = f.read().split("\n")
            self.assertTrue(body[0].startswith("# "))
            self.assertEqual([l.split("] ", 1)[1].split(":")[0] for l in body[1:] if l], ["bootd", "qml", "surface-manager"])
            self.assertTrue(os.path.exists(out + ".prev"), "the copy of the boot before")
            self.assertNotIn("unrelated", "".join(body))

    def test_the_hook_runs_boot_restart_after_boot_bind_and_before_apply_and_keeps_the_events_after_it(self):
        with open(os.path.join(TOOLS, "autostart.sh"), encoding="utf-8") as f:
            text = f.read()
        order = [text.index(s) for s in ('"$CLI" boot-bind', '"$CLI" boot-restart', '"$CLI" apply', 'bootcap.sh" events')]
        self.assertEqual(order, sorted(order))
        self.assertRegex(text, r"\*'auto-reverted to stock'\*\) exit 0")
        self.assertIn("[up ${up_s:-?} s]", text, "every hook line carries the boot's monotonic clock")


@unittest.skipUnless(shutil.which("sh"), "needs sh")
class BootRestart(unittest.TestCase):
    """verb_boot_restart and its helpers from tools/lumatv, run under `set -u` against stubs of the set.

    On 2026-10-25 17:51 the set came back on HDMI 2 and Home followed only after apply's restart at 55 s: the restart
    waited for 30 s of preparation. It now comes right after boot-bind, and the landing keeps a TV that is set to start
    on its last input (general.homeAutoLaunch off) on that input.
    """

    FUNCS = ("get_general_setting", "foreground_app_id", "boot_landing_app", "land_on_app", "verb_boot_restart")

    def run_verb(self, active=True, pending=True, has_shadow=False, fg="com.webos.app.hdmi2", auto=None, launch_works=True,
                 shader=False, boot=True):
        with tempfile.TemporaryDirectory() as d:
            bindir = os.path.join(d, "bin")
            os.mkdir(bindir)
            with open(os.path.join(bindir, "luna-send"), "w") as f:
                f.write(
                    '#!/bin/sh\n'
                    'd="%s"\n'
                    'case "$3" in\n'
                    ' *getSystemSettings) if [ -e "$d/auto" ]; then echo "{\\"returnValue\\":true,\\"category\\":\\"general\\",\\"settings\\":{\\"homeAutoLaunch\\":\\"$(cat $d/auto)\\"}}"; else echo \'{"returnValue":true,"settings":{}}\'; fi ;;\n'
                    ' *getForegroundAppInfo) echo "{\\"returnValue\\":true,\\"appId\\":\\"$(cat $d/fg)\\",\\"windowId\\":\\"\\"}" ;;\n'
                    ' */launch) echo "$4" >> "$d/launched"; if [ -e "$d/launch_works" ]; then echo "$4" | sed -n \'s/.*"id":"\\([^"]*\\)".*/\\1/p\' > "$d/fg"; fi ;;\n'
                    'esac\n' % d)
            os.chmod(os.path.join(bindir, "luna-send"), 0o755)

            def mk(name, text=""):
                with open(os.path.join(d, name), "w") as f:
                    f.write(text)
            mk("fg", fg + "\n")
            mk("messages")
            os.mkdir(os.path.join(d, "shadow"))
            for flag, name in ((active, "active"), (pending, "pending"), (has_shadow, "has_shadow"), (launch_works, "launch_works")):
                if flag:
                    mk(name)
            if auto is not None:
                mk("auto", auto + "\n")
            defs = (
                'set -u\nd="%s"\nPATH="%s:$PATH"\n%s\n'
                'STATE=$d; BOOT_PENDING=$d/pending; VERIFIED=$d/verified; MSGLOG=$d/messages; SHADOW_ROOT=$d/shadow\n'
                'SETTINGS_GET_URI=luna://com.webos.settingsservice/getSystemSettings\n'
                'HOME_LAUNCH_URI=luna://com.webos.applicationManager/launch; HOME_APP_ID=com.webos.app.home\n'
                'STANDBY_WAIT=0; DISPLAY_NAME=LumaTV; reports_before=\n'
                'take_mutation_lock() { return 0; }\n'
                'emit_error() { printf \'{"ok":false,"error":"%%s"}\\n\' "$1"; }\n'
                'log_line() { echo "$1" >> "$d/log"; }\n'
                'uptime_s() { echo 20; }\n'
                'surface_start_uptime() { echo 3; }\n'
                'compositor_has_shadow() { [ -e "$d/has_shadow" ]; }\n'
                'all_binds_live() { return 0; }\nenv_live() { return 0; }\n'
                'panel_is_active() { [ -e "$d/active" ]; }\n'
                'helper_start() { echo helper >> "$d/calls"; }\n'
                'restart_surface() { echo restart >> "$d/calls"; reports_before=x; : > "$d/fg"; %s }\n'
                'home_back_after_restart() { echo home >> "$d/calls"; }\n'
                'disarm_restart_crashes() { echo disarm >> "$d/calls"; }\n'
                'do_revert_internal() { echo revert >> "$d/calls"; }\n'
                'ntfy_notify() { :; }\nsleep() { :; }\n'
            ) % (d, bindir, "LUMA_BOOT=1" if boot else "unset LUMA_BOOT",
                 'echo "shader compilation failed" >> "$MSGLOG";' if shader else "")
            script = defs + "".join(function(n) for n in self.FUNCS) + "verb_boot_restart; echo rc=$?\n"
            r = subprocess.run(["sh", "-c", script], capture_output=True, text=True, timeout=30)

            def read(name):
                p = os.path.join(d, name)
                if not os.path.exists(p):
                    return []
                with open(p) as fh:
                    return fh.read().split("\n")[:-1]
            return r.stdout.strip(), read("calls"), read("launched"), read("log"), r.stderr

    def test_home_comes_back_after_an_early_restart_with_the_helper_started_first(self):
        out, calls, launched, log, err = self.run_verb(auto="on")
        self.assertIn('"restarted":true', out, err)
        self.assertIn('"landed":"home"', out)
        self.assertEqual(calls, ["helper", "restart", "home"])
        self.assertEqual(launched, [])
        self.assertTrue(any("compositor restarted at uptime 20 s" in l for l in log), log)

    def test_an_unset_setting_is_home_too(self):
        out, calls, launched, log, err = self.run_verb(auto=None)
        self.assertIn('"landed":"home"', out, err)
        self.assertEqual(calls, ["helper", "restart", "home"])

    def test_a_tv_set_to_start_on_its_last_input_keeps_that_input(self):
        out, calls, launched, log, err = self.run_verb(auto="off", fg="com.webos.app.hdmi2")
        self.assertIn('"landed":"com.webos.app.hdmi2"', out, err)
        self.assertEqual(launched, ['{"id":"com.webos.app.hdmi2"}'])
        self.assertEqual(calls, ["helper", "restart", "disarm"], "Home is not launched over the input")

    def test_an_input_that_does_not_come_back_gives_home(self):
        out, calls, launched, log, err = self.run_verb(auto="off", launch_works=False)
        self.assertIn('"restarted":true', out, err)
        self.assertEqual(calls, ["helper", "restart", "home"])
        self.assertTrue(any("did not come back, Home instead" in l for l in log), log)

    def test_off_with_home_in_front_is_home(self):
        out, calls, launched, log, err = self.run_verb(auto="off", fg="com.webos.app.home")
        self.assertIn('"landed":"home"', out, err)
        self.assertEqual(launched, [])

    def test_whatever_boot_bind_did_not_verify_is_left_to_apply(self):
        for kw, why in (({"pending": False}, "boot-bind did not verify"), ({"has_shadow": True}, "has the module already"),
                        ({"active": False}, "the set is not on")):
            out, calls, launched, log, err = self.run_verb(**kw)
            self.assertIn('"restarted":false', out, err)
            self.assertIn(why, out)
            self.assertEqual(calls, [], kw)

    def test_it_is_the_boot_hooks_step_only(self):
        out, calls, launched, log, err = self.run_verb(boot=False)
        self.assertIn('"ok":false', out, err)
        self.assertEqual(calls, [])

    def test_a_shader_that_does_not_compile_reverts_the_set_and_says_so(self):
        out, calls, launched, log, err = self.run_verb(shader=True)
        self.assertIn("auto-reverted to stock", out, err)
        self.assertIn("revert", calls)



class MaintenanceGuards(unittest.TestCase):
    def source(self):
        return ''.join(function(n) for n in ('json_escape', 'emit_error', 'maintenance_present', 'maintenance_human_stop', 'maintenance_guard'))

    def shell(self, source, root):
        import shlex
        return subprocess.run(['sh', '-c', 'set -u\nSTATE=' + shlex.quote(root) + '\n' + source], capture_output=True, text=True, timeout=8)

    def test_read_does_not_initialize_and_emergency_sets_durable_veto(self):
        with tempfile.TemporaryDirectory() as root:
            os.mkdir(os.path.join(root, 'maintenance'))
            self.assertEqual(self.shell(self.source() + '\nMAINTENANCE_ACCESS=read\nmaintenance_guard\n', root).returncode, 0)
            self.assertFalse(os.path.exists(os.path.join(root, 'maintenance-human-stop')))
            self.assertEqual(self.shell(self.source() + '\nMAINTENANCE_ACCESS=emergency\nmaintenance_guard\n', root).returncode, 0)
            self.assertTrue(os.path.isdir(os.path.join(root, 'maintenance-human-stop')))
            code = 'ensure_config() { echo changed > "$STATE/created"; }\ncfg_get_bool() { :; }\ncfg_get_str() { :; }\n' + function('cfg_read_all') + '\nMAINTENANCE_ACCESS=read\ncfg_read_all\n'
            self.assertEqual(self.shell(code, root).returncode, 0)
            self.assertFalse(os.path.exists(os.path.join(root, 'created')))

    def test_marker_created_while_waiting_on_real_lock_is_rechecked(self):
        import fcntl
        import shlex
        with tempfile.TemporaryDirectory() as root:
            with open(os.path.join(root, 'mutate.lock'), 'w') as held:
                fcntl.flock(held, fcntl.LOCK_EX)
                code = self.source() + function('take_mutation_lock') + '\nMAINTENANCE_ACCESS=write\nLOCK_WAIT=3\nLOCKFILE="$STATE/mutate.lock"\ntake_mutation_lock\n'
                p = subprocess.Popen(['sh', '-c', 'set -u\nSTATE='+shlex.quote(root)+'\n'+code], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                time.sleep(.1)
                os.mkdir(os.path.join(root, 'maintenance'))
                fcntl.flock(held, fcntl.LOCK_UN)
                stdout, stderr = p.communicate(timeout=5)
                self.assertNotEqual(p.returncode, 0, stderr)
                

    def test_persist_off_and_home_escape_still_work(self):
        with tempfile.TemporaryDirectory() as root:
            os.mkdir(os.path.join(root, 'maintenance'))
            os.mkdir(os.path.join(root, 'compositor'))
            open(os.path.join(root, 'compositor', 'enabled'), 'w').close()
            os.symlink('old-hook', os.path.join(root, 'hook'))
            code = self.source() + function('verb_persist')
            code += '\nMAINTENANCE_ACCESS=emergency\nAUTOSTART_LINK="$STATE/hook"\nCOMP="$STATE/compositor"\nremove_early_hook() { :; }\nset_cfg_bool() { printf "%s %s" "$1" "$2" > "$STATE/config-write"; }\nlog_line() { :; }\nmaintenance_guard; verb_persist off\n'
            self.assertEqual(self.shell(code, root).returncode, 0)
            self.assertFalse(os.path.lexists(os.path.join(root, 'hook')))
            self.assertFalse(os.path.exists(os.path.join(root, 'compositor', 'enabled')))
            with open(os.path.join(root, 'config-write')) as f:
                self.assertEqual(f.read(), 'persist false')
            code = self.source() + function('take_mutation_lock') + function('homestub_escape')
            code += '\nMAINTENANCE_ACCESS=write\nLOCK_WAIT=1\nLOCKFILE="$STATE/mutate.lock"\nHS_SKIP="$STATE/skip"\nlog_line() { :; }\nhomestub_drop() { echo "$1" > "$STATE/dropped"; }\nhs_open_home() { :; }\nhomestub_escape\n'
            import fcntl
            with open(os.path.join(root, "mutate.lock"), "w") as held:
                fcntl.flock(held, fcntl.LOCK_EX)
                self.assertEqual(self.shell(code, root).returncode, 0)
            self.assertTrue(os.path.exists(os.path.join(root, 'skip')))
            with open(os.path.join(root, 'dropped')) as f:
                self.assertEqual(f.read().strip(), 'lg')

    



if __name__ == "__main__":
    unittest.main()
