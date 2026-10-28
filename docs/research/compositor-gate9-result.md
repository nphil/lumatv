# Compositor gate 9: the eighth gate's x1 and wd-crash failures, fixed offline

Offline work of 2026-10-26 on the failures of the eighth gate (`slot-2026-10-26-0003`, round9-result.md row
"Compositor gate 8"). Nothing here has run on the set. Facts marked [INFERENCE] were not observed there.

## Resume here

State (2026-10-26, commits `ff6e30a`, `ae6d795`, `a240187` on `main`): every change below is built and host-tested
(`cd compositor && cargo test`, the same with `--features ui` under `LIBGL_ALWAYS_SOFTWARE=1`, clippy clean both ways,
`cargo check --target aarch64-unknown-linux-gnu`, `sh compositor/sm/build.sh`, `python3 -m unittest
tests.test_cutover`, `sh compositor/tools/build-tv.sh`). Next: one TV slot ("TV re-gate" below). Open after it, if the
slot shows them: a `frame gap` on X1's surface that still comes with a `flip reached` line (read it against the new
`major faults` and `run-queue wait` numbers), and Netflix's `tvq-pb-101`, which is not this note's.

## What the eighth gate's lines were

**No driver buffer warmed mid-run.** X1 binds no `zwp_linux_dmabuf_v1`; its buffers are the driver's
(`mali_buffer_sharing`), so each new one is imported and warmed at its commit. The one hold of the run,
`tick: frame held while a driver buffer warms` (22585.004) to `warm done` (22585.008), was X1's first buffer, 1 ms
after its window mapped. X1's other two buffers held nothing: the log has no second `frame held` line, which comes
again whenever a tick is held after `warm done`.

**The 33.37 ms page-flip stamp at X1's mapping** is on every gate that logs the stamps (1219, 1418, 1606, 1730,
0003), with the hold or without it. The flip before it landed 14-15 ms before X1's first commit, while X1's window
existed without a buffer: window creation and every property set raised the scene's layout flag, and the next tick
redrew the unchanged picture. X1's first commit then came 1-2 ms before the next refresh (by the log's stamps),
too late for it, so its frame landed on the one after: two refreshes from the redraw, logged as a skipped refresh
although no frame was due at the one between. The host test reproduces the redraw (two frames drawn where one is due);
that it is the flip on the set is [INFERENCE].

**The 20.4 ms frame gap** came 25.3 s in (1516 steady frames), in the same millisecond as `kms: flip reached 3.6 ms
after it landed`: the loop was busy when the flip landed, so X1's callback, and with it X1's next commit, went out
late. X1's own worst gap was 19.7 ms. What held the loop for 3.6 ms has no line of its own (nothing logs under 8 ms).

**The trial's 23.2 ms loop pass** was the session's first atomic commit, the one carrying the mode, made blocking:
`present: fence export 0.6 ms, atomic commit 21.8 ms` (13.3-16.1 ms on the sixth and seventh gates).

**wd-crash's reason** read `compositor closed the bridge socket`: the kernel closes a killed process's sockets before
the process can be reaped, so the supervisor's bridge hang-up handler ran ahead of its child watch.

## Changes

- Supervisor (`sm/lumatv-sm.c`, `ff6e30a`): on the bridge's hang-up or end of read, the child gets up to 300 ms to be
  reaped; a reaped child's own status is the reason (`compositor killed by signal 9`, `compositor exited with N`). A
  compositor still alive after closing the socket keeps the old reason. Test `test_bridge_gone` (a socketpair child
  killed with SIGKILL, one exiting with 3, one alive with its end closed).
- Scene (`scene.rs`, `ae6d795`): the plan compares what it draws and where (surface, rectangle, opacity, row order,
  stacking, holes) with the last plan and redraws only what changed; the layout flag and its setters are gone. A
  window made, named or given a state before its first buffer draws no frame. `Scene::redraw_all` forces a full one.
- Flips first (`main.rs`, `state.rs`, `kms/mod.rs`): each loop pass handles the DRM fd before anything else; the tick
  takes landed flips before it draws and again after it presents; a landed flip's callbacks are flushed to the clients
  at once instead of at the end of the pass. The KMS output reads the card only when poll says an event is there.
- First commit non-blocking (`kms/mod.rs`): the commit with the mode goes out `NONBLOCK | ALLOW_MODESET`; a driver
  that refuses it gets the blocking one (`kms: the first commit (with the mode) refused non-blocking`). One line says
  which: `kms: the mode went out with the first frame (non-blocking)`.
- Skipped refreshes judged (`surface.rs`, `scene.rs`, `kms/timing.rs`, `kms/mod.rs`): each surface stamps when its
  oldest undrawn commit arrived, the plan carries the oldest stamp it shows, and a flip two refreshes after the last is
  a missed refresh only when that content was in early enough for a typical frame to make the refresh between (a frame
  of no new commit counts as due). A miss keeps the line the gates count, `kms: page-flip stamp N ms after the last`;
  a refresh with nothing new is `kms: page-flip interval N ms, nothing due at the refresh between`. `ctl status` reads
  `odd stamps N (M missed)`; the pacer's `lost` now has the content time it was built for.
- Gate (`tools/tv-gate.sh`, `a240187`): x1's detail line ends with the compositor's major page faults and its loop
  thread's run-queue wait over the run (`/proc/PID/stat`, `/proc/PID/task/PID/schedstat`), the two causes of a loop
  delay that leave no log line.

Host tests: `warm.rs::x1s_start_holds_each_new_buffer_and_draws_nothing_for_its_window` (X1's start on a driver that
warms each new buffer: Home, X1's window with no buffer, three swapchain buffers each held while it warms, the first
again warm; fails on the old code with a frame drawn for the window), `flips.rs::a_landed_flip_has_its_callbacks_out_
before_the_next_frame_is_drawn` (an output that holds frames until their flip and draws for 150 ms; fails on the old
code with the callback after the draw), `kms::timing` tests for a missed and an idle skip, `test_cutover` cases for
the idle line.

## TV re-gate

Needs the set to itself for about 10 minutes (Nitin's OK for the slot), on and showing Home. From the repository root
on the workstation:

    python3 compositor/tools/tv_next_slot.py --no-standby

It builds and deploys the bundle of HEAD, runs the 120 s trial and the whole gate, and leaves LG's compositor live.
Evidence lands in `docs/research/compositor/slot-<date>-<HHMM>/`. Pass for this note's items:

1. `summary.txt`: `trial verdict: PASS`. In `trial/compositor.log`: `kms: the mode went out with the first frame
   (non-blocking)` and no `present: ... atomic commit` over 4 ms or `loop:` over 20 ms at Home's mapping. A
   `page-flip stamp` line over 20 ms left at Home's mapping is a real miss: read it with the `tick: frame held` and
   `warm done` lines before it.
2. `GATE x1: PASS`. `gate/x1-compositor.txt` has no `page-flip stamp` line at X1's mapping (at most a `page-flip
   interval ... nothing due` line), no `frame gap`, and the detail line ends `major faults +N, run-queue wait +X ms`.
   If x1 fails on a `frame gap`, the `flip reached` line beside it and those two numbers say where the loop went:
   major faults above 0 point at pages read back from swap, a run-queue wait of tens of milliseconds at the CPU.
3. `GATE wd-crash: PASS` with `reason: compositor killed by signal 9`.
4. Nothing else regresses: `live`, `volume`, `hdmi`, `standby`, `homekey`, `wd-hang` and `escape` pass as on the eighth
   gate. `netflix` is outside this note.

## On the set, 2026-10-26 14:00 (`compositor/slot-2026-10-26-1400/`)

Gate FAIL on frame timing only; LG's compositor left live. Fixed: the first-commit stall is gone and `wd-crash` passes
with `reason: compositor killed by signal 9`. Every other check passes (firmware, homeheld, live, volume, netflix, hdmi,
standby, homekey, wd-hang, wd-crash, escape). Still failing: the trial has a `page-flip stamp 33.36 ms` (one missed
refresh) and x1 has 3 client swap gaps of 20-28.1 ms in 3598 frames (59.95 fps), one `frame gap 20.1 ms` after 2694
steady frames, with its `flip reached 3.4 ms after it landed` line beside it.

## Gate 10: the ninth gate's frame timing

### Resume here

State (2026-10-27): rounds 1 to 5 are on the set (below; round 5 is 12a1d0b, `compositor/slot-2026-10-27-2100/`); round 6's
changes ("Round 6 changes" at the end) are built and host-tested, not on the set. The go-live configuration has CompShell's
shell client up (`org.nphil.lumatv.shell`, run by the compositor from the state dir's `shell/Shell.qml`). X1_PRIME stays 4
(round 4's parity run). Round 6 checks, in one slot (`python3 compositor/tools/tv_next_slot.py`, or by hand in this order):

1. `tv_gate.py --only keys,ime,shell,frames`: keys, ime and shell PASS as in round 5; `ctl shell` reads `systemMapped true`
   and `systemLive false` with nothing of the layer up; frames PASS with the toast's `first_frame_ms` under 50 (40 wanted)
   and `burst_over20 0`. The session's log has `egl: context priority realtime (asked for realtime)` (or `high`), `loop
   thread: SCHED_FIFO 20`, and at the shell's start `shell: client started` with no `loop:` line beside it; the system
   layer's three buffers warm (`warm:` lines) in the seconds after `shell: system layer window mapped`, not at the toast.
2. `cutover.sh trial 120` with the shell up: `odd stamps N (0 missed)`, no `loop:` line over 8 ms in the first 10 s. A
   `page-flip stamp` left in the first 6 s is read against its `trace:` lines: `gpu done` over 8 ms is the GPU still
   shared with qml-runner's start (the context priority did not take), a frame drawn 5-7 ms before its refresh is the
   plan's optimism (`plan()` in `vendor/pacer.rs`, left as it is).
3. x1 three times (separate sessions): no `page-flip stamp` at x1's mapping (at most one `late` there, from the first
   frame's GPU time), `late` unchanged mid-run, the client's gaps over 20 ms no more than LG's 4-7. A gap at about 55 s
   again, with the compositor's flips on the grid, is outside LumaTV (the shell client no longer outranks the probe).
4. The full gate (`tv_gate.py --skip volume`): everything of round 5 again; standby and homekey in their own run as in
   round 5 if the first session's timer ends before them.

Read `gate/x1-compositor.txt` (`trace:`, `warm:`, `standin:`) against `x1.log` (`swap N returned`, `priming swap`, `gap`).

### What the ninth gate's lines were

**The missed refresh at a client's start** (`page-flip stamp 33.36 ms` in the trial at Home's mapping and in x1's first
second, on every gate since the fifth) is the third swapchain buffer's warm: each new `mali_buffer_sharing` buffer is
imported and warmed at its commit, the tick holds the frame while the warm's fence is pending, and the third buffer's
fence takes 10-12 ms on the set (the first 4-15 ms, the second 1-5 ms; trial log, x1-compositor.txt), past the pacer's
latch, so the frame that should have shown it slipped a refresh. The hold also cost a client's first frame up to a
refresh more: a held tick re-armed the fixed tick, not a poll, so the warm's end was found at the next fixed tick (0-16.7
ms later). What the fence waits on (the driver's mapping on its `mali-cmar-backe` thread, the GPU job, or both) is not
known; the new `warm:` line measures it.

**The 20.1 ms frame gap after 2694 steady frames**, with its flip reached 3.4 ms after it landed, has no line of its own
below 8 ms; the loop thread's run-queue wait over the 60 s was 1177.6 ms (major faults 0): the loop shared the CPUs at
nice 0 with X1, WebAppMgr and LG's daemons (LG's own compositor is reniced to -19 minutes after its start,
`perf-tweaks.md`), so a wake-up for a flip could wait behind whoever held the CPU. [INFERENCE: that this wait was the
3.4 ms; the new `loop: woke` line says so or not.]

### Changes

- Stand-in for a warming buffer (`render/gl.rs`, `kms/frames.rs`, `damage.rs`, `kms/mod.rs`): while the driver is busy
  with a surface's new buffer and a page holds the last composed frame, the frame is drawn with the surface's rectangle
  copied from that page (opaque, scissored to the owed tiles) instead of being held; the page owes those tiles again
  (`Ring::owe`), the surface is remembered (`standins`), and once nothing is warming the output asks for a frame of its
  own (`ui_needs_frame`) that draws the buffer for real. A client's first frame goes out at once with the window not yet
  there; its content follows when the buffer is warm. Frames are still held when no page holds the last frame (the first
  composed frame, after a direct or split frame, after a ring forgets its pages). Lines: `standin: surface N keeps the
  last frame's content while its new buffer warms`, `warm: driver buffer N WxH: import X ms, warm draw submitted in Y ms,
  its fence done Z ms after`.
- A held frame polls (`kms/mod.rs`): `next_frame_at` is half a millisecond away while the last `warming` said so.
- The loop thread is SCHED_FIFO 20 (`sched.rs`, `--rt-priority N`, 0 keeps it time-shared; `loop thread: SCHED_FIFO 20`
  at start). Threads the loop spawns (control jobs, PNG writes) put themselves back in the time-shared class; the Mali
  blob's threads are made before the change and stay time-shared. LG's input threads (SCHED_RR 99) and the HDMI interrupt
  thread (FIFO 50) stay ahead of it.
- A flip the loop reached late gets a second line (`main.rs`): `loop: woke N ms after the flip's stamp, idle since M ms
  before it` (the CPU or the event was late) or `...; the pass before (what) ran until M ms after it` (the loop was
  busy). `ctl status` reads `reached late N (over 1 ms M, worst X ms)`.
- Gate (`tools/tv-gate.sh`): x1's detail line ends with the loop thread's scheduling class and priority (`loop thread
  SCHED_FIFO prio -21`); `loop stalls` counts only `loop: N ms in one pass` lines; `x1-compositor.txt` keeps the `warm:`,
  `standin:` and `loop: woke` lines. The X1 client (`arch/x1/x1client.c`) prints `first swap returned at monotonic T` and
  one line per gap over 20 ms (`gap N ms before swap K returned at monotonic T`), so each gap lines up with the
  compositor's log.

Host tests: `standin.rs` (a second buffer the driver stays busy with: the frame is drawn, not held, shows the first
buffer's content and carries the client's callback; nothing is drawn while it warms; the owed frame shows the new content
once warm and nothing after it), `damage.rs::tiles_a_page_copied_in_place_of_a_warming_buffer_stay_owed_by_that_page`.

### Rounds

**Round 1, 2026-10-27 20:00, commit `475357a` (`compositor/slot-2026-10-27-1659/`).** Trial PASS: `odd stamps 0 (0
missed)`, `held ticks 1`, `reached late 0 (over 1 ms 0, worst 0.1 ms)`; `loop thread: SCHED_FIFO 20`. x1 FAIL three
times, but the mid-run cause is gone: the loop thread's run-queue wait over the 60 s was 3.4, 7.1 and 4.6 ms (1177.6 on
the ninth gate), no `flip reached` line, 15-41 gaps of 17.5-20 ms (32 before). Gaps over 20 ms: 4, 3 and 7 (worst 32.0,
31.8, 26.5 ms), all but one group in the first 250 ms of the run, around the second and third swapchain buffers'
warms: the stand-in frames went out, but the warm's fence took (as seen at the ticks) 20.3 and 18.5 ms, 11.2 and 30.3 ms,
8.6 and 10.5 ms, and the frame drawn at the latch landed a refresh late each time (`page-flip stamp 33.36 ms`, `late` 1,
2, 1). The third run also lost a refresh mid-run (after 2569 steady frames): `present: fence export 7.7 ms` and `tick:
8.3 ms` at 44949.817, then `page-flip stamp 33.37` and `frame gap 33.6 ms`; and `fence export 4.2 ms` at 44966.245 with
a 21.0 ms client gap. [INFERENCE: the warm's job and the frame's draw are both submitted by the blob's `mali-cmar-backe`
thread, time-shared; the fence export waits for that submission, and a buffer's first job carries the driver's mapping
of it, so a slow turn of that thread holds the frame.]

Round 2 changes (`render/sync.rs`, `render/gl.rs`, `kms/mod.rs`, `sched.rs`, `config.rs`, `control.rs`, `main.rs`,
`tools/tv-gate.sh`, `arch/x1/x1client.c`, `arch/x1/x1heapshim.c`):

- The Mali blob's submission threads (`mali-cmar-backe`, `mali-utility-wo`) are SCHED_FIFO 19 (`--rt-mali N`, 0 keeps
  them time-shared; `mali thread: NAME (TID): SCHED_FIFO 19` lines at start); `ctl sched mali fifo [PRIO]` and `ctl sched
  mali other` switch them live for an A/B.
- The warm's fence is a native fence (a sync file) where the blob exports them: the `warm:` line reads the kernel's own
  signal time (`its fence signalled Z ms after (seen W ms after)`), no longer the tick that found it done.
- A start-up trace: a new buffer prepared starts 90 flips of `trace:` lines (each commit of a buffer, each tick held,
  each draw with its slot and panel time, each commit with its fence-export and atomic-commit times, each queued frame,
  each flip with the frame's lateness and the GPU's finish). The gate keeps them in `x1-compositor.txt`.
- The gate's x1 detail line ends with the `mali-cmar-backe` thread's run-queue wait over the run and its class. The X1
  client stamps its first eight swaps (`swap N returned at monotonic T (its draw started D ms before)`); the heap shim
  stamps each allocation.

**Round 2, 2026-10-27 20:26, commit `8a0049f` (`compositor/slot-2026-10-27-1726/`).** The bundle now carries
CompShell's shell client, which the compositor starts; its three windows mapped with Home's. Trial FAIL on one missed
refresh at the start (`page-flip stamp 33.36` at 45915.791: the second frame's draw came 6 ms after its commit because
`prepare` blocked 5.1 ms exporting the warm's native fence, and the tick before it was the fixed one), `odd stamps 108
(1 missed)` with the shell's idle commits. The trace settled the warm: with the Mali threads in FIFO its fence signals
0.1-0.8 ms after the draw (kernel stamps), so round 1's 11-30 ms were the tick that polled it plus the time-shared
submission thread. x1 with the shell up: FAIL, 168 gaps over 20 ms, 58.19 fps, `late` 1 to 13; the compositor's flips
were fine (12 odd stamps in 3600, loop run-queue wait 12.7 ms) and X1's commits were what came late (`frame gap 33 ms
on surface 205` 14 times): its wake-ups lost the CPU to qml-runner's rendering (about 10 fps while idle). x1 with
`shell/` moved aside: 1 gap (23.1 ms at swap 5), 59.95 fps, `late` 0, 10 gaps of 17.5-20 ms, loop wait 3.4 ms,
`mali-cmar-backe` wait 3.7 ms. The same with the Mali threads time-shared (`ctl sched mali other`): 3 gaps (20.6 at
swap 5, 20.4 and 22.4 mid-run), 23 of 17.5-20 ms, `mali-cmar-backe` wait 534.6 ms; FIFO stays. The trace explains the
swap-5 gap: X1's second commit reaches the compositor only after the first frame's callback (the blob holds commit N
until callback N-1, and its swap blocks only for a free buffer), Home redrew on X1's appearance, and the fixed tick
(its phase whatever it was last armed with) drew Home's frame alone 5 ms before a refresh, so X1's second frame queued
behind it and the client's cadence slipped a refresh; the client's gaps are its buffer releases, one per tick. The
client's swapchain allocations (8.6 MB of CMA each) also took 14 and 22 ms inside its second and third swaps in the
shell run.

Round 3 changes (`kms/mod.rs`, `kms/timing.rs`, `output.rs`, `main.rs`, `render/gl.rs`, `arch/x1/x1heapshim.c`):

- The fixed tick is on the panel's refresh grid while no run is on (`Output::idle_tick_at`, `FlipTiming::grid_latch_ns`):
  the next refresh's latch from the last flip's stamp and whole periods since, for 3 minutes after it, then the plain
  interval. The tick is re-armed when a flip lands, so the first flip of a run moves it to the pacer's latch at once.
  `ctl status` reads the refresh period as the panel keeps it (`refresh 16.67 ms (measured 16.6667, ...)`), to size
  that memory against the drift.
- The warm's fence is an EGL fence again (its export of a native fence waited for the submission thread, 5.1 ms once);
  `warm:` lines say `seen done ... (polled at the ticks)` as in round 1.
- The X1 probe's heap shim allocates the swapchain's three buffers at the first request (inside the first swap, which no
  gap is measured from) and serves the next two from that pool.

**Round 3, 2026-10-27 21:20, commit `fae341e` (`compositor/slot-2026-10-27-1820/`), the shell client up (CompShell's
`9514551`: its Home tick held while covered, qml-runner at nice 10, started once SAM's Home mapped).** Trial FAIL on one
missed refresh in the first second: the frame drawn at 49123.489 (GPU done 5 ms later) flipped two refreshes late at
.532 while qml-runner started (`loop: 15.9 ms in one pass (tick)` in the shell's start, its first frames' GPU work:
the next frame's GPU took 30.8 ms); the other 122 s clean, 102 odd stamps from the shell's 10 Hz wallpaper tick on Home
alone, none missed. x1 FAIL three times with 3, 5 and 3 gaps over 20 ms (59.93, 59.89, 59.92 fps); the compositor side
clean in runs 1 and 3 (`over 20 ms 0`, `late` unchanged, loop and `mali-cmar-backe` run-queue waits 3-4 ms), one missed
refresh in run 2 (`late` 1 to 3). What the traces show: (1) the client's fourth swap (26.9, 43.8, 32.8 ms) is its
swapchain filling: with the three buffers pre-allocated the first three swaps return at once and the fourth waits for
the compositor's first buffer release, which follows the first frame's callback (a refresh of vblank phase), the
client's second commit (2-6 ms) and the tick that draws it; (2) 2-3 gaps at swaps 5-13 from the first frames' timing: a
latch tick that fired a millisecond before the commit drew nothing, which ended the pacer's run, and the idle tick that
followed was budgeted for the 10-11 ms pipeline of frames drawn on an idle GPU, so the commit waited 14 ms for it;
(3) 1-2 mid-run gaps of 20-21 ms with the compositor's flips regular: the client's swaps follow the compositor's buffer
releases, one per tick, and the latch moved 3-4 ms later in one step when a slow frame left the pacer's window.

Round 4 changes (`kms/mod.rs`, `arch/x1/x1client.c`):

- The latch moves later by at most 1 ms per slot (`LATCH_STEP_NS`): the lead before the refresh is remembered per slot
  and a smaller one is approached a step at a time (earlier is free).
- The idle tick's budget caps the typical pipeline at 6 ms (`IDLE_TYPICAL_CAP_NS`) before the 6 ms lead.
- The trace also has each tick that drew nothing, each latch decision (`trace: next tick at T (latch, L ms before the
  refresh at R)`), each idle tick on the grid, and a drawing tick over 18.5 ms after the last (`trace: tick N ms after
  the last drawing tick`, 200 at most, run or no run of the start-up trace).
- The X1 probe primes its swapchain with `X1_PRIME` swaps (default 4) before its clock and prints `primed: N swaps in
  M ms before the clock`; `X1_PRIME=0` measures from the first swap as before.

**Round 4, 2026-10-27 22:00 (set's clock), commit `f0d3cb4` (`compositor/slot-2026-10-27-1900/`), the shell client up.** One
HomeLabber plan put the ipk, the bundle and `cutover install` on the set.

*Parity run under LG's compositor, before any session* (the X1 probe from the installed bundle, window type overlay over
Home; it runs there as it does under LumaTV's). `X1_PRIME=0`, 30 s: swaps 1-3 return within 3 ms of each other (the
heap shim's pool), the fourth 14 ms later, then gaps of 22.8 ms before swap 5 and 22.3 ms before swap 8 (the swapchain
filling); 59.93 fps; two more gaps over 20 ms mid-run (21.6 ms at swap 367, 22.8 ms at swap 1459); 4 in total. `X1_PRIME=4`,
60 s: the first eight measured swaps are all under 20 ms; 7 gaps over 20 ms in the run (worst 24.1 ms), 916 of 3597 in
17.5-20 ms, 59.94 fps. LG's own compositor therefore shows the start-up gaps without priming and mid-run gaps with it.
The probe's priming default stays 4 (the evidence the Gate 10 rule asked for).

*Trial* (`cutover.sh trial 120`, the shell client up): settled, two minutes healthy; `odd stamps 82 (0 missed)`, `reached
late 0 (over 1 ms 0, worst 0.4 ms)`, `held ticks 1`, `late 2 lost 14` of 1318 flips, render max 412 us. The 82 odd stamps
are the shell's 10 Hz tick on idle Home, none a missed refresh. The start-up miss of round 3 did not recur.

*x1, three runs of 60 s with the shell up* (X1_PRIME 4; `GATE x1: FAIL` each by the rule of zero over 20 ms):

| run | client gaps over 20 ms | worst | fps | compositor over 20 ms | `late` | loop / `mali-cmar-backe` run-queue wait |
|---|---|---|---|---|---|---|
| 1 | 2 (20.5 at swap 2, 20.5 at swap 2175) | 20.5 ms | 59.93 | 0 | 1 to 1 | 4.3 / 4.6 ms |
| 2 | 3 (21.8 at swap 2, 24.1 and 26.1 at swaps 53, 54) | 26.1 ms | 59.91 | 1 (`frame gap 29.0 ms`) | 2 to 2 | 4.7 / 3.8 ms |
| 3 | 1 (21.5 at swap 2) | 21.5 ms | 59.93 | 0 | 1 to 1 | 4.0 / 4.2 ms |

Every run has a gap of 20.5-21.8 ms at the first measured swap after the priming (the 6th swap): in run 3's trace the
compositor commits buffers 12 and 14 as new, their warms' fences are seen done 18.1 and 14.4 ms after the draw, the frame
for the client's next commit waits for a buffer release. [INFERENCE: that the warm is what holds the release; the trace
shows the order, not the dependency.] Run 2's gap at swaps 53-54: the compositor's flips stay on the grid, its idle
ticks draw nothing, and the client's commit comes 29 ms after its previous one, 13 ms after the flip's callback (normal:
4 ms); the compositor logged it as `frame gap 29.0 ms` and `page-flip interval 33.36 ms, nothing due at the refresh
between`. No loop stalls, no major faults, loop thread SCHED_FIFO.

*x1 verdict, both ways.* By the rule as written (zero gaps over 20 ms, start-up included): FAIL in all three runs. Against
LG's own compositor in the same probe (7 over 20 ms and worst 24.1 ms in 60 s with priming; 4 in 30 s without): 1-3 gaps
and worst 20.5-26.1 ms is no worse in count, with the one difference that LumaTV's gap at the 6th swap is in every run
while LG's first measured swaps were clean after priming. One 60 s run of LG's is the whole parity sample.

**Round 5, 2026-10-27 (set's clock 00:10-00:55), commit `12a1d0b` (`compositor/slot-2026-10-27-2100/`), the shell client up, the
pass rule of PLAN "Decided" (no late flip of the compositor's own; probe gaps over 20 ms no more than LG's 4-7 a run).**

*Trial* (`cutover.sh trial 120`): settled, two minutes healthy, but `odd stamps 51 (6 missed)`, `late 9 lost 9` of 1491 flips,
`reached late 0 (worst 0.9 ms)`, `held ticks 1`: FAIL by the rule of no missed refresh. All six missed refreshes (`page-flip
stamp 33.35-33.37 ms`) are in the first 5.5 s, at the shell's start; the other 118 s are clean. The log
(`trial/trial-compositor.log`) at the start: `shell: starting the client (SAM's Home has mapped)` and, 23 ms later, `loop: 24.5
ms in one pass (tick)` (the spawn of qml-runner on the loop thread, `shell_start`); the client's first buffer then comes
(`commit of buffer 63 ... (new: warming)`, `warm: ... warm draw submitted in 4.7 ms, its fence seen done 48.4 ms after`) and
the pass that takes it is `loop: 55.2 ms in one pass (clients draw-now)`, followed by `late 1 lost 1` at 60084.382 and four
more misses up to 60089.6 while the Home window's buffers warm. Round 3's start-up miss (one) had the same two lines. Both
are on the loop thread; a fix (the spawn off the loop thread, or the shell's start deferred until the first flips settle, and
the warm of a client buffer polled instead of waited for in `draw-now`) is not made here: it needs another deploy.

*x1, three runs of 60 s* (X1_PRIME 4, each its own session, shell up) and the full gate's run:

| run | client gaps over 20 ms | worst | fps | `late` | compositor over 20 ms | old rule (zero) | new rule |
|---|---|---|---|---|---|---|---|
| 1 | 0 | 19.6 ms | 59.94 | 3 to 3 | 0 | PASS | PASS |
| 2 | 1 (20.1 at swap 3) | 20.1 ms | 59.92 | 3 to 5 | 3 (two page-flip stamps of 33.37 ms at x1's mapping, `frame gap 22.2 ms` after 3310 steady frames) | FAIL | FAIL (2 late flips at the start) |
| 3 | 1 (20.8 at swap 3057) | 20.8 ms | 59.94 | 9 to 9 | 0 | FAIL | PASS |
| full gate | 2 (21.6 at swap 2, 27.4 at swap 3308) | 27.4 ms | 59.91 | 5 to 6 | 2 (`frame gap 22.9 ms` after 3303 steady frames, then `page-flip stamp 33.36 ms`) | FAIL | FAIL (1 late flip, mid-run) |

Loop thread SCHED_FIFO in all four, run-queue wait 5.0-5.8 ms, `mali-cmar-backe` wait 3.6-4.4 ms, no major faults, no loop
stalls. Against round 4 (f0d3cb4): the gap at the first measured swap is gone in runs 1 and 3 (it was in every round 4 run),
and the client's gaps are 0-2 a run (1-3 before). Left: the late flip at x1's mapping (run 2, same two stamps as the start of
every earlier round) and one mid-run late client commit in each of run 2 and the full gate, both about 55 s into the run
(after 3310 and 3303 steady frames); the compositor's flips were on the grid before each, and the commit came 22-23 ms
after the previous one. [INFERENCE: whether something periodic, in the shell client or the system, fires near 55 s; one
60 s run each is not a pattern.]

*Verdict.* x1: PASS in 2 of 4 runs by the new rule (1 and 3), FAIL in 2 (a late flip of the compositor's own at x1's start,
and one mid-run); the client's side is within LG's 4-7 in every run. Trial: FAIL (start-up of the shell). Full gate: every
check but x1 passed (standby and homekey in a run of their own; frames PASS), so the blockers for go-live are the late flips
at a client's start (the shell's in the trial, x1's in run 2) and one mid-run late client commit.

### Round 6 changes (offline, 2026-10-27): the shell's start, x1's start, the toast

Nothing of this has run on the set. Facts marked [INFERENCE] were not observed there.

**The trial's six misses are the shell client's start at the loop's own priority.** `shell_spawn` ran `Command::spawn` on
the loop thread: 23 ms between `shell: starting the client` (60084.176) and `shell: client started` (.199), the `loop: 24.5
ms in one pass (tick)` beside them (the fork and the exec of a 32-bit Qt process, the parent waiting for the exec). The
child inherited the loop's class: a fork keeps SCHED_FIFO 20 unless the parent set `SCHED_RESET_ON_FORK`, and the
`setpriority(10)` in the child sets a nice level that class ignores. So qml-runner started, loaded its QML and drew its
first frames (buffers 95, 108, 109 at 60 fps from 60084.805 to about 60090) level with the loop and ahead of the Mali
blob's submission threads (FIFO 19) and of every kernel worker, the DRM commit work included; the `lumatv-shell-http` thread
(spawned after the switch) was FIFO 20 as well. The lines: the 55.2 ms `clients draw-now` pass is Home's second swapchain
buffer (63, WebAppMgr's surface 80, not the shell's) prepared 41 ms after qml-runner's exec, `import 1.1 ms, warm draw
submitted in 4.7 ms` (the usual 0.3 and 0.2-0.3) and then 48 ms with nothing logged before the tick's poll found the fence
done: the loop, or the driver's thread it waited on, was off the CPU [INFERENCE on which; `prepare` had returned at .246].
Of the six `page-flip stamp` lines, two are frames the GPU returned late while qml-runner's first frames ran on it (`gpu
done` 13.6 and 11.9 ms at 60084.382 and 60086.434 against 4.7 in a run), one is a flip the kernel made a refresh late with
the fence 5.4 ms before the vblank (60086.200: frame drawn at .173, GPU done at .178, refresh .183, flipped .200), one is the
refresh lost after that one and the next (.217, `gpu done` 11.6 ms) had both landed a refresh late (60086.250), one is a
commit 10 ms after a flip, drawn at once, whose frame took 10.8 ms (60085.717: the refresh 7 ms away was out of reach and the
judge counted it from a typical 4.7 ms frame), and one is a commit drawn 6 ms before its refresh with a 5.6 ms GPU time,
0.7 ms short (60089.603).

**x1's two stamps at its mapping (run 2)** are one late frame counted twice. The client's first commit (60413.319) was
drawn at once for the refresh 8 ms away (`reachable` in `plan()`: 4.7 ms typical and 0.5 ms lead), its GPU took 6.1 ms
beside X1's own first frames, the fence was 2.4 ms before the vblank and the flip went a refresh late (.345). The second
frame, planned for the slot after it while it was on its way, landed one refresh after it (.362), the soonest it could: the
pacer had already moved its timeline on by one for the first landing, then judged the second against its old slot, moved
on again, and aimed the next frame at .394 with the .377 refresh reachable (`tick 42.7 ms after the last drawing tick`,
`page-flip stamp 33.37` at .395). The same two stamps are at X1's mapping on every round since the fifth gate. **The late
commit at about 55 s** (run 2 after 3310 steady frames, the full gate after 3303) came 13 ms after the callback (4 ms
usual) with the compositor's flips on the grid, its loop thread's run-queue wait 5.3-5.8 ms over the run and no stall:
the client lost the CPU to something above it. The shell client was above it (FIFO 20, its 25 s long poll and its 1 s
report timer the periodic work it has while idle) [INFERENCE: the trace ran out 90 flips in; what ran at that moment is
not in the slot]. Its `late` is the frame drawn for that commit: 10 ms after the previous flip, the next refresh 6.7 ms
away, the GPU asleep (`lost 0` from the pacer, `missed` from the judge's typical frame). If the gap comes again at the
same phase with the shell client time-shared, it is LG's.

**The toast's 44-52 ms** are a window made per toast: the shell's `sysWin` was `visible: sysLive`, so Qt destroyed its
surface, its EGL window and its three 8.6 MB CMA buffers at every close and made them again at the next toast, and the
compositor paid a window, three imports and three warms before the first frame (the trial's one mapping of the layer:
the type property at 60092.823, the first commit at .840, three `warm:` lines of 3.7, 8.7 and 7.4 ms, the first two
frames stood in for).

Changes:

- Scheduling (`sched.rs`): the loop's class is `SCHED_FIFO | SCHED_RESET_ON_FORK`, so every thread and process it creates
  starts time-shared; `time_shared()` and its calls in the PNG and control threads are gone (the flag does it). The Mali
  threads get the flag too. `describe()` masks the flag the kernel reports with the policy.
- The shell's start (`shell.rs`): `shell_spawn` hands a `SpawnJob` (the runner, the paths, the module lookup) to a
  `shell-spawn` thread and returns; the child arrives through a channel at a later tick (`shell_take_spawned`: `shell:
  client started, pid N`), `spawning` keeps a second start from being asked meanwhile, a failed start waits 10 s as
  before. The child is time-shared by the flag and nice 10 by the `pre_exec`; `PR_SET_PDEATHSIG` stays.
- The system layer stays mapped (`shell/Shell.qml`, `scene.rs`, `shell.rs`): `sysWin` is visible from two seconds after
  Home loaded (`sysWarm`) and walks its three buffers once with an 80 ms animation of an invisible item, so the compositor
  imports and warms them while nothing shows. The UI report carries `live` (`sysLive`, pushed on change); the scene leaves
  the layer's window out of the stack while the shell says it is not live (an app's buffer goes to the plane whole as
  before), and brings it back quietly: the join itself damages nothing, the layer's next commit is the frame that shows
  it. A commit of the idle layer in the 100 ms before `live` (the frame ahead of its word on the channel) makes the join
  draw at once instead. `ctl shell` reads `systemLive`; `live` defaults to true for a shell that never says.
- The pacer's timeline (`kms/timing.rs`, `kms/mod.rs`): a landing's shift (`Landed::shift_ns`, `late` refreshes) moves the
  slot of the frame queued behind it (`Timing::shift`), so that frame is judged against the moved timeline.
- The missed-refresh judge (`kms/timing.rs`): a skipped refresh counts as missed when the content was in early enough for
  the frame as it cost (its own pipeline, the GPU's finish included, as the pacer's `lost` reads it), no longer for a
  typical frame: a lone frame on a GPU asleep costs 10-11 ms, and a commit 7 ms before a refresh cannot make it.
- The GPU's priority (`render/egl.rs`): the context is made at `EGL_CONTEXT_PRIORITY_REALTIME_NV`, else
  `EGL_CONTEXT_PRIORITY_HIGH_IMG`, else the default, by the extensions the blob lists (the set's lists both,
  `arch/x2-run-01-logs.txt`); the level granted is logged (`egl: context priority realtime (asked for realtime)`). The
  compositor's draws and warm draws go ahead of the clients' jobs on the GPU, where the 6-14 ms frames of a client's start
  came from [INFERENCE: that the blob honours the level on the set; the line says].

Host tests: `sched::tests::what_a_fifo_loop_thread_spawns_is_time_shared` (a thread and a `sleep` the FIFO thread spawns
are both SCHED_OTHER; skips where SCHED_FIFO is refused, as in the workstation's container, and says so),
`shell::tests::the_client_is_started_off_the_loop_and_arrives_at_a_later_tick` (the tick that starts the client returns
without it; a later tick has it, one start counted; fails on the old code with the child set in the first tick),
`standin.rs::a_first_buffers_warm_holds_the_frame_and_no_pass_of_the_loop` (a first buffer the driver stays busy with: five
ticks each return at once with the frame held and the callback kept, the frame drawn once the driver is done),
`shell.rs::the_system_layer_is_drawn_only_while_live_and_joins_without_a_frame_of_its_own`,
`kms::timing::tests::a_frame_planned_behind_a_late_one_moves_with_the_timeline_and_is_not_late_twice` (fails on the old
code: the third frame lands a refresh early, aimed one too far), `a_skip_after_a_frame_that_paid_the_gpus_wake_up_is_not_
missed`. Expected on the set: the trial without the start-up misses (the client's start no longer outranks anything),
x1 without the second stamp at its mapping (the first is a late frame, not a skip, once the GPU takes the compositor's
draw first), the toast's first frame at a frame's cost (20-35 ms from the delivery: the client's render, the compositor's
draw at once, the refresh).
