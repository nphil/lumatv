"""Run with: python3 -m unittest tests.test_tool_rules

Rules the tool keeps on the set, checked in its source and, where one is cheap to run, in its
behaviour. The first: SAM (`sam.service`, the application manager) is never restarted, stopped or
killed by anything this package ships. It is a set rule: eARC, CEC and the picture mode are at risk
when it goes, and the one place that did it (the tile-icon revert) is gone.
"""
import os
import re
import subprocess
import tempfile
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


if __name__ == "__main__":
    unittest.main()
