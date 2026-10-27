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

State (2026-10-27): the changes below are built and host-tested (`cargo test`, the same with `--features ui`, clippy
clean both ways, `cargo check --target aarch64-unknown-linux-gnu`, `python3 -m unittest test_cutover` from `tests/`),
not yet on the set. Next: deploy (Face ID: bundle deploy and `cutover install` in one approval plan), then
`python3 compositor/tools/tv_gate.py --only x1` three times and the trial (`tv_next_slot.py --skip-gate --no-standby`)
in one TV slot; read `gate/x1-compositor.txt` for the `warm:`, `standin:` and `loop: woke` lines and `x1.log` for the
`gap ... at monotonic` lines, and record the round under "Rounds" below.

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

None on the set yet.
