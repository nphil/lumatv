"""Standby screen settings against a fake service."""
import json
import os
import stat
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
TOOLS = os.path.join(ROOT, "tools")
sys.path.insert(0, TOOLS)
sys.path.insert(0, HERE)

TOOL = os.path.join(ROOT, "tools", "lumatv")



def function(name):
    out, on = [], False
    with open(TOOL) as f:
        for line in f:
            if line.startswith(name + "() {"):
                on = True
            if on:
                out.append(line)
                if line.rstrip() == "}":
                    return "".join(out)
    raise AssertionError(name + " not found")






class Switches(unittest.TestCase):
    """Standby screen settings against a fake service."""


    def run_standbyscreen(self, arg, start, refuse=False):
        """`verb_standbyscreen arg` with a luna-send that keeps `lifeOnScreenMode` in a file, starting at `start`. Returns (stdout, mode after, prev file)."""
        with tempfile.TemporaryDirectory() as d:
            bindir = os.path.join(d, "bin")
            os.makedirs(bindir)
            mode = os.path.join(d, "mode")
            if start is not None:
                with open(mode, "w") as f:
                    f.write(start)
            fake = os.path.join(bindir, "luna-send")
            write = 'echo \'{"returnValue":false}\'' if refuse else (
                'v=$(printf \'%s\' "$4" | sed -n \'s/.*"lifeOnScreenMode":"\\([^"]*\\)".*/\\1/p\'); printf \'%s\' "$v" > ' + mode + '; echo \'{"returnValue":true}\'')
            read = ('m=$(cat ' + mode + ' 2>/dev/null); if [ -n "$m" ]; then echo "{\\"returnValue\\":true,\\"settings\\":{\\"lifeOnScreenMode\\":\\"$m\\"}}"; '
                    'else echo \'{"returnValue":true,"settings":{}}\'; fi')
            with open(fake, "w") as f:
                f.write("#!/bin/sh\ncase \"$4\" in\n  *'\"settings\"'*) " + write + " ;;\n  *) " + read + " ;;\nesac\n")
            os.chmod(fake, os.stat(fake).st_mode | stat.S_IEXEC)
            script = (f'PATH="{bindir}:$PATH"\nSTATE="{d}/state"\nLOGFILE="{d}/log"\nSETTINGS_GET_URI=x\nSETTINGS_SET_URI=y\n'
                      'take_mutation_lock() { return 0; }\nlog_line() { :; }\n'
                      + "".join(function(n) for n in ("json_escape", "emit_error", "get_general_setting", "set_general_setting", "verb_standbyscreen"))
                      + 'STANDBYSCREEN_PREV="$STATE/standbyscreen.prev"\n' + f'verb_standbyscreen {arg}\n')
            r = subprocess.run(["sh", "-c", script], capture_output=True, text=True, timeout=30)
            after = open(mode).read() if os.path.exists(mode) else None
            prev = os.path.join(d, "state", "standbyscreen.prev")
            return r.stdout.strip(), after, (open(prev).read().strip() if os.path.exists(prev) else None)

    def test_standbyscreen_off_darkens_the_panel_and_on_puts_back_what_was_there(self):
        out, after, prev = self.run_standbyscreen("off", "alwaysReady")
        self.assertEqual(json.loads(out), {"ok": True, "standbyscreen": "off", "was": "alwaysReady"})
        self.assertEqual((after, prev), ("off", "alwaysReady"), "the mode that was there is kept for `on`")
        out, after, _ = self.run_standbyscreen("status", "off")
        self.assertEqual(json.loads(out)["on"], True)
        out, after, _ = self.run_standbyscreen("status", "alwaysReady")
        self.assertEqual((json.loads(out)["on"], json.loads(out)["mode"]), (False, "alwaysReady"))

    def test_standbyscreen_off_twice_keeps_the_first_mode_and_on_without_one_is_the_settings_own_on(self):
        out, after, prev = self.run_standbyscreen("off", "off")
        self.assertEqual(json.loads(out)["standbyscreen"], "off")
        self.assertEqual(prev, None, "already off: there is nothing to put back, and nothing that was there is overwritten")
        out, after, _ = self.run_standbyscreen("on", "off")
        self.assertEqual((json.loads(out)["mode"], after), ("allEnabled", "allEnabled"), "Settings' own on (PageLook) when no mode was kept")

    def test_a_settings_service_that_refuses_is_an_error_and_nothing_is_claimed(self):
        out, after, _ = self.run_standbyscreen("off", "alwaysReady", refuse=True)
        self.assertFalse(json.loads(out)["ok"], out)
        self.assertEqual(after, "alwaysReady")


if __name__ == "__main__":
    unittest.main()
