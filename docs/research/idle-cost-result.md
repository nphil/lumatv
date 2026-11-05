# Idle cost: Home at rest with the calm sky (2026-11-06)

Evidence tags: `TV:` read on the set (file named), `HOST:` measured on the host harness (`tests/host_budget.py --surfaces idle --sky cloud --owner <gate receipts>`: the owner's theme and layout, wallpaper motion on, Mesa without a GPU, so its frame counts are the set's and its CPU is not), `[INFERENCE]` derived, not measured. No build here has been on the set.

## Result

`HOST:` a second of Home at rest takes the engine 8.5 ms (10.7 before) and the UI shell 3.6 ms (5.3 before), for the same 23 to 26 frames; a frame draws 55 layers (87) and 77 draws (117); every frame is the frame drawn whole (0 of 243 differ, `--reference`).

`[INFERENCE]` on the set: compositor 7.2 % to 4.6-5.3 % of a core, UI 4.4 % to 2.8-3.3 %, together 7.4-8.6 % against the owner's 8 % (11.5 % in session 7). The middle, 8.0 %, is on the line: the 8 % is decided by the set, not here.

## Where session 7's 11.5 % went

`TV:` `/data/home/tmp/lumatv-tv-20261105a/idle-after.json` (b1858b27, wallpaper motion on): 24.2 flips, 123.9 ticks and 157 loop wake-ups a second; the engine 1810 us a frame (place 221, plan 96, draw 590, other 893); the UI main thread 3.9 %.

1. **The loop never slept between frames.** `Tick::rearm` needs 250 ms without a `busy()` wake-up; every event except the output's and the UI's marked it, and the UI's marked it as well when a frame was due. The sky's restates (every 0.4 s), the camera stills' commits (2 a second), the UI's reports (`POST /v1/ui`) and luna answers kept the loop at the panel's rate: 5 ticks for each flip. At 0.11 ms a tick (the sky note's model) that is 1.1 % of a core.
2. **Every frame drew the whole screen.** A frame's own change is the sky box (0.2 Mpx) or the wallpaper's step (1 Mpx), but the pages of a 3-page ring owe what the frames since they were drawn changed, and a wallpaper step every other frame makes each page owe the screen. The page copy was on only with something over Home (round 27: Home's walks dropped frames at 120 Hz with it). `HOST:` 87 layers and 117 draws a frame.
3. **The GL state was saved and put back five times a frame.** `Engine::scoped` wraps planning, each stack, ending the frame and the housekeeping; `Gles::enter` and `leave` are 77 driver calls. `HOST:` 7.8 scopes a frame in the harness, 1.1 in the compositor's sequence now. `[INFERENCE]` 63-87 us a scope on the set from its 893 us of `other`.
4. **The empty plane over the windows was accounted every frame**: 70 us of the host's thread for an answer that is always "nothing changed" while nothing is over the windows.
5. **The shell drew all of Home for the sky and for each still.** The sky's restate (2.5 a second) was an apply of Home and a report built to find it as it was; each apply copied every camera's still (0.7 MB) before finding the item unchanged, built the layout again for each card's rectangle, and the report counted the smart-home tiles by copying them (75 us of 185).

## What was done

- **Lazy scopes** (`Engine::scoped_when`): planning, `finish` and `tidy` take the context's state only when a texture is to be made, a chunk landed, a picture copied or a floor brought up to date (exact predicates, `Renderer::plan_touches_gl`, `tidy_pending`); a canary (`Gles::touched`) says once and takes every scope again if a call reached the GL without one. `LUMATV_LAZY_SCOPES=0` turns it off.
- **Calm frames copy their pages** (`kms::frames`): a frame that comes 3 refreshes or more after the last, with nothing animating, copies what its page owes from the page drawn last and draws only what it changed (`LUMATV_AMBIENT_COPY=0` turns it off). Walks at the panel's rate are unchanged, so round 27's drops are not at risk.
- **The loop sleeps** (`main.rs`): the UI's commits, its reports, the shell channel and luna answers no longer count as busy; the engine's `ui_due_in()` and `Tick::rearm` carry what they asked for. Keys still hold 3 s, and clients, inputs and control stay busy.
- **No account for an empty plane** (`Renderer::account`): a plane with no layer, as it had none the frame before, answers "nothing changed" without comparing every layer.
- **A made sky restates by itself** (`HomeSurface::step_art`): its step sends the sprites' paths and nothing else, no report is built for a tick that drew nothing and asked nothing (at most 1 s between builds), and the card's sky does not wait for the atlas once it is made. The status bar's glass, status item and five icons that every apply sent again stay with the apply, which the clock's second and each still still run.
- **Cheaper applies and reports**: the item's still is borrowed, not copied; the drawn layout is made once for a layout and text size; the report counts tiles and walks the status bar's items where they are.
- **Measurement**: `host_budget.py` has the `idle` surface and `--sky`; its frame lines say where the time went (`why`, the phases, `scope`, the GL calls); `ctl status` and `tv-idle-probe.py` carry `scope`, `step` and `finish` beside the phases, so the set's `other` can be split.

## Host numbers

`HOST:` `idle`, 10 s, three pairs run one after the other, old build with `LUMATV_LAZY_SCOPES=0 LUMATV_AMBIENT_COPY=0` against the new.

| | before | after |
|---|---|---|
| frames a second | 23.5, 26.5, 26.5 | 23.5, 26.4, 24.1 |
| engine ms a second | 9.4, 11.0, 11.7 | 8.1, 9.5, 8.0 |
| shell ms a second | 4.8, 5.4, 5.8 | 3.4, 3.7, 3.6 |
| layers, draws a frame | 87, 117 | 55, 77 |
| scopes a frame | 7.8 | 1.1 |

An apply of Home at rest took 316 us (median), 128 us now; the report 185 us, about 90 us. Mesa's CPU is not the set's: the host's GL calls are about a third of a microsecond and the set's about one, so the scopes' share is understated here.

## On the set

`[INFERENCE]` with the sky note's model (compositor main thread 0.11 ms a tick and 1.81 ms a flip) calibrated on session 7 (123.9 ticks, 24.2 flips: 5.7 % of 6.7 % measured):

| | session 7 | predicted |
|---|---|---|
| ticks a second | 124 | about 30 (a flip and its events) |
| engine a frame | 1810 us | 1240-1490 us (scopes 250, draw 150, the empty plane 175 at the host's ratio of 3; or the host's 18 % for all of it) |
| compositor | 7.2 % (main 6.7, Mali 0.4) | 4.6-5.3 % |
| UI | 4.4 % | 2.8-3.3 % (the host's 33 % off, 25 to 35) |

## Not done, not measured

- Nothing here ran on the set. To check: ticks a second against flips (about 1.2 for 1), the engine's `scope`, `step`, `finish` and `other` per frame, the Home walks at 120 Hz (the gate's five surfaces) and the camera card's first still, which the copy path and the borrowed still touch.
- The wallpaper's steps (10 a second) and the sky's (15) are two clocks that meet every 0.2 s; one clock for both would give about 12 frames a second instead of 20 and is the largest saving left. It moves the sky's steps by up to 17 ms, so it was not made: the owner's call.
- A camera's still costs the UI about 0.75 ms on the host (0.2 ms for each of the decode, the fit and copy, the scrim and the apply) twice a second, a third of what is left of the shell's time. A painter that fits, scrims and writes the layer's pixels in one pass would take about a third of it.
- Per-layer vertices are made and sent again every frame (35 us of the host's thread for 129 layers); caching the layers that did not change needs stable offsets in the vertex buffer.
