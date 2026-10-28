#!/usr/bin/env python3
"""gpu-ui-impact.py against fake counter files: a flip counter that ticks at a steady rate and Mali counters that move, with a
command that holds a stretch of time (the "job") during which the flip stream stalls once."""
import importlib.util
import os
import subprocess
import sys
import tempfile
import threading
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
TOOL = os.path.join(HERE, "..", "gpu-ui-impact.py")


class Feeder(threading.Thread):
    """Writes the fake files: flips at |rate| a second (and a stall of |stall| s starting |stall_at| s in), Mali busy 30 : idle 70."""

    def __init__(self, flips, mali, rate, stall_at, stall):
        super().__init__(daemon=True)
        self.flips, self.mali, self.rate, self.stall_at, self.stall = flips, mali, rate, stall_at, stall
        self.stop = threading.Event()

    def put(self, path, text):
        with open(path + ".new", "w") as f:
            f.write(text)
        os.replace(path + ".new", path)

    def run(self):
        t0 = time.monotonic()
        n = 0
        step = 0
        next_flip = t0
        while not self.stop.is_set():
            now = time.monotonic()
            stalled = self.stall_at <= now - t0 < self.stall_at + self.stall
            if now >= next_flip and not stalled:
                n += 1
                next_flip = now + 1.0 / self.rate
                self.put(self.flips, "%d\n" % n)
            step += 1
            self.put(self.mali, "busy_time: %d\nidle_time: %d\n" % (step * 30, step * 70))
            time.sleep(0.004)


class UiImpact(unittest.TestCase):
    def run_tool(self, stall_at, stall, command, extra=()):
        with tempfile.TemporaryDirectory() as d:
            flips, mali = os.path.join(d, "flip_cnt"), os.path.join(d, "dvfs")
            feeder = Feeder(flips, mali, rate=20.0, stall_at=stall_at, stall=stall)
            feeder.put(flips, "0\n")
            feeder.put(mali, "busy_time: 0\nidle_time: 0\n")
            feeder.start()
            try:
                p = subprocess.run([sys.executable, TOOL, "--before", "1.0", "--after", "0.5", "--flips", flips, "--mali", mali, *extra, "--"] + command,
                                   capture_output=True, text=True, timeout=60)
            finally:
                feeder.stop.set()
                feeder.join()
            return p

    def keys(self, p):
        line = [l for l in p.stdout.splitlines() if l.startswith("UIIMPACT ")][0]
        return dict(kv.split("=") for kv in line.split()[1:])

    def test_steady_flips_and_mali_share(self):
        p = self.run_tool(1000, 0, ["sleep", "1"])
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        k = self.keys(p)
        for phase in ("before", "during", "after"):
            self.assertAlmostEqual(float(k[phase + "_fps"]), 20.0, delta=4.0, msg=p.stdout)
            self.assertAlmostEqual(float(k[phase + "_mali_pct"]), 30.0, delta=0.5, msg=p.stdout)
        self.assertEqual(k["during_late"], "0", p.stdout)
        self.assertEqual(k["exit"], "0")

    def test_a_stall_during_the_job_shows_as_late_flips_and_a_long_gap(self):
        p = self.run_tool(1.4, 0.5, ["sleep", "1.2"])
        k = self.keys(p)
        self.assertGreaterEqual(int(k["during_late"]), 1, p.stdout)
        self.assertGreaterEqual(float(k["during_gap_max_ms"]), 400.0, p.stdout)
        self.assertLess(float(k["before_gap_max_ms"]), 200.0, p.stdout)

    def test_the_command_exit_code_is_reported(self):
        p = self.run_tool(1000, 0, ["sh", "-c", "exit 3"])
        self.assertEqual(p.returncode, 1)
        self.assertEqual(self.keys(p)["exit"], "3")

    def test_missing_counters_are_n_a(self):
        p = subprocess.run([sys.executable, TOOL, "--before", "0.2", "--after", "0.2", "--flips", "/nonexistent/a", "--mali", "/nonexistent/b", "--", "true"],
                           capture_output=True, text=True, timeout=30)
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        self.assertIn("flips n/a/s", p.stdout)
        self.assertIn("before_fps=n/a", p.stdout)

    def test_a_command_is_required(self):
        p = subprocess.run([sys.executable, TOOL, "--before", "0.1"], capture_output=True, text=True, timeout=30)
        self.assertNotEqual(p.returncode, 0)
        self.assertIn("a command to run is needed", p.stderr)

    def test_the_sets_one_line_counters_are_read(self):
        # The set prints "busy_time: N idle_time: M" on one line (2026-10-28); a read line by line stopped on it.
        spec = importlib.util.spec_from_file_location("gpu_ui_impact", TOOL)
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        with tempfile.TemporaryDirectory() as d:
            one, two = os.path.join(d, "one"), os.path.join(d, "two")
            with open(one, "w") as f:
                f.write("busy_time: 1475864786 idle_time: 99\n")
            with open(two, "w") as f:
                f.write("busy_time: 7\nidle_time: 9\n")
            self.assertEqual(mod.read_mali(one), (1475864786, 99))
            self.assertEqual(mod.read_mali(two), (7, 9))


if __name__ == "__main__":
    unittest.main()
