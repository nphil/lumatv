"""Run with: python3 -m unittest tests.test_gate_lpc

The LPC 2026 pieces of the C4 gate (compositor/tools/tv-gate.sh) under sh, against a fake set: the loop's wake-up latency report, the
memory report, the synthetic background load and the scheduler-statistics line (`waits_line`) of the x1 check. The compositor's `ctl`
is a script that answers from files, the loader a script that runs its arguments, /proc a directory the test writes. The perf frame gate
(docs/research/perf/gate.py) is tested apart from the set: its load mode and the kernel-wait lines it prints under each surface.
"""
import contextlib
import io
import json
import os
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
GATE = os.path.join(ROOT, "compositor", "tools", "tv-gate.sh")

LOADER = '#!/bin/sh\nshift 2\nexec "$@"\n'
# The compositor's ctl: the reply for each command is a file named for it in $FAKE/ctl, every call is logged.
COMPOSITOR = r'''#!/bin/sh
# argv: ctl --runtime-dir DIR WORDS...
shift 3
echo "$*" >> "$FAKE/ctl-calls"
f="$FAKE/ctl/$(echo "$*" | tr ' ,' '__')"
[ -f "$f" ] && cat "$f" || echo '{"ok":true}'
'''
PGREP = '#!/bin/sh\necho 4242\n'
# A loadgen that prints its arguments and, told to stop, its summary.
LOADGEN = r'''#!/bin/sh
echo "args: $*"
trap 'echo "summary: decode 900 frames, ws 1200 messages, ble 600 wake-ups, download 48 MiB"; exit 0' TERM
while true; do sleep 0.1; done
'''
SLEEP = '#!/bin/sh\n/bin/sleep 0.05\n'


def write(path, text, mode=0o644):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(text)
    os.chmod(path, mode)


def read(path):
    with open(path) as f:
        return f.read()


class Fixture(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        d = self.tmp.name
        self.dep = os.path.join(d, "lumatv")
        self.out = os.path.join(d, "out")
        self.proc = os.path.join(d, "proc")
        self.fakebin = os.path.join(d, "fakebin")
        write(os.path.join(self.dep, "lib", "ld-linux-aarch64.so.1"), LOADER, 0o755)
        write(os.path.join(self.dep, "bin", "lumatv-compositor"), COMPOSITOR, 0o755)
        write(os.path.join(self.dep, "bin", "lumatv-loadgen"), LOADGEN, 0o755)
        write(os.path.join(self.fakebin, "pgrep"), PGREP, 0o755)
        write(os.path.join(self.fakebin, "sleep"), SLEEP, 0o755)
        os.makedirs(self.out)
        self.env = dict(os.environ, PATH=self.fakebin + os.pathsep + os.environ["PATH"], FAKE=d, LUMATV_D=self.dep, LUMATV_OUT=self.out,
                        LUMATV_STATE=os.path.join(d, "state"), LUMATV_PROC=self.proc)

    def tearDown(self):
        self.tmp.cleanup()

    def ctl_reply(self, words, doc):
        write(os.path.join(self.tmp.name, "ctl", words.replace(" ", "_").replace(",", "_")), json.dumps(doc, separators=(",", ":")))

    def ctl_calls(self):
        p = os.path.join(self.tmp.name, "ctl-calls")
        return read(p).split("\n")[:-1] if os.path.exists(p) else []

    def gate(self, *args):
        r = subprocess.run(["sh", GATE, *args], env=self.env, capture_output=True, text=True, timeout=60)
        return r.stdout + r.stderr

    def shell(self, script):
        r = subprocess.run(["sh", "-c", f". {GATE}\n{script}"], env=dict(self.env, GATE_LIB="1"), capture_output=True, text=True, timeout=60)
        return r.stdout + r.stderr


class Wake(Fixture):
    LINE = "tick p50 0.12 p99 0.80 max 3.10 ms (n 3600); flip p50 0.05 p99 0.40 max 1.20 ms (n 3600); input no samples; tick waited on a busy loop no samples"

    def test_the_report_is_the_loops_own_line_and_is_no_verdict(self):
        self.ctl_reply("wake", {"ok": True, "line": self.LINE})
        out = self.gate("wake").strip()
        self.assertEqual(out, "GATE wake: measured " + self.LINE)
        self.assertNotRegex(out, r"^GATE [^:]+: (PASS|FAIL)\b", "a report: tv_gate.py's verdict reads PASS and FAIL lines only")

    def test_reset_zeroes_the_distributions(self):
        self.assertEqual(self.gate("wake", "reset").strip(), "GATE wake: reset")
        self.assertIn("wake reset", self.ctl_calls())

    def test_a_probe_run_starts_the_classes_and_a_fifo_thread_on_cpu_0_and_on_cpu_1_reads_the_line_and_stops_them(self):
        self.ctl_reply("wake", {"ok": True, "line": self.LINE + "; probe fifo20 p50 0.06 p99 0.30 max 0.90 ms (n 10000)"})
        out = self.gate("wake", "probe", "5").strip()
        self.assertTrue(out.startswith("GATE wake: measured over 5 s with probes: tick p50 0.12"), out)
        self.assertIn("probe fifo20", out)
        calls = self.ctl_calls()
        self.assertEqual(calls[0], "wake reset")
        self.assertEqual(calls[1], "wake probe start fifo20,fifo19,other,fifo20@0,fifo20@1 1000")
        self.assertEqual(calls[-1], "wake probe stop")


class Memory(Fixture):
    def test_the_line_carries_what_is_pinned_and_what_the_set_has_left(self):
        self.ctl_reply("mem", {"ok": True, "rss_kib": 81920, "locked_kib": 61440, "swap_kib": 0, "lock": "locked what is touched in 0.8 ms",
                               "malloc": {"mmapped_blocks": 0, "arena_bytes": 25165824}})
        write(os.path.join(self.proc, "meminfo"), "MemTotal: 3000000 kB\nMemAvailable: 716800 kB\nCmaFree: 51200 kB\n")
        write(os.path.join(self.proc, "swaps"), "Filename Type Size Used Priority\n/dev/f2io-0 partition 1048576 262144 -2\n")
        write(os.path.join(self.proc, "4242", "limits"), "Limit Soft Limit Hard Limit Units\nMax locked memory 65536 65536 bytes\n")
        out = self.gate("mem").strip()
        self.assertTrue(out.startswith("GATE mem: measured rss 81920 KiB, locked 61440 KiB, swapped 0 KiB, malloc mmapped blocks 0; lock: locked what is touched"), out)
        self.assertIn("limit on locked memory: 65536 65536 bytes", out)
        self.assertIn("MemAvailable 700 MiB, CmaFree 50 MiB, swap in use 256 MiB", out)


class Load(Fixture):
    def test_start_runs_the_generator_in_the_background_and_stop_prints_what_it_did(self):
        out = self.gate("load", "start", "300")
        self.assertEqual(out.strip(), "GATE load: started for 300 s (decode,ws,ble,download)")
        self.assertEqual(self.gate("load", "status").strip().split(" (")[0], "GATE load: running")
        out = self.gate("load", "stop")
        self.assertTrue(out.startswith("GATE load: measured under it, summary: decode 900 frames, ws 1200 messages"), out)
        self.assertEqual(self.gate("load", "status").strip(), "GATE load: not running")
        self.assertNotRegex(out, r"^GATE [^:]+: (PASS|FAIL)\b")

    def test_the_arguments_reach_the_generator(self):
        self.gate("load", "start", "120", "ws,ble")
        self.gate("load", "stop")
        # the fake printed its arguments first, then its summary
        # (the log is the generator's whole output)
        log = read(os.path.join(self.out, "loadgen.log"))
        self.assertIn("args: run 120 --only ws,ble --download-mib 48", log)

    def test_a_second_start_does_not_start_a_second_generator(self):
        self.gate("load", "start", "300")
        self.assertEqual(self.gate("load", "start", "300").strip(), "GATE load: already running")
        self.gate("load", "stop")


class Waits(Fixture):
    def tasks(self, loop_stat, mali_stat):
        base = os.path.join(self.proc, "4242", "task")
        write(os.path.join(base, "4242", "comm"), "lumatv-composit\n")
        write(os.path.join(base, "4250", "comm"), "mali-cmar-backe\n")
        write(os.path.join(base, "4242", "schedstat"), loop_stat + "\n")
        write(os.path.join(base, "4250", "schedstat"), mali_stat + "\n")

    def test_the_mean_wait_of_a_wakeup_and_the_worst_waits_come_from_the_schedulers_own_numbers(self):
        self.tasks("1000000 2000000 10", "5000000 4000000 20")
        write(os.path.join(self.proc, "sys", "kernel", "sched_schedstats"), "0\n")
        # reset records the baseline; then the run: the loop waited 10 ms over 100 more slices, the Mali thread 1.2 ms over 12.
        self.shell("waits_reset")
        self.assertEqual(read(os.path.join(self.proc, "sys", "kernel", "sched_schedstats")).strip(), "1", "the statistics are switched on for the run")
        self.tasks("9000000 12000000 110", "9000000 5200000 32")
        sched = lambda b, w, io: (f"se.exec_start : 1.0\nse.statistics.wait_max : {w}\nse.statistics.block_max : {b}\n" f"se.statistics.iowait_count : {io}\n")
        write(os.path.join(self.proc, "4242", "task", "4242", "sched"), sched("12.500000", "4.200000", 3))
        write(os.path.join(self.proc, "4242", "task", "4250", "sched"), sched("0.000000", "1.100000", 0))
        out = self.shell("waits_line").strip()
        self.assertIn("loop: run-queue wait 100 us a wake-up on average, D-wait max 12.5 ms (3 I/O), run-queue wait max 4.2 ms", out)
        self.assertIn("mali-cmar-backe: run-queue wait 100 us a wake-up on average, D-wait max 0.0 ms (0 I/O), run-queue wait max 1.1 ms", out)
        self.assertEqual(read(os.path.join(self.proc, "sys", "kernel", "sched_schedstats")).strip(), "0", "and put back as found")

    def test_a_kernel_without_the_per_wait_statistics_says_so(self):
        self.tasks("0 0 1", "0 0 1")
        self.shell("waits_reset")
        self.tasks("0 500000 6", "0 0 1")
        out = self.shell("waits_line").strip()
        self.assertIn("loop: run-queue wait 100 us a wake-up on average, no per-wait statistics", out)

    def test_no_compositor_no_line(self):
        write(os.path.join(self.fakebin, "pgrep"), "#!/bin/sh\nexit 1\n", 0o755)
        self.assertEqual(self.shell("waits_reset; waits_line").strip(), "no compositor")


if __name__ == "__main__":
    unittest.main()


sys.path.insert(0, os.path.join(ROOT, "docs", "research", "perf"))
import gate  # noqa: E402


def late(at, kernel, cls="kbase", kernel_ms=14.0):
    waits = [{"class": cls, "comm": "lumatv-composit", "tid": 100, "hop": 0}] if kernel else []
    return {"at": at, "ms": 33.4, "kwait": {"ms": 33.4, "overrun_ms": 16.7, "kernel_ms": kernel_ms if kernel else 0.0, "score": 0.84 if kernel else 0.0,
                                           "kernel": kernel, "class": cls if kernel else None, "by_class": {cls: kernel_ms} if kernel else {},
                                           "waits": waits, "sched_ms": 0.0, "sched_behind": [], "verdict": "kernel" if kernel else "unexplained"}}


class FrameGate(unittest.TestCase):
    def result(self, gaps):
        return {"name": "home-dpad", "windows": [{"name": "home-dpad-open", "gaps": gaps}]}

    def test_each_late_frame_is_named_with_its_kernel_waits_and_a_total_counts_them_by_class(self):
        text = gate.kwait_lines([self.result([late(1.2, True), late(3.4, False), late(5.6, True, "mmap_sem", 9.0)])])
        lines = text.splitlines()
        self.assertEqual(lines[0], "home-dpad: kernel waits: 3 late frame(s): 2 kernel-caused (kbase 1, mmap_sem 1), 0 waiting for a CPU, 1 unexplained")
        self.assertEqual(len(lines), 4)
        self.assertIn("home-dpad-open +1.2 s: 33.4 ms (overrun 16.7): kernel wait 14.0 ms = 84 % of it: kbase 14.0 ms", lines[1], "the kernel-caused ones first, the biggest first")
        self.assertIn("not the kernel", lines[3])

    def test_a_surface_with_no_late_frame_prints_nothing(self):
        self.assertEqual(gate.kwait_lines([self.result([])]), "")
        self.assertEqual(gate.kwait_lines([{"name": "x", "windows": [{"name": "x-open", "gaps": [{"at": 1, "ms": 25}]}]}]), "", "a trace with no scheduler events has no attribution")

    def test_a_long_list_is_cut_and_says_how_many_more(self):
        text = gate.kwait_lines([self.result([late(float(i), True) for i in range(15)])], per_surface=4)
        self.assertTrue(text.endswith("... 11 more late frames"), text)

    def rig(self, started=True, summary="summary: decode 100 frames, ws 40 messages, ble 20 wake-ups, download 48 MiB"):
        rig = mock.Mock()
        rig.compositor_age_s.return_value = 1000
        rig.motion_state.return_value = {path: None for path in gate.MOTION_SWITCHES}
        rig.keys_read.return_value = (None, None)
        rig.panel_hz.return_value = None
        rig.load_start.return_value = started
        rig.load_stop.return_value = summary
        return rig

    def surface(self, name="home-dpad"):
        return {"name": name, "still": False}

    def test_the_load_starts_after_the_settle_and_ends_with_the_run_even_when_a_surface_raises(self):
        rig = self.rig()
        calls = []
        rig.wake.side_effect = lambda: calls.append("wake")
        rig.load_start.side_effect = lambda kinds, seconds: calls.append(("start", tuple(kinds), seconds)) or True
        rig.load_stop.side_effect = lambda: calls.append("stop") or "summary: x"
        ok = {"name": "home-dpad", "checks": [], "windows": []}
        runs = iter([ok, RuntimeError("the set went away")])

        def run_surface(*a, **k):
            r = next(runs)
            if isinstance(r, Exception):
                raise r
            return r

        out = io.StringIO()
        with mock.patch.object(gate, "run_surface", run_surface), contextlib.redirect_stdout(out), self.assertRaises(RuntimeError):
            gate.run_gate(rig, [self.surface(), self.surface("quick")], 3, "t", log=mock.Mock(), load=["decode", "ws"], load_seconds=900)
        self.assertEqual(calls, ["wake", ("start", ("decode", "ws"), 900), "stop"])
        self.assertIn("background load decode,ws: started", out.getvalue())
        self.assertIn("background load ended: summary: x", out.getvalue())

    def test_a_load_that_did_not_start_is_said_and_the_run_goes_on_without_it(self):
        rig = self.rig(started=False)
        out = io.StringIO()
        with mock.patch.object(gate, "run_surface", return_value={"name": "home-dpad", "checks": [], "windows": []}), contextlib.redirect_stdout(out):
            results = gate.run_gate(rig, [self.surface()], 3, "t", log=mock.Mock(), load=["ble"])
        self.assertEqual(len(results), 1)
        self.assertIn("DID NOT START", out.getvalue())
        rig.load_stop.assert_not_called()

    def test_no_load_asked_no_load_touched(self):
        rig = self.rig()
        with mock.patch.object(gate, "run_surface", return_value={"name": "home-dpad", "checks": [], "windows": []}), contextlib.redirect_stdout(io.StringIO()):
            gate.run_gate(rig, [self.surface()], 3, "t", log=mock.Mock())
        rig.load_start.assert_not_called()
        rig.load_stop.assert_not_called()

    def test_an_unknown_workload_is_refused_by_name(self):
        err = io.StringIO()
        with contextlib.redirect_stderr(err), self.assertRaises(SystemExit) as ex:
            gate.main(["--surfaces", "home-dpad", "--load", "decode,gpu"])
        self.assertEqual(ex.exception.code, 2)
        self.assertIn("unknown workload(s) gpu", err.getvalue())

    def test_the_loaders_commands_name_no_pattern_that_matches_their_own_shell(self):
        rig = mock.Mock()
        sent = []
        rig.sh.side_effect = lambda cmd, timeout=120: sent.append(cmd) or (0, "123\n", "")
        gate.Rig.load_start(rig, ["decode", "ws"], 900)
        gate.Rig.load_stop(rig)
        self.assertIn("--only decode,ws --download-mib 48", sent[1])
        for cmd in sent:
            if "pgrep" in cmd or "pkill" in cmd:
                self.assertNotIn("lumatv-loadgen", cmd, "pgrep -f and pkill -f match command lines, the shell's own included: " + cmd)


class CrashSummary(Fixture):
    def bundle(self, name, kind, pending):
        root = os.path.join(self.tmp.name, "crash")
        write(os.path.join(root, name, "reason.txt"), f"reason: compositor killed by signal 9\nkind: {kind}\n")
        write(os.path.join(root, name, "compositor.log.tail"), "[srv 1.000] info a\n[srv 2.000] info b\n")
        write(os.path.join(root, name, "trace.txt"), "1.000 F flip\n2.000 D frame\n3.000 S ticks\n")
        write(os.path.join(root, "latest"), name)
        if pending:
            write(os.path.join(root, "pending"), name)
        self.env["LUMATV_CRASH"] = root

    def test_the_wd_checks_name_the_bundle_the_supervisor_left(self):
        self.bundle("20261028-031244-001", "problem", True)
        out = self.shell("crash_summary").strip()
        self.assertEqual(out, "20261028-031244-001 (pending yes; 2 log lines, 3 ring lines, kind problem)")

    def test_an_escape_is_saved_but_not_pending(self):
        self.bundle("20261028-031244-002", "escape", False)
        self.assertIn("pending no", self.shell("crash_summary"))
        self.assertIn("kind escape", self.shell("crash_summary"))

    def test_no_bundle_says_none(self):
        self.env["LUMATV_CRASH"] = os.path.join(self.tmp.name, "nothing")
        self.assertEqual(self.shell("crash_summary").strip(), "none")
