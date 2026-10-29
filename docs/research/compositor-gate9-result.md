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

### Round 10, 2026-10-28 (set's clock 05:48-06:46), forge/main `48ee380` (`compositor/slot-2026-10-28-0600/`)

The whole package of HEAD (ipk built by `build-ipk.sh`, bundle with the LPC memory and idle-tick work, hole order, own
keyboard and wallpaper off by default) on the set. `cutover install` refuses while the compositor is mounted, so the live one
was disabled first (`cutover disable`) and the install went through Main's grant; `/tmp/lumatv-r9` stayed as the rollback.

- Full gate (`--skip volume`) and `--only keys,ime,shell,frames` x2, all under LumaTV's compositor: every BLOCKING check
  passes in all three (firmware, homeheld, live, x1 [59.95 fps, no gap over 20 ms], netflix, hdmi, keys, ime [keyboard 1920x438
  at (0, 642) every time], shell 11 of 11, standby, homekey, wd-hang, wd-crash with a crash bundle each, escape); the new lines
  `wake` (tick p99 0.04 ms, max 0.38 ms) and `mem` (locked 547820 KiB, swapped 0) are readings.
- `frames` fails on MINOR lines only: isolated single 33 ms intervals (`burst_over20` 1-3 in home-dpad, picker, toast), toast first
  frame 68.2 ms (full gate) and 34.0 ms / 67.1 ms (the two runs), and once the quick panel's first frame 125.2 ms; picker burst end
  165.8-176.8 ms (limit 180).
- `tools/tvslot/r19_checks.py` under LG's compositor (`--no-standby`), no FAIL or ERROR: speed PASS (every threshold met);
  settings, plugins, layout and ble EYES (20 Settings pages captured, none changed; the layout editor opened and discarded with
  `layout.json` unchanged by md5; BLE read-only scan answered unknowns 1-3: `le/startScan` callable by `luna-send`, `scanRecord`
  filled, `always` changes nothing seen); banners SKIPPED with `inputcommon` off (baseline). `lumatv inputcommon on`, then the
  banner check alone: the dark glass pill shows (16 frames, 1.2-4.5 s after the launch call) and the INFO card is dark glass (EYES);
  its two structural lines FAIL on stale expectations, not on the set: the script counts 30 `.qml` files and looks for
  `upper/Components/LumaStyle.qml`, the tree has 35 and `LumaStyle.qml` is at `UserInterfaceLayer/Components/`. `inputcommon`
  stays on.
- GeForce NOW (`org.nphil.lumatv.gfn`) opened once: the sign-in screen (PIN, address, QR, "Back leaves"), no sign-in; closed.
- The update-block read check is not in `r19_checks.py`. `lumatv status` and `lumatv updateblock status` fail on the set with
  `updateblock_json: not found` / `verb_updateblock: not found`: in `tools/lumatv` the block that defines `updateblock_json`
  (the "LG's system updates, blocked by the Homebrew Channel" comment up to the function) sits inside the body of
  `capture_start()` (around line 2328), so the function exists only after `capture_start` has run, and `verb_updateblock`
  is not defined where the dispatcher looks for it.
- Re-enabled persistently (`cutover enable`): `enabled (persistent)`, `GATE live-enabled: PASS`, keys and ime PASS again,
  `settled: two minutes healthy`, failures 0.

### Round 11, 2026-10-28 (set's clock 06:47-08:25), the ipk of `48ee380` still installed

- Live TV does not come to the front under LumaTV's compositor (three launches, 07:30, 07:50 and 07:51): `applicationManager/launch com.webos.app.livetv`
  answers `returnValue true`, `ctl windows` lists "Live TV" and "Live Dmost" cards mapped at 1920x1080, but SAM's foreground and the panel stay on Home
  for 20 s and more (HDMI inputs come to the front as before). Under LG's compositor on 10-27 the same app showed channel 2-1
  (`lg-restyles/tv/banners-r18/channel-ok.png`). Counted as blocking for a go-live decision, not as a reason to disable the compositor (Main, 2026-10-28).
  Closed through SAM each time; `lumatv inputs status` running `[]`.
- Cause, from SAM's own log of two more launches (08:27:59 and 08:31:21, evidence outside the repo in `/tmp/livetv-ev/launch5` and `launch6`, found by LiveTvFix):
  Live TV is foreground 0.65 s after the call; 0.8 s later the dmost service launches Live Dmost (`caller_id com.webos.service.dmost.client`); the compositor
  reports it as the only foreground entry, `windowGroup true`, `windowGroupOwner false`, `windowGroupOwnerId com.webos.app.livetv`; SAM's
  `current_fullscreen_foreground_apps` becomes empty and 1.0 s later its last-input handler launches Home (`caller_app_id last_input_handler`, `mode last_input`).
  Fix: `a5e2c50` (a window attached to a surface group is a member, not a card), not installed on the set; the next TV run is the new build, same launch.
- An unplanned reboot gave the runbook's reboot check: at 08:00:00 the TV was put into Active Standby (`power/powerOff`, `remoteKey`, uptime 24.3 h,
  87,484 s) and rebooted within about a minute; tvpower's power-on reason for the new boot is `rebootByOnRegular` (LG's own regular reboot; nothing new
  in `/var/spool/faultmanager`, no crash report, compositor `failures 0`) [INFERENCE: it fires at the first standby after 24 h of uptime; the day's earlier
  standby entries came before that]. The set came back in Active Standby by itself: `lumatv compositor status` `persistent`, `mounted`, `live`,
  `firmware verified`, `settled: two minutes healthy` at 08:04:16; the boot hook mounted `inputcommon`, `voice` and `keys` again, `apply` succeeded in 64 s,
  and `theme.json` kept `remote.quickAccess` and `remote.quickPanelEdit`. Standby means nobody saw the panel: a cold boot's pictures and Home on the next
  wake were not looked at.
- `/var/log` is tmpfs, so the reboot took SAM's lines for the 07:50 and 07:51 launches with it; the compositor and supervisor logs copied at 08:00:40 survive.

### Round 12, 2026-10-28 (set's clock 05:38-06:30 PDT, TV clock 08:40-09:25), forge/main `0f2190c` (`compositor/slot-2026-10-28-0900/`)

The whole package of `0f2190c` (LiveTvFix's window groups `a5e2c50`, the foreground list `0f2190c`, the update-block tool fix `dbc3f9b`, Settings fixes,
`ALLOWED_METHODS`): `lumatv compositor off` (the TV had rebooted, `/tmp/lumatv` was gone, so no `cutover disable` was possible), install through
Main's grant, `/tmp/lumatv-r10` kept as the rollback, then the gate under LumaTV's compositor and `enable` persistent.

- Full gate (`--skip volume`): firmware, homeheld, live, netflix, keys, ime (keyboard at (0, 642)), shell 11 of 11, standby, homekey, wd-hang, wd-crash
  (crash bundles) and escape PASS. `hdmi` FAILED on the gate's own pattern (`mapped 0`): `ctl windows` now writes `exposed`, `front` and `inGroup`
  between `appId` and `mapped`, and `tv-gate.sh` matched the old key order; the window was mapped and in front (`fg ... hdmi2`). The pattern no
  longer depends on the order; `--only hdmi` PASSES. `x1`: client 5 gaps over 20 ms in the full gate (20.x, 20.x, 21.x, 33.x and 34.9 ms; 59.86 fps),
  the compositor's side clean (`late` 1 to 1, no stamp, no stall); two reruns 1 gap each (21.5 ms, 59.93 fps), compositor clean.
- `--only keys,ime,shell,frames`: keys, ime, shell PASS; frames FAIL on MINOR lines (home-dpad and picker `burst_over20` 1-3, picker burst end
  184.7 ms, toast first frame 56.5 ms in the full gate and 80.6 ms in this run).
- Live TV (`livetv-check.py`, receiver muted, in a session): `launch com.webos.app.livetv` returns true; SAM's foreground is `com.webos.app.livetv` at
  4, 8, 12, 16, 20 and 24 s and stays (round 11: Home within 20 s); `ctl windows` lists Live TV Fullscreen and Live Dmost `inGroup true` as the member
  of group `com.webos.app.livetv` in `dmostLayer` (journal: `surface group "com.webos.app.livetv": surface 219 attached to layer "dmostLayer"`,
  `window 7: fullscreen (a group member follows its owner)`), `video` has one fullscreen window; no `LAUNCH_LASTAPP` or `last_input_handler` line in
  SAM's log. Back (412) left Live TV in front (the foreground stayed `livetv`: Back is Live TV's own); closing it through SAM and launching Home
  gave Home. The panel capture is 6863 bytes (the capture service does not read the video plane), so the channel picture itself was not seen.
- `lumatv updateblock status`: `"blocking":true,"state":"blocked"`, flag and all four hosts. Settings > System > Support > Update, under LumaTV's
  compositor (pictures `update-page/`): Block LG system updates is on and drawn undimmed, Check for Updates says "Updates blocked"; the row's
  subtitle reads "Homebrew Channel: Not permitted to send to org.webosbrew.hbchannel.service." (the page's own call to the channel is refused on
  the bus for the shell's client, while `lumatv updateblock status` reads the flag; not looked into). A navigation mistake of mine (a first
  attempt opened LG's Sound > Advanced Settings by pressing OK on its row, then RIGHT and DOWN keys; nothing else was pressed there; the Dolby Atmos
  setting could not be read back).
- Re-enabled: `cutover enable` persistent, `GATE live-enabled: PASS`, `settled: two minutes healthy`, failures 0.

### Round 13, 2026-10-28 (set's clock 09:30-09:56; `compositor/slot-2026-10-28-1300/`, results in `keyboard-result.md`, `videoplane-saver-result.md`, `venc-result.md`)

- `lumatv standbyscreen off` (Always Ready off, `general.lifeOnScreenMode` `off`, `prev alwaysReady`): LG's setting reads off. Then `power/powerOff` (`remoteKey`)
  took the set out of reach: port 22 refused for more than 6 minutes (before, Active Standby kept SSH up). A magic packet booted it from cold (`up 0 min`
  at 09:51:59, power state Active). So with Always Ready off the standby is a real off, and the way back is a wake-on-LAN packet and a cold boot
  (about 20 s to the supervisor). [INFERENCE: a venc probe stuck in the kernel since 09:41 may have contributed; the boot cleared it.]
- The cold boot was a second runbook reboot check: `compositor-boot: mounted` 19 s after boot, `persistent`, `mounted`, `live`, `firmware verified`, `failures 0`,
  `settled: two minutes healthy` at 09:54:09; `keyboard lg`, `wallpaper gpu` and `standbyscreen off` all survived the boot.

### Round 14, 2026-10-28 (set's clock 09:34-10:52 PDT), forge/main `a1555df` (`compositor/slot-2026-10-28-r14/`)

- Installed through the standing grant (exit 0), compositor persistent from the gate's end. Full gate `--skip volume`: every step PASS except `frames`; two more
  runs of `keys,ime,shell,frames`: `keys`, `ime`, `shell` PASS both times. `frames` (3 cycles each): home-dpad, picker PASS in all three runs; quick had one burst
  interval over 20 ms in the first run and none in the next two; toast first frame 70.2, 67.4, 60.3 ms (MINOR by Nitin's rule, no burst end). `wd-hang`, `wd-crash`,
  `escape`, `homekey`, `standby`, `firmware`, `x1` (59.95 fps, no gap over 20 ms), `netflix`, `hdmi` PASS.
- **YouTube and Plex die under LumaTV's compositor (BLOCKING, found by the app switcher's check; no earlier round launched either).** Both exit with SIGSEGV (SAM
  `exit_status 11`): cobalt 32-42 ms after connecting, with no window; `plxnative` maps a window ("committed without a buffer") and ends 0.55-0.6 s later.
  Under LG's compositor both are in front 2 s after the launch. Netflix, Apple TV, Search, HDMI and Live TV are not affected. Plex's own `/tmp/plxnative-crash.log`:
  `addr=0x24 pc` in `libwayland-client` `wl_proxy_get_version` with a NULL proxy, called from `libSDL2` `SetMRCUTimer` (LG's SDL patch: it marshals a request on the
  webOS seat proxy at window data +0x60). The compositor implements `wl_webos_input_manager.get_webos_seat` (`server/input_manager.rs`); neither client's journal shows that it was asked
  or answered, and both clients' bound lists lack globals the browser binds (`wl_data_device_manager`, `wl_subcompositor`, `wl_webos_xinput_extension`, `text_model_factory`; cobalt also `wl_shm`, `wl_output`).
  Evidence: `compositor/slot-2026-10-28-r14/appcompat/` (README first). Because of it the set ended the round on LG's compositor (`compositor off`, Home stub on).
- Features, per their docs (`img/`): own keyboard, volume bar, store theme, app switcher, transitions, overlays, OLED care and capture 1-5 PASS under LumaTV's compositor and
  are left off or as found, waiting for go-live; the video wallpaper FAILS (black margin); 120 Hz: the mode works, the surfaces do not hold it. Each doc's "TV round 2026-10-28 (round 14)" has the numbers.
- Left on: the store theme (`store on`, parts `look,motion`; it is a WAM overlay and does not depend on the compositor). Left off: `remote.appSwitcher`, `overlays.enabled`, `oledcare`, `transitions`,
  capture, `keyboard lg`, `wallpaper gpu`; the `wall` player is installed and stopped.
- Receiver: off at the start; the TV's wake turned it on at 0.585 unmuted. Muted for the round, set back to 0.585 unmuted at the end; it follows the TV into standby.

### Round 15, 2026-10-28 (PDT 13:08-14:19, the TV's clock is 3 h ahead), forge/main `f67c511` = `c1a2736` + changelog (`compositor/slot-2026-10-28-r15/`)

Installed by Main (`install.py`, `tv-first-light.sh deploy`, `cutover install`; the first run was cut after `install.py` at HomeLabber's 60 s default and run again): helper sha256 `7256caac39fc…`, QML tree equal to the ipk's,
compositor md5 `03438190fe7f…` (2,449,000 bytes). Receiver through Home Assistant's MCP (off at the start, on at 0.585 unmuted with the TV's wake, muted for the round, unmuted before the standby, off with the TV);
the gate ran with its REST calls disabled and `--skip volume`, as in rounds 9-14. Every session ended with LG's compositor live; `theme.json`, `Tokens.js` and `layout.json` are byte for byte as found.

- Full gate: firmware, homeheld, live, netflix, hdmi, keys, ime, shell 11 of 11, standby, homekey, wd-hang, wd-crash and escape PASS. `x1` failed once (1 client gap of 22.6 ms, 59.93 fps; the compositor's side
  clean: no stall, no frame gap, `late` 0 to 0); two reruns PASS (59.94 and 59.95 fps, no gap over 20 ms). `apps` and `frames` FAIL.
- `gate.sh apps` cannot pass as written. `app_running` (tv-gate.sh:84) looks for `"id":"APP"` in SAM's list and `luna-send -f` prints `"id": "APP"`, so every app reads as not running at 10 s (YouTube and Plex:
  in front within 1 s, `alive-at-10s no`) and `closed-in-15s` is always yes; the fix is `| tr -d ' \n'` before the grep, as in `fg`. The exit status is read from /var/log/messages after a line mark, and that file
  rotates about every minute, so a crash can be missed; the copy used below looks it up by pid in the rotated files too. A copy with both fixed ran the apps below in the same sessions.

| app | id | in front | after 10 s | closed through SAM |
|---|---|---|---|---|
| YouTube | youtube.leanback.v4 | 1.0 s | running | no failure logged |
| Plex | com.beb.plxnative | 1.0-1.5 s | running | SIGSEGV (exit 11) at 3 of the 4 closes that followed a YouTube run, none at 3 closes of Plex alone |
| Netflix | netflix | 1.5 s | running | no failure logged |
| Prime Video | amazon | 2.0 s | running | no failure logged |
| Apple TV | com.apple.appletv | window after 50-60 s (LG's compositor: 50 s) | running once up | exit 9 when closed before it was up; LG's compositor needs 10 s and also ends it with 9 |
| Disney+ | - | not installed (no id or title in SAM's 150 apps) | - | - |
| Crunchyroll | com.crunchyroll.webapp | 6.5 s | running | no failure logged |
| LG browser | com.webos.app.browser | 1.0 s | running | exit 15 (SIGTERM); the same pid first connects and leaves at 0 ms |
| LG Channels | com.webos.app.lgchannels | 1.0 s | running | no failure logged |
| GeForce NOW, NVIDIA | com.geforcenow.play | 0.5 s | ended itself, LG's browser in front | see below |
| GeForce NOW, LumaTV's | org.nphil.lumatv.gfn | 2.5 s | running | no failure logged |
| Moonlight (extra) | com.limelight.webos | 1.0 s | running | exit 0 |

- Plex's crash is a new one (round 14's was `SetMRCUTimer`): SIGSEGV, libSDL2-2.0.so.0.14.0 `+0x875c6` (`ldr r0, [r7, #0x18]`, r7 = 0) reached from `+0x724b4`, when SAM closes it after 10 s with its connection
  kept (`plex-crash-log-r15.txt`, 3 reports). Under LG's compositor YouTube, Plex, Plex closed with exit 0 three times. Recipe: YouTube 10 s, close, Plex 10 s, close through SAM. The seat state a client leaves
  behind is the first thing to look at [INFERENCE]. The gate's own check never saw it (its first read of the exit status came before SAM logged it).
- The NVIDIA GeForce NOW web app opens LG's browser and ends itself within 3 s under both compositors. With that browser in front `launch com.webos.app.home` leaves Home in front under LG's compositor; under
  LumaTV's Home is raised and the browser raises itself again (`set_state fullscreen` after Home's) and keeps the foreground (5 s in the test, about 3 minutes in the first session, where it also held Live TV
  and the wallpaper step off the screen) until SAM closes it. LG's browser started directly gives Home back at once under both (`stray-browser.txt`, `browser-home.txt`, `control/`).

Surface frame gate at 60 Hz (full set, 3 cycles; `frames-60-full-gate.json`):

| surface | result | numbers |
|---|---|---|
| home-dpad, quick, picker | PASS | first frame 24.0 / 26.6 / 31.3 ms; burst end 160.7 (quick) and 148.1 ms (picker); no interval over 20 ms in a burst |
| toast | MINOR | first frame 62.2 ms (limit 50), no burst; round 14: 60-70 |
| home-widgets | PASS | first frame 27.8 ms, p90 render 0.06 ms, no interval over 20 ms |
| switcher | MINOR | first frame 72.1 ms (limit 50): the compositor asked the shell 49.7 ms after the key; p90 0.06 ms, no dropped frame, no interval over 20 ms |
| keyboard-luma | MINOR | first frame 9.1 ms, p90 0.08 ms, no interval over 20 ms; `log_errors` 1: LG Search's own `SearchView.qml:294:13: Unable to assign [undefined] to QQuickItem*`, not on the gate's ignore list |
| keyboard-lg | PASS | first frame 14.5 ms, p90 0.12 ms, no interval over 20 ms |
| settings-scroll, apps-grid, transitions-netflix, transitions-settings | not run | refused before a key: Home's layout is the owner's (`layout.json`), not the shipped one |

- 120 Hz (`--only frames --refresh 120 --frames-quick`): idle Home 10 s: `hz 120 divider 1 shell 120`, DE2VSYNC 120.9/s, osd_irq 120.1/s, frames 146, late 0, lost 0, missed 0, pipeline p50 4.75 p99 4.88 ms. After the
  surfaces (the measure covers them): frames 6276, late 72, lost 0, missed 0, pipeline p99 5.0 ms, max 11.49 ms. `slides:` none logged, before or after (no `[FrameMotion] N steps in M ms` line in shell.log).
  The slides step per refresh with one or two skips: quick 17 frames in a 150 ms burst, intervals of 8.3 ms except 2-3 of 16.7 ms (`burst_over20` 14 over 6 windows; round 14: 32, all 16.7 ms), picker 10 (round 14:
  35), toast 3, home-dpad `dropped` 2, burst ends 158.9 / 156.3 / 93.9 ms, home-widgets PASS; settings-scroll and apps-grid not run (layout). Back at 60 afterwards (`shell 60`, divider 2).
- Live TV (`lumatv transitions on`, put back off; Home in front before each of 3 cycles): in front 1 s after the call and through 8 s, Home back within 1 s of the leave. Launch: 60, 50, 52 frames, `lateGaps` 0, 0, 1;
  leave: 12 frames each, `lateGaps` 0 (round 14: 16 frames, 2); `veilCaps` 0, `skipped` 0, `maxGapMs` 18, then 24. The one late frame, launch 3: `lateLog` `{"ageMs":7807,"gapMs":24,"phase":"launch"}`, journal
  `transitions: frame 24 ms after the last in a launch (tick before 1.4 ms, this 1.3 ms, last render 0.4 ms, held ticks 1, buffers prepared 24, imported in a frame 0)`, ring `P 3.8 ms` and `P 2.4 ms in one pass (clients
  draw-now)` in the first second: the loop's passes were short, so the 24 ms was a wait outside them [INFERENCE: display, driver or another process].
- Video wallpaper, as shipped: `wallpaper video` ready, player `playing wall-silk-tuned`, `wall.hole` true, but videooutputd answers both `setDisplayWindow` (`displayOutput` 3840x2160, `fullScreen` true, `sourceInput`
  1920x1080) with `{"errorCode":4,"errorMessage":"Invalid Parameters"}`; LG's VIDEO capture says `REFUSED_NO_SIGNAL`, the DISPLAY capture has a black margin (mean luma 0.00): `vplane_wall.py probe` says `black`.
  The request has no `originalInput`. The same context with `originalInput` 0,0,1920,1080 is accepted and the VIDEO capture is then the clip (probe: `silk-tuned` at 6.9 s, correlation 0.962, gain 0.86-0.90, verdict
  `mismatch` at the probe's 0.98 bar); 3840x2160 for both (Netflix's) is accepted too; `displayOutput` 1920x1080, no `displayOutput`, or `fullScreen` false without `originalInput` is refused (`wall/placement-variants.txt`).
  Fix: `originalInput` = the source rectangle in `video::set_display_window` (luna.rs), as `set_crop_region` and Netflix's placement carry it. The DISPLAY capture stays black at the margin after an accepted placement
  (it holds the UI plane; a hole reads black), so the probe needs the VIDEO capture; the panel itself was not looked at.
- `tv-wlprobe.py` against LG's (`wlprobe-diff.txt`): fresh session: 13 seats all `capabilities(7)`, no `wl_seat.name`, `wl_starfish_pointer` and `wl_starfish_output` offered, `timer_value(5000)`, `wl_webos_seat.info` with
  device names (`LGE RCU` is index 1 as on LG), no `wl_output.scale`: the notes of 644249e hold. After other clients ran the timer reads 300000 (one value for all clients, the last set). Also different, not in the notes:
  32 globals against 23, no `qt_hardware_integration` or `wl_drm`, `wl_webos_system_ux` added, `wl_compositor` v4, `wl_output.geometry` with size and model, 2 `wl_shm` formats against 14, `wl_keyboard.repeat_info(0, 0)` and a 1140 byte keymap.
- Not run: `volume` (the gate reads the receiver over REST), and the four surfaces above (they need `lumatv layout reset` and the owner's file put back). End state: filmMaker, only Home running, `inputs` running `[]`,
  Active Standby at 14:19 PDT, Always Ready (`alwaysReady`), LG's compositor live (flag off, failures 0), Home stub on, `transitions` off, `wallpaper gpu`.
- Follow-up, 14:50-15:03 PDT, Plex's close and the browser GeForce NOW opens (`slot-2026-10-28-r15/plexclose/`, `gfn-browser/`). The SIGSEGV is the second `SDL_Quit`: LG's SDL runs `callback_forceQuit` 1000 ms
  after the close event, and in the close that crashed the first quit stopped after `WebOS_EGL_UnloadLibrary unload` (`UnloadWEBOSBASELibrary start` follows it by 2-9 ms in the 10 clean closes), so the
  second ran at +1012 ms and died in libSDL2 `+0x875c6` (r7 NULL), `APP_CLOSED` exit 11 at +1059 ms. Clean closes end at 120-150 ms under LumaTV's compositor (LG's: 87-98 ms, exit 0); the compositor logs nothing during
  the stall. Crashes: 1 of 9 YouTube-then-Plex closes in this follow-up, 4 of 13 over the round, 0 of 3 without YouTube; no stalled close was caught with a sampler or sweeps running (0 of 8).
  The browser calls `Browser::RequestWindowFullscreen()` every 0.3 s under LumaTV's compositor and once under LG's; after Home is launched LG minimizes it (`WebOSExported::updateWindowState`, `getForegroundAppState`
  `"state":"minimized"`, the browser's `SetVisible[0]` 0.155 s later) while under LumaTV's its next request raises it above Home 0.23 s after the launch and it stays in front.

### Round 16, 2026-10-28 (PDT 16:10-18:27, the TV's clock is 3 h ahead), forge/main `64e015d` (`compositor/slot-2026-10-28-r16/`)

Installed by Main through HomeLabber (102 s): compositor 2,482,232 bytes (md5 `8765469985f8…`, the persisted copy equal), helper sha256 `4fa08196c652…`. Receiver through Home Assistant's MCP: off at the start; the TV's wake turned it on at 0.585 unmuted;
muted for the round and again after each unmute (a volume key, the standby's wake and the receiver's own CEC state unmute it); set back to 0.585 and unmuted before the standby; off with the TV. The gate ran with its REST calls disabled and
`--skip volume` (the volume check by hand, below).

- Gate run 1 (16:14) was void from its start: the Home stub had gone back to LG's Home before it began (`/var/lib/lumatv/log`, 19:13:22: `LumaHome's heartbeat stopped (5 s, file 6 s old), giving LG's Home back`, fault 1 of 2),
  about 30 s after the wake, with the TV idle on Home under LG's compositor, where the guard reads LumaHome's beat (under LumaTV's it stands down). With LG's Home in place LumaHome steps aside and the shell maps no Home window:
  `shell` "home mapped false", the frame surfaces unreachable. Aborted at the frame step, `lumatv homestub on` (strikes cleared), rerun from its start. Its other lines agree with run 2 (x1: one gap of 20.1 ms in 60 s, 59.96 fps, the compositor's side clean): `gate-run1-aborted-lines.txt`.

- Gate run 2 (full, `--skip volume`; `gate-run2/`, `gate-full-gate-lines.txt`): firmware, homeheld, live, x1 (59.95 fps, no gap over 20 ms), netflix, hdmi, apps (YouTube 1.5 s, Plex 1.5 s, GeForce NOW's browser in front, Home launched over it holds),
  keys (the gear and Input now reach the shell: `gear-shell`, `input-shell`), ime, standby (PreparePowerOff/On 1/1), homekey, wd-hang, wd-crash, escape and the `live` after each PASS. `shell` FAIL, `frames` FAIL (MINOR lines below), `GATE: FAIL`.
  The stub was `stub`, faults 0, at the start and after the run.
- `shell`: every sub-check ok (idle Home 1 commit/s and 1 % CPU, round 15: 10/s, 3 %; D-pad, gear, Input, volume display, toast, quick panel over an app, covered Home 0 %) except `saver`: `power2/turnOnScreenSaver` answered, the shell's `saver` stayed false for 15 s, and LG's own
  saver did not start either (no `com.webos.app.screensaver` window). Not the stub fault (run 2 had the stub on, faults 0; run 1 failed it too but had no Home). Later in the same build and session `turnOnScreenSaver` made the shell's saver true within 1 s in 4 of 4 tries, a key
  ended it, also after the gate's own sequence (volume key, toast, Home Hub app and its quick panel, close, Home, 4 s): `features/saver/log.txt`. So the failure belongs to the gate session's state at that point, cause not found. One lead: the shell's vote
  (`SaverVote.qml`) opens `registerScreenSaverRequest` subscriptions on both tvpower lists at each renewal and the compositor's list of the shell's luna subscriptions only grows: 26 at the gate's saver step, 141 after the app loops.
- `frames` at 60 Hz, full set, 3 cycles (`frames-60-full-gate.json`): home-dpad PASS (first frame 32.7 ms); picker PASS (24.0 ms, burst end 157.4); keyboard-luma PASS (12.0 ms), keyboard-lg PASS (5.0 ms). MINOR: quick `burst_over20` 3 (first frame 35.4, burst end 152.9 ms);
  settings-scroll `burst_over20` 5 (first frame 38.9); toast first frame 71.0 ms (limit 50); apps-grid first frame 67.2 ms; switcher first frame 70.4 ms (the compositor asked the shell 46.0 ms after the key); home-widgets 1 dropped frame (first frame 29.9);
  transitions-netflix 3 launches and 3 leaves, `lateGaps` 4 (worst 32 ms), `log_errors` 3 (Netflix's own previous-crash report, `LOW_MEMORY` killed by the system, in its log); transitions-settings 2 launches (the surface asks 3), `lateGaps` 0. The set's memory is short: swap in use 599 of 600 MiB
  after the gate (round 15: 476 MiB), MemAvailable 493 MiB, CmaFree 70 MiB.

Apps, each from a fresh start under LumaTV's compositor (gate's apps check: the card mapped and SAM naming it foreground, running and mapped 10 s on with the connection kept, closed through SAM; `apps/`):

| app | id | in front | after 10 s | closed through SAM |
|---|---|---|---|---|
| YouTube | youtube.leanback.v4 | 1.0-1.5 s | running (16 of 16) | exit 0 every time |
| Plex | com.beb.plxnative | 1.0-1.5 s | running (16 of 16) | **SIGSEGV (exit 11) in 6 of 8 closes that followed a YouTube run**; see below |
| Netflix | netflix | 1.5 s | running | exit 0 |
| Prime Video | amazon | 2.5 s | running | exit 0 |
| Crunchyroll | com.crunchyroll.webapp | 6.5 s | gone, Home in front (retest below: stays) | no status |
| LG Channels | com.webos.app.lgchannels | 1.0 s | gone, Home in front at 4.7 s (retest: stays) | no status |
| LG browser | com.webos.app.browser | 1.5 s | running | exit 15 (SIGTERM), as round 15 |
| GeForce NOW, NVIDIA | com.geforcenow.play | 0.5 s | LG's browser in front, Home launched over it stays in front 6 s later (the gate) | browser exit 15 |
| GeForce NOW, LumaTV's | org.nphil.lumatv.gfn | 2.5 s | running | exit 0 |
| Moonlight | com.limelight.webos | 4.0 s | running | exit 0 |
| Apple TV | com.apple.appletv | 52 s | running once up | exit 9 (SAM's kill after a 5 s hold; round 15: 9 as well) |

- **PlxNative's close still crashes (BLOCKING).** YouTube 10 s, close through SAM, Home 4 s, Plex 10 s, close through SAM, 8 times (`apps/ytplex-rows.txt`): Plex's own crash log gets a report 0.5 s after 6 of the 8 closes (00:20:33, 00:21:06, 00:21:39, 00:22:46, 00:23:53, 00:24:27 Z; the iterations began at 00:20:04 and ran
  33 s each); an earlier run of the same loop that did not stop at 8 crashed 23 of its 25 (`apps/plxnative-crash-log-*.txt`), and three more iterations on a fresh compositor crashed 3 of 3. YouTube closed with exit 0 every time. SAM's `APP_CLOSED` line for a crash comes about 1 s after the app has left `running`, so the gate's `apps` check and the
  `exit` column of my loop read it too early: gate run 1's Plex crashed (23:17:40 Z) while its check said PASS. The signature is new: round 15's was libSDL2 `+0x875c6` (r7 NULL), these are in libwayland-client only (`addr 0x0`, pc `+0x4acc`, lr `+0x4f21`, and `addr 0x3c`/`0x7c`, pc `+0x4494`), the SIGSEGV that `8bfa77d` meant to remove. The client's connection
  closes 11.0 to 11.8 s after its launch, with or without the crash, so the compositor's journal does not tell them apart.
- The compositor of the long session (pid 20102) ended it spinning: 99.9 % of one core on its loop thread (`/proc/PID/syscall` read `running` 8 of 8 times, all 62 CPU minutes in the main thread, the other threads under 1 %), `top` load average 15, 61:58 CPU minutes after 71 minutes, with every panel-care switch off, `hz` 60, no recording. A fresh compositor on the same
  Home idles at 1.5 to 1.8 % with the same 70 to 78 passes a second and 10 frames drawn and 10 to 12 commits a second, so the pass rate is the build's and the spin is a state the session produced. Not the cause (each measured on the spinning one or on a fresh one): 120 Hz (not set), MaliitServer (killed, still 100 %), the shell's frame-callback mode, a client that crashes
  (3 of 3 Plex crashes, 1.5 %), 40 Home and app switches (the shell's tvpower screen-saver subscriptions 16 to 81, 1.3 to 1.7 %), `ctl` clients closed or killed mid-reply. The spin began about 14 minutes into the session by the CPU time [INFERENCE], during the first, unbounded loop of YouTube and Plex launches (about 25 Plex crashes in 15 minutes) [INFERENCE: a leak per crashed client];
  the process was ended by the session restart at 18:17 before this was tried. The set also ran short of memory by then: swap 599 of 600 MiB used, MemAvailable 543 MiB; Netflix reported `LOW_MEMORY`; Crunchyroll and LG Channels left the front within the 10 s of one run (retest: both stay).

Feature checks, each switched on for its check from Home and put back (`features/<name>/log.txt`, pictures in `img/`):

- Keyboard (`keyboard luma`): steps 1 to 3 PASS: `can` has `keyboard`, `own.flag` true, LG's Search (`launchMode vkb`) gets the shell's sheet 2.0 s after the launch, 1920x454 at y 626, `asked` 0, `serving`, `grabbing`, the journal's `entered; the shell's keyboard serves it`
  and `is on the screen`; OK, Right, OK typed `gh` (`counts.texts` +2, `surround` len 2); a held Right for 2 s moves 9.5 steps a second (`counts.nav` +19). Fall-backs PASS: `keyboard lg` brings maliit's panel (1920x438 at y 642, `asked` +1), and with the shell restarted as the
  field opened maliit's keyboard still came up. Back with the keyboard up: the sheet goes and Search closes with it (the journal has Search's text model leaving and its window unmapping right after the sheet went, and the same under maliit's keyboard; round 14 saw Search stay). The Backspace step was not
  verified (my key path typed a letter: the focus starts on the key last pressed, not on the first). Not run: web fields, hardware keyboard, Netflix/YouTube search, voice (the frame markers are the gate's keyboard surfaces: PASS).
- App switcher (`remote.appSwitcher`, then `recents`): PASS on steps 1 to 7: off, Home twice gives two `launchDefaultApp` and no switcher; on, `can` has `switcher` within 3 s; one tap over Plex is Home alone (0.3 s); twice within 0.25 s opens it with `Home twice within 400 ms`
  and `the shell placed the cards`; Right then OK puts the next app in front in 0.3 s, Back closes, on Home one tap opens and a second closes it, pointer hover and click open a card, Back from the top of Home with `recents` opens it, Down then OK on the focused card sent
  `closeByAppId` and the app left SAM's list, Quit all asks and Cancel changes nothing, `ctl switcher quitall` closes the three apps and none of LumaTV's own. Pictures (`switcher-open.jpg`): Plex's card shows its picture; YouTube's card is black and Netflix's
  is not drawn (label only), both reported `live` by `ctl switcher` (round 14: Netflix black, INFO): MINOR. Step 8 and 9 are the gate's `homekey`/`escape` and the `switcher` surface (first frame 70.4 ms, MINOR above). Set back: `appSwitcher` and `recents` false.
- Transitions (`lumatv transitions on`, Quick Access off and the number keys on for the run, all put back): off by default and off again (launches 9 to 9) PASS. Plex from its dock tile (number key 4): origin tile 134.4 px, `launches` +1, `leaves` +1, Home back in 0.2 s, `lateGaps` +1 (23 ms);
  YouTube by launch: `lateGaps` 0; Plex then YouTube over it: `switches` +1, `lateGaps` 2 (31 ms); HDMI 2: `lateGaps` 0, `veilCaps` 0, `skipped` 0; reduced motion: `lateGaps` +1. Launch then Home 100 ms later: Home was lost and Plex stayed in front (round 14: Home won), INFO.
  Live TV, 3 cycles (in front within 1 s and through 8 s, Home back within 1 s in all): launch 57, 28, 26 frames, leave 12, 12, 10 frames; `lateGaps` 2, 0, 1: cycle 1 a 30 ms gap in the launch and a 56 ms gap in a fade, cycle 3 a 52 ms gap in the leave (`lateLog`
  `{"ageMs":4000,"gapMs":52,"phase":"leave"}`); the journal says `tick before 1.2 ms, this 1.1 ms, last render 0.5 ms, held ticks 0`, and the ring has `P 35.6 to 47.5 ms in one pass (input)` lines around them: the loop was busy in a pass its input handling woke (the `ctl` reads of the
  check itself are control-socket input [INFERENCE]); round 15: 0, 0, 1 gaps of 24 ms. MINOR. Idle Home with transitions on: 626 ticks in 10 s.

- Overlays (`overlays.enabled`, set from Home: Home's 2 s theme poll sleeps while an app covers it, so a switch turned on over an app reads `ui.cards.on` false and shows nothing; my first run set it over Netflix and failed steps 1 and 2 for that reason): PASS. Off: a door card shows nothing and `takes` is `[]`;
  on: `ui.cards.on` true. A timer card over the Netflix film appears within 1.0 s top right from y 134 (`card-over-film.jpg`), the film keeps its video window and subtitles; `takes` is `[]` at first and `["ok","back"]` 1 s later (armed); two Right presses reach the app (keys delivered +4, dropped 0);
  OK on a card with no button takes it away, a new card and Back dismiss it and Netflix stays in front; an alert and then a doorbell: the doorbell shows first and the alert after it is dismissed (`alert-doorbell.jpg`, `alert-after-doorbell.jpg`); two timers stack 16 px apart (`two-timers.jpg`); a score card shows the leader in the full ink (`score.jpg`);
  an LG alert raised over a card takes the screen and the card steps aside (`dnd-alert.jpg`); a held Back (1.4 s) leaves Netflix for Home; switched off again, nothing shows and `takes` is `[]`. Over Home the door shows no card (the helper reads the door while an app is in front): INFO. Not run: the keyboard and PIN do-not-disturb cases, a real Home Assistant record, 5 minutes of cards over Netflix, the frame gate's `overlay-*` surfaces.

- OLED care, the shell's own screens (`oledcare.enabled`, `stepSec` 10, `dimAfterMin` 1, `dimOverMin` 2): steps 1, 2, 6 and 7 PASS. `enabled` within 2 s; Home's offset walks one pixel on one axis a beat ((0,0), (-1,0), (-2,0), (-3,0), (-3,-1), (-3,-2), (-3,-3): 9 beats in 100 s), never past 3;
  with the moving wallpaper off (`wallpaperMotion.enabled` false: a moving Home is not still) the dim goes 0.94 to 0.85 in 0.01 steps over about 2 minutes and holds at the floor, a remote key puts it back at once; with the saver up the offset stands still; switched off, the offset is (0, 0) in the next read.
  Not run: 3 (video), 4 against the off numbers (see the idle tick below), 5 (frames).
- App care (`appDim`, `appShift`; `appDimAfterMin` 1, `dimOverMin` 1): FAIL, it never engages. `app.window` names the card in front (Plex, LG's browser) and `stillSec` counts, but it falls back to 0 every 4 to 5 s on every page tried (Plex's library, LG's browser start page, and a static `data:` page of two light boxes in the browser), so
  `app.engaged` stayed false over 150 s hands off on two apps; steps 2 to 5, 8 and the 7 are not exercised. On the browser's start page the compositor's commit count moves in bursts of 1 to 12 every 4 to 5 s. Lead [INFERENCE]: `Scene::app_view` counts a commit that has no damage rectangle as a full redraw (`s.current.damage` `None` is the whole surface),
  and these clients commit now and then without one. Both switches put back off. The pointer put-back read (0.03 s, level 1, offset (0, 0)) holds trivially.
- Box dim (`boxDim`, `boxDimAfterMin` 1): FAIL, it never engages for the same reason (`restart` `change` all the time, `cells` 0 over 80 s on the static page), so the capture pair, the key and the saver steps were not run. It samples: 16 samples in 80 s, and the sample pass costs 56.0 ms the first time and 2.80 ms
  from then on (`boxdim: sample N took 56020, 2823, 2801, 2795, 2836 us`; the check asks `sampleUsMax` under 2000 on the Mali): FAIL on the bar, the first one is a 3-frame hitch. Moving through Netflix's menus with `boxDim` on and off (2 rounds each, `ctl refresh` measure, 520 frames each): late 0, pipeline p99 4.75 ms both ways, max 6.2 and 7.6 ms on against 4.9 and 4.8 ms off (the map never engaged, so this is the sampling's cost only).
- Capture (`capture.enabled` in `helper.json`, helper restarted, put back and the files and folder removed): steps 1 to 5 PASS. Off: 403 and `disabled`; on: `GET /v1/capture` `enabled` `available`; a screenshot at Home 1920x1080 in 85 ms, not black; with a toast up the toast is in it, with the quick panel up the panel is, with the cursor shown the cursor sprite is
  (`cap-toast.jpg`, `cap-cursor.jpg`); over the Netflix film the app's controls are in it and the film's area is black (mean luma 11). Writes to the helper need the `X-Luma-Client` header. The encoder steps (6 to 12) wait for the second slot.
- Refresh guard (read-only parts): FAIL. `getPanelUsageTime` with `{}` answers `Failed to validate against schema: Missing mandatory field: 'subscribe'` on 10.2.1-5202 (the helper's reader and `refresh_guard.py` send `{}`): `GET /v1/state/panel` says `read` false, `due` null, and `refresh_guard.py status` says `getPanelUsageTime: returnValue false`; the
  tooling's wake then goes ahead without the guard ("could not be read; waking anyway", as in both gate runs). With `{"subscribe":false}` it answers `panelUsageTime` 25,993 ticks (4,332.2 h); `getPixelRefresherInfoList` gives `offrsCount` 609, `offrsLastTime` 4,330, `offrsInterval` 4: not due until 4,334 h. The Settings row "Panel refresh" stays hidden
  because the topic never reads.

- Cameras (overlays on, `features/cameras/`; `ha-cameras-result.md` steps 1 to 4, 5, 6, 8, 10): the bridge is up (`enabled`, `paired`, port 6054, state `connected`, 14 cameras shared, pop-up kinds person, vehicle, animal and doorbell, 12 s; the helper log has `cameras: the LumaShow integration connected (lumashow 0.2.0, Home Assistant 2026.10.0)`).
  A frame of `camera.exterior_front_door_camera`: 200, `X-Luma-Decoder: hardware`, 960x1280 BMP, decode 9.7 to 10.4 ms, 0.46 to 0.55 s each over ten calls (the first 0.04 s, from memory). A test event (`POST /v1/cameras/event` needs the `X-Luma-Client` header) pops up over Home
  within 1.2 s: top right, the camera's live picture, `OK expands, Back dismisses` (`popup-home.jpg`); a second `person` event within 30 s is `held: cooldown`, two doorbell events both show; OK opens the full view (`full-view-home.jpg`), Back closes it and the `cameras.popup` topic is null.
  Over Netflix playing: the pop-up over the film with the subtitles untouched (`popup-netflix.jpg`), `takes` `["ok","back"]`, two Right presses seek the film (the app's), OK opens the full view, Back closes it and Netflix is where it was; nothing left: the popup topic is null and `lumatv-jpeg`
  ends after its 30 s. PASS. MINOR: a test event carries no time (`at` 0), so the card says `29887237 min ago`. The grid and the live stream (`cameras.live`) were not run.

- Video wallpaper (`wallpaper video`, then `wallpaper gpu`; `features/wall/`): PASS. The compositor's `setDisplayWindow` now carries `originalInput` 0,0,1920,1080 beside `sourceInput` (`displayOutput` 3840x2160, `fullScreen` true) and videooutputd answered `{"returnValue":true}` twice, no `Invalid Parameters`;
  `player` `playing` (`wall-silk-tuned`), `wall.hole` true, `ctl windows` `video` has the placement. LG's VIDEO capture is the clip: `vplane_wall.py probe --video` says `ok`, `silk-tuned` at 7.2 s, full screen and unshifted, correlation 0.998, light 0.95 / 0.93 / 0.91 (R / G / B) of the clip's. The DISPLAY capture is
  black in the hole (`probe`: `black`, margin mean luma 0.00), as in round 15: LG's capture does not see the plane under LumaTV's page. `/proc/lg/de/win0/status` before: `h/v active 0 / 0`, `svp 0`, `lowdelaymode 0`; after: `h/v active 1920 / 1080`, `scanType 1`, `vFreq 600`, `lowdelaymode 1`. The panel itself was not looked at (no camera on
  the set), so the picture and the GPU wallpaper's hand-over (a step of about 7 %) stay unseen. Restored: `wallpaper gpu`, the player idle.
- Idle cost: `ctl status` ticks 70 to 78 a second on idle Home with every panel-care switch off and on (`stats.ticks` and `ctl wake` n over 40 s; 10 frames drawn and 10 to 12 commits a second); a fresh compositor does the same at 1.5 to 1.8 % of one core, so the passes are cheap.

- 120 Hz (`--only frames --refresh 120 --frames-quick`): stopped by Main at 18:25 after the idle read, no surface ran. `GATE refresh`: `hz 120 divider 1 shell 120` over 10 s, DE2VSYNC 120.9/s, osd_irq 120.3/s, 121 frames, late 0, lost 0, missed 0, pipeline p50 4.75 and p99 4.89 ms, Mali busy 11.7 %, 59 degrees, `slides: none` (nothing had slid yet);
  `gate refresh 60` and the divider back to 2 in the clean-up.
- Findings outside the feature list: the Home stub's guard gives LG's Home back (fault 1 of 2) when LumaHome's beat stops for 5 s with the panel on and LG's compositor live, 30 s after a wake from Active Standby with the TV idle on Home (the first run); the gate's frame stage (`gate.py`) leaves `wallpaperMotion.enabled` false when it is interrupted (the owner's file
  then says false and the next run restores to false); `SaverVote.qml` opens `registerScreenSaverRequest` subscriptions on both tvpower lists at each renewal and the shell's `lunaOpen` list only grows (26 at the gate's saver step, 141 after the loops, 81 after 40 switches on a fresh compositor).
- End state (18:27 PDT, on Main's word to stop): LumaTV's compositor session ended by the gate's clean-up (LG's compositor live, `lumatv compositor status` `live` false, no `lumatv-compositor` process), Home stub `stub` with faults 0, `theme.json`, `Tokens.js`, `layout.json` and `helper.json` byte for byte as found (md5 checked), `keyboard lg`, `transitions off`,
  `wallpaper gpu`, capture off, overlays and care switches as found, `inputs` running `[]`, only Home running, `filmMaker`, Always Ready (`alwaysReady`), the TV on at Home (not put into standby: the refresh guard's standby tracking, step 3, was not run). Receiver: found off, on at 0.585 unmuted after the TV's wake; left on, unmuted, 0.58
  (the integration's `volume_set` rounds 0.585 to 0.58).
- Not run: `volume` in the gate (done by hand: receiver 0.605, Volume+ 0.61, Volume- 0.605, PASS); the 120 Hz surfaces; keyboard steps 5 to 7, 9 (as the gate's surfaces), 11 and the Backspace key; transitions 3 to 5, 8 and 10 with a measured eye on the frames (the gate's surfaces ran at 60 Hz); overlays 7 (real records), 8 and 10; OLED care 3 to 5; app care and box dim beyond the
  stillness and sample cost (they never engaged); the Settings OLED care group (my key path ended on AI access, nothing pressed there, `img/s3-oled-care-tab.jpg` is that page; the group's render is unseen); refresh guard steps 3, 5, 6; cameras 7, 9, 11 and the live path; capture 6 to 12 (encoder); the venc second slot (pending, only on Main's word).

### Round 17, 2026-10-29 (the TV's clock 23:54 on the 28th to 02:27, PDT 20:54 to 23:27 on the 28th; the TV's clock is 3 h ahead of PDT), forge/main `0d6d249` (`compositor/slot-2026-10-29-r17/`)

Installed by Main through HomeLabber (44 s, 23:54:04 to 23:54:48 on the set; the set had booted at 21:53:58). Verified on the set: helper sha256 `021b2a9495fa…` (the bundle's), compositor 2,584,328 bytes md5 `96aaafcd0cf7…` (`/tmp/lumatv/bin`, the persisted copy and `compositor/deploy/bin` equal), `x2cli` equal; only Home was running (PlxNative, in front when Nitin left, was gone after the install).
Receiver through Home Assistant's MCP: found on, 0.36, unmuted, TV Audio, DOLBY DIGITAL; muted for the round and again each time the set unmuted it (the Netflix and HDMI checks of the gate, 00:31, the standby's wake: its own CEC state); the gate's shell check's Volume+ left it at 0.365 (put back at the end).
The gate ran with its REST calls disabled and `--skip volume` (the volume check by hand, below).

- **Full gate at 60 Hz** (00:00 to 00:46 EDT, `gate-full-gate-lines.txt`, `gate-run/`, `gate-full-stdout.txt`): firmware, homeheld, live, netflix, hdmi, apps, keys, ime, shell (`saver` ok: the vote leak is fixed, `lunaSubscriptions` 8 after about 30 app starts against 141 in round 16), standby (PreparePowerOff/On 1/1), homekey, wd-hang, wd-crash and escape PASS, each `live` after them PASS.
  **x1 FAIL**: 1 gap over 20 ms of 3598 (20.6 ms, 59.95 fps, the compositor's side clean: loop stalls 0, frame gaps 0, pacer late 0), at t=18 s; `frames` FAIL (below); `GATE: FAIL`. Three more runs of the x1 check alone (01:16 to 01:22, `x1-rerun-*.txt`): 1 gap (21.1 ms; the compositor logged 1 loop pass or page-flip interval over 20 ms), 4 gaps (worst 23.8 ms, pacer late 0 to 1, the compositor logged 2),
  1 gap (20.0 ms, compositor clean); 59.93 to 59.95 fps each. 4 of 4 runs fail the rule (round 16: one gap of 20.1 ms in run 1, none in run 2). `--kwait` could not name the wait: `the loop thread is not in the trace: pass --tid` (LumaTV's loop thread is `ld-linux-aarch64`, the tool looks for `surface-manager`).
- `frames` at 60 Hz, full set, 3 cycles (`frames-60-full-gate.json`): home-dpad PASS (first frame 28.2 ms), quick PASS (36.7; burst end 153.4 ms), picker PASS (27.7), home-widgets PASS, keyboard-lg PASS. MINOR: toast first frame 86.6 ms and no burst end (limit 50 / 180; the shell was handed the notification 44 to 112 ms after the spawn; round 16: 71.0),
  settings-scroll `burst_over20` 5 (round 16: 5), apps-grid first frame 59.5 ms (67.2), switcher first frame 102.0 ms (the compositor asked the shell 50.1 ms after the key; round 16: 70.4), transitions-netflix `lateGaps` 1 (worst 78 ms; round 16: 4 of 32), transitions-settings `burst_over20` 1 and 1 launch (the surface asks 3: 1 launch, 2 leaves, 1 change of the front left alone),
  keyboard-luma `burst_over20` 1 (an interval of 33.4 ms at the open; round 16: PASS). `GATE wake (frames)`: tick p50 0.02 p99 0.04 max 10.5 ms, flip max 0.23 ms, input max 0.12 ms; `mem`: rss 61,384 KiB, swapped 0.
- **Standby and the panel refresh** (gate's standby check, 00:34 to 00:42; `refresh-standby-watch.txt`: the helper's `panel` topic and LG's numbers every 45 s over SSH, nothing touching the panel): the topic tracked the standby at once (`standby {since, dueAtStart true, countAtStart 609}`); the driver printed `the set has been in standby 0 min (from the caller) and LG's panel refresh is due (run 609 at 4330 h; panel at 4335.5 h, every 4 h); waiting until 15 minutes of standby have passed`;
  LG's `offrsCount` went 609 to 610 and `offrsLastTime` 4330 to 4335 between 6.0 and 6.8 minutes of standby, so **the count rises at the end of the run (about 7 minutes)**, and the wake guard ended its wait on it (woke at 7.5 minutes, the topic `due false`, `lastStandbyMinutes 7`). Refresh guard checks 1, 3, 4 and 5 PASS, 2's Settings row below; 6 (the helper's guard with an assistant) not run: no paired assistant.

Apps, each from a fresh start under LumaTV's compositor (the gate's check per app: the card mapped and SAM naming it foreground, running and mapped 10 s on with the connection kept, closed through SAM, SAM's `APP_CLOSED` status, then 4 s for a crash to be logged; `apps/`, journal slices in `apps/list/`, `apps/ytplex/`):

| app | id | in front | after 10 s | closed through SAM |
|---|---|---|---|---|
| YouTube | youtube.leanback.v4 | 1.0 s | running (10 of 10) | exit 0 every time SAM logged it |
| Plex | com.beb.plxnative | 1.0 to 1.5 s | running (every time) | **SIGSEGV (exit 11) in 9 of 10 closes after a YouTube run, 3 of 5 alone, 4 of 4 in the stall runs**; see below |
| Netflix | netflix | 1.0 s | running | closed, no status line |
| Prime Video | amazon | 2.0 s | running | closed, no status line |
| Crunchyroll | com.crunchyroll.webapp | 6.0 s | running (stays) | closed, no status line |
| LG Channels | com.webos.app.lgchannels | 1.0 s | running (stays) | closed, no status line |
| LG browser | com.webos.app.browser | 1.0 s | running, connection lost once | closed, no status line (round 16: exit 15) |
| GeForce NOW, NVIDIA | com.geforcenow.play | LG's browser in front | Home launched over it stays in front 6 s later (3 of 3, and the gate's) | closed |
| GeForce NOW, LumaTV's | org.nphil.lumatv.gfn | 2.5 s | running | closed, no status line |
| Moonlight | com.limelight.webos | 1.0 s | running | closed, no status line |
| Apple TV | com.apple.appletv | 51 s | running once up | closed, no status line (round 16: exit 9) |

- **PlxNative's close still crashes (BLOCKING).** YouTube 10 s, close through SAM, Home 4 s, Plex 10 s, close, 10 times (`apps/ytplex-rows.txt`): Plex's own crash log gets a report at 9 of the 10 closes (the 10 closes' SAM lines: exit 11 for 6, none for 3, exit 0 for 1, so SAM's line is no detector: the gate's apps check passed at 00:03 while its Plex close crashed, 1 report in the log before the loop).
  Every report is the signature of round 15, not round 16's: SIGSEGV in libSDL2 `+0x875c6` (`pc` minus the library's base, `addr 0x18`, `lr +0x724b5`); the libwayland-client crash of round 16 is gone. The 3 other loop runs: Plex alone 3 of 5 (`apps/plexalone-rows.txt`), 4 of 4 in the stall runs: YouTube before it is not the cause.
  What the app does (its SDL lines in the syslog, `apps/plexstall-poll.txt`, and its threads read 0.35 s and 0.8 s after the close request, `apps/plexstall-syscalls*.txt`): a close request at t, `SDL_Quit start` at +24 to 32 ms, `Wayland_DestroyWindow` at +35 to 44 ms, `WebOS_EGL_UnloadLibrary unload` at +46 to 47 ms, and then nothing for 950 ms:
  in a clean close `UnloadWEBOSBASELibrary start` follows 9 ms later. The stalled quit has its main thread in an untimed futex wait and the Mali blob's `mali-cmar-backend` thread in `poll(1 descriptor, 1000 ms)` on the app's Wayland socket to the compositor (the other end is `ld-linux-aarch64` = the compositor, read from `/proc/net/unix`),
  so the app waits for something from the compositor, or for that poll's 1 s timeout; LG's SDL runs `callback_forceQuit` 1000 ms after the close event, the second `SDL_Quit` runs while the first is stuck, and dies (`APP_CLOSED` exit 11 at about +1.05 s). The compositor logs nothing and holds nothing: during the stall `held` reads `owedCallbacks 0`, `owedBuffers 0` or 1 (the frame in flight), `displayHeld 0`;
  it destroyed Plex's surface at +79 to 98 ms (surfaces 31 to 29, windows 4 to 3) and sees the connection end at +1.09 to 1.10 s with the process; the clean close ends its connection at +84 ms. Lifetimes of the client in the journal: 11.93 to 12.37 s in the 9 crashes, 10.97 s in the clean one (disconnect before the foreground push to Home).
  Round 15: 1 of 9; round 16: 6 of 8 (libwayland-client); now 9 of 10.
- **Idle after the loops** (the compositor 20 minutes old, about 30 app starts, 9 Plex crashes behind it; `apps/status-after-loops.json`, `ring-after-loops.txt`): 1.9 % of one core (loop thread 1.7 %), 73.6 ticks, 12.2 frames drawn and 12.4 commits a second, **102 to 127 loop passes a second** (the ring's one-second lines: woken by clients, tick, flip, luna, draw-now, and the control socket while `ctl` polled; round 16's healthy loop 70 to 78), no `loop: N passes` line in the journal, no spin.
  `held`: `owedCallbacks 0`, `displayHeld 0`, `lunaSubscriptions 8` (141 after round 16's loops), `surfaces 29`, `windows 3`, `seat` keyboards, pointers and touches 26 each: the counts of the two live clients (the shell and WebAppMgr; the gate's end state had four clients and 58 surfaces, the same per client). The gate session's log has two `input: /dev/input/eventN: No such device` warnings (the virtual remotes of the checks going away) and no spin.
- **120 Hz** (`--only frames --refresh 120`, 01:23 to 01:56, `gate-120/`, `frames-120-full.json`, `gate-120-lines.txt`): FAIL. Idle Home, 10 s after the switch: `hz 120 divider 1 shell 120`, DE2VSYNC 120.7/s, osd_irq 120.1/s, 145 frames, late 0, lost 0, missed 0, pipeline p50 4.75 p99 4.94 ms, Mali busy 13.3 %, 59 degrees, `slides: none`.
  After the surfaces (the measure left running since the switch, 43,234 frames): late 105, lost 2041, missed 11, pipeline p50 4.75, p99 5.25, max 341.3 ms, Mali 13.4 %, 58 degrees, the shell's slides (steps/ms/for) `15/115/110` mostly and `6/121/110`, `5/110/110`; back at 60 Hz (divider 2): late 0, lost 0. Per surface (first frame ms, `burst_over20`, `dropped`, slides and their fewest steps; limit 13 steps for 110 ms at 120 Hz):
  home-dpad 21.3, 1, 0; quick 19.2, 2, 0, burst end 193.4 ms (limit 180), 24 slides, 13; picker 89.6 (limit 50), 0, 0, 24 slides, 6; toast 31.8, 23, 0, burst end 278.5 ms; settings-scroll 26.1, 32, 3, 16 slides, 7; apps-grid 29.4, 36, 1; home-widgets 21.9, 0, 3; switcher 64.2 (limit 50), 0, 0; transitions-netflix ok; transitions-settings 2 launches of 3 and 2 log errors, 5 slides, 6;
  keyboard-luma 845 and keyboard-lg 1134 intervals over 10 ms (8 and 17 dropped): LG's Search and maliit draw at 60 Hz, so every interval of their bursts is over the 120 Hz threshold: not the compositor's. Round 15's quick set at 120 Hz: quick 14, picker 10, toast 3, home-dpad 2 (`burst_over20`).
- **Direct scan-out at 120 Hz** (`--scanout direct`, scanout-research.md check (d); `gate-120-scanout-stdout-interrupted.txt`, `scanout/scanout-counters-interrupted-run.json`): stopped at 02:23 after 8 of 12 surfaces for a hardware test, so no per-surface table; the switch's counters read live: **0 direct frames of 22,946** composed. Refused: `layers` 22,606 (`2 pictures on the screen: a system layer, keyboard or member over the window`),
  `geometry` 327 (`the window does not cover the output`: Home's offset from the OLED care's orbit, the owner's theme has `oledcare.enabled` true; the first reading of the run, 100 frames: geometry 74, layers 13, transition 12, video 1), transition 12, video 1. No import, framebuffer, test-commit or acquire count (`framebuffers` imports 0, failures 0), descriptors created 43, destroyed 33, retained 6, tap active, shell env `WAYLAND_AFBC=0`.
  Two things kept every frame composed: the OLED care's one-pixel orbit moves Home off the origin, and a second picture (the system layer: the receiver was muted for the round, so the mute mark [INFERENCE]) is over Home. The comparison with round 15's 120 Hz numbers is therefore not a test of the plane; checks (a) and (b) were not run.
- **LG's quick settings** (Nitin's question; `quicksettings/`): LG's `com.webos.app.quicksettings` lives inside LG's surface-manager, so its lines in the syslog can only come while LG's compositor is live. The syslog keeps 5 to 25 minutes (it rotates every 1 to 5); the lines of 00:42 to 00:46 were still in it when a watcher started, which copied each new line from then on: **no line of it while LumaTV's compositor was live** (live periods in `quicksettings/lumatv-live-timeline.txt`).
  Each start of LG's compositor (every fallback and every `cutover.sh disable`) preloads it hidden: `created` then `CLOSE_SYSTEMUI_BY_APPID` 0.15 to 0.45 s later, never `visible` (00:42:44 homekey, 00:44:37 wd-crash, 00:45:32 escape, 01:16:07, 01:18:15, 01:22:38, 01:56:15, 02:23:54 session ends). **Twice it came to the foreground and was visible** (`NL_VSC visible:true`): 00:43:43.5 to 00:43:45.5 in the wd-hang check (compositor stopped at 00:43:38, LG's started; the tooling's Home recovery closed it [INFERENCE]) and 01:20:37.0 to 01:20:44.6 after the second x1 rerun's `cutover.sh disable`,
  both at an LG compositor start, with no key from the tooling (neither step injects one) and LumaTV not live; what asked for it is not in the retained logs (a physical remote key at that second is the candidate). The syslog before 00:26 is gone: the gate's `keys` and `shell` checks (gear and Menu through the shell, `gear-shell ok`, `quick-home ok`, `quick-app ok`) ran under LumaTV's compositor.
- **LG ads and ACR since the boot** (read-only, the set booted at 21:53:58; the declutter hook): `bootd.log` has `com.webos.service.pbsw` (pbs) subscribing at +16.8 s and `contentminer` at +17.9 s, and systemd has `contentminer.service` active 21:54:17 to 21:54:41 (it ended by itself; masked now): **both ran at boot**. The hook's first pass: units killed 21:55:08, group `ads` on 21:55:11 = **73 s after the boot**; until it binds the stubs the on-demand services (`acr2`, `admanager`, `adoverlay-service`, `sportsalarm`, `pbs`) can be launched by ls-hubd as the real programs.
  For `acr2`, `admanager`, `adoverlay-service` and `sportsalarm` there is **no record at all for that window**: the syslog keeps about 25 minutes, the early-boot log covers the first 13 s and holds none of the four, no process accounting exists, no file of theirs was written in it, so whether they ran in the first 73 s is not provable here. After the pass they did not run: all five stubs are mounted, nothing of them is running, and the 8 kept minutes of syslog show only stub launches that exit 0 in about 70 ms (pbs 10, sportsalarm 12; acr2, admanager and adoverlay-service none).
  `declutter.sh early` masks units only; the stubs of the on-demand ones come with the first pass at +73 s. Closing it takes the stubs (or the masks) into the earliest boot hook.
- **GeForce NOW app account** (header tier, Play, Sign out): not run, the app is not in this ipk (a package of its own): the one on the set is the 2026-10-26 build (`lumatv-gfn-app` md5 `db62520227a0…`, no `Sign out`, `stale_account` or `account.subscription.get` in it), `rustui/build/gfn` has no build of `bcefff5`. Nitin does not need to sign in again.
- **End state** (02:25 EDT, on Main's word to stop for Nitin's hardware test): LG's compositor live (`cutover.sh disable` by the gate's clean-up, no `lumatv-compositor`), Home stub `stub` faults 0, only Home running, `inputs` running `[]`, `filmMaker`, Always Ready (`alwaysReady`), the TV on at Home (not put into standby, so the panel refresh waits: LG ran one at 00:34 to 00:41, `offrsCount` 610, the next is due at 4339 h; the panel was at 4335.5 h then).
  `layout.json`, `Tokens.js` and `helper.json` byte for byte as found (md5 checked), `theme.json` as found except `oledcare.stepSec` 30, `dimAfterMin` 5 and `dimOverMin` 10 (the shipped defaults; the found 10, 1 and 2 were the test timings of earlier rounds; the `oledcare`, `appDim`, `appShift` and `boxDim` switches stay on as found), `keyboard lg`, `transitions` on (as found), `wallpaper gpu`.
  Receiver: found on at 0.36 unmuted (TV Audio); left on, 0.36, unmuted, TV Audio, DOLBY DIGITAL.
- **Not run (stopped at 02:20 on Main's word for a hardware test)**: the off-screen scan-out proof (a) and the KMS pattern cases (b); every feature check of the list (OLED care: own screens, app dim and shift, box dim, refresh guard's Settings row, the OLED care group's picture; volume hold; keyboard Backspace; transitions with measured frames; overlays 7, 8 and 10; capture 6 to 12; cameras 7, 9, 11 and the live path), and the Netflix and Prime lag measurement;
  Run: gate, x1 reruns, the app loops and table, Plex's close diagnosis, idle and held counts, 120 Hz, the privacy read, the quick-settings log.
- Findings outside the list: the gate's `apps` check can pass over a crashing Plex close (SAM's `APP_CLOSED` line is missing or late in 4 of 9 crashes: it read a clean close at 00:03 and the app's own crash log has the report); `--kwait` cannot find LumaTV's loop thread (`ld-linux-aarch64`); the gate's driver extracts its tarball over the old files in `/tmp/lumatv-tv-out/out/gate`, so a run's directory holds earlier rounds' files (the evidence here was re-extracted clean).

### Round 18, 2026-10-29 (the TV's clock EDT 09:21 to 11:40; PDT + 3 h), forge/main `54faa35`, then `28f6d8b`, `6bff4a6`, `2a68516` and `750bd99` (`compositor/slot-2026-10-29-r18/`; the gates ran on `2a68516`)

Built on the host (about 5 minutes with the earlier rounds' target directories copied in): helper sha256 `4e7793b4…`, `ui` bundle (`lumatv-ui` 33.4 MB), card and gfn apps, the gfn ipk (38.2 MB) inside the main ipk (81.35 MB, 151 QML compiled, 0 failed). Installed six times through HomeLabber under standing grant `0073bf55` (uses 1 to 6 of 20; each read back by md5 on the set): `54faa35` at 09:32 (138 s), `28f6d8b` at 10:02, the same again at 10:04 (deleting the scratch wrapper below left `bundle.manifest` listing `lib/lumatv-ui` at 345 bytes and the session start refused: the install rewrites it), `6bff4a6` at 10:42, `2a68516` at 11:00, `750bd99` at 11:37. `LT gfn status`: `installed` and `current` true, build `6ded73c8eb68dc39` = shipped (the app on the set before was the 2026-10-26 build). The set was in Active Standby with the receiver off at the start; its wake at 09:39:14 switched the receiver on over CEC (0.36, unmuted, TV Audio, DOLBY DIGITAL); it was muted through Home Assistant's MCP, unmuted by the volume keys twice (muted again each time), and Nitin's 3 s Volume+ hold took it 0.36 to 0.425 (13 steps, no step back at release; put back to 0.36 and muted). The gate ran with its REST calls disabled.

- **Read-only (steps 3 to 5, `readonly/`)**: the refresh guard reads LG's numbers (count 611, last run 4339 h, every 4 h, panel 4339.5 h = 26037 ticks / 6, not due: LG ran a refresh in standby after round 17, so this round could only measure the not-due path). Early-boot scripts: `bam`, `rec-info-adapter`, `rec-listmgr` call a bare `luna-send` first, `nn-proxy.sh` and `kdump.sh` a bare `grep`: a hook before kdump's +13.4 s is possible through the units' `EnvironmentFile`s (the kdump one already carries `PATH=/var/lib/lumatv/earlybin`). `declutter.sh status` (the app's own tool; the checklist's `dc-dev` copy has no `early` object): `early.hook` true, `thisBoot` false (no early pass yet), `/sys/power/mode` `resume`; lean profile 15/16 in settings and 3/4 in connectivity: `option.dbgLogUpload` is not false (nothing changed). Scan-out proof (a): `stock`, `afbc0` and `big` all give AB24 AFBC `…0151`, 3 of 3 imports ok, 0 refused; the `afbc0` hint is ignored (route B closed); `big` CmaFree 52.9 to 13.9 MB during, back to 53.2 after; `x2 check ok 6 bad 1` in all three (the one bad is a 7th check after the client had gone).
- **The Rust shell's first run on the set (`rust-first-run/`)**: round 17's bundle had no `lumatv-ui`; `54faa35`'s `ui` bundle could not start it. Six defects, each reported and fixed during the round: (1) the compositor looked for `lumatv-ui` beside `current_exe()`, which under the supervisor's private loader is the loader (`28f6d8b`, `ead2a6e`); (2) the 64-bit ELF's interpreter `/lib/ld-linux-aarch64.so.1` does not exist on the set, so it must be started through the loader (`ead2a6e`); (3) `ctl shell rust +kind` on a compositor started with no kinds opened the client's channel after the loop had registered its wake fd, so `rust.connected` stayed false (`ead2a6e`); (4) the client sent `Host: 127.0.0.1` without the port and the helper refuses it (`host_not_allowed`; `28f6d8b`); (5) `scene.rs shell_home_covered` returned true with no `Layer::Home` in the stack, so the Rust Home never activated and every key was dropped (`28f6d8b`); (6) two mute marks with the QML volume display and the Rust OSD both on (`28f6d8b`, `LumaOverlay.qml`). Round 18's workaround until `28f6d8b` was a scratch `lib/lumatv-ui` wrapper execing the loader (removed). On `28f6d8b`: runtime `+home` works (pid set, connected, engine, `rust.ui.home.home` true, `wants` true, `covered` false; the D-pad moved focus statusbar, widgets, dock with 12 keys delivered and 0 dropped), `+osd +toast` (toast in 0.19 s of the call, the volume bar at the first poll, `gl errors 0`), the quick panel, input picker and drawer open on the gear, Input and `show drawer` and close on Back, 24 Settings pages open with `ctl shell show settings PAGE` (`rust-settings/`), `+cameras` opens "Scrypted isn't linked" (steps 1 to 9 need the bridge: Nitin's). `6bff4a6` (HomeGapsFix) fixed what `28f6d8b` still got wrong (the helper closes each connection and the client reused it: "helper not answering" in AI access, During the screensaver and Receiver and the quick panel's Receiver row; an empty `extraLargeIcon` hid `largeIcon`: 11 of 23 dock icons blank; the At home card read a topic no plugin publishes; Profiles names under the avatars; the Settings bar): all six checked with screenshots (`img/r18-v6-*.jpg`). A seventh finding is open: **the focus plate of a focused row has its rounded corners cut square at both ends by about 12 to 16 px** in the quick panel and in Settings (Nitin saw it; `img/r18-clip-*.jpg`): the list viewport's clip is exactly the list's width (`settings/surface/list.rs:72`) and the plate is grown by the focus lift. And an eighth: the **first Rust kind enabled on a running compositor stalls its loop** (the engine attach compiles the shader programs there, "compiled and linked in 1547 ms") and the supervisor's heartbeat took it for two missed beats and fell back to LG's compositor at 10:35:16 (`crash-10-35-16/`; a gate loop was launching YouTube at the same time, so the mix is not clean); from then on the kinds were put in `shell-rust.conf` before the compositor started.
- **Shader link off the loop (`750bd99`, ShaderStall's `db122f7`; installed 11:37 after the gates)**: fresh session with no kinds, `ctl shell rust +settings +quick +picker +drawer` at runtime: the command returned in 0.04 s, the client answered 1.78 s later, no fallback (failures 0). Log: `ui: 48 programs linked on a thread of its own in 1301 ms, drawn once in 22 ms`, `engine built in 1.8 ms: 0 programs linked here, 48 taken from the cache`, `the engine attached in 2.2 ms on the loop thread`; `passUsMax` 25.5 ms; three `ctl shell restart`: built in 2.4, 2.5, 2.3 ms with 0 linked, attached in 2.8, 2.9, 2.7 ms (`rust-first-run/shaderstall-check.txt`). The 10:35 fallback is not reproduced on this build.
- **Nitin on the set (10:07 to 10:40)**: Rust Home, gear panel and a 3 s Volume+ hold: "felt pretty smooth"; the Magic Remote's pointer showed when he only picked the remote up. The device reports `ABS_X` 0 to 1920 and `ABS_Y` 0 to 1080 with fuzz 0 and flat 0, so any 1 px change is a motion event and `seat::pointer_motion` shows the cursor at once. His decision: the pointer shows only on a turn of the wheel, never on motion or a wheel press, and an arrow hides it. The Magic Remote devices (`LGE M-RCU - Builtin [0..2]`, `TONE+`, `W-RCU`, `Simple Premium`, event2 to event7) declare REL = WHEEL (code 8) only, no hi-res wheel; their keys include `KEY_ENTER` 28 (not `KEY_OK` 352) and `BTN_LEFT` 272, `BTN_MIDDLE` 274. A real wheel turn and press were not recorded (the passive recorder stayed empty). `2a68516` (PointerWheel `ae3c348`): `ctl pointer aim 100 100` leaves the cursor hidden, `ctl wheel` shows it at (100, 100), an arrow hides it, `remote.pointerShow` `movement` takes effect in 0.8 s and `wheel` again in 0.5 s (the `ctl pointer X Y` command still shows it: it is the control path).
- **Step 7, PlxNative's close x15 (BLOCKING): FAIL** (`apps/`, build `6bff4a6`, callbacks at the draw): the app's own crash log got a report at 12 of 15 closes (passes 1, 3 to 9, 11, 12, 14, 15; clean 2, 10, 13), all `SIGSEGV libSDL2 +0x875c6, addr 0x18, lr +0x724b5` (24 to 36 reports; SAM logged `exit_status 11` for 8 of them). YouTube 15 of 15 clean; GeForce NOW's browser step ok. The compositor's journal separates the passes exactly: the 12 crashes have the connection living 11.86 to 12.29 s with Home's foreground push before the disconnect, the 3 clean ones 10.90 to 10.95 s with the disconnect first: the crash is the 1 s stall after the close request. The flip A/B (`ctl callbacks clients landing`, 10 passes, `2a68516`): 10 of 10 crashed. So `112d9cc` (callbacks at the draw) takes it from 100 % to 80 % and does not fix it (round 17: 9 of 10; round 16: 6 of 8). PlexDeep owns the root cause now.
- **Step 8, x1 (BLOCKING), 10 runs (`x1/`, build `2a68516`)**: default (callbacks at the draw) PASS, PASS, FAIL; flip (`--x1-callbacks landing`) PASS, PASS, FAIL; client SCHED_FIFO 10 (`--x1-rt 10`) FAIL, PASS, PASS; the PLAN's strict rule (`--x1-strict`) PASS. 59.94 to 59.96 fps in all ten, and the only client gap over 20 ms in nine traced runs were def1's 20.1 ms (the client's own wake-up: attribution COMPLETE, the gate passes it) and land3's 20.4 ms. The three failures are two INCONCLUSIVE runs (def3, land3: the trace and the raw log disagree about a gap, which is never a pass) and rt1's pacer `late` 1 to 2 with no gap. No gap paired with a late flip, a loop wait or a late callback. Round 17: 4 of 4 failed the strict rule.
- **Not run, and why**: steps 9 and 10 (the surface frame gate at 60 and 120 Hz, the rest of the gate), 10a (the automated `--only rust` over all kinds), 17 to 25 (OLED care, app dim, box dim, transitions, Backspace, overlay cards, capture, cameras, saver), 26 to 29 and 31 (hardware planes and scan-out (b) to (d), OSD0_1, Prime lag: display changes that need Nitin at the set, and no raw register writes this round), 30 (Netflix lag), 32 and 33 (a reboot needs the Face ID gate; the consents are Nitin's), 34 (the panel is not due, so the 15-minute standby rule cannot be measured): the round was stopped after step 8 (its budget): the time before went to the Rust shell's start-up defects, five redeploys and Nitin's pointer and clipping reports. Their order for the next round: 10a, 9, 10, then 17 to 25.
- **Process incidents**: three gate starts between 10:14 and 10:32 died with the tool call that launched them while the set's own `gate.sh apps` loops kept running (two at once, with a round-17 sampler and a `gate.sh evidence` behind them) until killed at 10:39: when a host driver is killed its set-side script must be killed too, and a driver must be started detached (`setsid nohup`, returning at once). One of those starts switched the set to LumaTV's compositor at 10:32 while Nitin was in a menu (his urgent screenshot came out black; ended and returned to LG's compositor at once). The compositor session that crashed at 10:35 is the one above.
- **Go-live: NO**: PlxNative's close crashes at 12 of 15 (the blocking gate), and the frame, shell and refresh gates (9, 10, 34) were not run. LG's compositor is live and the persistent flag is untouched.
- **End state** (11:40, `end-state.json` against `start-state-post-install-final.json`): LG's compositor live (`flag` off, no `lumatv-compositor`, failures 0), the persistent flag untouched; no Rust kind on (`shell-rust.conf` empty), the scratch wrapper off the set (`lib/lumatv-ui` absent since the 10:04 install), `750bd99` installed; only Home running, `inputs` running `[]`, `filmMaker`; `theme.json`, `layout.json`, `helper.json` and `Tokens.js` byte for byte as after the last install that rewrote them (md5 `07242387…`, `f5954cd0…`, `48b5aabe…`, `ee51e2db…`; the pointer-switch test's two `theme set` were undone by putting the file back), `keyboard lg`, `transitions` on, `wallpaper gpu`, Always Ready as found (`alwaysReady`), no `ctl` switch left on (`callbacks` back to early). Receiver: found off, switched on by the set's wake (0.36, unmuted, TV Audio, DOLBY DIGITAL); put back to 0.36 and unmuted, then off with the set's standby (CEC, 11:39:54), as at the start. The set was put in Active Standby at 11:39:54 (nobody had used the remote since about 10:40; the panel refresh is not due, so the 15-minute rule was not in play). No gate, driver or scratch process left on the set or here; my scratch files under /tmp on the set removed.

### Round 18b, 2026-10-29 (the TV's clock EDT 12:11 to 13:10), forge/main `f2e4b5d` (`compositor/slot-2026-10-29-r18b/`)

Built and installed twice from `f2e4b5d` under grant `0073bf55` (uses 7 and 8; compositor md5 `fe01e3b8…`, `lumatv-ui` `b1e7f83e…`, read back on the set). `ctl status` read `"clientCallbacks":"commit"` in every session.

- **Step 7, PlxNative's close x15 on `commit`: PASS** (`apps/legA/`). `GATE apps: PASS`, 15 of 15 YouTube and 15 of 15 Plex passes `alive-at-10s yes connection-lost 0 closed-in-15s yes`, SAM `exit_status` 0 for every close, GeForce NOW's browser step ok. Plex's crash log kept its 46 reports (the newest, 11:08 EDT, predates the run). Plex's connection lived 10.86 to 10.97 s in all 15 (Home's foreground push after the disconnect), YouTube 11.28 to 11.33 s (the first 11.81 s); `callbacksAtCommit` 6,427.
- **Leg B, the old pacing** (`ctl callbacks clients early`, `LUMATV_APPS_REPEAT=5 gate.sh apps`, `apps/legB/`): FAIL, Plex crashed in passes 2 to 5 (4 of 5; the crash log grew 46 to 50, each report `SIGSEGV addr 0x18`, pc ending `75c6`, lr ending `24b5`; SAM `exit_status 11` in passes 3 and 4); connections 11.89 to 11.95 s with the foreground push before the disconnect; pass 1 clean. Back on `commit` afterwards. This reproduces the round 17 and 18 failure and the cure.
- **Step 8, x1 on `commit`, four runs** (`x1/c1..c4`): c1 PASS (raw strict PASS, worst 19.6 ms), c2 INCONCLUSIVE (the trace implies 1 gap, the raw log counted 0; raw strict PASS, worst 19.2 ms), c3 PASS (2 client-only gaps of 20.4 and 21.8 ms reported and not counted, raw strict FAIL), c4 PASS (raw strict PASS, worst 20.0 ms). 59.94 to 59.95 fps in all four, `loop stalls 0`, no gap that pairs with a late flip, a loop wait or a late callback. The INCONCLUSIVE run is not a pass; its rerun (c4) is.
- **Rest of the gate** (`gate-rest/`, `--only shell,frames,standby --frames-quick`): shell FAIL (every row ok except `quick-app`: the quick panel over an app, `overlay 'quick', wants false, system mapped true, home drawn false`); frames FAIL (home-dpad and apps-grid ok; quick first frame 120.1 ms with `burst_over20` 1, picker `burst_over20` 1, toast first frame 60.0 ms, settings-scroll first frame 51.4 ms and `burst_over20` 4, home-widgets `burst_over20` 2); standby PASS (rounds 1 and 1, fallbacks 0, wake ok; the panel was not due).
- **Focus-plate clip (`c65415b`), Rust quick panel and Settings Picture** (`clip/`): both ends of the focused row's plate are rounded and whole, in the quick panel ("Sleep timer") and in Settings ("AI Picture Pro"); round 18's reproduction cut both ends square by 12 to 16 px.
- **Go-live: NO.** Step 7 and x1 pass, but the shell (`quick-app`) and frames checks fail, so LG's compositor stays live and the persistent flag is untouched.
- **End state**: LG's compositor live (flag off, failures 0), no Rust kind on, only Home running, `inputs` running `[]`, `filmMaker`, `alwaysReady`, Active Standby; `theme.json`, `layout.json`, `helper.json` and `Tokens.js` as after round 18's last install (md5 `07242387…`, `f5954cd0…`, `48b5aabe…`, `ee51e2db…`); the receiver was off when the round began and is off again.

### Round 18b triage (offline): `quick-app` and the frame lines

- **`quick-app` failed in the gate's reader, not in the layer.** `ctl shell` prints its keys sorted, and since `d196b64` (after round 17's build) it carries a `rust` object ahead of `stats`, `systemLive`, `takes`, `topics`, `ui` and `wants` with keys of the same names. The check's `shell_bool wants` took the first `"wants":`, the Rust client's (`false` while no Rust kind is on), for the QML client's. The same read doubled the numbers of the result line (`starts 0` then `1`). The step's own journal has `shell: the system layer wants the keys` and `focus -> surface 107` after the gear tap, and `overlay 'quick'` and `system mapped true` were read right: the layer asked for the keys and had them. Round 17, the last gate on the old shape, passed the step with `wants` read from the QML client (`lets go of the keys` after Back). `tv-gate.sh` now reads every `shell_*` value, the diagnostics and the final line through `rustjson.py --without rust` (`34118b6`); `GateShell` in `tests/test_cutover.py` models the QML client's report with the Rust client's beside it: on the old script it reproduces round 18b's line, on the new one a healthy shell passes every row, and a layer that never wants the keys (with the Rust client saying true) and a Back that leaves the panel up still fail `quick-app` by name.
- **The frame lines are the known minor class, and the callbacks at the commit were not in play.** The surfaces of `--frames-quick` run over Home with the stub's card behind it and no app drawing, so the cards' pacing (`f2e4b5d`) has nothing to act on: of the session's 16,657 frame callbacks 15,682 went out ahead of their flip (the shell's), so at most 975 (6 %) were anything else, over 20 minutes that held the Home Hub's card and the stub's [INFERENCE: no other card was open]. The loop was never busy when a tick came due (`tick waited on a busy loop` max 0.79 ms, n 310; round 17 66.5 ms, round 16 32.8 ms), `p90_render` 0.05 to 0.39 ms, the compositor 1.9 % CPU on idle Home (1.5 to 1.8 % earlier), and the GPU is no busier while the panels slide (the Mali's busy time in the first gap of a quick or picker window, median 12.8 and 12.9 ms; round 16 12.0 and 12.5, round 17 13.4 and 13.2). In all 157 gaps over 20 ms that the frame gate names, the compositor's loop thread ran 6.0 ms at most (4 % of a gap) and waited for a CPU 2.9 ms at most (in the 58 gaps of 70 ms or less 1.35 and 0.88 ms): the late intervals are frames the shell did not make, not frames the compositor made late. The per-run base rates of the 23 earlier runs (rounds 8 to 17) of the four surfaces they all had: quick first frame over 50 ms in 1 run (125 ms, round 10) and `burst_over20` in 4; picker `burst_over20` in 11 (up to 5); toast first frame over 50 ms in 17 and `burst_over20` in 10; home-dpad `burst_over20` in 4.

  | first frames of three cycles (ms), `burst_over20` / `dropped` | round 16 | round 17 | round 18b |
  |---|---|---|---|
  | home-dpad | 29, 18, 33; 0 / 0 | 5, 20, 28; 0 / 0 | 30, 28, 18; 0 / 0 |
  | quick | 3, 35, 1; 3 / 0 | 3, 37, 19; 0 / 1 | 4, 38, **120**; 1 / 1 |
  | picker | 24, 23, 22; 0 / 0 | 27, 27, 27; 0 / 0 | 36, 18, 35; 1 / 0 |
  | toast | 71, 48, 65; 0 / 0 | 8, 87, 59; 0 / 0 | 60, 48, 22; 0 / 0 |
  | settings-scroll | 35, 32, 39; 5 / 3 | 37, 32, 30; 5 / 2 | **51**, 36, 33; 4 / 2 |
  | apps-grid | 67, 30, 28; 0 / 2 | 60, 37, 33; 0 / 1 | 37, 30, 37; 0 / 0 |
  | home-widgets | 30, 29, 17; 0 / 3 | 27, 19, 29; 0 / 0 | 19, 32, 19; 2 / 0 |

  MINOR, none noticeable in use: toast 60 ms (limit 50; the lowest worst case of the three rounds), picker and home-widgets one 33 ms interval at the start of a burst (home-widgets' two are in two of three cycles, 33.4 ms after a first frame at 18.6 ms), settings-scroll `burst_over20` 4 (5 and 5 before) and a first cycle 1.4 ms over the limit (51, 36, 33 [INFERENCE: the cold open of the Settings page]), and quick's third cycle, where the compositor asked the shell 0.12 ms after the key and the shell's first frame came at 120 ms (cause not found; round 10: 125 ms, one in 69 opens until now). The D-pad walks draw twice as many frames as before (home-widgets 314 against 137 a walk, apps-grid 279 against 197, home-dpad 95 against 58) and home-widgets' `over20` fell from 31 to 4: `ca9e353` (Oct 29, 01:53, after round 17's build) made the focus, card and scroll motion 260 ms with a 60 ms rest and eases the lift down as well as up [INFERENCE: the change between the builds that moves the walks' motion], so these surfaces' counts and first frames are not comparable one to one with rounds 15 to 17.
- **What the commit pacing costs an app, on the model** (`flips.rs`, `a_card_that_draws_on_every_callback_...`): a card that draws its next frame on each callback commits once a refresh with the callbacks at the draw or the flip, and once for each spare buffer at the commit (two a refresh with three buffers, the first of the two replaced before it is drawn), so an app that animates continuously does twice its own drawing while it does. The loop's work does not change (a tick draws once whatever was committed). Not measured on the set: an app animating while an overlay slides over it. `tv_gate.py --frames-clients early` runs the surfaces with the apps' callbacks at the draw and `gate.sh callbacks` now counts the ones sent with a commit, so that one session can A/B it: the quick set has no app drawing (the count says so), so the A/B is for the full set, whose switcher, transitions and keyboard surfaces have apps running, and for the over-an-app surfaces (`quick-netflix`, `picker-netflix`, `overlay-netflix` of `docs/research/perf/gate.py`, `ctl callbacks clients early` or `commit` first). A cap on the callbacks answered at the commit would put the present thread's wait back [INFERENCE: the wait is what crashed PlxNative's close, 12 of 15], so it is not made without the 15-pass apps loop (`--apps-repeat 15`) after it.

### Round 18c, 2026-10-29 (the TV's clock EDT 13:42 to 14:53), forge/main `5888f19`, `f2e4b5d` still installed (`compositor/slot-2026-10-29-r18c/`)

No install: the set kept round 18b's build (compositor md5 `fe01e3b8…`, `lumatv-ui` `b1e7f83e…`, read back). The gate ran from `5888f19`, which includes the reader fix `34118b6`. Callbacks `commit`, Rust kinds off, receiver muted through Home Assistant and put back (unmuted at 0.36).

- **Shell alone** (`shell/`): PASS, `quick-app ok`, tail `starts 1, exits 0, luna calls 169, refused 0`.
- **Full gate** (`--skip volume --apps-repeat 15`, `gate-full/`, 13:45 to 14:34): firmware, homeheld, live, netflix (Dolby Vision, mode back to filmMaker), hdmi, keys, ime (keyboard at y 642), shell (quick-app ok, luna calls 411, refused 0), standby (rounds 1 and 1, fallbacks 0; the panel was not due when it ran), homekey, wd-hang (3.0 s, crash bundle), wd-crash (0.5 s, crash bundle), escape (recovery 16 s) all PASS. Apps: 15 of 15 for YouTube and for Plex, every row `alive-at-10s yes connection-lost 0 closed-in-15s yes`, SAM `exit_status` 0 for every close, GeForce NOW's browser step ok, Plex's crash log stays at 50 reports (no new one). Tick wake-up p99 0.04 ms, max 0.77 ms; `mem` swapped 0.
- **x1** (`x1/`): six runs on `commit`, 59.94 to 59.95 fps, `loop stalls 0`, pacer late unchanged, no gap that pairs with a late flip, a loop wait or a late callback. PASS in three (full gate worst 20.4 ms client-only; two with raw strict PASS, worst 19.4 ms), INCONCLUSIVE in three (the trace implies one gap more than the raw log counted; 1, 1 and 0 client-only gaps over 20 ms, worst 21.2 ms), none with a compositor-attributable gap.
- **Frames** (full set, 12 surfaces, 3 cycles): FAIL on the known minor class, `dropped` 0 and `gl errors` 0 everywhere, `p90_render` 0.05 to 0.37 ms. Home-dpad, home-widgets and both keyboards ok. MINOR, none noticeable in use: quick `burst_over20` 3 (first frame 35.0 ms, burst end 169.0 ms); picker first frame 118.9 ms with the compositor asking the shell 0.19 ms after the key (one cycle; round 18: 89.6 ms) and `burst_over20` 2; toast first frame 75.4 ms; settings-scroll `burst_over20` 4; apps-grid `burst_over20` 1; switcher first frame 73.3 ms (the compositor asked the shell 51.3 ms after the key; round 18: 64.2); transitions-netflix `burst_over20` 1 and 2 late gaps (33 ms) in 3 launches; transitions-settings 2 of 3 launches and 1 late gap (33 ms).
- **Refresh**: the panel became due during the run (usage 4343 h, last run 4339 h, interval 4 h, count 611); the round ends in Active Standby untouched so LG's run can start.
- **Go-live: YES, by the manual route** (`golive/log.txt`): `cutover.sh enable` (`verified.list` already held 10.2.1-5202): `enabled (persistent)`, `cutover status` mounted yes, flag persistent, firmware verified; `gate.sh live` PASS; `settled: two minutes healthy` at 14:49:27; `lumatv restart` (compositor restart) at 14:50:08, `gate.sh live` PASS again, `settled` again at 14:52:09, failures 0, escaped false. `--enable-after` does not fire while `frames` fails on the minor class. Not done: the runbook's reboot and the key walk with Nitin.
- **End state** (`end-state.json`): LumaTV's compositor live (flag persistent, LG's the automatic fallback, hold Home 650 ms the way back), only Home running, `inputs` running `[]`, `filmMaker`, Active Standby (`alwaysReady`), `theme.json`, `layout.json`, `helper.json`, `Tokens.js` unchanged by md5, the receiver unmuted at 0.36 on TV Audio before it went off with the TV, as at the start.

### Round 19, 2026-10-29 (the TV's clock EDT 16:03 to about 18:15), forge/main `3ff3b31`, `5e4fafd`, `3f8efad`, then `4163c65` installed (`compositor/slot-2026-10-29-r19/`)

LumaTV's compositor was live (persistent) throughout except for the installs. Four installs under grant `826d5330` (uses 1 to 4 of 20; exit 0 in 59.3, 234.8, 55.3 and 47 s; read back equal each time). The second one's `lumatv apply` restarted LG's compositor and the panel stayed black about 3 min until `recover-home.sh` ran. Last build: compositor md5 `a7045972…`, `lumatv-ui` `e7efd254…`, helper sha256 `08a1d306…`.

- **Step 10a, `gate.sh rust` on `3f8efad`** (`rust-gate-2/`): `GATE rust: PASS`. home (drawn +104, gl errors +0, D-pad status bar > widgets > status bar), osd (first 230 ms), toast 110, alert 100, quick 220, picker 90, drawer 100, settings 370, cameras 80, saver 380; menu, confirm, switcher, cards, popup, capture and layout not driven. Two gate defects fixed in the tools: the Home check's D-pad walk assumed the QML Home's start (`rust_home_dpad`), and the shell and keys readers read the QML client's fields only (`tv-rustjson.py --merge-rust` folds in the Rust client's panel, osd, toasts, saver, wants and can).
- **Full gate with the Rust kinds on** (home, osd, toast, quick, picker, drawer, settings, entry; `--skip volume --apps-repeat 15`, 17:08 to 17:24, stopped for Nitin's looks): firmware, homeheld, live, netflix, hdmi, keys and ime PASS; x1 INCONCLUSIVE (59.95 fps, 3 client gaps over 20 ms, worst 20.7 ms, pacer late 7 to 9, the trace implies 4 gaps against 3 raw). **Apps FAIL: PlxNative's close crashed in passes 5, 9, 11 and 14 (11 of 15 clean)**, YouTube 15 of 15. New signature: SIGSEGV addr 0x0 in `libwayland-client.so.0.20.0` +0x4acc (`get_next_argument` with a NULL signature: the client parsed an event of an object or opcode it does not know); rounds 17 and 18 crashed in libSDL2 +0x875c6, addr 0x18. Home's foreground push comes before the disconnect in 3 of the 4 crashes, after it in every clean pass. **Shell FAIL** only on `saver (a key did not end it)`: with the saver kind off the QML saver is on, and while the Rust Home wants the keys it never gets them (`shell_rust_takes_key`). Not run: frames at 60 and 120 Hz, standby, homekey, wd-hang, wd-crash, escape, x1 three times, the kinds-off A/B of apps, and the hardware-plane steps (A1/A3 `ctl afbc` and `ctl pagealpha`, P0 to P2, V2 to V6, the orbit by offset).
- **BootBlocks, one plain restart** (16:49:56, resume boot; `bootblocks/`): update block PASS (bound at 13.26 and 14.17 s, `nsuImageSize` 0, the four names on loopback, no update prompt); iot-client stub PASS (21 programs stubbed, `ls-hubd` launches ended in 74 and 39 ms); the doc's timing bars FAIL (`stubbedSec` 19.08 against 16.9, `maskedSec` 22.08, uptime 24.47; pbs started at 16.72 s, contentminer at 17.61 s); `early.json`'s bootId is the same across resume boots, so `thisBoot` cannot tell them apart. The restart cleared a lginput2 thread that had spun at 100% of a core since 15:33 (trigger unproven). Not run: the second boots (after `snapshot-boot-manager --remove`, and the skip-file boot).
- **Nitin's looks and what they found**: the Rust Home's highlight and icons look right (sharper than QML); Settings (all three pages), the TV Name entry sheet (Back cancels, name unchanged), the gear panel, the input picker, the drawer (grouped by source), toasts, alerts and the saver drew right. Defects: Settings leaked every key and OK to Home underneath (fixed in `5e4fafd`); Home at rest did not open the switcher (fixed in `3f8efad`); the mute mark covered the quick panel's title (fixed in `3f8efad`); the top dock row's name was drawn below its tile and covered the row beneath, and launching an app showed an "Opening" pop-up and a black frame, returning flashed black (HomeParity, `cf6dcab` for the name, the launch and return not re-tested); the card glass reads more opaque than QML's; the gear panel opens smoothly with an occasional flicker (the Rust shell logged "the scene is full" for the quick panel, the entry sheet and the saver, once each); the Home dock has no confirm on Uninstall (his removals of Homebrew's hyperhdr loader and GeForce NOW were deliberate; fixed in `f7b4610`).
- **Phantom launches: the cause is key 1199.** Three launches had no key within seconds and each came 3 to 9 ms after the Magic Remote's "on table" sleep (17:32:57 `com.geforcenow.play`, 17:37:36 punktfunk, 17:37:50 the hyperhdr loader). A passive recording of `/dev/input/event*` (read-only, no grab, `wake/evdev-capture-2.txt`) shows, on event2 `LGE M-RCU - Builtin [0]`, EV_KEY 1199 (down and up, one stamp) at every lay-down (21:54:48.434, 21:57:26.089; lginput2: `uinput: but=(CUSTOM_KEY_CURSOR_OFF)` right after `_mrs_cond_a2sr_ on table`), and `input.rs` took 1199 for the wheel press, so the focused tile launched. The real wheel press is EV_KEY 28 (`[KEY_OK]`, 197 ms down), the wheel notch EV_REL code 8, and 1198 (CUSTOM_KEY_CURSOR_ON) a keep-alive every 0.5 s while the pointer is active. `f7b4610` makes 1199 the cursor-off (hides the pointer, no client hears a key); `9ba69a2` swallows a release that had no press.
- **The remote's first 4 s after a rest are lost: LumaTV's compositor does not answer LG's input manager.** At each reconnect (hidraw0 added) lginput2 calls libim over the POSIX queue `/IM_MQ_WEBOS`, whose reader is LG's compositor; nothing reads it now, so `IM_MQ_Connect` and `IM_RegisterCursorInfoCallback` time out at 2 s each and the remote is attached 4.000 s later (8 of 8 reconnects, e.g. 21:46:35.527 to 21:46:39.528); the first key then reaches the kernel 3 ms after, and the compositor sees every evdev key within 0.19 ms (`ctl wake` input p99 0.05 ms, n 10,448). The queue holds 2,520 bytes of unread requests (35 of 72 bytes), lginput2 leaks four queue descriptors per reconnect (60 now, limit 1024), and the remote's cursor registration never completes (`registerCursorCallback ret : FALSE`). Fix: answer the queue from the compositor or a helper (ImQueue); not landed this round. LumaTV's GeForce NOW app (`org.nphil.lumatv.gfn`) opened in 2.0 s (shaders compiled and linked 1,302 ms at its start, no cache); the compositor had 10 frame-interval warnings in its 71 s, so the late responses are not the compositor's [INFERENCE: the 4 s reconnect window]. The punktfunk tile's pulse was not analysed (the Rust Home model has no launch pulse).
- **Default decision: NO kinds as the default.** The rule (a failed blocking check leaves the kinds off) applies: PlxNative 11 of 15 with the kinds on, and no kinds-off run to tell whether the Rust Home causes it. The kinds were switched off at 17:43 on Nitin's report of keys and menus opening by themselves, and switched on again at Main's and Nitin's request at 18:07 after install 4 (the 15 kinds in `shell-rust.conf`: alert, confirm, displaylab, drawer, entry, home, keyboard, menu, osd, picker, quick, saver, settings, switcher, toast; pointer mode `wheel`). Per kind: osd, toast, alert, quick, picker, drawer, settings, entry passed the automated gate and Nitin's looks and wait for the apps A/B; saver must go with home; home is the open one (launch and return transitions, the phantom fix and the uninstall confirm not yet gated); switcher, keyboard, menu, confirm and displaylab passed no gate this round (the switcher looked right after `3f8efad`; the Display lab's read-only baseline is `osd_sr_gain 0x10`, `osd_sr 0x1`, `osd0_vsp 0x02000001`, holes 0, and its door answers `available true`); cameras stays off (needs the Scrypted bridge).
- **Receiver** (`media_player.living_room_avr`): found 0.52 unmuted on TV Audio, DOLBY DIGITAL; the restart moved it to 0.50 unmuted; muted for the gate and the Display lab; restored at the end of the round (see the slot's `end-state.json`).
