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
