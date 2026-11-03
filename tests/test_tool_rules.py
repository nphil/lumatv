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
