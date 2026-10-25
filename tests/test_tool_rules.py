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
                f.write('NODE="%s"\nTILEICONS="%s"\nSTATE="%s"\nLOGFILE="%s/log"\nREVERT_CHANGED=0\n' % (node, tile, d, d))
                f.write('log_line() { echo "$1" >> "%s/loglines"; }\n' % d)
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


if __name__ == "__main__":
    unittest.main()
