# Compositor gate 9: the eighth gate's x1 and wd-crash failures, fixed offline

Offline work of 2026-10-26 on the failures of the eighth gate (`slot-2026-10-26-0003`, round9-result.md row
"Compositor gate 8"). Nothing here has run on the set. Facts marked [INFERENCE] were not observed there.

## Go-live steps

LumaTV's compositor live for good, LG's as the automatic fallback. Needs Nitin's OK, the set on with Home in front, and no
other session. Every command below is the inner part of

    docker exec -e HOME=/data/home Cody sh -c 'export PATH=/data/home/.local/bin:/data/home/.cargo/bin:$PATH; cd /data/home/lumaglass && <inner>'

(`R` stands for `sh compositor/tools/tv-first-light.sh`). `R` drops the set's exit status: success and failure are read from
the printed lines named below. `/tmp/lumatv` is tmpfs, so after any reboot `R deploy` comes before any `R cutover ...`.

What decides the fallback, all host-tested (`tests/test_cutover.py`, `tests/test_compositor.py`, the supervisor's own C
tests): a persistent enable refuses a firmware build that is not on `compositor/firmware/verified.list`; at every boot the
early hook mounts only when the flag, LG's binary, the build, the bundle, the supervisor's `--would-run` and Home-not-held
all say yes, and a boot that never completed skips the mount once; the supervisor falls back by itself on a compositor exit,
two missed beats, a frozen output (6 s) or an empty panel (45 s), and refuses to start after two consecutive failures (a run
that never settled in 2 minutes counts, a power cut included); 10 healthy minutes clear the count. A recorded escape (hold
Home 650 ms) or a changed firmware keeps LG's until `enable` is run again.

1. Preflight, read-only: `R lumatv status` shows `"persist":true,"early":true` (without them no boot ever mounts it) and
   `R lumatv compositor check` answers JSON. The installed tool must be this tree's (`R lumatv compositor check` failing or
   old means the ipk is older than `tools/lumatv`).
2. Go live. `verified.list` is empty, so the persistent enable is refused until a gate has passed on this build. One command
   does the gate, the list entry, install and enable (about 25 min, the set is Nitin's meanwhile):

       python3 compositor/tools/tv_gate.py --skip volume --enable-after

   Success is `GATE: PASS`, `verified.list: added`, `enabled (persistent): LumaTV's compositor stays live` and
   `GATE live-enabled: PASS`. Then commit `compositor/firmware/verified.list` and the new `docs/research/compositor/gate-<date>.txt`.
   A gate that already passed on this build in an earlier round needs no second one: `R deploy`, `R cutover install`,
   `R cutover firmware record` (its `build ... end` block, carriage returns removed, goes at the end of
   `compositor/firmware/verified.list`; commit it), `R deploy`, `R cutover install`, `R cutover enable`.
   `enable failed: the supervisor fell back` (exit 9) leaves LG's live with the flag removed; `refusing a persistent enable`
   (exit 5) says the build is not listed.
3. After it, in this order: `R cutover status` (`mounted ... yes`, `flag: persistent; firmware: verified`, no `fallback:` in the
   log tail); Home in front and the keys on the remote (Home, arrows, OK, Back, volume, the Netflix button) with Nitin; one
   standby and wake (panel back, Home in front, `R cutover status` still `mounted yes`); then one reboot (`python3
   docs/research/arch/tv.py reboot`, Nitin's OK, about 90 s). After the reboot: `R lumatv compositor status` has `"live":true`,
   `"flag":"persistent"` and `"failures":0`; `python3 docs/research/arch/tv.py "grep -a compositor-boot /var/lib/lumatv/log |
   tail -3"` ends `compositor-boot: mounted`; `/var/lib/lumatv/compositor/sm.log` has `start:` and, 2 minutes later, `settled:
   two minutes healthy`; Home in front and the keys again. If LG's compositor is live instead, `R lumatv compositor check`
   names the reason (`decision`, `why`).

Rollback, any one of:

- `R cutover disable` (flag removed, mount removed, LG's compositor restarted, Home recovered; after a reboot `R deploy` first).
- `R lumatv compositor off` (the same through the installed tool; needs no deploy).
- `R lumatv revert` (everything of LumaTV to stock, the compositor included).
- At the set: hold Home 650 ms. LG's compositor is back at once and the escape is recorded, so later boots stay on LG's until
  `R cutover enable` (or `R lumatv compositor on`) is run again.

A redeploy of the bundle after go-live is `R cutover disable`, `R deploy`, `R cutover install`, `R cutover enable`: `install`
and `enable --session` refuse while the mount is in place.

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

State (2026-10-28, after round 9): LumaTV's compositor is LIVE (persistent, firmware 10.2.1-5202 verified, bundle r9 =
808256c + 889bb21 + 09740aa on the ipk of 808256c). What is left is the Optimisation list at the end of "Rounds" and the
runbook's reboot check (step 3, with Nitin). A later bundle goes through `cutover disable`, deploy, `install`, `enable`.

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

**Round 6, 2026-10-28 (set's clock 02:06-03:10), commits `4198076` then `1fadfcb` (`compositor/slot-2026-10-28-0200/`).**

*First run (4198076): the shell client never ran.* `shell: client exited (signal: 9 (SIGKILL)) after 0 s` at every one of 5
starts, then `giving up`; shell FAIL (connected false), frames FAIL (no frames), keys and ime PASS on their non-shell paths.
Cause, from round 6's own change: the client is forked on a per-start thread (`shell-spawn`) and carries
`PR_SET_PDEATHSIG` SIGKILL, which Linux fires when the forking THREAD exits, not the process. Fixed in `1fadfcb` (one spawn
thread for the life of the process); the host test `the_client_outlives_the_start_that_forked_it` fails on the old code (the
client is dead 300 ms after its start) and passes on the fix. The bundle of `1fadfcb` was installed by Main (standing grant).

*Second run (1fadfcb).* `egl: context priority realtime (asked for realtime)`, `loop thread: SCHED_FIFO 20`.

- Trial 120 s with the shell up: settled, `odd stamps 70 (0 missed)`, `late 0`, `held ticks 1`, `reached late 1 (over 1 ms 3,
  worst 7.9 ms)`. Round 5 had 6 missed refreshes at the start; now none. One `loop: 11.1 ms in one pass (clients)` at the
  shell's start (2.0 s in), over the 8 ms the Resume asked for; no refresh lost with it.
- x1, three runs of 60 s in separate sessions: client gaps over 20 ms 2, 1 and 1 (worst 20.8, 21.3, 21.8 ms; 59.93, 59.95,
  59.93 fps); the compositor's side clean in all three: `late` unchanged (1 to 1 each), no `page-flip stamp`, no `frame
  gap`, no loop stall, run-queue waits 2.3-4.4 ms. The gap at the first measured swap is back in two of three (20.8 and 21.8
  ms, the swapchain filling) and one mid-run gap each in runs 1 and 2 (20.6 at swap 3337, 21.3 at swap 3179). By the rule of
  PLAN "Decided" (no late flip of the compositor's own; probe gaps no more than LG's 4-7) all three PASS; by the old rule of
  zero they FAIL. The full gate's own x1 run: PASS by the old rule too (59.95 fps, no gap over 20 ms).
- Full gate (`tv_gate.py --skip volume`): firmware, homeheld, live, x1, netflix, hdmi, keys, ime, standby, homekey, wd-hang,
  wd-crash and escape PASS (standby and homekey now run in the first session: the timer fix held). shell FAIL on two steps
  and frames FAIL on the toast; see the classification in the Resume.
- `shell`: `saver` fails in every run (`saver false after turnOnScreenSaver`): `tvpower/power2/turnOnScreenSaver` answers
  `-100 Invalid State change Request`, and it answers the same under LG's compositor with the shell not involved (probed
  by hand, TV `Active`, Home in front). Round 5 passed this step the same evening. Not LumaTV's. `osd` failed once in
  three runs (full gate: `osd false, system window mapped true after Volume+`), passed in the other two.
- `frames` (3 cycles): home-dpad ok (first frame 30.8 ms), quick ok (28.0 ms, burst end 161.3), picker ok in the full gate
  (31.3 ms, burst end 148.1, `burst_over20` 0; `shell callbacks: 3492 of 8941 ahead`) and `burst_over20` 3 in the earlier
  run (single 33 ms intervals in the opens and closes; first frames 17.9-35.9 ms); toast first frame 61-71 ms in the
  earlier run (61.4, 63.1, 70.7) and 60.8, 82.1, 118.0 ms in the full gate, `delivered_ms` 29-107. Round 5 had 28-52 ms.
  The window staying mapped did not bring the toast's first frame down to the 20-35 ms expected; it is longer than round 5's.

**Round 7, 2026-10-28 (set's clock 00:25-01:35), commit `808256c` (`compositor/slot-2026-10-28-0300/`), the shell client up; no
go-live (BLOCKING items below).** Causes found offline from round 6's evidence and fixed in 808256c and 889bb21:

- The volume display's `osd` failure (round 6, 1 of 3): `shell_handle` returned after drawing the join's frame, before it
  stored the report's state (`osd true`), and the client never resends an identical report, so `ctl shell` kept `osd false`;
  which of the layer's frame and the `live` word came first decided it. The state is stored first. Host test
  `the_state_in_the_report_that_brings_the_layer_live_is_kept_when_the_join_draws_a_frame` (fails on the old order).
- The quick panel over an app (round 7, full gate, 1 of 8 runs: `Back left overlay 'quick', wants true`): the layer had gone
  idle, its `wants` word reached the compositor before its `live` word, and the focus is looked for among the windows in the
  stack, which has no idle layer; nothing moved the focus when the layer joined, so Back went to the app. Focus is looked for
  again when `live` changes (889bb21). Host test `a_layer_that_wanted_the_keys_while_idle_gets_them_when_it_goes_live`.
- The join's window for a commit that came ahead of the `live` word is 400 ms (was 100). Round 7's own journal lines show it
  was not what held the toast: in all 23 joins of the three frame-gate sessions the layer's last commit was 0-48 ms old and the
  join drew it, quiet once (`shell: system layer live; its last commit N ms ago, join draws it|quiet`).

*Results (808256c):*

- `--only keys,ime,shell,frames` x3: keys PASS 3 of 3 (`gear-shell`, `input-shell` at the press); ime PASS 2 of 3 (the run that
  failed had maliit's window 1920x1080 at (0, 0), not the 1920x438 at (0, 642) of the other two: the `bottom` rule that the
  gate's own comment says fails "in four of six runs" with that window; typing and closing worked); shell PASS 2 of 3 (`osd` and
  `saver` ok in all three; the failure was `dpad`: focus `dock` after Up with the keys counted, 7 to 11, in the run whose ime
  step had the full-screen window); frames FAIL 3 of 3 on the toast (first frame 53.7, 71.4, 78.2 ms; limit 50) and single 33
  ms intervals (`burst_over20` 1 in home-dpad, 2 in picker in run 1; 2 in quick in run 2; none in run 3). home-dpad 27.7-32.2
  ms, quick 30.3-37.0 ms (burst end 155-163), picker 22.9-31.9 ms (burst end 148.6-160.5), callbacks ahead 3102-3307 of
  4144-4255.
- Full gate (`--skip volume`): firmware, homeheld, live, netflix, hdmi, keys, ime, standby, homekey, wd-hang, wd-crash and
  escape PASS; x1: client 1 gap over 20 ms (21.2 ms, 59.93 fps), compositor side clean (`late` 0 to 0, no stamp, no stall,
  run-queue waits 3.0 / 3.5 ms) = PASS by the PLAN rule, FAIL by zero; shell FAIL on `quick-app` (above); frames FAIL on the
  toast alone (104.2 ms; home-dpad 29.6, quick 33.6, picker 26.3 ms all ok).
- `--only shell` x4 on the same bundle: 4 of 4 PASS (the quick panel over an app included).
- Toast, the open item: qml-runner at nice 0 instead of 10 made it worse, not better (first frames 83, 113, 125 ms against 56,
  55, 65 ms at nice 10, same session, `ab-nice/`), so priority is not it, and the kept, warmed toast slot (808256c) did not
  bring it down either (53.7-78.2 ms in the frame gate, 104 ms in the full one; round 6: 61-118). The join is not it (above).
  889bb21 journals `shell: toast delivered to the client` beside the join line: the gap between the two lines minus the
  commit age is the client's time from the delivery to its first frame of the toast, which the next round reads.

### Optimisation list (MINOR, by Nitin's rule of 2026-10-27: not noticeable in use)

- Toast first frame 54-104 ms from the delivery (limit 50; round 5: 28-52): a notification appearing 3-6 refreshes late.
- Single 33 ms intervals in the frame gate's bursts (`burst_over20` 0-2 per run in home-dpad, quick and picker), isolated.
- x1: 1 client gap of 20.8-21.8 ms in most runs (swapchain filling or one late client commit); LG's compositor shows 4-7.
- One `loop: 11.1 ms in one pass (clients)` at the shell client's start; no refresh lost; the trial of round 6 had 0 missed.
- maliit's keyboard window is 1920x1080 at (0, 0) in some runs, which the gate's `bottom` rule rejects; the keyboard works.
- `shell` step `saver`: tvpower answered `-100 Invalid State change Request` to `turnOnScreenSaver` for a while in round 6, also
  under LG's compositor; it passed in all eight of round 7's runs.
- `escaped` stays set after the gate's escape step until the next `cutover.sh enable`.
- helper/ at c37660b-era HEADs did not compile on 2026-10-28 (stray `)` in `service.rs`, `sched_param` fields in
  `gfn_boost.rs` on musl); the ipks of rounds 6b and 7 carry the helper built at 1fadfcb.
- Round 8 (r8 = 808256c + 889bb21), keys,ime,shell,frames x3: keys 3/3, shell 3/3 (`dpad`, `osd`, `quick-app`, `saver` all ok), the
  toast's first frame 28-33 ms in all three (54-104 ms in round 7), frames failing only on isolated single 33 ms intervals
  (`burst_over20` 1-2) and, once, a picker burst end of 191.6 ms (limit 180); ime failed 2 of 3 with maliit's 1920x1080
  window at (0, 0), fixed by 09740aa (r9).
- Round 9 (r9 = r8 + 09740aa), the go-live run: ime x5 at the bottom (`1920x438 at (0, 642)` in all five, maliit's window
  1920x1080 in three and 1920x438 in two); full gate `--skip volume`: every safety and function check PASS (firmware, homeheld,
  live, x1 [59.95 fps, no gap over 20 ms], netflix, hdmi, keys, ime, shell 11 of 11, standby, homekey, wd-hang, wd-crash,
  escape); `frames` FAIL on MINOR lines only: toast first frame 63.4 ms, `burst_over20` 1 in home-dpad and picker, `dropped` 1 in
  each, picker burst end 184.9 ms (limit 180).
- The `dpad` failure of round 7 (1 of 3, then 0 of 8 runs): the keyboard was not on screen at the time (the Search app's
  window had closed 17 s before; the journal's `focus ->` line after it named a different surface than in the passing run);
  the shell's state at the start of the step was identical in both runs. No cause found.
- `lumatv status` on the installed tool (the ipk of 808256c) ends with `updateblock_json: not found` (line 2978 of
  `tools/lumatv`); the rest of the JSON is printed.

### Go-live, 2026-10-28 05:39 (set's clock), by Main's and Nitin's authority

Manual route of the runbook, with the r9 bundle already on the set (no second gate): `cutover.sh firmware record`, the entry
added to `compositor/firmware/verified.list` (10.2.1-5202, fingerprint `e34f8de94ef657a4`), the list put on the set,
`cutover.sh install`, `cutover.sh enable`: `enabled (persistent): LumaTV's compositor stays live`, `GATE live-enabled: PASS`.
`lumatv compositor status`: `"flag":"persistent","mounted":true,"live":true,"firmware":"verified","failures":0`. Then, under
LumaTV's compositor: keys PASS, ime PASS (keyboard at the bottom), Netflix PASS (Dolby Vision, picture mode back to filmMaker),
HDMI 2 PASS, one standby and wake PASS (`PreparePowerOff` 1, `PreparePowerOn` 1, mounted yes, Home on the panel). Not done:
the reboot of the runbook's step 3 (needs Nitin), and the escape (not triggered after go-live). Rollback as in "Go-live steps".
