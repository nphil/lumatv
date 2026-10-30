"""Run with: python3 -m unittest tests.test_apply_keeps_helper

An apply that fails and rolls back stops the helper (`do_revert_internal` stops it) and, until 2026-10-28, nothing started it again:
Home lost its weather, profile and status data until the owner started it by hand. Apply starts it again on every way out, and a
compositor restart that is refused because a recovery of LG's Home is running does not roll anything back.
"""
import os
import subprocess
import tempfile
import unittest

from tests.test_tool_rules import function


def run(body, calls_file):
    script = body
    r = subprocess.run(["sh", "-c", script], capture_output=True, text=True, timeout=30)
    calls = []
    if os.path.exists(calls_file):
        with open(calls_file) as f:
            calls = f.read().split()
    return r, calls


class ApplyKeepsTheHelper(unittest.TestCase):
    def verb(self, apply_rc, running):
        with tempfile.TemporaryDirectory() as d:
            calls = os.path.join(d, "calls")
            script = (
                'apply_run() { return %d; }\n'
                'helper_json() { printf %%s %s; }\n'
                'helper_start() { echo start >> "%s"; }\n'
                % (apply_rc, "'{\"running\":true}'" if running else "'{\"running\":false}'", calls)
            ) + function("verb_apply") + "verb_apply; echo rc=$?\n"
            r, got = run(script, calls)
            return r.stdout.strip(), got, r.stderr

    def test_a_failed_apply_starts_the_helper_its_rollback_stopped(self):
        out, calls, err = self.verb(1, running=False)
        self.assertEqual(out, "rc=1", err)
        self.assertEqual(calls, ["start"])

    def test_a_good_apply_starts_it_when_it_is_not_running(self):
        out, calls, err = self.verb(0, running=False)
        self.assertEqual((out, calls), ("rc=0", ["start"]), err)

    def test_a_running_helper_is_left_alone(self):
        out, calls, err = self.verb(1, running=True)
        self.assertEqual((out, calls), ("rc=1", []), err)


class RecoveryDoesNotRollBackTheSet(unittest.TestCase):
    def test_a_refused_restart_is_not_a_failed_set(self):
        text = function("restart_surface")
        self.assertIn("return 2", text)
        with open(os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "tools", "lumatv"), encoding="utf-8") as f:
            source = f.read()
        branch = source.split("restart_surface; rs=$?", 1)[1].split('if [ "$rs" -ne 0 ]', 1)[0]
        self.assertIn('[ "$rs" -eq 2 ]', branch)
        self.assertNotIn("do_revert_internal", branch)
        self.assertIn("return 1", branch)


if __name__ == "__main__":
    unittest.main()
