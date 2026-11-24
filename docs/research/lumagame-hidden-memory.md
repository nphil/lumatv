# LumaGame hidden: what it holds, what it could release

Scoping only; no code changed. Read from the code and, on the C4 on 2026-11-24, from `/proc/<pid>/smaps`, `/sys/kernel/debug/mali0/ctx/<pid>_<id>/mem_allocs`,
`/sys/kernel/debug/dma_buf/bufinfo` and `/proc/meminfo`, with LumaGame hidden behind LG Home (pids 15443 `lumagame-main`, 15465 `lumagame-ui`, no engine process).
MB are 1,024 kB. Figures marked [INFERENCE] are estimates to confirm before the change ships. Context: [`memory-map.md`](memory-map.md), [`cma-holders.md`](cma-holders.md).

## What the hidden app holds

`CmaFree` reads 170 MB at LumaTV Home before LumaGame starts, 82 MB with its Home up, 40 MB on a game page after a stream and about 5 MB hidden at LG Home:
it only goes down, because nothing is given back once the app is hidden. The kernel lends free contiguous memory to movable heap pages, so the heap
LumaGame keeps is what `CmaFree` loses.

| Where | MB | What |
|---|---:|---|
| `lumagame-ui` PSS (all RAM, no swap) | 197.5 | 126.2 anonymous in three 64 MB-aligned arenas of 59, 50 and 16 MB (non-main-thread malloc arenas, [INFERENCE]), 39.4 main heap, 15.4 `libmali` (file), 10.4 `layer-N.rgba` (26 maps, one of 8 MB), 4.1 binary |
| Mali context of `lumagame-ui` (not in PSS) | 74 | 48.5 native (197 allocations: three of 8.6, 8.0 and 7.9, the rest small) and 25.7 imported (3 swapchain buffers of 8.6, `dma_buf_te`, ordinary pages) |
| `lumagame-main` + launcher | 10.3 | the 32-bit feeder and its shell |
| Engines, recorder | 0 | none runs hidden (below) |

So the hidden app costs about 208 MB of PSS plus about 74 MB of GPU memory, not 208 in all. The 126 MB of anonymous memory is the process heap, not GPU buffers
(`memory-map.md` reads it as GL and GPU buffers); the GPU buffers are the `/dev/mali0` mappings (120 maps, 71 MB of address space, 0 PSS).

## Items, expected release and reopen cost

| # | Item | Releases (MB) | Cost to reopen |
|---|---|---|---|
| 1 | Art caches and textures. CPU: the decoded page backdrops held ahead (`art/ahead.rs`, 4 x 8.3 = 33 at most, `778930b3`), Home's art layer parked while a page is open (8.3, `613c7448`), the live art layer's pixels (8, `layer-507.rgba`), copies under the scrim (about 8-16). GPU: the three 1080p-class textures (24.5) and the cover, glyph and plate textures. The glyph atlas itself is 0.2 MB, no cover atlas exists (covers are per-tile layers). | CPU 50-60 held, plus whatever freed heap the arenas keep (they are 82-98 % resident): 60-90 [INFERENCE]; GPU about 25 | Hero backdrop decode on the first Home frame, 414-483 ms (`778930b3`); covers come from the disk cache. First frame after reopen about 0.5 s later than now [INFERENCE] |
| 2 | EGL/GLES context and surfaces | GPU about 50 (25.7 swapchain + about 24 of programs, tiler and small allocations); `libmali` 15.4 stays mapped unless the process exits | The card window and engine are built at start only (`window::open`, `Engine::new`, `GameUi::build`): re-creating them is a cold start without the exec, about the same cost as item 5, with new code paths. Not recommended on its own |
| 3 | Decode threads and recorder. The three `game-decode-N` threads hold nothing at rest beyond free lists in item 1's arenas. The recorder's ring (108 MB, 128 MB with queue and fragments) is built per stream by the engine (`opennow-streamer` `clip.rs`, Punktfunk `session.rs`) and dropped at the stream's end; it is never in `lumagame-ui` | 0 hidden. Confirmed: no engine process on the set, and the code drops the recorder with the session | none |
| 4 | Engine processes (GFN core and streamer, Steam, Punktfunk). `Supervisor::sync` stops every engine when the app is not in front and no session is active, and starts them again over the saved catalog | 0 hidden: only `lumagame-main`, its launcher and `lumagame-ui` run | Engine start on the way back (unmeasured), already paid today |
| 5 | The whole UI process and its feeder | about 280 (208 PSS + 74 GPU); `CmaFree` returns to about the 170 MB level of before the first start [INFERENCE] | Cold launch. Window at 397 ms with the kept shader binaries, 1,975 ms with none (`HANDOFF.md`); plus UI build, glyph atlas settle, engine start and the first backdrop decode. Time to the first Home frame is not measured; expected 1-2 s [INFERENCE] against the immediate relaunch from hidden |

The idle trim that exists (`render/src/idle.rs`: GPU pool and cached backdrops, `malloc_trim`) runs only from the foreground loop, after 5 s of quiet. A hidden app
never reaches it, which fits the arenas still being 82-98 % resident after hours hidden.

## Recommended plan

1. **On hide, at once** (no dwell): store a scaled copy of the last frame (about 2 MB at 960 x 540) for the task-manager hook; stop and empty the ahead decoder; drop the
   parked and live art layers; clear the renderer as `Op::Clear` does for a gone client and trim; `malloc_trim`. Expected 85-115 MB back (60-90 CPU, about 25 GPU), `CmaFree`
   up by the CPU part. Reopen: Home draws with placeholders and the backdrop fades in after about 0.5 s.
2. **After 10 minutes hidden**, and never while a session, a recording or replay, a TV-game launch, a helper pad lease or a sign-in is active: leave through the normal
   close path (state saved, engines already down). Expected 280 MB back in all. Reopen is a cold launch.
3. Not the middle step (release only the GL context): about 50 MB for nearly the cost of step 2, and new start-up code.

LumaTV's task manager keeps working: while the process lives, `snapshot` is answered from the stored copy instead of reading the GPU page back (`hook_snapshot` reads
`renderer.read_page(0)`, which step 1 empties), and `quit` takes the same `Close` path as today. After step 2 the card is gone with the app, as for any closed app, because
the switcher lists running apps (`switcher.rs`); the owner sees LumaGame leave the task manager after 10 minutes.

Decisions for the owner: the dwell (10 minutes proposed), and whether LumaGame disappearing from the task manager after it is acceptable. To confirm before shipping:
heap in use against freed at hide (`LUMASHOW_CENSUS=1` prints it), `CmaFree` before and after each step, and the first Home frame after a cold launch from the
lifecycle trace (`LUMAGAME_LIFECYCLE_TRACE=1`, `FromApp::Ready`).
