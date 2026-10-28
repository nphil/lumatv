"""Run with: python3 -m unittest tests.test_kwait

docs/research/perf/kwait.py: which kernel waits made a frame late, from a capture.sh trace. Every trace here is written line by line in
ftrace's own text (sched_switch with prev_state, sched_wakeup, a stacktrace trigger's `<stack trace>`), so the parser, the wait
intervals, the wakeup chain, the labels, the 3 ms rule and the score are each read off a case with a known answer.
"""
import json
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
PERF = os.path.join(os.path.dirname(HERE), "docs", "research", "perf")
sys.path.insert(0, PERF)
import kwait  # noqa: E402

LOOP = 100


def ev(t, cpu, comm, pid, name, args):
    return f"{comm:>16}-{pid:<5} [{cpu:03d}] d..2 {t:.6f}: {name}: {args}"


def switch(t, cpu, prev, ppid, state, nxt, npid):
    return ev(t, cpu, prev, ppid, "sched_switch",
              f"prev_comm={prev} prev_pid={ppid} prev_prio=100 prev_state={state} ==> next_comm={nxt} next_pid={npid} next_prio=120")


def wakeup(t, cpu, waker, wpid, comm, pid):
    return ev(t, cpu, waker, wpid, "sched_wakeup", f"comm={comm} pid={pid} prio=100 target_cpu={cpu:03d}")


def vblank(t):
    return ev(t, 0, "swapper/0", 0, "drm_vblank_event_delivered", "crtc=0 seq=1")


def stack(t, cpu, comm, pid, symbols):
    return [f"{comm:>16}-{pid:<5} [{cpu:03d}] d..2 {t:.6f}: <stack trace>"] + [f" => {s}" for s in symbols]


def trace(*parts):
    lines = []
    for p in parts:
        lines += p if isinstance(p, list) else [p]
    return lines


def frames(*times):
    return [vblank(t) for t in times]


def run_trace(lines, gap=20.0):
    events, stacks = kwait.parse_lines(lines)
    ctx = kwait.Context(events, stacks, LOOP)
    return [ctx.frame(g0, g1) for g0, g1 in kwait.late_frames(events, gap)]


class Parsing(unittest.TestCase):
    def test_a_stack_belongs_to_the_switch_it_follows_on_its_own_cpu(self):
        lines = trace(
            switch(1.0, 0, "lumatv-composit", LOOP, "D", "swapper/0", 0),
            stack(1.0, 0, "lumatv-composit", LOOP, ["__schedule", "schedule", "kbase_jd_submit"]),
            switch(1.0001, 1, "other", 7, "D", "swapper/1", 0),
            stack(1.0001, 1, "other", 7, ["__schedule", "io_schedule"]),
        )
        events, stacks = kwait.parse_lines(lines)
        self.assertEqual(len(events), 2)
        self.assertEqual(sorted(len(s) for s in stacks.values()), [2, 3])
        by_pid = {kwait.SWITCH.match(events[i][5]).group(2): syms for i, syms in stacks.items()}
        self.assertEqual(by_pid[str(LOOP)][-1], "kbase_jd_submit")
        self.assertEqual(by_pid["7"][-1], "io_schedule")

    def test_symbol_offsets_and_a_stack_of_another_task_are_left_out(self):
        lines = trace(
            switch(2.0, 0, "lumatv-composit", LOOP, "D", "x", 9),
            stack(2.0, 0, "someone-else", 55, ["wrong_task"]),
            stack(2.0, 0, "lumatv-composit", LOOP, ["rwsem_down_read_slowpath+0x24c/0x590", "do_page_fault+0x1c/0x30"]),
        )
        _events, stacks = kwait.parse_lines(lines)
        self.assertEqual(list(stacks.values()), [["rwsem_down_read_slowpath", "do_page_fault"]])

    def test_the_waits_run_from_the_switch_out_to_the_wakeup_and_from_there_to_the_switch_in(self):
        lines = trace(
            switch(1.000, 0, "lumatv-composit", LOOP, "D", "swapper/0", 0),
            wakeup(1.010, 1, "kworker/1:0", 31, "lumatv-composit", LOOP),
            switch(1.012, 0, "swapper/0", 0, "R", "lumatv-composit", LOOP),
        )
        events, stacks = kwait.parse_lines(lines)
        waits = kwait.build_waits(events, stacks)[LOOP]
        self.assertEqual([(w.kind, round(w.ms, 1)) for w in waits], [("D", 10.0), ("R", 2.0)])
        self.assertEqual((waits[0].waker, waits[0].waker_comm), (31, "kworker/1:0"))


class Labels(unittest.TestCase):
    CASES = {
        "kbase": ["__schedule", "schedule", "schedule_preempt_disabled", "__mutex_lock_slowpath", "mutex_lock", "kbase_jd_submit", "kbase_ioctl"],
        "mmap_sem": ["__schedule", "schedule", "rwsem_down_read_slowpath", "do_page_fault", "do_translation_fault", "do_mem_abort", "el0_da"],
        "io": ["__schedule", "schedule", "io_schedule", "wait_on_page_bit_killable", "filemap_fault", "__do_fault", "handle_mm_fault"],
        "swap": ["__schedule", "schedule", "io_schedule", "swap_readpage", "do_swap_page", "handle_mm_fault"],
        "memory": ["__schedule", "schedule", "io_schedule_timeout", "congestion_wait", "shrink_inactive_list", "try_to_free_pages", "__alloc_pages_slowpath"],
        "fence": ["__schedule", "schedule", "dma_fence_default_wait", "dma_fence_wait_timeout"],
        "futex": ["__schedule", "schedule", "futex_wait_queue_me", "futex_wait", "do_futex"],
        "cgroup": ["__schedule", "schedule", "percpu_rwsem_wait", "__percpu_down_read", "cgroup_can_fork", "copy_process"],
        "mutex": ["__schedule", "schedule", "__mutex_lock_slowpath", "mutex_lock", "some_driver_ioctl"],
    }

    def test_each_stack_is_the_class_it_names(self):
        for expected, symbols in self.CASES.items():
            self.assertEqual(kwait.label_stack(symbols), expected, symbols)

    def test_an_rwsem_sleep_that_is_not_the_address_spaces_is_not_mmap_sem(self):
        self.assertNotEqual(kwait.label_stack(["schedule", "rwsem_down_write_slowpath", "some_fs_op"]), "mmap_sem")

    def test_a_kbase_mutex_is_kbase_not_a_bare_mutex(self):
        self.assertEqual(kwait.label_stack(self.CASES["kbase"]), "kbase")

    def test_an_unknown_stack_is_none(self):
        self.assertIsNone(kwait.label_stack(["schedule", "msleep", "my_thing"]))


class Attribution(unittest.TestCase):
    def late(self, *parts, **kw):
        """One late frame: vblanks at 1.000 and 1.0167, then 1.0500 (33.3 ms: the frame between is 16.7 ms over)."""
        r = run_trace(trace(frames(1.000, 1.0167, 1.0500), *parts), **kw)
        self.assertEqual(len(r), 1, r)
        return r[0]

    def test_a_loop_blocked_in_the_gpu_driver_for_most_of_the_overrun_is_kernel_caused_by_kbase(self):
        r = self.late(
            switch(1.0200, 0, "lumatv-composit", LOOP, "D", "swapper/0", 0),
            stack(1.0200, 0, "lumatv-composit", LOOP, ["__schedule", "schedule", "mutex_lock", "kbase_jd_submit"]),
            wakeup(1.0340, 1, "mali-cmar-backe", 205, "lumatv-composit", LOOP),
            switch(1.0341, 0, "swapper/0", 0, "R", "lumatv-composit", LOOP),
        )
        self.assertEqual((r["verdict"], r["class"]), ("kernel", "kbase"))
        self.assertAlmostEqual(r["kernel_ms"], 14.0, places=1)
        self.assertGreaterEqual(r["score"], 0.8)
        self.assertEqual(r["waits"][0]["source"], "stack")
        self.assertEqual(r["waits"][0]["woken_by"], "mali-cmar-backe-205")
        self.assertIn("kbase 14.0 ms", kwait.describe(r))

    def test_the_wait_is_followed_to_the_thread_that_woke_the_loop_and_what_that_one_was_blocked_in(self):
        # The loop sleeps on mmap_sem from 1.020 to 1.040; the thread that releases it (a Mali worker) was itself reading a page from flash
        # from 1.022 to 1.038: the loop's wait is mmap_sem, the cause underneath it I/O, one hop away.
        r = self.late(
            switch(1.0200, 0, "lumatv-composit", LOOP, "D", "swapper/0", 0),
            stack(1.0200, 0, "lumatv-composit", LOOP, ["__schedule", "schedule", "rwsem_down_read_slowpath", "do_page_fault"]),
            switch(1.0220, 1, "mali-utility-wo", 206, "D", "swapper/1", 0),
            stack(1.0220, 1, "mali-utility-wo", 206, ["__schedule", "schedule", "io_schedule", "wait_on_page_bit", "filemap_fault"]),
            wakeup(1.0380, 2, "mmcqd/0", 80, "mali-utility-wo", 206),
            wakeup(1.0400, 1, "mali-utility-wo", 206, "lumatv-composit", LOOP),
        )
        self.assertEqual(r["verdict"], "kernel")
        classes = {(w["class"], w["hop"]) for w in r["waits"]}
        self.assertEqual(classes, {("mmap_sem", 0), ("io", 1)})
        # The loop's own 20 ms is cut to the frame's window (1.0167 to 1.0500); the union is what is over the overrun.
        self.assertAlmostEqual(r["kernel_ms"], 20.0 - 0.0, delta=3.5)
        self.assertEqual(r["by_class"]["mmap_sem"] > r["by_class"]["io"], True)

    def test_a_chain_is_followed_three_hops_and_no_further(self):
        parts = []
        # LOOP waits on t1 (200), woken by 200, which waited on 201 ... a chain of five threads, each blocked for the whole stretch.
        names = [LOOP, 200, 201, 202, 203, 204]
        for i, tid in enumerate(names[:-1]):
            parts.append(switch(1.0200 + i * 1e-5, i, f"t{tid}", tid, "D", "swapper/0", 0))
            parts.append(wakeup(1.0420 - i * 1e-5, 5, f"t{names[i + 1]}", names[i + 1], f"t{tid}", tid))
        r = self.late(*parts)
        self.assertEqual(sorted({w["hop"] for w in r["waits"]}), [0, 1, 2, 3], r["waits"])
        self.assertEqual(len(r["waits"]), 4, "the loop and three hops")

    def test_a_wait_under_three_milliseconds_is_not_a_kernel_wait(self):
        r = self.late(
            switch(1.0200, 0, "lumatv-composit", LOOP, "D", "swapper/0", 0),
            wakeup(1.0225, 1, "kworker/1:0", 31, "lumatv-composit", LOOP),
        )
        self.assertEqual((r["kernel"], r["kernel_ms"], r["waits"]), (False, 0.0, []))
        self.assertEqual(r["verdict"], "unexplained")

    def test_a_short_wait_beside_a_big_overrun_is_not_enough(self):
        r = self.late(
            switch(1.0200, 0, "lumatv-composit", LOOP, "D", "swapper/0", 0),
            stack(1.0200, 0, "lumatv-composit", LOOP, ["schedule", "io_schedule"]),
            wakeup(1.0245, 1, "mmcqd/0", 80, "lumatv-composit", LOOP),
        )
        self.assertAlmostEqual(r["kernel_ms"], 4.5, places=1)
        self.assertEqual((r["kernel"], r["verdict"]), (False, "unexplained"), "4.5 ms of 16.6 is a share, not the cause")
        self.assertEqual(r["by_class"], {"io": 4.5})

    def test_a_loop_that_waited_for_a_cpu_is_scheduler_caused_and_says_who_had_it(self):
        r = self.late(
            wakeup(1.0200, 0, "swapper/0", 0, "lumatv-composit", LOOP),
            switch(1.0360, 0, "qml-runner", 900, "R", "lumatv-composit", LOOP),
        )
        self.assertEqual(r["verdict"], "sched")
        self.assertAlmostEqual(r["sched_ms"], 16.0, places=1)
        self.assertEqual(r["kernel_ms"], 0.0)
        self.assertIn("run-queue wait", kwait.describe(r))

    def test_with_no_stack_a_block_request_inside_the_wait_labels_it_io_and_the_label_says_it_is_inferred(self):
        r = self.late(
            switch(1.0200, 0, "lumatv-composit", LOOP, "D", "swapper/0", 0),
            ev(1.0250, 0, "lumatv-composit", LOOP, "block_rq_issue", "179,0 R 0 () 1234 + 8 [lumatv-composit]"),
            wakeup(1.0400, 1, "mmcqd/0", 80, "lumatv-composit", LOOP),
        )
        self.assertEqual((r["class"], r["waits"][0]["source"]), ("io?", "context"))

    def test_with_no_stack_a_gpu_job_running_under_the_wait_labels_it_kbase_inferred(self):
        r = self.late(
            switch(1.0200, 0, "lumatv-composit", LOOP, "D", "swapper/0", 0),
            ev(1.0210, 1, "mali-cmar-backe", 205, "mali_job_slots_event", f"gpu=0 event={(1 << 24) | (0 << 16)} tgid=100 pid=100 job_id=7"),
            ev(1.0390, 1, "swapper", 0, "mali_job_slots_event", f"gpu=0 event={(2 << 24) | (0 << 16)} tgid=100 pid=100 job_id=7"),
            wakeup(1.0400, 1, "mali-cmar-backe", 205, "lumatv-composit", LOOP),
        )
        self.assertEqual((r["class"], r["waits"][0]["source"]), ("kbase?", "context"))

    def test_with_nothing_to_go_on_a_big_wait_is_other_and_still_kernel(self):
        r = self.late(
            switch(1.0200, 0, "lumatv-composit", LOOP, "D", "swapper/0", 0),
            wakeup(1.0400, 1, "kworker/1:0", 31, "lumatv-composit", LOOP),
        )
        self.assertEqual((r["verdict"], r["class"]), ("kernel", "other"))

    def test_no_wakeup_in_the_trace_ends_the_wait_at_the_switch_in(self):
        r = self.late(
            switch(1.0200, 0, "lumatv-composit", LOOP, "D", "swapper/0", 0),
            stack(1.0200, 0, "lumatv-composit", LOOP, ["schedule", "dma_fence_wait"]),
            switch(1.0400, 0, "swapper/0", 0, "R", "lumatv-composit", LOOP),
        )
        self.assertEqual((r["verdict"], r["class"]), ("kernel", "fence"))

    def test_the_loops_own_waits_only_count_inside_the_late_frame(self):
        # A long wait that ended before the late frame began is not its cause.
        r = self.late(
            switch(1.0010, 0, "lumatv-composit", LOOP, "D", "swapper/0", 0),
            wakeup(1.0150, 1, "kworker/1:0", 31, "lumatv-composit", LOOP),
        )
        self.assertEqual((r["kernel_ms"], r["kernel"]), (0.0, False))

    def test_late_frames_are_the_gaps_over_the_limit_and_a_still_screen_is_not_one(self):
        events, _ = kwait.parse_lines(frames(1.0, 1.0167, 1.0500, 1.0667, 3.0))
        self.assertEqual(kwait.late_frames(events), [(1.0167, 1.05)])
        self.assertEqual(len(kwait.late_frames(events, 20.0, 5000.0)), 2, "with no ceiling the idle gap counts")

    def test_the_summary_counts_by_verdict_and_class(self):
        frames_ = [{"verdict": "kernel", "class": "kbase"}, {"verdict": "kernel", "class": "kbase"}, {"verdict": "kernel", "class": "io"},
                   {"verdict": "sched", "class": None}, {"verdict": "unexplained", "class": None}]
        s = kwait.summarize(frames_)
        self.assertEqual((s["late"], s["kernel"], s["sched"], s["unexplained"], s["by_class"]), (5, 3, 1, 1, {"kbase": 2, "io": 1}))
        self.assertEqual(kwait.summary_line(s), "kernel waits: 5 late frame(s): 3 kernel-caused (kbase 2, io 1), 1 waiting for a CPU, 1 unexplained")
        self.assertEqual(kwait.summary_line(kwait.summarize([])), "kernel waits: no late frame")


class Command(unittest.TestCase):
    def test_the_command_reads_a_trace_file_and_prints_each_late_frame_and_the_summary(self):
        lines = trace(
            frames(1.000, 1.0167, 1.0500),
            switch(1.0200, 0, "lumatv-composit", LOOP, "D", "swapper/0", 0),
            stack(1.0200, 0, "lumatv-composit", LOOP, ["schedule", "mutex_lock", "kbase_mem_alloc"]),
            wakeup(1.0340, 1, "mali-cmar-backe", 205, "lumatv-composit", LOOP),
        )
        with tempfile.TemporaryDirectory() as d:
            path, out = os.path.join(d, "t.trace"), os.path.join(d, "r.json")
            with open(path, "w") as f:
                f.write("\n".join(lines) + "\n")
            r = subprocess.run([sys.executable, os.path.join(PERF, "kwait.py"), path, "--comm", "lumatv-composit", "--json", out], capture_output=True, text=True)
            self.assertEqual(r.returncode, 0, r.stderr)
            self.assertIn("kbase 14.0 ms", r.stdout)
            self.assertIn("kernel waits: 1 late frame(s): 1 kernel-caused (kbase 1)", r.stdout)
            with open(out) as f:
                doc = json.load(f)
        self.assertEqual((doc["tid"], doc["summary"]["by_class"]), (LOOP, {"kbase": 1}))



CAPTURE = os.path.join(PERF, "capture.sh")


class CaptureScript(unittest.TestCase):
    """capture.sh against a directory that plays tracefs, a fake pgrep and a fake /proc: the stack trigger and the filters it arms for the
    compositor's threads, and what it puts back at the end."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        d = self.tmp.name
        self.t, self.perf, self.proc, self.bin = (os.path.join(d, n) for n in ("tracing", "perf", "proc", "bin"))
        for sub in ("sched/sched_switch", "sched/sched_wakeup", "drm/drm_vblank_event_delivered", "drm/drm_vblank_event", "block/block_rq_issue"):
            os.makedirs(os.path.join(self.t, "events", sub))
        for name in ("tracing_on", "trace", "buffer_size_kb", "trace_clock", "trace_marker", "events/enable"):
            open(os.path.join(self.t, name), "w").close()
        for tid in (100, 101):
            os.makedirs(os.path.join(self.proc, "4242", "task", str(tid)))
        os.makedirs(self.bin)
        os.makedirs(self.perf)
        self.write(os.path.join(self.bin, "pgrep"), "#!/bin/sh\n[ -e \"$NO_COMPOSITOR\" ] && exit 1\necho 4242\n", 0o755)
        self.write(os.path.join(self.bin, "pidof"), "#!/bin/sh\nexit 1\n", 0o755)
        self.write(os.path.join(self.perf, "sample.py"), "")
        self.env = dict(os.environ, PATH=self.bin + os.pathsep + os.environ["PATH"], LUMATV_TRACEFS=self.t, LUMATV_PERF_DIR=self.perf,
                        LUMATV_PROC=self.proc, NO_COMPOSITOR=os.path.join(d, "none"))

    def tearDown(self):
        self.tmp.cleanup()

    def write(self, path, text, mode=0o644):
        with open(path, "w") as f:
            f.write(text)
        os.chmod(path, mode)

    def read(self, *parts):
        with open(os.path.join(self.t, *parts)) as f:
            return f.read().strip()

    def capture(self, *args):
        r = subprocess.run(["sh", CAPTURE, *args], env=self.env, capture_output=True, text=True, timeout=60)
        return r.stdout + r.stderr

    TRIGGER = "stacktrace if prev_state & 2 && (prev_pid == 100 || prev_pid == 101)"

    def test_a_full_capture_takes_the_stack_of_each_compositor_thread_that_blocks(self):
        self.capture("start", "t", "full")
        self.assertEqual(self.read("events", "sched", "sched_switch", "trigger"), self.TRIGGER)
        self.assertTrue(os.path.exists(os.path.join(self.perf, ".kstack")))
        self.assertFalse(os.path.exists(os.path.join(self.t, "events", "sched", "sched_switch", "filter")), "full records every thread's switches")

    def test_a_kwait_capture_records_only_the_compositors_threads_and_the_events_the_waits_are_labelled_from(self):
        self.capture("start", "t", "kwait")
        self.assertEqual(self.read("events", "sched", "sched_switch", "filter"), "(prev_pid == 100 || prev_pid == 101) || (next_pid == 100 || next_pid == 101)")
        self.assertEqual(self.read("events", "sched", "sched_wakeup", "filter"), "pid == 100 || pid == 101")
        self.assertEqual(self.read("events", "sched", "sched_switch", "trigger"), self.TRIGGER)
        for e in ("sched/sched_switch", "sched/sched_wakeup", "drm/drm_vblank_event_delivered", "block/block_rq_issue"):
            self.assertEqual(self.read("events", e, "enable"), "1", e)

    def test_a_basic_capture_has_no_trigger_and_no_filter(self):
        self.capture("start", "t", "basic")
        self.assertFalse(os.path.exists(os.path.join(self.t, "events", "sched", "sched_switch", "trigger")))
        self.assertFalse(os.path.exists(os.path.join(self.perf, ".kstack")))

    def test_the_end_of_a_capture_takes_the_trigger_and_the_filters_off_again(self):
        self.capture("start", "t", "kwait")
        self.capture("stop", "t")
        self.assertEqual(self.read("events", "sched", "sched_switch", "trigger"), "!" + self.TRIGGER)
        self.assertEqual(self.read("events", "sched", "sched_switch", "filter"), "0")
        self.assertEqual(self.read("events", "sched", "sched_wakeup", "filter"), "0")
        self.assertFalse(os.path.exists(os.path.join(self.perf, ".kstack")))

    def test_with_no_compositor_it_says_there_is_nothing_to_take_stacks_of(self):
        open(os.path.join(self.tmp.name, "none"), "w").close()
        out = self.capture("start", "t", "full")
        self.assertIn("no compositor thread to take kernel stacks of", out)
        self.assertFalse(os.path.exists(os.path.join(self.perf, ".kstack")))

    def test_a_kernel_that_refuses_the_trigger_is_told_so_and_the_capture_goes_on(self):
        os.makedirs(os.path.join(self.t, "events", "sched", "sched_switch", "trigger"))   # a write to a directory fails
        out = self.capture("start", "t", "full")
        self.assertIn("no stacktrace trigger on this kernel: kernel waits are labelled from context", out)
        self.assertFalse(os.path.exists(os.path.join(self.perf, ".kstack")))
        self.assertEqual(self.read("tracing_on"), "1", "the capture itself started")


if __name__ == "__main__":
    unittest.main()
