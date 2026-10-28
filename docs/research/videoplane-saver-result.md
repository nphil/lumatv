# The art saver and the moving wallpaper on the video plane: `lumatv-vplane`

Built, host-tested and run once on the set (TvRound8, 2026-10-25 23:35-23:45; evidence `docs/research/round8/tv-2026-10-25/`). The art mode's picture comes from the TV's idle hardware decoder (a still, looped) instead of the GPU. The player works in the Active power state; in LG's Screen Saver power state the panel stays black, so the video saver is off (`theme.screensaver.video`) until that is solved. The saver keeps its GPU path as the fallback. The player's second role plays Home's moving wallpaper ("The moving wallpaper", below): built and host-tested under LumaTV's compositor, off by default (`lumatv wallpaper video`), not yet run on the set.

## Resume here

Done (host-tested; counts under "Checks run"):

- `vplane/`: the 32-bit player service `lumatv-vplane` (C, ARM and host builds), its decoder-library and compositor stand-ins and the sampler for the TV steps (`vplane/tools/measure.py`).
- `tools/vplane_clips.py` and `payload/vplane/clips/`: the six prints as looped HEVC stills (2.07 MB), caption bitmaps and the clock's glyph atlas.
- `panels/SaverVideo.qml` (the saver's end of the mailbox), the video branch of `panels/Screensaver.qml`, `payload/vplane/{vplane.sh,role.json}` (install, supervisor, LS2 role), `lumatv vplane ...` (CutoverWatchdog).
- The player's window is double-buffered (`vpl_wl.c`): one pool of two 1920x1080 ARGB buffers (2 x 8,294,400 = 16,588,800 bytes), each frame painted whole into the buffer the compositor has released and committed; a buffer is never written between its commit and its `wl_buffer.release`. Before, one shared buffer was repainted in place and a compositor that read it late could take a half-painted frame (veil filled, caption not yet laid: CaptionFlake). With both buffers held the frame waits for a release and is painted from the state as it is then, and the print is not switched under the black until the black frame has been committed. Host-tested with a stand-in compositor that reads late, holds a buffer until the next commit replaces it and flags every write to a held buffer; the TV check is step 16 of "TV steps".
- The art saver's shuffle bag is kept across restarts (`ArtDeck.js`, `/var/lib/lumatv/theme/saver-deck.json`), on by default, off with `theme.screensaver.art.keepOrder: false`.
- After the set's run: the role carries every name LG's library registers (the hub refused its nameless handle), the player sets `XDG_RUNTIME_DIR` and `WAYLAND_DISPLAY` itself (LG's AV connector needs them), the window is opaque black around the picture (the margins showed Home's wallpaper), and the twelve hour clock is twelve hours in both savers (Qt's `h:mm` alone is 0-23).
- The moving wallpaper (host-tested only): the player's `wall` role, `tools/vplane_wall.py` and `wall-silk.vpc`, the compositor's wallpaper layer with LG's hole order, `WallVideo.qml` and the gating in `LumaHome.qml`, the tint-glass mode, `lumatv wallpaper video|gpu`.

On the set (TV, `--once great-wave 20`, Active state): the print on the video plane (1856x1024, placed by the exported window, drifting), caption and corner clock in the window, Home stays foreground, exit 0. In the Screen Saver state (`power2/turnOnScreenSaver`): the player plays (60 units a second, queue 0-1, nothing skipped) and LG's Flutter saver is evicted by the screensaver-type window, with Mali 1.9 % against LG's saver at 21.9 %, flips 12.3/s, `DE2VSYNC` 121.7/s, the player at 3.6 % of a core and 25 MB RSS, `CmaFree` 149-158 MB; but the display and graphic captures are black while the video capture shows the print and the output is connected and not muted [INFERENCE: LG blanks the output in its Screen Saver power state]. A key wakes to Home in about a second; `MemAvailable` fell 772 to 739 MB (over the 20 MB bar). Also seen: under LG's compositor LumaTV's own GPU saver is not visible in the Screen Saver state, LG's Flutter saver draws its clock over it.

Open, in this order:

1. The black panel in the Screen Saver state. Now avoided rather than solved: `saver-own-result.md` answers LG's idle question with no, so LG's saver is never started, the power state stays Active and the saver (this player included) draws there, where the player showed the print; the Active-state run is its TV check, step 5. A saver LG started anyway (`own` false) still meets the black panel; for that case: find what LG blanks (a capture with the panel watched, `playVideo` as LG's own video saver sets it, `tvpower` and `panelcontroller` logs). Needs the TV.
2. The `MemAvailable` drop of 34 MB after unload: read against the other idle readings of that session and the GPU saver's code, with an offline cycle test, in `saver-own-result.md` ("The 34 MB question"); its step 9 is the TV measurement over five cycles.
3. The moving wallpaper on the set (the steps and measurements under "The moving wallpaper") and its path under LG's compositor (the Home stub's `<video>`), which is designed and not built.

Evidence tags: `TV:` measured on this set, `SRC:` LG's firmware tree in the repo, `PUB:` public source, `[INFERENCE]` not run.

## Verdict

Measured in the Screen Saver state with the picture playing: Mali 1.9 % against 21.9 % for LG's saver in the same window (the census read 54.5 % earlier, `hardware-census.md`), the player 3.6 % of a core. The GPU saving is real; the picture is not yet visible in that state. What is still unmeasured: the loop seam, the drift's smoothness, the output rate under a still in the Active state.

## Decode path

NDL DirectMedia v2 (`libNDL_directmedia.so.1`) from a 32-bit root service that is a bare Wayland client with its own app id. The service keeps the libraries LG ships and the 64-bit shell out of each other's way, as `lumatv-capture` does.

| Path | Verdict | Basis |
|---|---|---|
| **NDL DirectMedia v2** | Used | Verified on an OLED77C5 (webOS 10.3.1), PUB https://github.com/webosbrew/samples; Jellyfin's `ndlplay` and Moonlight use it, and an app id need not be an installed app (PUB jellyfin-webos-native `docs/ndl.md`, `display.md`). Table row 1 in `opennow-result.md`. |
| Starfish media API directly | Fallback | Same pipeline, more knobs; used if NDL refuses a still. |
| Raw V4L2 (`video21`) | No | LG's nodes are plumbing for the pipeline; a raw decode skips the plane routing. |
| GStreamer, `libNDL_vt` | No | No documented entry; `libNDL_vt` is a GL texture and the GPU cannot sample a video plane. |
| Web `<video>` | Not for the saver | WAM runs with `--max-activated-media-players=1` (TV: `lgtv/uiinv/ps.txt`); right for the Home wallpaper (below). |

Order of calls, enforced by the stand-in library (`vplane/tests/fake_ndl.c`): `DL_Initialize`, `SetWindowId`, `Init`, `Load` (HEVC, 1856x1024), `SetAppState(foreground)`, `Play`. The window id comes from `wl_webos_foreign.export_element` before the first commit; `set_exported_window` places the picture. Whole access units, one per 1/60 s, monotonic PTS (the player has no clock of its own: opennow-result.md). Teardown takes the window first, then unloads the decoder under `alarm(8)`; a hang ends the process with status 3 and the supervisor starts a fresh one.

LS2: LG's library registers `com.webos.rm.client.<session>` and `com.webos.media.client.*`, both already `private` groups in LG's client permissions (SRC `arch/sm-perm-01.txt`). `payload/vplane/role.json` allows those two names, with `com.webos.media` as the one name they call and answer; it is written for the staged path and named by a `manifests.d` manifest, as `payload/capture/capture.sh` does (ambient-result.md, 16:21 and 16:27). Whether the library registers more is the TV's to say: a denial is in `/var/log/messages` and the template is widened for the name it shows, not before.

## Content

The picture is a still: one IDR and 119 predicted skip units at 60 fps, looped from unit 0 (looping on a non-IDR is nonconforming: POC would go back). 60 fps keeps the UI plane at 60 Hz, because the output rate follows the video pipeline (a web page held 50 Hz: perf-tweaks.md, Side finding). The first predicted units refine a little (up to 4 KB), so a faint breathing at every loop is possible; `--loop-sec` (default 2) is the knob (step 9).

- Canvas 1856x1024 centred on the 1920x1080 screen: 32 and 28 pixels of room on each side for the shift. Baked in: the wall gradient, the matte, the frame, the print at `artLevel` (0.78) and a 40 px black vignette. The generator reads the composition from `ArtPiece.qml`, `Screensaver.qml` and `theme.json`, and `check` fails when they drift apart; its output is byte-identical on regeneration.
- Not baked: the dim, the caption and the clock. Baked at the deepest dim (`dimTo` 0.6) the caption's contrast falls to 2.1:1; drawn over the dim by the window it measures 4.5:1 for the artist and 3:1 or better for the rest and the clock at levels 1.0 and 0.6 (Lucent: contrast measured on the composited picture; tests in `test_vplane_clips.py`). The rest of the caption (24 px, regular) is large text.
- Burn-in: the shift is done by window position, not baked: the exported rectangle moves every `moveMs` (4 s) by `round(30 sin(2 pi t / 540))` by `round(20 sin(2 pi t / 380))`, whole pixels, the saver's own formula (`driftAmp*`/`driftPeriod*` in `Screensaver.qml`; `check` reads them). `NDL_DirectVideoSetArea` is the documented alternative if the exported window cannot move; a baked orbit is the contingency. A print change goes out through black: the veil rises over `fadeMs`, holds 400 ms, the clip switches in the dark, the veil falls. The clock stays visible; the caption fades with its print.
- `HwJpeg2`'s helper (`GET /v1/image/{root}/{file}?w=&h=`, `lumatv-jpeg` NV12) is the other way to put a still on a video plane; not needed while the HEVC loop works.
- The saver's clock text was Qt's `"h:mm"`, which is 0-23, so the twelve hour face showed 23:39 (seen on the set); both savers now fold the hour (`clockText()` in the QML, `vpl_clock_text` in the player).

## Who draws the words

| Mode | Setting | The picture, veil, caption, clock | Works where |
|---|---|---|---|
| Window (default) | `videoOverlay` unset | The player's own window, above the video, draws them | LG's compositor: the hole (`Eos.PunchThrough`, `z: -1`, SRC `views/punchThrough.js`) sits under the window's pixels, so they show |
| Shell | `videoOverlay: "shell"` | The saver draws them above a transparent hole; the player's window draws nothing (`overlay:false`) | A compositor that draws the shell above the video plane |

Fixed with the moving wallpaper (`render/software.rs`, `render/gl.rs`, `scene.rs` of LumaTV's compositor): holes were cleared after the windows were drawn, which erased a window's own pixels inside the video rectangle. Each hole is now cleared just before its owner window's item, as LG's compositor orders it (`Eos.PunchThrough` under the surface), so what a window draws over its own video stays (window mode's veil, caption and clock; an app's controls over its video) and windows above are untouched. Also there: `Screensaver` windows are Cards-layer and foreground-eligible (`window.rs`, `layer()`).

The order of the prints is the saver's in both modes (its bag, kept across restarts): the first print is named in `start` (`clip`), each next one by `next {clip}`, and the player never turns a page by itself (`auto:false`; it does when the saver's index has no prints).

## Window type, and LG's own saver

Default `-w screensaver`: `_WEBOS_WINDOW_TYPE_SCREENSAVER` with `playVideo: true`. In LG's compositor (SRC `views/StarfishScreenSaverView.qml`) a new saver window makes `onSurfaceAdded` call `close()` on the current one, so the player's window evicts LG's Flutter `com.webos.app.screensaver`, the app behind the as-found 54.5 % [INFERENCE: the animation is LG's saver app]. With `playVideo` the controller sends all apps to the background (SRC `controllers/StarfishViewStateController.qml:615-625`), so the Home stub is held back too. LG's own key path for the saver, `cancel()` to `power2/turnOnScreen`, works for a window it does not own. The saver view is not in the access-policy lists (SRC `controllers/StarfishAccessControlPolicy.qml`) [INFERENCE: it accepts any]; a refusal is `closeWindow(..., windowPolicy)` in the compositor log.

If the screensaver type is refused, or LG's app comes back over it: `-w card`, `-w overlay`, `-w default` (`STATE/vplane/args`, one line, read at the next start). Home no longer ticks under the saver: the wallpaper timer and the widgets' `live` flag are gated on `saverUp` (`LumaHome.qml`).

## Wake and the way out

Chrome over full-screen media never blocks the way out (Lucent). The player takes every `wl_seat` with keyboard and pointer: any key press or pointer motion or button runs the wake command (`luna-send -n 1 luna://com.webos.service.tvpower/power/turnOnScreen '{}'`, `-W` to change) and the player stops itself and takes its window away. It does not depend on the shell being alive: the request's `ttlMs` (30 s) is a watchdog, so a dead shell cannot leave a picture up. The saver's own key path (the host hands a key to `key()`) stays for keys that reach the shell. Whether LG's compositor delivers keys to a screensaver-type window's client at all is step 13.

## Start, stop, state

The saver and the player talk through two files in `/tmp/lumatv-vplane/` (`file:///tmp/{{shortId}}-vplane/`), neither needing the other to be there: `req.json` (ops `start`, `update`, `renew`, `next`, `status`, `stop`; `seq`, `sent`, `ttlMs`; an old request is ignored by age) and `status.json` (`state` idle, opening, playing, error; `reason`; the clip, `veil`, `drift`, `memAvailKb`, `cmaFreeKb`, `rssKb`, ...). Idle, the player has no window, no decoder and no bus client: it writes a status line a minute. Playing, every 5 s.

| Saver state | Meaning | Draws |
|---|---|---|
| `off` | video not asked, or no player (no status file, or one gone stale) | the GPU's art, at once |
| `starting` | asked, not yet playing; at most `startMs` (6 s) | black |
| `playing` | the picture is on the video plane; renewed every 10 s | the player's window, or in shell mode the saver's words |
| `failed` | the player said no, did not answer, went quiet (12 s) or the owner turned the switch off | the GPU's art; the player is told to stop |

`videoUsable` is false (GPU only) for a preview (`forced`) and any style but art. Nothing runs while the saver is down.

## Memory

| Item | Size | Basis |
|---|---|---|
| Window buffers | 2 x 8.3 MB shared memory (16,588,800 bytes, one memfd pool), resident once the veil, caption or clock is drawn | 1920x1080x4, two for the double buffering |
| Decoder | about 8-20 MB CMA while playing, released at unload | [INFERENCE] one reference frame, no B-frames (2.85 MB per NV12 frame); 4K Dolby Vision holds 137 MB on the display engine's windows (opennow-result.md) |
| Clips | 2.07 MB in the ipk and `STATE/vplane/clips`; one key unit (250-400 KB) is read to feed | `payload/vplane/clips` |
| Floor | the player will not start under 100 MB `MemAvailable` (`-m`) | `MemAvailable` 684-735 MB at Home (TV: hardware-census.md) |

`CmaFree` is not a guard: CMA is lent to movable pages and the kernel migrates them out when the decoder asks (memory-result.md, CMA); it reads 4-9 MB while 4K video plays and allocations made beforehand hold (opennow-result.md). The step list reads it before, during and after.

## Fallback

No player, an error from the player, no answer in 6 s, a heartbeat gone stale for 12 s, a restart of the shell: the saver draws the art itself and asks the player to stop. The owner's switch is the only way to the video path, and it is off. Turning it off while the picture is up gives the art to the GPU from black.

## The shuffle bag across restarts

`ArtDeck.js` keeps the art saver's bag in `/var/lib/lumatv/theme/saver-deck.json`, one small file with a deck for each collection of prints, keyed by the folder of its index (`art/` for the shipped prints). Eight collections at most, none over 2000 prints. A deck holds the ids still to deal in the round, the ids the collection had when it was saved and the print dealt last. After a restart the rest of the round is dealt on; a print that is no longer in the index, or whose file is gone, is dropped; a print that has come since is dealt in at a random place; a new round does not begin with the last print. A file that cannot be read counts as none. In order mode there is no bag and nothing is written. On by default; `lumatv theme set '{"screensaver":{"art":{"keepOrder":false}}}'` turns it off. Host-tested against a seeded file (a restart goes on with the round, drops what is gone, deals in what is new, leaves other collections alone); the `TextFile` calls on the set (`exists` after a `name` change, `write`) are the one thing not yet seen there.

Home has no wallpaper slideshow, so there is no second bag (planned in PLAN.md, "Wallpaper slideshow"; "not built" in `settings-slice-result.md`). When it is built, `ArtDeck.js` fits as it stands: ids, a key for each folder, a check that the file exists.

## The moving wallpaper

Home's GPU wallpaper (`liveWall`: the picture through a slow warp on a 64x36 mesh) costs 10 % Mali and 12.4 flips a second while Home idles (hardware-census.md, rank 4). The video wallpaper is the same picture as a looped clip on the video plane, so an idle Home draws no wallpaper frames. Built and host-tested for LumaTV's compositor, off by default, not yet run on the set. Under LG's compositor Home keeps the GPU wallpaper (below). Renders: `docs/research/videowall/` (`tests/qmlhost/render_wall.py`: the host's Home over a frame decoded from the shipped clip as the plane: `home-gpu.jpg`, `home-video-backdrop.jpg`, `home-video-tint-worst.jpg`; the planes `plane-frame.jpg`, `plane-worst.jpg`).

### The switch

`lumatv wallpaper video|gpu|status` (`theme.wallpaperMotion.source`, `gpu` by default). `status` names what is there: the clip, the player (installed, running, role) and the compositor (`own` or `lg`). `video` takes effect only while the whole contract holds: the shell is LumaTV's compositor's, the dark material is in use, `theme.wallpaper` is the shipped picture and the look and scrim are the ones the clip was made with (`wall.json` `look`; `vplane_wall.py check` names the key that moved). Otherwise the GPU wallpaper draws, so no setting puts a wrong picture behind Home. The player and the clip come with the owner's `lumatv vplane install`.

### Planes and draw order

Bottom to top: the video plane, the UI plane. LumaTV's compositor keeps a window for the player at its lowest layer (`Layer::Wallpaper`, type `_WEBOS_WINDOW_TYPE_LUMATV_WALLPAPER`: 16x16, transparent, never focused) that exports the whole 1920x1080 as the video element. The compositor clears that rectangle of the UI plane (the hole) just before the window's own item, so beneath everything else, and Home's window is transparent where its wallpaper was: Home's panes, text and icons draw over the plane, which shows between them. This is LG's order (`Eos.PunchThrough` under its surface) and the fix for the erase-after-draw finding above.

The window is in the stack only while Home is the drawn front (`build_stack`). An app, an input, a card, the saver or standby takes it out in the same frame and the compositor ends the session at once (`wall_handover`, called from `luna_notify` before the topics go out), so the next program finds the decoder free; direct scan-out of a full-screen app is untouched. The topic `wall {hole}` tells the shell that the hole is in Home's picture.

### Clip

`tools/vplane_wall.py wall` makes `wall-silk.vpc`: `liveWall`'s vertex warp (the shader's own mesh, scrim and light) of the shipped wallpaper over 30 s, whole turns of every phase so the end meets the start, 20 pictures a second each shown three times at 60 Hz (1800 units), HEVC Main10 at constant QP 18: 651,239 bytes, 10,233 for the key unit. `wall.json` carries the look, the poster frames' tint and luma and the glass opacity. `check --deep` decodes the clip and bounds the seam (mean step under 0.6 of a level, 90th percentile under 2) and the stability of repeated pictures.

On 6 s of the loop, the same pictures: 8 bit CRF 14, 10.7 MB; 10 bit CRF 18, 2.2 MB; 10 bit constant QP 18, 133 KB, mean error 0.108 of a level and 0.76 at worst between neighbouring pictures; 8 bit QP 16, 342 KB, steps up to 1.3. Constant QP because constant quality spends bits refining the repeated pictures; 10 bit because the picture is a dark gradient that bands at 8. If the set's pipeline refuses Main10: `vplane_wall.py wall --bit-depth 8 --qp 16`.

### Player

A request carries `role`: `saver` (also what a request without one means) or `wall`. One decoder, one player: a saver `start` takes over a running wall (idle, "the saver has the player"); a wall `start` while the saver plays is ignored; a request for the other role never advances `last_seq`. The wall role plays only `wall-*.vpc` clips (not counted among the prints), 1920x1080 only, with no overlay, drift, veil or clock and no wake on keys or pointer. `status.json` has `role` and `walls`; `lumatv-vplane -n` prints `walls` and `wallsOk`; `--once wall-silk 20` plays one by hand.

### Home

`WallVideo.qml` is the controller, on the saver's mailbox with `role:"wall"`. It asks for the clip while Home wants it (switch, contract, `hostActive`); `up` is `want && playing && settled && hole`, `settleMs` (250 ms) after the first frame with the hole in Home's picture. Then the GPU wallpaper fades out over `layerMs`, Home's wallpaper timer ticks once a second and the widgets' `live` flag stops while the saver is up. The way back is the other order: `up` falls, the GPU wallpaper is back at full opacity in the same frame and the player is told to stop `endMs` (250 ms) later, so a planned hand-over shows no black plane. The player is renewed every 10 s; unrenewed for 30 s it takes its window away by itself.

| Event | Who ends the clip | Home shows |
|---|---|---|
| An app, an HDMI input, Netflix or a card takes the front | The compositor, in the frame the app is drawn (`wall_handover`); the shell's stop follows | The app |
| The saver starts | Home stops wanting it (`saverUp`); the saver's `start` takes the player | The saver |
| Standby, screen off | `hostActive` false | Nothing |
| The switch goes off, or the look, material or wallpaper change | Home stops wanting it: the GPU wallpaper first, the stop after | The GPU wallpaper, no black frame |
| Player error, silence, status not playing, window gone | `failed`: `up` falls, the GPU wallpaper in the next frame (poll 500 ms; the compositor's topic at once) | The GPU wallpaper; a decoder that dies under a playing clip may show black for the frames it takes [INFERENCE] |
| Player not installed or not running | Never started; asked again every 15 s | The GPU wallpaper |
| The shell dies | The player's ttl; the compositor closes the window when Home is not drawn | Home's own |

### Glass

The GPU cannot sample the plane, so there is no blur-behind and a pane has two looks (`wallpaperMotion.glass`). `backdrop` (default): panes keep the blurred still of the same wallpaper, so the hand-over from the GPU wallpaper is invisible (`home-gpu.jpg`, `home-video-backdrop.jpg`). `tint`: flat translucent panes at the clip's `glass.alpha` (`Lu.homeFill`): 0.80 for `wall-silk`, which keeps the ink at 8.4:1 over the brightest plane the clip reaches (worst luminance 0.4423, Lucent's distance rule is 7:1); `home-video-tint-worst.jpg` is Home over the per-pixel brightest of the poster frames, where every flat pane pixel measures 9.5:1 or more.

The tint sampler (`Tint.js`, the accent tokens) is unchanged and samples the wallpaper PNG; the poster frames' tint (`#505070`, luma 0.294) agrees with it (0.297).

### LG's compositor

Not built. There LumaHome (z 2) draws inside the base container above the Home stub's surface, which is held at opacity 0, and a hole belongs to the window that owns it: the player's window cannot be put beneath LumaHome, and one above it clears the hole over the glass. The remaining design is the Home stub: a variant page (`payload/homestub/index.html`) plays an HTML5 `<video loop>` and exports its video window, the container keeps the stub's hole while LumaHome draws over it (the surface's opacity 0 may switch the hole off [INFERENCE]), and a foreground-video signal does the hand-over. It needs the TV: whether the web pipeline decodes the clip from a local file at 1080p60, where the hole lands, what Netflix finds afterwards. Until then `videoWallBackend` is empty there and Home draws the GPU wallpaper.

### TV steps and measurements

Receiver muted through Home Assistant, Filmmaker picture mode, `vplane/tools/measure.py` on the set, nothing written outside `/tmp` before the owner's OK to install.

1. Baseline, Home idle on the GPU wallpaper: `measure.py run home-gpu 30 --proc lumatv-compositor`: Mali %, flips/s, `vsync_per_s`, CPU of the compositor and the shell, `CmaFree`, `MemAvailable`, temperature. The census figures to beat: 10 % Mali, 12.4 flips/s.
2. `lumatv vplane install` and `start` (the owner's OK); `lumatv-vplane -n`: `walls` 1, `wallsOk` 1.
3. `lumatv-vplane --once wall-silk 20` with Home in front: the pipeline takes Main10 (else regenerate at 8 bit), `status.json` says `playing` and how long after the request, the picture is on the plane under Home, no black before the first frame.
4. `lumatv wallpaper video`: the time from the switch to `up` (journal, `status.json`), the GPU wallpaper gone, the glass over the plane (capture); `measure.py run home-video 30 --proc lumatv-vplane`: Mali (expected under 3 %), flips/s (expected near the clock's 1 a second), the player's and the shell's CPU; `CmaFree` and `MemAvailable` before, during and after three cycles of `video` and `gpu`.
5. Output rate and the gates: `DE2VSYNC` per second; `compositor/tools/tv_gate.py --only shell,frames` with the wall up (the idle commits a second and CPU of Home at rest against the GPU wallpaper's, the surface frame gate unchanged), and a focus move, a panel and an app launch as smooth as before (while Home shows, the UI plane is composed per flip, not scanned out directly).
6. Hand-overs, ten each, the decoder released and the app's first frame timed: Home to Netflix, Plex and an HDMI input (`status.json` back to `idle`, `CmaFree` back); the saver's start and a key out of it; standby and wake; `lumatv wallpaper gpu` with Home up (no black frame in a capture).
7. Failure: `kill -9` the player with Home up (the GPU wallpaper back in under a second, the black frames counted in a capture); the clip deleted; the role refused; a player that never answers.
8. Picture: the loop seam over three loops, the warp against the GPU's, colour and brightness of the plane against the GPU wallpaper (a capture of the margins), the glass in both looks over the brightest part, banding after an idle hour.
9. Keys and pointer in Home with the wall up (the wallpaper window takes none).
10. The figures against the GPU wallpaper's; if they hold, the owner turns `lumatv wallpaper video` on.

### TV round, 2026-10-28: blocked by what is installed, baseline taken

`lumatv wallpaper status` on the set (the ipk of `48ee380`, LumaTV's compositor live): `"source":"gpu","ready":false`, missing "the video-plane player is not installed", "the video-plane player is not running" and "the package has no wallpaper clip"; `lumatv vplane status`: `installed:false, role:false, clips:0, walls:0, running:false`. The package does carry the player (`payload/vplane/lumatv-vplane`, `role.json`, `vplane.sh`) and `clips/wall-silk.vpc` with `wall.json`; nothing of it is installed, and the third line is only `walls:0` read from the installed clips (the message should name the clip only once the player is installed). `lumatv vplane install` copies the player and writes an LS2 role, an install that was not run tonight, so steps 2 to 10 did not run and `wallpaper` stays `gpu`. Main10 acceptance, the hand-overs, the `kill -9` fallback and the loop seam wait for that install.

Step 1 ran (`measure.py run home-gpu 30 --proc lumatv-compositor --proc qml-runner`, idle Home, GPU wallpaper, LumaTV's compositor, receiver muted): Mali 12.6 % busy, 11.9 flips a second, `DE2VSYNC` 120.3 and `osd_irq` 119.9 a second, CPU 0.9 % of the online cores' time, `CmaFree` at least 86.8 MB, `MemAvailable` at least 471.2 MB, 55 C; the compositor 2.6 % of a core and 49.1 MB RSS, the shell client (`qml-runner`) 4.5 % and 237.5 MB RSS. Against the census's 10 % and 12.4 flips a second, the wallpaper's GPU cost is the figure the video path has to beat.

## Packaging

Wired by CutoverWatchdog (`7e36a87`, `cutover-result.md`; `tests/test_lumatv_vplane.py`, `tests/test_package_tools.py`):

- `tools/build-ipk.sh` requires and stages `vplane/build/arm/lumatv-vplane` (`sh vplane/build.sh`, zig), `vplane.sh`, `role.json` and the clips under `payload/vplane/`.
- `lumatv vplane install|start|stop|remove|status` runs `vplane.sh` (`start STATE /tmp/<shortId>-vplane <appId>.vplane`). `apply` starts the player only where `status` says installed and role written, and never installs; `revert` removes it as it does the colour reader.
- After a package update the staged program stays the old one until the owner runs `lumatv vplane install` again.

The way back is `lumatv vplane remove` (the role, the manifest, the binary and the clips) and `lumatv theme set '{"screensaver":{"video":false}}'`.

## Checks run (offline)

- `tests/test_vplane_player.py` (62): the real player on the host against a stand-in decoder library and a stand-in compositor (call order, feed rate, loop, window types, veil, black around the picture, caption, clock and drift pixel for pixel, page turns, wake, ttl, refusals, teardown, the runtime dir).
- `tests/test_vplane_clips.py` (24) and `tools/vplane_clips.py check --deep`: formats, geometry, QML numbers, contrast.
- `tests/test_vplane_saver.py`, `tests/test_saver_deck.py`, `tests/artdeck.test.js`: the saver against a stand-in player on the QML host, and against the real player (the request the QML writes is the one the C reads); the shuffle bag across restarts.
- `tests/test_vplane_install.py` (12), `tests/test_vplane_measure.py` (9).
- `compositor/crates/lumatv-compositor/tests/vplane_host.rs` (5): the player against the real wayland-server and compositor, the wallpaper hole under Home included; `tests/protocol.rs` takes the new hole order (what an app draws over its own video stays).
- `tests/test_vplane_wall_clips.py` (22) and `tools/vplane_wall.py check --deep`: the clip's formats, geometry, look against the QML, the seam, the poster numbers, the encoder options. `tests/test_vplane_player.py`'s `Wall` class (9), `tests/test_lumatv_wallpaper.py` (8), `tests/wallvideo.test.js` (7).
- `tests/test_vplane_wall.py` (16, QML host): the controller against a stand-in player (start, hole, hand-overs, failure inside a second, retry, ttl, the saver taking the player), Home's gating, the tint glass, the shell's `wall` topic, and the renders asserted (the plane shows through where Home draws nothing, the glass is the GPU wallpaper's, 7:1 over the worst plane).

## Tonight's check (TvRound8, Nitin's OK to install)

Needs `lumatv vplane` (7e36a87) and the saver (d4b7ca7) in the ipk, and `vplane/build/arm/lumatv-vplane` built (`sh vplane/build.sh`). Receiver muted through Home Assistant, Filmmaker picture mode. Stop at the first failure, send back what is named, then run step 8. The saver is entered with `luna-send -n 1 luna://com.webos.service.tvpower/power2/turnOnScreenSaver '{}'` (LG's own idle detector calls it after 180 s) and left with a key.

1. Baseline, the GPU art: `lumatv theme set '{"screensaver":{"style":"art","video":false}}'`, enter the saver, `python3 /tmp/measure.py run gpu-art 30 --out /tmp/vp/gpu-art.json` (`scp vplane/tools/measure.py` to `/tmp` first), wake. Pass: prints, caption and corner clock as before; the figures are the baseline.
2. `lumatv vplane install`. Pass: `{"ok":true,"vplane":{"installed":true,"role":true,"clips":6,...}}`.
3. `lumatv vplane start`. Pass within 3 s: `/tmp/lumatv-vplane/status.json` says `"state":"idle"` and `"clips":6`; `lumatv vplane status` says running. Send back `grep -E "LSHUB|Invalid permissions" /var/log/messages | tail` if anything is named.
4. `/var/lib/lumatv/bin/lumatv-vplane -n`. Pass: JSON that names the library and six clips, exit 0.
5. One print: `lumatv vplane stop`, then `/var/lib/lumatv/bin/lumatv-vplane --once great-wave 20`. Pass: exit 0 after 20 s, the print on the screen (dark wall, matte, frame) within 6 s, Home still the foreground app afterwards (`getForegroundAppInfo`), and note whether LG's own saver is seen over it. Fail: the last 40 lines of stderr and `grep -E "windowPolicy|Invalid permissions|LSHUB|AccessControl" /var/log/messages`; then `echo '-w card' > /var/lib/lumatv/vplane/args`, start again and repeat, then `-w overlay`, `-w default`.
6. The saver: `lumatv vplane start`, `lumatv theme set '{"screensaver":{"style":"art","video":true}}'`, enter the saver. Pass: within 6 s `status.json` says `"state":"playing"` and the print is on the video plane with the dim, caption and corner clock over it; no `SaverVideo` or `saver did not load` line in the log. Then `python3 /tmp/measure.py run video-art 30 --proc lumatv-vplane --out /tmp/vp/video-art.json` and `python3 /tmp/measure.py table /tmp/vp/*.json`. Expected: Mali under Home's 10 %, flips/s under 5, `vsync_per_s` 60 or 120, the player's CPU and RSS small; send the table.
7. A key press ends it: Home within about a second, `status.json` back to `idle`, no picture left, `MemAvailable` within 20 MB of before step 6.
8. Way back: `lumatv theme set '{"screensaver":{"video":false}}'`, `lumatv vplane remove`; pass: `installed:false`. Leave it installed only if all passed and Nitin wants it.
9. The kept order (on by default): enter the saver, `cat /var/lib/lumatv/theme/saver-deck.json`: pass when `decks["art/"]` has the six ids in `known`, five in `bag` and the print on the screen as `last`; after `lumatv restart` and the saver again, the first print is the head of that `bag`.

## TV steps

Steps 1-7 of "Tonight's check" ran once (Resume here has the figures). What follows is the full list and is mostly open: the loop seam, the drift's smoothness, the output rate in the Active state, the memory over three cycles, and the Screen Saver state's black panel. Receiver muted through Home Assistant before any playback; Filmmaker picture mode; nothing written outside `/tmp` until the install; abort on a failed unit. The sampler: `scp vplane/tools/measure.py` to `/tmp/`, then `python3 /tmp/measure.py run LABEL 30 --proc lumatv-vplane`; `table` puts the runs side by side.

1. Baseline, as found: `measure.py run as-found 30` in the Screen Saver state (Mali %, flips/s, vsync/s, CPU, `CmaFree`, temperature); `luna-send` `getForegroundAppInfo`; the window list. Then the same for LumaTV's GPU saver (`screensaver.video` off, art style).
2. `lumatv-vplane -n -c /var/lib/lumatv/vplane/clips`: the library and the clips are read (prints JSON, exit 0).
3. `--once great-wave 20 -w screensaver`, then `-w card`, `-w overlay`: does a picture appear, does Home stay foreground; the compositor log for `windowPolicy`.
4. Install the role (Nitin's OK): `vplane.sh install`; `/var/log/messages` for `LSHUB_NO_ROLE_FILE`, `LSRegisterPubPriv FAILED`, denied names: widen `role.json` for the name shown.
5. LG's saver: does the player's window replace `com.webos.app.screensaver` (eviction), does `tvpower` start it again, does the Mali share fall.
6. Output rate during the 60 fps clips: `/proc/interrupts` `DE2VSYNC` per second and vblank (60 expected), flips/s of the UI plane.
7. Hold or trickle: the player feeds 60 units a second; compare with feeding the key unit once and nothing after (CPU, the picture's stability).
8. Drift: the exported rectangle moves every 4 s; no flicker or blank frame at a move; if it flickers, `NDL_DirectVideoSetArea`.
9. Loop seam: 60 s on one print, a close look at the loop point (the breathing); `--loop-sec` 1, 2, 4.
10. CPU and RSS of the player, playing and idle (`measure.py --proc lumatv-vplane`).
11. `CmaFree` and `MemAvailable` before, during and after, three saver cycles; none may stay down after unload.
12. Veil, caption and clock over the hole: visible in window mode under LG's compositor; the same in shell mode where the shell is above the video.
13. Wake: a remote key and a pointer move end it; latency from key to Home; the player's wake command and LG's own `cancel()` do not fight.
14. Home still ticking under the saver: Mali and CPU with the saver up and Home foreground-gated or not.
15. The three side by side: as found, LumaTV's GPU saver, the video saver: `measure.py table`.
16. Whole frames: with the saver up on the video, run `--once great-wave 40 -w screensaver` (or leave the saver through three print changes) with `measure.py run buffers 30 --proc lumatv-vplane`. Pass: no frame in a screen capture shows the print's dim without its caption or the clock, the compositor log has no `wl_buffer`/`wl_shm` error and no "holds both buffers" line in the player's stderr (that line is the wait for a release, harmless once, a fault if it repeats every step), and the player's RSS is about 8 MB above the single-buffer run's 25 MB (both buffers resident, about 33 MB).

## Not verified on the TV

Beyond what the run shows: the loop seam; the exported window moving without a glitch; the output rate in the Active state; the CMA figures over several cycles; `TextFile` `exists` read straight after a `name` change and `write` (the deck's file check; the host stand-in does both); the key and pointer path to a screensaver window in the Screen Saver state. The whole moving wallpaper: Main10 on the set's pipeline, the compositor's hole under the wallpaper window, the hand-over times, the Mali and flips figures; none of it has run on the set.

## Needs Nitin

- The role file (an LS2 permission grant) and the new root process: `lumatv vplane install` when he says so.
- The switch: `lumatv theme set '{"screensaver":{"video":true}}'`, only after steps 1-6.
- The moving wallpaper: `lumatv vplane install` (above), then `lumatv wallpaper video`, only after steps 1-9 of "TV steps and measurements".

### TV round 13, 2026-10-28 (`compositor/slot-2026-10-28-1300/wallpaper/`): installed, plays, does not reach the plane under Home

LumaTV's compositor live (`0f2190c`), Home in front, receiver muted, Nitin's OK for `lumatv vplane install`.

- Step 1 (baseline, GPU wallpaper): Mali 12.7 % busy, 12.3 flips a second, `DE2VSYNC` 120.6 and `osd_irq` 119.9 a second, CPU of the compositor 3.0 %, of the
  shell client 4.9 %, `CmaFree` lowest 9.7 MB, `MemAvailable` 628 MB, 53 C.
- Step 2: `lumatv vplane install` `{"installed":true,"role":true,"clips":6,"walls":1}`, `start` running, `lumatv-vplane -n` `clipsOk 6`, `wallsOk 1`; no
  `LSHUB`/`Invalid permissions` lines for the player.
- Step 3: `lumatv-vplane --once wall-silk 20` at Home: `playing` 5 s in, `fed 302`, window `_WEBOS_WINDOW_TYPE_LUMATV_WALLPAPER` exported (`_Window_Id_6`), exit 0,
  Home stayed the foreground app. `CmaFree` fell to 5.6 MB while it played (11 MB before): the decoder takes about 5 MB of the 11 that are free.
- Step 4: `lumatv wallpaper video` alone changes nothing: `ready true` but the player stays `idle`. Home's own gate is the cause: `wallPick` needs the look the clip
  was made with (`wall.json` `look.motion`: amplitude 4, speed 1, depth 0.25, scale 1, detail 0.35, vignette 0, specular 0, bloom 0, saturation 1, contrast 1) and
  Nitin's `wallpaperMotion` is amplitude 12, depth 0.45, scale 1.2, detail 0.5, vignette 0.9, bloom 0.25, saturation 1.25, contrast 1.25, so the GPU wallpaper
  draws as designed. To test the plane anyway his look was set to the clip's through `lumatv theme set` (his `theme.json` was copied first and put back, md5
  equal, then `lumatv wallpaper gpu`): after `ctl shell restart` (the clip manifest is read at the shell's start; installing the player afterwards is not seen
  until then) the player went `playing wall-silk`, `fed` 904 in 15 s, `queue 1`, `playingMs` 15282.
  But the compositor's `wall` topic stayed `hole false`, the shell's `wall.up` false, `ctl windows` `video` `[]` (no exported video window placed), and SAM's log
  has `avc.impl tryConnect "No render_status"` for the player's decoder: the picture was never put on the plane under Home. Measured over 27 s with the player
  playing: Mali 12.5 % (12.7 % on the GPU wallpaper), 12.1 flips a second, `DE2VSYNC` 120.5, CPU 15.8 % in all (the player 4.1 %, the compositor 3.1 %, the shell
  client 5.2 %), `CmaFree` lowest 3.3 MB. No saving, and 3 MB of CMA left. FAIL; steps 5 to 10 not run.
- End: `lumatv wallpaper gpu` (`source gpu`), Nitin's look back, player stopped, `vplane` left installed. `lumatv wallpaper video` is not turned on.
