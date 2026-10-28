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

State (2026-10-27): rounds 1 to 3 are on the set (below); round 4's changes are built and host-tested (`cargo test`,
the same with `--features ui`, clippy clean both ways, `cargo check --target aarch64-unknown-linux-gnu`, `python3 -m
unittest test_cutover` from `tests/`), not yet on the set. The go-live configuration has CompShell's shell client up
(`org.nphil.lumatv.shell`, run by the compositor from the state dir's `shell/Shell.qml`), and the x1 check and the trial
must pass with it. Next (Main, 2026-10-27): one Face ID for the ipk, the bundle deploy and `cutover install` from one
HEAD with CompShell's `04e9fd3`; then `tv_gate.py --only shell`, `--only keys,ime,shell,frames`, the trial
(`cutover.sh trial 120`), x1 three times, and the parity run (the X1 probe's first swaps under LG's compositor:
`X1_PRIME=0` with LG's live, before the session) that decides whether the probe primes its swapchain (`X1_PRIME`,
default 4 only with that evidence). Read `gate/x1-compositor.txt` (`trace:`, `warm:`, `standin:`) against `x1.log`
(`swap N returned`, `priming swap`, `gap`), and record the round under "Rounds".

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
