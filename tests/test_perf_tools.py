"""Run with: python3 -m unittest tests.test_perf_tools

docs/research/perf/sample.py (the fork-free sampler capture.sh uses) and keypath.py (key to first
frame from a full trace), on synthetic input. The set is not touched.
"""
import os
import subprocess
import sys
import tempfile
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
PERF = os.path.join(os.path.dirname(HERE), "docs", "research", "perf")
sys.path.insert(0, PERF)
import keypath  # noqa: E402
import sample  # noqa: E402


class Sampler(unittest.TestCase):
    def test_one_line_has_the_fields_the_shell_sampler_wrote(self):
        line = sample.sample(os.getpid())
        fields = line.split("|")
        self.assertEqual(len(fields), 14)
        self.assertGreater(float(fields[0]), 0)                       # uptime
        self.assertTrue(fields[1].startswith("cpu0 "))                # per-CPU jiffies, ';' separated
        self.assertTrue(fields[1].endswith(";"))
        self.assertTrue(fields[6].startswith("MemFree: "))            # squeezed spaces, as `tr -s ' '`
        self.assertNotIn("  ", fields[6])
        self.assertTrue(fields[7].startswith("pgscan_kswapd ") or fields[7] == "" or "pgmajfault" in fields[7])
        self.assertTrue(fields[10].startswith("minflt="))
        self.assertIn("rss=", fields[10])
        self.assertTrue(fields[11].startswith("online="))

    def test_the_rss_field_is_what_the_gate_reads(self):
        import gate
        rows = gate.parse_rss(sample.sample(os.getpid()) + "\n")
        self.assertEqual(len(rows), 1)
        self.assertGreater(rows[0][1], 0)

    def test_a_missing_process_leaves_the_field_empty_and_the_line_whole(self):
        fields = sample.sample(2 ** 22 + 12345).split("|")
        self.assertEqual(len(fields), 14)
        self.assertEqual(fields[10], "")

    def test_it_runs_until_the_flag_goes_and_forks_nothing(self):
        with tempfile.TemporaryDirectory() as d:
            open(os.path.join(d, ".running"), "w").close()
            out = os.path.join(d, "x.sys")
            p = subprocess.Popen([sys.executable, os.path.join(PERF, "sample.py"), out, str(os.getpid()), d])
            time.sleep(1.6)
            os.remove(os.path.join(d, ".running"))
            self.assertEqual(p.wait(timeout=5), 0)
            with open(out) as f:
                self.assertGreaterEqual(len(f.read().splitlines()), 2)

    def test_capture_sh_prefers_the_python_sampler(self):
        with open(os.path.join(PERF, "capture.sh")) as f:
            text = f.read()
        self.assertIn("sample.py", text)
        self.assertIn("sample > $D/$name.sys", text)                   # the shell one stays as the fallback


def ev(t, name, args="", pid=1, comm="x", cpu=0):
    return (t, cpu, name, pid, comm, args)


def switch(t, cpu, prev, nxt, nxt_pid):
    return ev(t, "sched_switch", "prev_comm=%s prev_pid=1 prev_prio=120 prev_state=S ==> next_comm=%s next_pid=%d next_prio=120" % (prev, nxt, nxt_pid), cpu=cpu)


class KeyPath(unittest.TestCase):
    def test_the_response_is_the_first_frame_that_starts_a_run(self):
        frames = [10.017, 10.5, 10.600, 10.617, 10.634, 10.65]       # a lone tick, then an animation
        first, resp = keypath.response_frame(frames, 10.0)
        self.assertAlmostEqual(first, 10.017)
        self.assertAlmostEqual(resp, 10.600)

    def test_without_a_run_the_response_is_the_first_frame(self):
        first, resp = keypath.response_frame([10.04, 10.3], 10.0)
        self.assertEqual(first, resp)
        self.assertEqual(keypath.response_frame([], 10.0), (None, None))

    def test_busy_time_and_waits_are_counted_up_to_the_gpu_job(self):
        sm = 7
        events = [
            ev(100.000, "tracing_mark_write", "P:x-open"),
            switch(100.001, 1, "idle", "surface-manager", sm),                   # runs 4 ms
            switch(100.005, 1, "surface-manager", "idle", 0),
            ev(100.020, "sched_wakeup", "comm=surface-manager pid=%d prio=120 target_cpu=1" % sm),
            switch(100.020, 1, "idle", "surface-manager", sm),                   # runs 6 ms
            ev(100.026, "mali_job_slots_event", "gpu=0 event=16842752 tgid=%d pid=%d job_id=1" % (sm, sm)),
            switch(100.026, 1, "surface-manager", "idle", 0),
            ev(100.035, "workqueue_execute_start", "work struct 1: function commit_work"),
            ev(100.041, "drm_vblank_event_delivered", "file=1, crtc=0, seq=1"),
            ev(100.058, "drm_vblank_event_delivered", "file=1, crtc=0, seq=2"),
        ]
        w = keypath.analyse_window(events, 100.0, 0.2, sm, keypath.frame_times(events))
        self.assertAlmostEqual(w["first_ms"], 41.0, places=1)
        self.assertAlmostEqual(w["response_ms"], 41.0, places=1)                 # followed by a frame 17 ms later
        self.assertAlmostEqual(w["gpu_ms"], 26.0, places=1)
        self.assertAlmostEqual(w["flip_ms"], 35.0, places=1)
        self.assertAlmostEqual(w["busy_ms"], 10.0, places=1)
        self.assertEqual(w["waits"], 1)

    def test_the_capture_samplers_own_forks_are_noise_and_other_tasks_are_listed(self):
        sm = 7
        events = [
            ev(100.000, "tracing_mark_write", "P:x-open"),
            switch(100.000, 2, "idle", "grep", 20), switch(100.010, 2, "grep", "ls-hubd", 21),
            switch(100.030, 2, "ls-hubd", "idle", 0),
            ev(100.040, "drm_vblank_event_delivered", "file=1, crtc=0, seq=1"),
        ]
        w = keypath.analyse_window(events, 100.0, 0.2, sm, keypath.frame_times(events))
        self.assertAlmostEqual(w["noise_ms"], 10.0, places=1)
        self.assertEqual([c for c, _ in w["others"]], ["ls-hubd"])

    def test_the_table_prints_a_row_for_a_window_with_no_frame(self):
        text = keypath.format_table([("x-open", {"first_ms": None})])
        self.assertIn("no frame", text)


if __name__ == "__main__":
    unittest.main()
