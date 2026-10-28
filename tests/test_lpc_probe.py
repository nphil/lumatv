"""Run with: python3 -m unittest tests.test_lpc_probe

compositor/tools/tv-lpc-probe.sh reads the set's /proc and /sys and writes nothing (docs/research/lpc-tv-result.md, items 3, 5 and 7). Here it reads a
fake tree, with a `sleep` that swaps in the second snapshot of every file that has one (`NAME.2`), and a `pgrep` that knows the three processes.
"""
import os
import re
import shutil
import stat
import subprocess
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.path.join(HERE, "..", "compositor", "tools", "tv-lpc-probe.sh")

INTERRUPTS = """           CPU0       CPU1       CPU2       CPU3
 18:         10          0          0          0     GICv3  30 Level     arch_timer
 38:       1000          0          0          0     GICv3  64 Level     ohci_hcd:usb1
 39:        500          0          0          0     GICv3  65 Level     xhci-hcd:usb2
 52:       3000       2000          0          0     GICv3  80 Level     DE2VSYNC
 53:        100          0          0          0     GICv3  81 Level     osd_irq
 60:          0          0          0          0     GICv3  90 Level     mali
 70:         50         50          0          0     GICv3  91 Level     eth0
IPI0:        100        100        100        100  Rescheduling interrupts
"""
INTERRUPTS2 = """           CPU0       CPU1       CPU2       CPU3
 18:         15          0          0          0     GICv3  30 Level     arch_timer
 38:       1600          0          0          0     GICv3  64 Level     ohci_hcd:usb1
 39:        500          0          0          0     GICv3  65 Level     xhci-hcd:usb2
 52:       3000       2660          0          0     GICv3  80 Level     DE2VSYNC
 53:        100          0          0          0     GICv3  81 Level     osd_irq
 60:          0          0          0          0     GICv3  90 Level     mali
 70:         50         50          0          0     GICv3  91 Level     eth0
IPI0:        100        100        100        100  Rescheduling interrupts
"""
SOFTIRQS = """                    CPU0       CPU1       CPU2       CPU3
          HI:          0          0          0          0
       TIMER:       1000        500          0          0
      NET_RX:        200          0          0          0
       BLOCK:          5          0          0          0
"""
SOFTIRQS2 = SOFTIRQS.replace("1000        500", "1100        550").replace("200          0", "230          0")


def stat_line(cpu):
    return "4242 (lumatv-composit) S " + " ".join(["0"] * 35) + " %d 0 0\n" % cpu


def write(root, rel, text):
    path = os.path.join(root, rel)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(text)
    return path


def tree(root):
    """The set as the probe sees it: before, and `.2` for after the sleep."""
    proc = os.path.join(root, "proc")
    write(proc, "interrupts", INTERRUPTS)
    write(proc, "interrupts.2", INTERRUPTS2)
    write(proc, "softirqs", SOFTIRQS)
    write(proc, "softirqs.2", SOFTIRQS2)
    write(proc, "irq/default_smp_affinity", "f\n")
    for irq, aff in ((18, "0-3"), (38, "0"), (39, "0-3"), (52, "0-1"), (60, "1-3"), (70, "0-3")):
        write(proc, "irq/%d/smp_affinity_list" % irq, aff + "\n")
    write(proc, "4242/task/4242/stat", stat_line(1))
    write(proc, "4242/task/4242/stat.2", stat_line(0))
    # threads: the compositor's loop and a helper thread of it, its shell client (a child), the helper; "slices" is the third schedstat field
    # (two threads of the shell client share a name, as the two QNetworkAccessManager threads do on the set: one with a count of 9,000 runs that grows by
    # 10 and one with 100 that does not)
    for pid, tid, comm, before, after in ((4242, 4242, "lumatv-composit", 1000, 1620), (4242, 4243, "shell-http", 40, 44),
                                          (4300, 4300, "qml-runner", 500, 510), (4300, 4310, "QNetworkAccessM", 9000, 9010),
                                          (4300, 4311, "QNetworkAccessM", 100, 100), (777, 777, "lumatv-helper", 70, 190)):
        write(proc, "%d/task/%d/comm" % (pid, tid), comm + "\n")
        write(proc, "%d/task/%d/schedstat" % (pid, tid), "1 2 %d\n" % before)
        write(proc, "%d/task/%d/schedstat.2" % (pid, tid), "1 2 %d\n" % after)
    # CPU time and preempted runs of two threads: the compositor's loop (6 a second preempted of 62) and the helper's (all 12 of its 12, 5 % of a CPU; its
    # name holds a space, as "Qt bearer threa" does)
    write(proc, "4242/task/4242/status", "nonvoluntary_ctxt_switches:\t500\n")
    write(proc, "4242/task/4242/status.2", "nonvoluntary_ctxt_switches:\t560\n")
    write(proc, "777/task/777/stat", "777 (lumatv helper) S 1 777 777 0 -1 0 0 0 0 0 100 0 0 0 20 0 1 0 1 2 3\n")
    write(proc, "777/task/777/stat.2", "777 (lumatv helper) S 1 777 777 0 -1 0 0 0 0 0 120 30 0 0 20 0 1 0 1 2 3\n")
    write(proc, "777/task/777/status", "nonvoluntary_ctxt_switches:\t600\n")
    write(proc, "777/task/777/status.2", "nonvoluntary_ctxt_switches:\t720\n")
    write(proc, "stat", "cpu  1 2 3 4\nintr 1000000 5 6\nctxt 5000000\n")
    write(proc, "stat.2", "cpu  1 2 3 4\nintr 1042000 5 6\nctxt 5090000\n")
    write(proc, "4242/maps", "".join("%08x-%08x rw-p 00000000 00:00 0\n" % (i * 4096, i * 4096 + 4096) for i in range(37)))
    write(proc, "4242/stat", "4242 (lumatv-composit) S 1 4242 4242 0 -1 4194560 3000 0 12 0 5 6 0 0 20 0 12 0 1 2 3\n")
    write(proc, "4242/limits", "Limit                     Soft Limit           Hard Limit           Units\n"
                               "Max address space         unlimited            unlimited            bytes\n"
                               "Max locked memory         67108864             67108864             bytes\n"
                               "Max realtime priority     0                    0\n")
    write(proc, "4242/status", "Name:\tlumatv-composit\nVmHWM:\t  120000 kB\nVmRSS:\t  110000 kB\nVmLck:\t   90000 kB\nVmSwap:\t    1200 kB\nThreads:\t12\nCapEff:\t0000000000004000\n")
    write(proc, "meminfo", "MemTotal:        1800000 kB\nMemFree:          100000 kB\nMemAvailable:     700000 kB\nSwapTotal:        500000 kB\nSwapFree:         400000 kB\n"
                            "Mlocked:           90000 kB\nCmaTotal:         200000 kB\nCmaFree:           50000 kB\nBuffers:            1000 kB\n"
                            "SwapCached:         2000 kB\nShmem:            150000 kB\nUnevictable:        5000 kB\nAnonPages:        600000 kB\nMapped:            90000 kB\n")
    write(proc, "swaps", "Filename\tType\tSize\tUsed\tPriority\n/dev/zram0\tpartition\t500000\t100000\t-2\n")
    # what holds the swap: the three LumaTV processes (4242 has 1,200 kB of it, the shell client 30,000, the helper 400), a web app, the surface manager and a
    # kernel thread with none; the kernel's swap counters move by 100, 500 and 1,000 in the run
    write(proc, "4300/status", "Name:\tqml-runner\nVmRSS:\t   60000 kB\nVmSwap:\t   30000 kB\n")
    write(proc, "777/status", "Name:\tlumatv-helper\nVmRSS:\t    1600 kB\nVmSwap:\t     400 kB\n")
    write(proc, "100/status", "Name:\twebapp\nVmRSS:\t   90000 kB\nVmSwap:\t   40000 kB\n")
    write(proc, "101/status", "Name:\tsurface-manager\nVmRSS:\t   70000 kB\nVmSwap:\t   25000 kB\n")
    write(proc, "102/status", "Name:\tkworker\n")
    write(proc, "vmstat", "pgmajfault 200\npswpin 1000\npswpout 5000\npgpgin 7\n")
    write(proc, "vmstat.2", "pgmajfault 1200\npswpin 1100\npswpout 5500\npgpgin 9\n")
    write(os.path.join(root, "sys"), "block/zram0/disksize", "524288000\n")
    write(os.path.join(root, "sys"), "block/zram0/comp_algorithm", "[lz4] lzo\n")
    write(os.path.join(root, "sys"), "block/zram0/mm_stat", "104857600 31457280 33554432 0 0 0 0 0\n")
    os.makedirs(os.path.join(root, "tmp", "lumatv"))
    with open(os.path.join(root, "tmp", "lumatv", "bundle.bin"), "wb") as f:
        f.write(os.urandom(300 * 1024))         # random: a filesystem that compresses would count a run of one byte as nothing
    write(root, "tmp/xdg/token", "t")
    for name, val in (("vm/swappiness", 60), ("vm/overcommit_memory", 0), ("kernel/sched_rt_runtime_us", 950000), ("kernel/sched_schedstats", 1)):
        write(proc, "sys/" + name, "%s\n" % val)
    sysdir = os.path.join(root, "sys")
    write(sysdir, "devices/system/cpu/online", "0-1,3\n")
    # the fake commands: pgrep knows the three processes; sleep swaps the second snapshot in, once
    bindir = os.path.join(root, "bin")
    os.makedirs(bindir)
    pgrep = write(bindir, "pgrep", "#!/bin/sh\ncase \"$*\" in\n  *bin/lumatv-compositor*) echo 4242 ;;\n  \"-P 4242\") echo 4300 ;;\n  *'lumatv-helper start'*) echo 777 ;;\nesac\n")
    sleep = write(bindir, "sleep", "#!/bin/sh\n[ -e \"$LUMATV_PROC/.swapped\" ] && exit 0\ntouch \"$LUMATV_PROC/.swapped\"\n"
                                    "for f in $(find \"$LUMATV_PROC\" -name '*.2'); do mv -f \"$f\" \"${f%.2}\"; done\n")
    for p in (pgrep, sleep):
        os.chmod(p, os.stat(p).st_mode | stat.S_IXUSR)
    return proc, sysdir, bindir


class LpcProbe(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp()
        self.proc, self.sys, self.bin = tree(self.root)
        self.addCleanup(shutil.rmtree, self.root, True)

    def run_probe(self, *args):
        env = dict(os.environ, LUMATV_PROC=self.proc, LUMATV_SYS=self.sys, LUMATV_TMP=os.path.join(self.root, "tmp"), PATH=self.bin + os.pathsep + os.environ["PATH"])
        r = subprocess.run(["sh", SCRIPT, *args], env=env, capture_output=True, text=True, timeout=60)
        self.assertEqual(r.returncode, 0, r.stderr)
        return r.stdout

    def test_the_script_parses_and_help_prints_its_own_header(self):
        self.assertEqual(subprocess.run(["sh", "-n", SCRIPT]).returncode, 0)
        out = self.run_probe("help")
        self.assertIn("Read-only probes for the LPC 2026 round", out)
        self.assertIn("lpc-probe.sh irq [SECS] [LABEL]", out)
        self.assertNotIn("PROC=", out)

    def test_irq_lists_the_busiest_first_with_each_cpu_and_the_cpus_it_may_use(self):
        out = self.run_probe("irq", "1", "home").splitlines()
        self.assertTrue(out[0].startswith("lpc-probe irq: 1 s, home; CPUs online 0-1,3; default affinity f"), out[0])
        rows = [l for l in out if l[:1] in "* " and l.strip() and l.split()[0].lstrip("*").isdigit()]
        names = [l.split()[-1] for l in rows]
        # the vsync's 660 on CPU1 in a second, OHCI's 600 on CPU0, the timer's 5 after them; the ones that are quiet are starred
        self.assertEqual(names[:2], ["DE2VSYNC", "ohci_hcd:usb1"], rows)
        vsync = rows[0].split()
        self.assertEqual(vsync[0], "*52")
        self.assertEqual(vsync[1], "660.0")
        self.assertEqual(vsync[2:6], ["0", "660", "0", "0"])
        self.assertEqual(vsync[6], "0-1")
        ohci = rows[1].split()
        self.assertEqual(ohci[0], "*38")
        self.assertEqual(ohci[1], "600.0")
        self.assertEqual(ohci[2:6], ["600", "0", "0", "0"])
        self.assertEqual(ohci[6], "0")                      # OHCI is bound to CPU0
        self.assertIn("arch_timer", names)                  # it fired five times
        timer = [l for l in rows if "arch_timer" in l][0]
        self.assertEqual(timer[0], " ")                     # not one of the named
        quiet = {l.split()[-1]: l for l in rows if l.startswith("*")}
        for name in ("xhci-hcd:usb2", "mali", "eth0", "osd_irq"):      # named lines are shown even when nothing fired
            self.assertIn(name, quiet)
            self.assertEqual(quiet[name].split()[1], "0.0")
        self.assertNotIn("Rescheduling", "\n".join(rows))   # the IPI row did not move

    def test_irq_adds_the_softirqs_and_where_the_loop_thread_ran(self):
        out = self.run_probe("irq", "1", "stream")
        self.assertRegex(out, r"softirq TIMER\s+100\s+50\s+0\s+0")
        self.assertRegex(out, r"softirq NET_RX\s+30\s+0\s+0\s+0")
        self.assertNotIn("softirq HI", out)
        line = [l for l in out.splitlines() if l.startswith("loop thread")][0]
        self.assertIn("(pid 4242) ran on:", line)
        self.assertIn("CPU1 10 %", line)                    # the first of the ten samples, before the sleep that swapped the files
        self.assertIn("CPU0 90 %", line)
        self.assertTrue(line.endswith("over 10 samples"), line)

    def test_wakeups_counts_each_thread_of_the_compositor_its_client_and_the_helper_a_second(self):
        out = self.run_probe("wakeups", "10", "idle-home")
        lines = out.splitlines()
        self.assertEqual(lines[0], "lpc-probe wakeups: 10 s, idle-home")
        by_pid = {l.split()[1].rstrip(":"): l for l in lines if l.startswith("pid ")}
        self.assertEqual(sorted(by_pid), ["4242", "4300", "777"])
        self.assertIn("pid 4242: 62.4 runs/s, 0.0 % CPU:", by_pid["4242"])        # 620 + 4 runs in ten seconds
        self.assertIn("lumatv-composit[4242] 62.0,", by_pid["4242"])                # 6 a second preempted of 62: not worth a word
        self.assertNotIn("shell-http", by_pid["4242"])                              # 0.4 a second and no CPU: left out of the line, kept in the total
        self.assertIn("pid 4300: 2.0 runs/s, 0.0 % CPU:", by_pid["4300"])
        self.assertIn("qml-runner[4300] 1.0,", by_pid["4300"])
        self.assertIn("pid 777: 12.0 runs/s, 5.0 % CPU:", by_pid["777"])
        self.assertIn("lumatv-helper[777] 12.0 (5.0 % CPU, 100 % preempted),", by_pid["777"])
        self.assertEqual(lines[-1], "system: 4200 interrupts/s, 9000 context switches/s")

    def test_two_threads_with_one_name_are_each_read_from_their_own_count(self):
        # The first reading of this probe kept one count a name: the thread whose count is 9,000 was given the other's 100 as its start and read 891 runs a
        # second for 1, as the shell client's network thread read 592 on the set while its servers were asked once a second.
        out = self.run_probe("wakeups", "10", "twins")
        line = [l for l in out.splitlines() if l.startswith("pid 4300:")][0]
        self.assertIn("QNetworkAccessM[4310] 1.0,", line)
        self.assertNotIn("891", out)
        self.assertNotIn("QNetworkAccessM[4311]", line, "the one that did not run is not listed")
        self.assertIn("2.0 runs/s", line, "and the process's total is the sum of the true ones")

    def test_swap_says_who_holds_what_the_swap_has_and_what_no_process_holds(self):
        out = self.run_probe("swap", "10", "full")
        lines = out.splitlines()
        self.assertEqual(lines[0], "lpc-probe swap: 10 s, full; swap used 100000 kB of 500000 kB (20%), held by processes 96600 kB, "
                                   "by no process (tmpfs and shared memory, the kernel) 3400 kB")
        self.assertIn("/dev/zram0", out)
        self.assertIn("zram0: disksize 524288000, algorithm [lz4] lzo, mm_stat (original, compressed, memory used) 104857600 31457280 33554432", out)
        self.assertIn("pages moved in 10 s: swapped in 10 a second, swapped out 50 a second, major faults 100 a second", out)
        for k, v in (("SwapCached", 2000), ("Shmem", 150000), ("Mlocked", 90000), ("Unevictable", 5000), ("AnonPages", 600000)):
            self.assertIn("%s: %d kB" % (k, v), out)
        self.assertIn("LumaTV (compositor, shell client, helper) holds 31600 kB of the swap", out)
        holders = out.split("the biggest holders (kB swapped, kB resident):\n")[1].split("the biggest of")[0].splitlines()
        self.assertEqual([h.split()[4] for h in holders], ["webapp", "qml-runner", "surface-manager", "lumatv-composit", "lumatv-helper"], holders)
        self.assertEqual(holders[0].split()[:2], ["40000", "90000"])
        self.assertTrue(all(h.endswith("[LumaTV]") for h in holders if "qml-runner" in h or "composit" in h or "helper" in h))
        self.assertFalse(any("[LumaTV]" in h for h in holders if "webapp" in h or "surface" in h))
        tmp = out.split("tmpfs pages are swapped out with nobody's name on them):\n")[1].splitlines()
        # which is bigger is du's word and a compressing filesystem (the workstation's) gives a new file no blocks yet: the names are what is asserted
        self.assertEqual(sorted(t.split()[-1].rsplit("/", 1)[1] for t in tmp), ["lumatv", "xdg"], tmp)

    def test_limits_reads_what_the_compositor_may_lock_and_what_the_set_has_left(self):
        out = self.run_probe("limits")
        self.assertIn("lpc-probe limits: compositor pid 4242", out)
        self.assertIn("Max locked memory 67108864 67108864 bytes", out)
        self.assertIn("VmLck: 90000 kB", out)
        self.assertIn("maps: 37", out)
        self.assertIn("faults: minflt 3000, majflt 12", out)
        self.assertIn("CapEff: 0000000000004000", out)
        self.assertIn("MemAvailable: 700000 kB", out)
        self.assertIn("CmaFree: 50000 kB", out)
        self.assertIn("/dev/zram0", out)
        self.assertIn("vm/swappiness: 60", out)
        self.assertIn("kernel/sched_rt_runtime_us: 950000", out)
        self.assertNotIn("Buffers", out)                    # only the lines that decide whether a lock fits

    def test_it_changes_nothing_it_reads(self):
        def files():
            found = {}
            for dp, _, names in os.walk(self.root):
                for f in names:
                    with open(os.path.join(dp, f), "rb") as fh:
                        found[os.path.join(dp, f)] = fh.read()
            return found
        before = files()
        for args in (("irq", "1", "home"), ("wakeups", "1"), ("limits",), ("swap", "1")):
            self.run_probe(*args)
        after = files()
        after.pop(os.path.join(self.proc, ".swapped"))
        # the fake sleep moved each `.2` file over its first one and nothing else happened to the tree: no file made, none written
        moved = {k for k in before if k.endswith(".2")}
        self.assertEqual(set(after), {k for k in before if k not in moved})
        for k, text in after.items():
            self.assertEqual(text, before.get(k + ".2", before[k]), k)

    def test_the_script_has_no_command_that_writes_the_set(self):
        with open(SCRIPT) as fh:
            body = "\n".join(l for l in fh.read().splitlines() if not l.lstrip().startswith("#"))
        for bad in ("sysctl", "taskset", "chrt", "renice", "irqbalance", "mount ", "kill", "echo 0", "echo f"):
            self.assertNotIn(bad, body)
        # the redirections that write: the script's own four temporary files and the bit bucket (the rest are awk's comparisons)
        targets = set(re.findall(r">\s*([^\s;|)&]+)", body))
        self.assertEqual(targets - {"$a", "$b", "$sa", "$sb", "/dev/null", "0", "1", "="}, set(), targets)


if __name__ == "__main__":
    unittest.main()
