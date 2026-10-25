# LumaShow lessons for LumaTV (2026-10-25)

## Resume here

Survey of /data/home/LumaShow by a read-only scout. LumaShow numbers are from the Echo Show (960x480, Mali-T720, 59.64 Hz); re-measure on the C4 (1080p, Mali-G52). Ranked list below; tags: a QML surfaces now, b Rust helper, c Rust compositor, d frame-probe/gate harness, e startup/memory/standby.

RANKED TRANSFER LIST (tags: a=QML surfaces, b=Rust helper, c=Rust compositor, d=frame-probe/gate harness, e=startup/memory/standby; PLAN = already in LumaTV PLAN, conflict marked)

A. Compositor rendering (highest value)

1. (c) **Hardware overlay planes for the wallpaper and opaque ground.** Overlay route draws wallpapers into one plane and everything above into another, composed by display hardware; each plane redrawn only when changed. Default since 2026-10-02 (`overlay-with-direct-fallback`, commit 1ee8c0d, merge 33db207). Measured T2 render p50 4.88 vs 6.16 ms (direct); T3 6.36 vs 6.95 ms; same screens passing. Overlay gain by tier (2026-09-29 table): median frame 4-22% shorter; T2 passing 18/20 both routes. Ground rule: a picture/shape that is opaque and covers the whole panel, with everything under it, moves to the ground plane (`scanout/plan.rs`). Home wallpaper ground: GPU median of pull frames 3.98 -> 2.69 ms; render 6.18 -> 4.79 ms. PLAN: consistent ('direct scan-out' is in own-compositor gains). New detail: the opaque-ground detection and the 'plain fill' profile line.

2. (c) **Direct scan-out beats copy at late commit.** VERIFY.md: direct route holds 60 fps with commit up to 11 ms after the previous refresh; copy route drops to 30 fps at >=9 ms. Per-frame CPU: copy 5.1 ms -> direct 1.9 ms; compositor CPU 29% -> 11% of a core. Copy route moved 1.84 MB per frame. Missed refreshes ~1 in 10,000 on both. PLAN: consistent; numbers are new.

3. (c) **Fence after draw, not glFlush.** On Show's Mali, `glFlush` does not start the scan-out draw; a fence after the draw does (`gl::Fences::kick`). First frame 8.6 -> 3.4 ms. Pitfall: an `eglFlush`/`glFlush` assumption will silently cost a frame. [INFERENCE] LG Mali-G52 driver needs its own check; it is a cheap test.

4. (c) **Pacer: start late, aim early.** `crates/compositor/src/pacer.rs`. Start time = as late as the slowest recent frame (of 8) would still make its refresh, plus SLACK_NS = 5.5 ms (commit-arrival window). Target refresh = the next one unless a typical frame cannot make it. Refresh timeline phase learned from the flip-return timestamp (FBIOPAN_DISPLAY, vsync to within 0.1 ms on that kernel). FLIP_LEAD 0.5 ms. Slot advances exactly one refresh per frame so animation steps are even. PLAN: not present. Check LG's vsync source (webOS compositor flip signal); [INFERENCE] the phase trick needs a vsync timestamp.

5. (c) **First animation frame after a scene-reshaping commit starts at once.** Commit 76110dce. Alarm frame 6.6 -> 13.1 ms and notifications 5.7 -> 11.6 ms before the fix; the latch sized on the old scene lost a refresh. Generic lesson: any pacing estimate must reset when the layer set changes.

6. (c, a) **Forecast-driven quality ladder ('Smooth first: Full').** `ladder.rs`. Five steps, each a pure function of the layers: (1) live blur becomes a cached picture, (2) wallpaper at most 30 fps, (3) wallpaper stands still and 3D orbit stops, (4) shadow half-size and sheen half-strength, (5) glass becomes its tint. Trigger: pacer headroom predicts late frames (headroom = refresh - median typical frame - lead - slack; 10.77 ms at 59.64 Hz). Only at a settled boundary (not during a touch or settle), one step per 8-frame window, give-back needs 10 s without a step and 3 s of predicted fit, at most ~12 changes/min. Cost below Full: two assignments per frame; at Full with no step, median of 8 numbers. Setting offered only if the compositor can do Full. Not yet proven on device at the time of writing: 'nothing here has run on a Show' for the forced_load proof. Status: new to LumaTV; a strong candidate for a 'hard 60 fps' fallback. Product choice for Nitin: whether a visible quality step is acceptable.

7. (c) **Glass ideas (product-visible or not).** `still_glass`: glass that stands is drawn once into a panel-sized plate and copied (needs 3 standing frames; keyed on content so any change re-plates). T3 p50 -1.7 to -5.1 ms on five screens (ha_dashboard 12.1 -> 7.0). `adaptive`: moving surface loses refraction, blur held; dialog p95 15.4 -> 12.6 on ha_dashboard. `half_res`: blur at half resolution, -0.25 to -0.5 ms p50. Defaults since 10-01 (commit 14da3fb, T3 mean render p50 8.90 -> 6.94 ms; no screen slower). Nitin saw no visible difference. Pitfall: `still_glass` on intercom made plates it never used (one-in-two-frame change); fixed with the 3-frame stand rule.

8. (c) **Feathered-clip glass uses its plate.** The home hero card under a clip was blurred live every frame: 4.8 ms of each shade pull frame. Fix: plate keyed on clip + feather; blended copy, not overwrite. Shade pull frames 426 -> 8 over 12.5 ms.

9. (c) **Blur hold with per-picture phase.** A moving blur is recomputed every 4th frame; each held blur gets a phase = rank among held pictures, so only one is recomputed per frame. Before: three blurs in the same frame, 12-14 ms against 8.3 spread out. Pure scheduling; cheap to copy.

10. (c) **Damage tracking.** 16-px tiles, `damage::Region` up to 6 rectangles, damage rings of 3 pages (direct) / 2 (copy). `merge_passes` priced by layer count: alarms scroll 5 rects collapse to one cover, 7.8 -> 6.4 ms; p95 6.99 -> 6.39 ms. Generic; PLAN lists damage tracking, so not new in kind, but the merge pricing is new.

11. (c) **Skip frames that show nothing.** A commit that changes no visible layer draws no frame (`engine::shows_nothing`, trace code 'N'). Catch-up frame: triggered after 34 ms quiet with >=60,000 px changed (`idle::CATCH_UP_AFTER_NS`, `CATCH_UP_MIN_PX`); first-frame 11.5 ms fix.

12. (c) **Covered-layer culling.** Layers under an opaque cover are not drawn (`LUMASHOW_DRAW_OFF=cull,damage,rects` ablates it). Ties to item 14.

13. (c) **Pictures not copied under an opaque curtain.** `Renderer::draw_frame` / `looked_at`: picture-text was copying 1,961,000 px per frame; now glyphs are quads (see 17).

14. (c) **Draw-cost model.** `DRAW_WORTH` = 63 us per 6,144 px layer draw; full-pass about 10 ns/px on Mali-T720 at 299 MHz; full-screen layer ~0.8 ms. Used by the staged/merge planner. For LG re-measure (4.5x pixels at 1080p) [INFERENCE: ~3.6 ms for one full-screen layer at the Show's clock before any GPU speed difference].

15. (c) **Defined-texture rule.** Every texture a frame can show is made defined before compositing (transparent black targets, blanked unreadable pictures, no backdrop if blur chain failed). Fixed 'garbage tiles' on the black Show (10-08). Test: `gl::POISON` fills undefined textures with noise; `LUMASHOW_POISON=seed`. Adopt for any GPU texture pool.

16. (c) **Staged group build.** Build inert groups off the interaction path, then one request, 2 refreshes later; a curtain covers quiet_* screens (MAX_GROUP_DEPTH = 9). Before: worst frame 12.6-12.8 ms on those screens. (After-number not in notes.)

17. (a/c) **Text as glyph quads with prepared page.** Shell text is drawn from a page of glyph rows (1024 -> 1344 rows); quiet_* screens 17.0-22.7 ms -> 8.3-12.3 ms via `page::renew_for`. Relevant to QML text-heavy surfaces: the QML analogue is pre-rasterised text textures per page.

18. (c) **GPU warm-up of programs and render targets (not pixels).** First big screen 53-60 ms -> 36.5-40.9 ms; 38-46 ms once at start, reported as a startup cost. PLAN has 'shader warm-up' already: overlap. New: warm targets, and report the startup cost. `LUMASHOW_WARMUP=off` to ablate.

19. (c) **Clock: compositor is sole GPU clock writer.** Boost while frames flow (299/416/520 MHz); `Request::HoldGpuClock`; `gpu_clock.rs`. Overclock to 1.43 GHz needs a kernel OPP (p9 kernel: Nitin OK needed). PLAN: 'governor no effect' applies to CPU; GPU boost is different. Check LG's GPU clock access [INFERENCE: probably none].

20. (c) **Allocation-free frame path, tested.** `alloc_tests.rs`: 0 allocations over 120 dragged frames once warm; `ui/tests/no_allocation.rs` app side. Pattern is reusable as a gate.

21. (c) **Pictures known opaque only after upload.** `renderer::Texture::opaque`, pictures >= screen size; first frame draws them, then they move to ground.

22. (c, b) **Glass dither as vertex varying.** 6.54 -> 6.48 ms p50 (commit 03294cf). Minor; low priority.

B. Helper process and panel ownership

23. (b) **64-bit display helper, 32-bit compositor.** `crates/displayd`: separate 64-bit process because the display manager's interface needs 64-bit; compositor stays 32-bit for its GL driver. PLAN: consistent with 'X2 pass 32-bit libmali buffer import' and 64-bit helper plan. Reuse the split as-is.

24. (b) **One owner of the panel + guard process.** `displayd` takes an exclusive lock at start; a guard is forked before anything is shown and restores the fbdev page (the page the panel showed at that time) if the service dies. A second service or reset finds the lock held. PLAN: no equivalent noted. Adopt the pattern for LumaTV's shell-to-panel handover.

25. (b, c) **`Planes` trait: device-agnostic overlay contract.** `crates/planes/src/lib.rs`: offer, begin, buffer, present, check, capture, reset; 'an error says the frame is not known to be on the panel'; reset must confirm before anything else is put on. Device-specific parts live in `display_mtk` (ion, FBIOPAN) and `service::Buffers`. Reuse the trait; webOS would supply its own impl [INFERENCE: depends on whether webOS exposes overlay/hardware composition at all].

26. (b) **Nice -10 on the display service.** Reduced stamped-late gaps from 24,600 frames to 3 events (graphics-lab, measurement section). CONFLICT with PLAN: PLAN says 'SCHED_FIFO/CPU pinning/governor no effect'. The LumaShow result is a scheduling priority on the display helper, a different measurement. Flag for Nitin/main agent; not a rejection of PLAN, but a second data point that must be re-tested on LG before adoption.

C. Harness and gates (d)

27. (d) **Gate values.** Pass rule: zero missed refreshes; no frame gap > 17.2 ms; render <= 12.5 ms; input latency <= 100 ms; 'in motion' gap 25.2 ms. Values are for 59.64 Hz; at 60 Hz a frame is 16.67 ms, so re-derive (CONFLICT in value, not method). PLAN gates: no frame > 20 ms, first frame <= 50 ms, fully open <= 180 ms, RSS flat over 500 cycles. Compare: LumaShow's 17.2 ms is tighter than PLAN's 20 ms. Both could be kept: the tighter one as the 'smooth' gate.

28. (d) **Missed refresh defined by the pacer model, not by a timer.** A refresh is 'missed' only if a compositor that started the frame as soon as its content was there would have made it (`pacer::Pacer::presented`). Avoids counting refreshes the scene never asked for.

29. (d) **Late-stamp artifact fix.** `presented_ns` stamped late: 4 gaps of 17.7-20.5 ms, each followed by a 1-3.7 ms short gap. Fix: pair each late stamp with its short gap before counting. Pitfall: any frame-probe that reads a late vsync stamp will count phantom 20 ms gaps.

30. (d) **Tiered sweep with ablation switches.** Screens run at T3, T2, T1, T0 (tiers = effect load); each config has its own switches (`LUMASHOW_DRAW_OFF`, `LUMASHOW_LADDER=off`, `LUMASHOW_WARMUP=off`, `LUMASHOW_SCANOUT`, glass `ideas` on/off). 'Off, on, on, off' ordering to cancel drift. Reusable for LumaTV's gate matrix.

31. (d) **Timeline trace.** `LUMASHOW_TRACE` writes a per-frame trace (crates/timeline); used to find the alarm/notifications latch and the blur-phase collisions. PLAN has frame probe; add per-frame trace fields: start, ready, presented, slot, layers drawn, quality level.

32. (d) **Cold play vs measured play.** Each screen: one cold play (warm-up cost, first upload) then a measured play; report both. Cold worst render 11.3-12.6 ms vs measured 6.3-7.1 ms (T3 motion). Also 'heaviest upload of one frame' (1995 KiB cold, 0 measured).

33. (d) **Run-to-run spread is real.** Same build: T3 passing 19/17, then 20/19, then 19/20 with ha_dashboard at 12.5 ms. Gate decisions on one run are unreliable; use 2-5 runs per config.

34. (d) **Host-screenshot pitfall.** On the overlay route, a framebuffer page screenshot shows what's under the planes, not the panel; `fbdump` now saves the display controller's capture. Four screens had produced one identical picture before the fix (commit 718f704c). Any LumaTV screenshot harness must capture the composed output.

35. (d) **Compositor lifecycle verification script.** `tools/compositor-verify.sh` scenarios: animate, repeat, drag, soak, late, churn, churn7, second client refused, kill, idle, shutdown, app; each prints PASS/FAIL against legacy numbers. Status 2026-10-03: 22 PASS, 0 FAIL. Pattern to copy for LumaTV's gate runner.

36. (d) **Baseline numbers.** 60 s drag: 59.64 fps, 0 missed, 3,579 frames; commit -> screen p50 13.33 ms (VERIFY.md). Motion engine fixture (T3): render p50 3.2 / p95 6.1 ms; input-to-flip p50 24.1 / p95 28.2 / p99 33.1 / max 67.9 ms over 3,291 frames.

D. Startup, memory, standby (e)

37. (e) **Idle trim after 5 s quiet.** Frees pool render targets, backdrops not used for 60 frames, still plates not used, graveyard textures, and heap pages (malloc_trim or mallopt M_PURGE, looked up at run time). Never frees a layer's texture, glyph atlases, scan-out buffers. RSS 77.3 -> 69.9 MB (black Show), 70.8 -> 69.4 (white). Relevant to PLAN's 'RSS flat over 500 cycles': trim addresses the idle floor, not the Home-return growth. CONFLICT-ish: PLAN says the 40-48 MB Home stub frees memory but doesn't move gaps; trim is the complementary idle-side lever.

38. (e) **RSS census.** `LUMASHOW_CENSUS=1` logs RSS split (anon, file, shmem), heap arena in use/free, GPU KiB by kind, pool sizes, atlas pages, before and after each trim. Resident size is defined as GPU bytes mapped + free libc heap + touched file pages. Use this definition when comparing LumaTV RSS numbers (PLAN's RSS gate has no stated definition).

39. (e) **Screen off = no frames.** `Request::Display` (protocol v5): while the app has the screen dark, the compositor draws nothing; on wake, backlight on with the first frame. A dark screen was found drawing 60 fps at a quarter of a core (10-01 soak). Standby lesson for LG: confirm no frame is drawn while the panel is off.

40. (e) **Wi-Fi watcher escalation.** Router silent 1 min -> reconnect, 5 min -> vendor drivers, 15 min -> restart. Found after a 25-minute dead link (10-02). Not perf, but relevant to unattended standby recovery. [INFERENCE] LG's network stack differs.

E. Conflict list vs LumaTV PLAN (main agent to decide)
- PLAN 'SCHED_FIFO/CPU pinning/governor no effect' vs LumaShow nice -10 on display helper (item 26): re-test on LG.
- PLAN 'threaded render loop black screen': LumaShow runs a separate render thread and a present thread (RT render thread in 009d work). Different driver; do not adopt the thread split on LG without a device test.
- PLAN gates (20 ms / 50 ms / 180 ms) vs LumaShow gates (17.2 / 12.5 / 25.2 / 100 ms) at 59.64 Hz (item 27): combine, re-derive for 60 Hz.
- PLAN 'Home stub frees 40-48 MB but doesn't move gaps' vs LumaShow first-frame warm-up numbers (item 18): LumaShow says first-screen cost is GPU program and target setup (53-60 -> 36-41 ms with warm-up). Overlaps PLAN's shader warm-up; test whether target warm-up explains LG's first-frame gaps.
- PLAN 1080p/120 Hz/4K rejected: no conflict; LumaShow is 960x480 at 59.64 Hz; its per-pixel numbers do not transfer without re-measuring.

F. Pitfalls
- glFlush is not a scan-out submission on Mali (item 3).
- Late stamps create phantom gaps (item 29).
- Overlay-route screenshots show what's under the planes unless captured from the display controller (item 34).
- GPU textures may hold noise; define them (item 15).
- First-play cost hides in the cold numbers; always report cold and measured separately (item 32).
- Single-run gate results are noisy (item 33).
- Ops: LumaShow's device work runs under a device lock; never touch the TWRP/recovery partition; kernel changes need Nitin's OK (HANDOFF). Principle 0 applies to any LG device change: none made here.

G. Reuse directly (crate, module, what, device-specific assumptions)
- `crates/compositor/src/pacer.rs` (pure logic; host-tested): reuse. Assumes a flip return that is vsync-accurate (FBIOPAN). LG needs an equivalent vsync signal.
- `crates/compositor/src/ladder.rs`: reuse logic; step definitions tied to Lucent layers and LumaShow's protocol. Needs LumaTV's own layer model.
- `crates/compositor/src/damage.rs`, `scanout/plan.rs` (ground detection): reuse concepts; plan code depends on the `planes` offer.
- `crates/planes` (trait only): reuse as the overlay contract. `service.rs` is the helper client; `display_mtk` implementation is MediaTek-specific (ion, FBIOPAN, layer format); not reusable on webOS.
- `crates/displayd` (process split, lock, guard, reset): reuse pattern. Assumes fbdev page to restore and a MediaTek display manager interface.
- `crates/timeline` (trace): reuse. Check its clock source on LG.
- `crates/compositor/src/idle.rs`, `renderer/trim.rs`, `census.rs`: reuse logic; depends on Mali GL (`Gles` pools) and glibc `malloc_trim`/`mallopt` (verify LG libc).
- `crates/compositor/src/still.rs` (plates), blur hold: reuse; depends on the GL pool.
- `gl::Fences::kick` and `gl::POISON`: reuse; Mali-specific fence behaviour must be checked on the G52 driver.
- `tools/graphics-lab.sh`, `tools/compositor-verify.sh`, `tools/lab-frames.sh`: patterns reusable; paths and device lock are Show-specific.
- `compositor-protocol` (versions, Event::Frame fields): pattern reusable; do not copy versions into LumaTV's protocol.

H. Coverage gaps (not read; main agent may want them)
- HANDOFF.md lines 300-2074 (log, milestones) and `notes/` (architecture, hw-64bit-plan, PROJECT-PLAN, DEFINITION-OF-DONE).
- Gate and perf logs (`gate-*`, `tmp-perf-*`, `ab-*`) and overnight notes: not read; summaries not extracted.
- `crates/ui` (scene, motion, group), `crates/appearance`, `crates/fetch`, `crates/settings-schema`, `crates/plugin-api`, `docs/appearance.md`, `docs/plugin-api.md`, `docs/durability.md`, `docs/power-loss.md`.
- `crates/compositor` source files beyond pacer.rs header and the named modules.
- Git log grep was done on `.git/logs/HEAD` only (no `git log` run; read-only rule), so the commit list is partial.

I. Not for LumaTV without Nitin's decision
- Quality ladder with visible steps (item 6): product choice.
- Any kernel OPP / clock change on LG: device change, Nitin OK.
- Anything that publishes or deploys: out of scope (Principle 0).

## Checked on the C4

- Item 26 (nice -10 on the display service): LG already runs surface-manager at nice -19 (read on the set 2026-10-25), stronger than LumaShow's -10, so there is nothing to adopt. LumaTV's own compositor (track C) starts at the same -19.
- Heap shim through `gfx64/launch.sh`, screen on, over idle Home (2026-10-25): 8 s 60.63 fps, 4 frames over 20 ms, worst 24.5 ms; 20 s 59.81 fps, 6 over 20 ms, worst 29.8 ms (one contiguous buffer, two `dma_buf_te` when CMA ran short). A run with the panel in standby reports unpaced frames (152.7 fps) and is refused by the smoke script.
