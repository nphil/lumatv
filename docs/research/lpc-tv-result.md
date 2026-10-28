# Linux Plumbers 2026 on the TV: what is applied, what the next round measures

Offline work of 2026-10-28 from LumaShow's notes (`LumaShow/notes/handoff/LPC2026.md`, `SmoothAudit.md` "LPC 2026 Kernel Lock
Contention..."). Nothing here has run on the set: every effect below is expected, not observed, and facts marked [INFERENCE]
were not seen on it. Host proof: `cargo test -p lumatv-compositor` (also `--features ui`), clippy, fmt, `tests/test_cutover`,
`tests/test_gate`, the new `tests/test_kwait`, `tests/test_gate_lpc`, `tests/test_lpc_probe`, the helper's `crash` tests and
the settings host (`tests/test_settings_coverage`). Already on the set before this round (`compositor-gate9-result.md`): loop
SCHED_FIFO 20, Mali submission threads FIFO 19, shell client SCHED_RESET_ON_FORK, the pacer's late-stamp fix.

| # | Item | State | Where |
|---|------|-------|-------|
| 1 | Wake-up latency per run | built | `src/wake.rs`; `ctl wake`; gate lines `wake` |
| 2 | Kernel wait behind a late frame | built | `docs/research/perf/kwait.py`, `capture.sh kwait`; `tv_gate.py --kwait` |
| 3 | No mmap/munmap/mprotect on the frame path | built, host-proven | `src/mem.rs`, `render/gl.rs`; `ctl mem`; `lpc-probe.sh limits` |
| 4 | Gate under background load | built | `bin/lumatv-loadgen`; `gate.sh load`; `tv_gate.py --frames-load` |
| 5 | Idle wake-ups | compositor built; helper census only | `src/ticker.rs`; `ctl idle`; `lpc-probe.sh wakeups` |
| 6 | Crash evidence bundle and Settings notice | built | `sm/lumatv-sm.c`, `src/ring.rs`, `helper/app/src/crash.rs`, `PageSupport.qml` |
| 7 | IRQ placement | probe and proposal only | `tools/tv-lpc-probe.sh irq`; nothing changed |

## 1. Wake-up latency

The loop records, per wake-up, how late it ran against the moment it could first have run: the frame timer (`tick`), the
page flip's event (`flip`, against the kernel's vblank stamp) and key or pointer events (`input`, against the evdev stamp,
now taken on the monotonic clock with `EVIOCSCLOCKID`). Work the loop was still doing on an earlier pass is not latency: it
is counted from the end of that pass and the wait goes in `busy`. Percentiles come from a log-linear histogram (about 6 %
wide at worst), the maximum is exact. `ctl wake` prints `tick p50 .. p99 .. max .. ms (n ..); flip ..; input ..; tick waited
on a busy loop ..` (and one `probe` part per class while a probe runs); `ctl wake reset` clears it.

A cyclictest-style probe (`ctl wake probe start fifo20,fifo19,other 1000`, `... stop`) runs one sleeping thread per class
and records what the scheduler gives each under the load of the moment: the loop's class, the Mali blob's, a client's. A
class takes a CPU after an `@` (`fifo20@0` against `fifo20@1`): the thread is kept there, which shows whether the CPU that
serves the USB host's interrupt wakes a real-time thread later than another; a CPU that does not exist or is offline gives
`no samples`. `gate.sh wake probe SECS` starts the three classes and both of those.

In the gate: the x1 detail carries `wake-up latency ...` and `waits ...` (the loop's mean run-queue wait and the longest
block, from schedstat; `gate.sh` turns `kernel.sched_schedstats` on for the run and restores it). `gate.sh wake [reset |
probe SECS | LABEL]` prints `GATE wake: measured ...`; `tv_gate.py` resets before the frame run and prints `wake` and `mem`
after it. These lines carry no verdict (`all_passed` ignores `measured`): there is no baseline, so the first rounds record
and the rule is set from three of them.

## 2. Kernel waits behind a late frame

`capture.sh kwait` (and `full`) set a `stacktrace` trigger on `sched_switch` when the previous state is uninterruptible
sleep for the compositor's threads. `kwait.py` finds each D-state interval of 3 ms or more on the loop thread or on a thread
it waits for through up to three wake-up hops, labels it from the kernel stack (`kbase` for the Mali driver, `mmap_sem`,
`swap`, `io`, `fence`, `futex`, `mutex`, `cgroup`, `memory`; a label from the events around it, when no stack is there, ends
in `?`), scales it by its share of the overrun and calls the frame kernel-caused at a score of 0.5 or more. `analyse.py`
prints it per window and per late frame; the frame gate's full captures get it by default, the 60 s x1 run only with
`tv_gate.py --kwait` (tracing would perturb the zero-gap rule's own run).

On a kernel that refuses the trigger the capture says so and the labels fall back to context [INFERENCE: 5.4.268's tracefs
accepts it; first use on the set will show].

## 3. mmap, munmap and mprotect on the frame path

On the set's 5.4 kernel each of them, and every page fault, takes the process's one `mmap_sem`; a loop trimming its heap
queues behind a thread that is paging a page in. What the compositor now does:

- `mallopt`: `M_MMAP_MAX 0` (no mmap for large blocks), `M_TRIM_THRESHOLD 256 MiB` (no trim), `M_TOP_PAD 8 MiB` (the heap
  grows in 8 MiB steps); `pregrow` touches 16 MiB at start, one 1080p ARGB upload (8.3 MB) and room for the scene, so the
  first frames find their room mapped. Other arenas grow by `mprotect`, but nothing on the loop thread allocates in them.
- `mlockall(MCL_CURRENT | MCL_FUTURE | MCL_ONFAULT)`: the lock follows pages in as they are touched. A plain `mlockall`
  would fault in the whole Mali blob and every thread's 8 MiB stack on a set with about 700 MB available. It is taken only
  with `CAP_IPC_LOCK` or `RLIMIT_MEMLOCK` of 128 MiB or more, and `MemAvailable` of 256 MiB or more; otherwise the log and
  `ctl mem` say why not. `--mlock off|auto` (auto by default).
- The shm upload built a `Vec` of the damaged rows per frame, up to 8.3 MB: above glibc's mmap threshold, so an mmap and a
  munmap (or a heap grown and trimmed) a frame. It is now one reused scratch (`upload_rows`, `pack_shm_rows`).

Audit of what else the loop asks of the kernel per frame: scene, damage and journal structures are small (heap, now never
trimmed); control replies are small and made when asked; the trace ring is one mapping made at start; the probe threads exist
only while a probe runs (a thread costs one mmap and one mprotect, measured). Not covered on the host: the Mali blob's own
mappings through kbase when a client's buffer is imported or buried; `lpc-probe.sh limits` reports the compositor's mapping
count and fault counts, and a count that grows through a gate run names what is left.

Host proof (`tests/frame_syscalls.rs`, a seccomp trap that counts the five calls and `brk`): malloc at its defaults, five
temporary buffers of 1 to 12 MiB: mmap x5, munmap x5; tuned and pre-grown: none; the server thread over 300 warm frames
drawn: none; a thread made and joined: mmap x1, mprotect x1. `ctl mem` reports RSS, high-water mark, `VmLck`, swap, the
malloc counters (mmapped blocks stay 0), `MemAvailable`, the limit, `CAP_IPC_LOCK` and the lock's line.

## 4. Background load

`lumatv-loadgen run SECS [--only KINDS] [--decode-ms N] [--download-mib N] [--tmp DIR]` stands in for what runs beside Home
on the set, each workload on a thread of its own: a camera-like decode (8 ms of work over a 3 MiB frame every 33 ms), Home
Assistant websocket chatter (a 400-byte message every 25 ms over loopback TCP: wake-ups, socket and softirq work), a BLE scan
stand-in (20 bursts of 300 us a second) and a download at SCHED_IDLE and the idle I/O class (about 16 MiB/s over loopback into
a file under `--tmp`, tmpfs on the TV, bounded to 64 MiB or a quarter of what is free). `gate.sh load start|stop|status` runs
it from the bundle; `tv_gate.py --frames-load` adds a second frame run under it (or `--only frames-load`), and the perf
harness takes `--load KINDS --load-seconds N`. The loaded run is a report (`GATE frames-load: measured ...`), not a rule:
the comparison with the unloaded run is the finding.

## 5. Idle wake-ups

LPC's Echo woke 474 times a second at idle. Census of what wakes on the TV, from the code (the probe reads the same from the
scheduler):

| Process | Timer | Per second |
|---------|-------|-----------|
| compositor | frame tick at the panel's rate, regardless of work (before) | 60 |
| compositor | frame tick when nothing has happened for 250 ms (input: 3 s), output idle (after) | 10 |
| compositor | with Home visible: the shell's 10 Hz wallpaper commit, each one waking commit, tick, flip [INFERENCE] | 35 to 50 |
| supervisor | heartbeat, the watchdog; unchanged | 2 |
| shell client | 1 s report timer; 10 Hz wallpaper while Home shows, held while covered | 1 to 10 |
| helper | Denon poll 2 s, framerate poll 2 s (250 ms while video plays), updateblock and cameras 10 s, preload and hub 30 s, screen and AI listener 60 s | about 2 |
| helper | ambient strip 33 ms (only while the strip is on) | 30 |

The compositor's slowed tick (`src/ticker.rs`) takes an idle, covered or still Home from 60 to 10 wake-ups a second, and any
event (a commit, a key, the shell's channel, a flip) draws at once and puts the tick back at the panel's rate; host-proven
by timer expirations (`tests/idle_tick.rs`: 10 against 60 a second). `ctl idle [MS]` and `--idle-tick-ms N` set it (0 is the
old 60 Hz), for an A/B on the set.

What the set itself recorded in round 7's frame gate (`compositor/slot-2026-10-28-0300/gate-full/frames.json`, not from
this work's probes): over the 8 s window `home-dpad-open` the four CPUs took 4,691 `arch_timer`, 225 `IPI`, 121 `DE2VSYNC`,
120 `osd_irq` (a 120 Hz panel), 66 gpu, 64 OHCI and 38 eth0 interrupts a second, with Home animating. In its 93 gaps (33 ms
to 1,018 ms, median 567 ms) the interrupt handlers took 2.1 to 6.5 % of the gap, median 2.4 %, and softirqs 1.6 % more at the
median; `arch_timer` was 84 % of the handler time at the median. The compositor's old tick was about 1 % of such a window's timer
interrupts, so the slowed tick alone moves the system's count little; the rest is LG's own services and the shell client, which
`lpc-probe.sh wakeups` attributes per thread at an idle Home.

The helper's timers sum to a couple of wake-ups a second, so coalescing them (a process-wide `PR_SET_TIMERSLACK`) would save
less than one and was left out: the same process paces the ambient strip (33 ms), the Hue stream (30 ms) and the remote's key
holds, which a 50 ms slack would blur. `lpc-probe.sh wakeups` prints runs a second per thread for the compositor, its shell
client and the helper, and the system's interrupts and context switches, to settle it.

## 6. Crash evidence

The compositor keeps a text ring of its last seconds in a file in its runtime directory (`lumatv-trace.ring`, 192 KiB, mmap
shared: the kernel owns the page cache, so it outlives a crash; roughly the last 40 s of a 60 fps stretch [INFERENCE]). It
holds the log, passes over 2 ms (`P`), late wake-ups (`W`), flips (`F`), frames (`D`), a 1 Hz snapshot with RSS (`S`) and
panics. `ctl ring [N]` prints it; `--ring off|auto|PATH`.

When the supervisor falls back with the compositor having failed, it writes `/var/lib/lumatv/crash/<YYYYMMDD-HHMMSS>-<NNN>/`:
`reason.txt`, `compositor.log.tail` (200 lines), `sm.log.tail`, `trace.txt` (the ring, oldest line first), `system.txt`.
At most about 330 KB each (two log tails of 200 lines and 64 KiB at most, the 192 KiB ring); the newest 8 are kept, older ones
removed by name. `latest` names the newest and `pending` the one nobody has read. An escape (Home held) is saved too but is
no problem and sets no `pending`.

The helper reads it on demand, nothing polls: `GET /v1/crash` and `POST /v1/crash/ack`. Settings > Support shows
"LumaTV recovered from a problem" at the top while a problem is pending, with a Details row that says why and when, after
how long, and where the log and trace are, and answers Close or Clear this notice. Clearing removes `pending` only; the
bundle stays. The gate's `wd-hang` and `wd-crash` lines end with `crash bundle <name> (pending yes; N log lines, M ring
lines, kind)`.

## 7. IRQ placement (probe and proposal; nothing changed)

Known: the USB host (OHCI) interrupt is served on CPU 0; LG hot-unplugs CPU 2 and 3 and its power manager offlines CPU 3 at
Home, so Home runs on CPUs 0 and 1 at times; the loop is FIFO 20 and the Mali submission threads FIFO 19, so an interrupt on
the loop's CPU is serviced first and the loop resumes after; the gamepad reader of the GFN app is already kept off CPU 0.
Round 7's capture (above) says how much one costs: OHCI fired 64 times a second with nothing typed at a USB device by the gate
[INFERENCE: a polled input device] and took up to 1.4 ms of handler time in a gap of 1,000 ms, about 20 us an interrupt; vsync
and osd 120 a second each at 0.1 to 0.8 ms. No gap of the round was an interrupt's doing (at most 6.5 % of any gap), so
placement is the lowest-yield item here and stays a probe.

`lpc-probe.sh irq [SECS] [LABEL]` reads, writes nothing: per-CPU deltas of `/proc/interrupts` over the run (OHCI, xHCI, the
display's vsync, gfx, mali, eth0 starred even when quiet, everything else that fired), each line's `smp_affinity_list`, the
online CPUs, the softirqs that matter (NET_RX, TIMER, TASKLET, SCHED), and where the compositor's loop thread ran, sampled ten
times a second. `tv_gate.py` runs it once at Home after the frame run; the stream case is run by hand during a Netflix or GFN
session.

Proposal, to be judged on those numbers and to need Nitin's OK before anything is written. Compare `fifo20@0` with `fifo20@1`
in `gate.sh wake probe 20`, at Home and in a stream. If CPU 0's p99 and maximum are not worse than CPU 1's by more than 0.2 ms
(ten interrupts' worth of OHCI), the interrupt is not the problem and LG's setup stays. If they are, and the loop thread spends
30 % or more of its `lpc-probe.sh irq` samples on CPU 0, move the USB host interrupt to CPU 1 (`echo 2 >
/proc/irq/<n>/smp_affinity`, the previous mask kept to put back; a reboot restores LG's) and repeat the gate's `wake` lines.
Pinning the loop's own thread is the other lever and is not proposed: LG's hotplug moves a pinned thread when its CPU goes.

## The next TV round

Needs `compositor/tools/build-tv.sh` (it now bundles `lumatv-loadgen` and `lpc-probe.sh`) and Nitin's OK for the gate as
always. Reads only, except the loaded run's synthetic work.

1. `python3 compositor/tools/tv_gate.py --kwait --frames-load`: the zero-gap rule as before; then `wake`, `mem`, `irq` and
   `wakeups` lines after the frame run, and `GATE frames-load: measured ...`.
2. Right after start, `ctl mem`: the lock's line (`locked what is touched ...` or the reason), `VmLck` growing with RSS,
   `mmapped_blocks 0`. `lpc-probe.sh limits` before and after a gate run: `maps:` flat, `majflt` flat, and after a memory
   hog (`memory-result.md`) no compositor major faults.
3. `ctl idle 0` against `ctl idle 100`, each with `lpc-probe.sh wakeups 10 LABEL` on a still Home: the compositor's runs a
   second, about 60 and about 10 expected; then Home visible against covered.
4. `lpc-probe.sh irq 10 stream` during Netflix or GFN, beside the `home` one the gate printed; apply the proposal's test.
5. `gate.sh wake probe 20` once at Home and once under `gate.sh load start`: the FIFO 20 class against `other`, and `fifo20@0`
   against `fifo20@1` (the proposal's test).
6. The `wd-crash` gate line's bundle summary; then Settings > Support on the TV shows the notice; Details; Clear.
7. Record the first three runs' wake-up percentiles and set the gate's wake rule from them.

Not testable offline: kbase's mappings per buffer, whether the trace trigger exists on 5.4.268, the real MemAvailable at the
lock, the wake-up numbers themselves.
