"""Run with: cd tests && python3 -m unittest test_vplane_measure

vplane/tools/measure.py, the sampler of the video-plane saver's TV steps: what each counter's text is read as, the figures a run makes of its
samples, and a whole run and table against a stand-in /proc and /sys that the test moves on (MEASURE_ROOT).
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
TOOL = os.path.join(REPO, "vplane", "tools", "measure.py")
sys.path.insert(0, os.path.dirname(TOOL))
import measure  # noqa: E402


class Parsers(unittest.TestCase):
    def test_the_mali_counters(self):
        self.assertEqual(measure.parse_dvfs("busy_time: 5450 idle_time: 4550\n"), (5450, 4550))
        self.assertEqual(measure.parse_dvfs("busy_time: 12\nidle_time: 34\n"), (12, 34))
        for junk in ("", "busy_time: 12\n", "idle_time: x busy_time: y", "gpu_busy: 3"):
            self.assertIsNone(measure.parse_dvfs(junk), junk)

    def test_the_cpu_line_counts_iowait_as_idle(self):
        text = "cpu  100 0 50 800 20 0 5 0 0 0\ncpu0 50 0 25 400 10 0 2 0 0 0\nintr 1\n"
        self.assertEqual(measure.parse_stat(text), (155, 975))
        self.assertIsNone(measure.parse_stat("cpu0 1 2 3 4 5 6 7 8\n"))

    def test_interrupts_are_summed_over_the_cpus_and_found_by_the_last_word(self):
        text = ("           CPU0       CPU1       CPU2       CPU3\n"
                " 104:        120          0          0          3     GIC-0 104 Level     osd_irq\n"
                " 175:       1210          3          0          0     GIC-0 175 Level     DE2VSYNC\n"
                " 176:          9          0          0          0     GIC-0 176 Level     mali-gpu\n"
                "IPI0:          0          0          0          0  CPU wakeup interrupts\n")
        self.assertEqual(measure.parse_interrupts(text, ("DE2VSYNC", "osd_irq", "nothing")), {"osd_irq": 123, "DE2VSYNC": 1213})

    def test_memory_process_and_temperature(self):
        mem = "MemTotal: 1 kB\nMemAvailable:  699000 kB\nCmaTotal: 692000 kB\nCmaFree:   49152 kB\n"
        self.assertEqual(measure.parse_meminfo(mem), {"MemAvailable": 699000, "CmaFree": 49152})
        stat = "123 (Web (1) app) S 1 2 3 4 5 6 7 8 9 10 600 400 0 0 20 0 1 0 0 0 0\n"      # utime 600, stime 400 after the name
        self.assertEqual(measure.parse_pid_stat(stat), 1000)
        self.assertIsNone(measure.parse_pid_stat("garbage"))
        self.assertEqual(measure.parse_rss_kb("Name: x\nVmRSS:\t   8192 kB\n"), 8192)
        self.assertEqual(measure.parse_rss_kb("Name: x\n"), 0)
        self.assertEqual((measure.parse_temp("57\n"), measure.parse_temp("SoC temperature 57.5 C"), measure.parse_temp("")), (57.0, 57.5, None))


def synthetic(n=11, procs=True):
    """One sample a second: Mali 54.5 % busy, 60 flips/s, no vsync, 20 % of the cores, 25 % of a core in the player."""
    out = []
    for i in range(n):
        s = {"t": 100.0 + i, "busy": 545 * i, "idle": 455 * i, "flips": 1000 + 60 * i, "vsync": 7 + 0 * i, "osd": 120 * i,
             "cpu_busy": 20 * i, "cpu_total": 100 * i, "CmaFree": (48 - (i % 3)) * 1024, "MemAvailable": (700 - i) * 1024, "temp": 57.0 + (i == 5)}
        if procs:
            s["procs"] = {"player": {"ticks": 25 * i, "rss_kb": 8000 + i * 100}}
        out.append(s)
    return out


class Figures(unittest.TestCase):
    def test_the_figures_leave_the_warm_up_out(self):
        f = measure.summarize(synthetic(), warm=3.0)
        self.assertEqual(f["seconds"], 7.0)
        self.assertEqual(f["mali_pct"], 54.5)
        self.assertEqual(f["flips_per_s"], 60.0)
        self.assertEqual(f["vsync_per_s"], 0.0)
        self.assertEqual(f["osd_irq_per_s"], 120.0)
        self.assertEqual(f["cpu_pct"], 20.0)
        self.assertEqual(f["cma_free_min_mb"], 46.0)
        self.assertEqual(f["mem_avail_min_mb"], 690.0)
        self.assertEqual(f["temp_max"], 58.0)
        self.assertEqual(f["procs"]["player"], {"cpu_pct": 25.0, "rss_mb_max": 8.8})

    def test_a_run_shorter_than_the_warm_up_still_has_figures(self):
        f = measure.summarize(synthetic(3), warm=3.0)
        self.assertEqual((f["seconds"], f["mali_pct"]), (2.0, 54.5))
        self.assertIn("error", measure.summarize(synthetic(1)))

    def test_a_set_with_no_counter_leaves_that_figure_out_rather_than_saying_zero(self):
        bare = [{"t": 0.0, "procs": {}}, {"t": 5.0, "procs": {}}]
        f = measure.summarize(bare, warm=0)
        self.assertEqual(sorted(f), ["samples", "seconds"])


class Whole(unittest.TestCase):
    """A run against a stand-in /proc and /sys that moves on as the TV's would."""

    def setUp(self):
        self.root = tempfile.mkdtemp(prefix="measure-")
        self.addCleanup(shutil.rmtree, self.root, True)
        self.tick = 0
        self.running = True
        self.put("/proc/123/comm", "lumatv-vplane\n")
        self.put("/proc/123/cmdline", "/var/lib/lumatv/bin/lumatv-vplane\0-d\0/tmp/x\0")
        self.put("/proc/124/comm", "sleep\n")
        self.put("/proc/124/cmdline", "sleep\0100\0")
        self.step()
        self.thread = threading.Thread(target=self.feed, daemon=True)
        self.thread.start()
        self.addCleanup(self.stop)

    def stop(self):
        self.running = False
        self.thread.join(2)

    def put(self, path, text):
        full = self.root + path
        os.makedirs(os.path.dirname(full), exist_ok=True)
        with open(full + ".new", "w") as f:
            f.write(text)
        os.replace(full + ".new", full)

    def step(self):
        i = self.tick
        self.put("/sys/kernel/debug/mali0/dvfs_utilization", "busy_time: %d\nidle_time: %d\n" % (500 * i, 500 * i))
        self.put("/proc/lg/fbdev/fb0/flip_cnt", "%d\n" % (6 * i))
        self.put("/proc/stat", "cpu  %d 0 0 %d 0 0 0 0 0 0\ncpu0 1 2 3 4 5 6 7 8\n" % (10 * i, 90 * i))
        self.put("/proc/interrupts", "           CPU0\n 175:  %d  GIC DE2VSYNC\n" % (12 * i))
        self.put("/proc/meminfo", "MemAvailable:  %d kB\nCmaFree:  %d kB\n" % (700 * 1024, 40 * 1024))
        self.put("/proc/lg/pm/temperature", "56\n")
        self.put("/proc/123/stat", "123 (lumatv-vplane) S 1 1 1 0 0 0 0 0 0 0 %d 0 0 0 20 0 1 0 0 0 0\n" % (3 * i))
        self.put("/proc/123/status", "Name:\tlumatv-vplane\nVmRSS:\t   9216 kB\n")
        self.tick += 1

    def feed(self):
        while self.running:
            time.sleep(0.05)
            self.step()

    def run_tool(self, *args):
        env = dict(os.environ, MEASURE_ROOT=self.root)
        return subprocess.run([sys.executable, TOOL, *args], env=env, capture_output=True, text=True, timeout=30)

    def test_a_run_prints_its_figures_and_a_table_puts_runs_side_by_side(self):
        out = os.path.join(self.root, "a.json")
        r = self.run_tool("run", "video saver", "1.2", "--every", "0.1", "--warm", "0.3", "--proc", "lumatv-vplane", "--out", out)
        self.assertEqual(r.returncode, 0, r.stderr)
        f = json.loads(r.stdout)
        self.assertEqual(f["label"], "video saver")
        self.assertAlmostEqual(f["mali_pct"], 50.0, delta=0.1)
        self.assertAlmostEqual(f["flips_per_s"], 120.0, delta=30)             # 6 flips per 50 ms step; the steps' own timing varies
        self.assertAlmostEqual(f["cpu_pct"], 10.0, delta=0.1)
        self.assertEqual(f["cma_free_min_mb"], 40.0)
        self.assertEqual(f["mem_avail_min_mb"], 700.0)
        self.assertEqual(f["procs"]["lumatv-vplane"]["rss_mb_max"], 9.0)
        self.assertGreater(f["procs"]["lumatv-vplane"]["cpu_pct"], 0)
        with open(out) as fh:
            doc = json.load(fh)
        self.assertGreater(len(doc["samples"]), 5)
        second = os.path.join(self.root, "b.json")
        self.run_tool("run", "as found", "0.8", "--every", "0.1", "--warm", "0.2", "--out", second)
        t = self.run_tool("table", out, second)
        self.assertEqual(t.returncode, 0, t.stderr)
        lines = t.stdout.splitlines()
        self.assertEqual(len(lines), 3)
        self.assertTrue(lines[0].startswith("label"))
        self.assertTrue(lines[1].startswith("video saver"))
        self.assertTrue(lines[2].startswith("as found"))
        self.assertIn("Mali %", lines[0])
        self.assertIn("lumatv-vplane", lines[1])

    def test_nothing_is_written_outside_the_chosen_file_and_a_missing_counter_is_a_dash(self):
        self.stop()
        os.unlink(self.root + "/sys/kernel/debug/mali0/dvfs_utilization")
        out = os.path.join(self.root, "c.json")
        r = self.run_tool("run", "no mali", "0.5", "--every", "0.1", "--warm", "0", "--out", out)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertNotIn("mali_pct", json.loads(r.stdout))
        t = self.run_tool("table", out)
        self.assertIn("-", t.stdout.splitlines()[1])


if __name__ == "__main__":
    unittest.main()
