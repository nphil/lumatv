# The art saver on the video plane: `lumatv-vplane`

Built and tested offline; nothing has run on the set. The art mode's picture comes from the TV's idle hardware decoder (a still, looped) instead of the GPU, so the GPU draws a black rectangle while the saver is up. The saver keeps its GPU path as the fallback and the owner's switch is off (`theme.screensaver.video`).

## Resume here

Done (2026-10-25, offline, host-tested; counts under "Checks run"):

- `vplane/`: the 32-bit player service `lumatv-vplane` (C, ARM and host builds), its decoder-library and compositor stand-ins and the sampler for the TV steps (`vplane/tools/measure.py`).
- `tools/vplane_clips.py` and `payload/vplane/clips/`: the six prints as looped HEVC stills (2.07 MB), caption bitmaps and the clock's glyph atlas.
- `panels/SaverVideo.qml` (the saver's end of the mailbox), the video branch of `panels/Screensaver.qml`, `payload/vplane/{vplane.sh,role.json}` (install, supervisor, LS2 role).
- The art saver's shuffle bag can be kept across restarts (`ArtDeck.js`, `/var/lib/lumatv/theme/saver-deck.json`), in the GPU and the video path alike, behind `theme.screensaver.art.keepOrder` (off): the library passes its tests; the saver's restore from a seeded file did not pass its host test at the integration cut, so the default is off until it does.

Open: the TV steps below. The role file is an LS2 permission grant and the player a new root process, so both need Nitin's OK. Until steps 1-6 pass, `screensaver.video` stays off. Not built: the moving wallpaper (design below), the Home gating hook, the compositor's hole draw order.

Evidence tags: `TV:` measured on this set, `SRC:` LG's firmware tree in the repo, `PUB:` public source, `[INFERENCE]` not run.

## Verdict

Expected: the saver's GPU cost falls from the as-found Screen Saver's 54.5 % Mali and 60 flips/s (TV: `hardware-census.md`, Live counters) to a static black rectangle plus a veil, caption and clock redrawn about once a minute [INFERENCE]. What can sink it, each a TV step: LG's compositor refusing the player's window (`windowPolicy`), the output rate not staying 60 Hz under a still, and a visible seam where the clip loops.

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
- Latent defect, not touched: the saver's clock text is Qt's `"h:mm"`, which is 0-23, so the "twelve hour" face shows 17:05. The player reproduces it so the two savers agree (`vpl_plan.h`).

## Who draws the words

| Mode | Setting | The picture, veil, caption, clock | Works where |
|---|---|---|---|
| Window (default) | `videoOverlay` unset | The player's own window, above the video, draws them | LG's compositor: the hole (`Eos.PunchThrough`, `z: -1`, SRC `views/punchThrough.js`) sits under the window's pixels, so they show |
| Shell | `videoOverlay: "shell"` | The saver draws them above a transparent hole; the player's window draws nothing (`overlay:false`) | A compositor that draws the shell above the video plane |

Finding in LumaTV's own compositor (`compositor/crates/lumatv-compositor`, `render/software.rs:125`, `render/gl.rs:776`, `scene.rs`): holes are cleared after the windows are drawn, which erases a window's own pixels inside the video rectangle. Window mode would show no veil, caption or clock there; any app's controls over its own video suffer the same. Fix: clear holes before drawing the windows above them, or use shell mode. Also there: `Screensaver` windows are Cards-layer and foreground-eligible (`window.rs`, `layer()`).

The order of the prints is the saver's in both modes (its bag, kept across restarts): the first print is named in `start` (`clip`), each next one by `next {clip}`, and the player never turns a page by itself (`auto:false`; it does when the saver's index has no prints).

## Window type, and LG's own saver

Default `-w screensaver`: `_WEBOS_WINDOW_TYPE_SCREENSAVER` with `playVideo: true`. In LG's compositor (SRC `views/StarfishScreenSaverView.qml`) a new saver window makes `onSurfaceAdded` call `close()` on the current one, so the player's window evicts LG's Flutter `com.webos.app.screensaver`, the app behind the as-found 54.5 % [INFERENCE: the animation is LG's saver app]. With `playVideo` the controller sends all apps to the background (SRC `controllers/StarfishViewStateController.qml:615-625`), so the Home stub is held back too. LG's own key path for the saver, `cancel()` to `power2/turnOnScreen`, works for a window it does not own. The saver view is not in the access-policy lists (SRC `controllers/StarfishAccessControlPolicy.qml`) [INFERENCE: it accepts any]; a refusal is `closeWindow(..., windowPolicy)` in the compositor log.

If the screensaver type is refused, or LG's app comes back over it: `-w card`, `-w overlay`, `-w default` (`STATE/vplane/args`, one line, read at the next start). Home still ticks under the saver unless gated: a hook for `LumaHome.qml` (`running: root.hostActive && !layers.saverUp` on the wallpaper timer, the clock and the like) would also help the GPU saver.

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

`videoUsable` is false (GPU only) for a preview (`forced`), any style but art, and `Lu.textScale > 1` (the clips' words are drawn for the TV's own text size). Nothing runs while the saver is down.

## Memory

| Item | Size | Basis |
|---|---|---|
| Window buffer | 8.3 MB shared memory, resident once the veil, caption or clock is drawn | 1920x1080x4 |
| Decoder | about 8-20 MB CMA while playing, released at unload | [INFERENCE] one reference frame, no B-frames (2.85 MB per NV12 frame); 4K Dolby Vision holds 137 MB on the display engine's windows (opennow-result.md) |
| Clips | 2.07 MB in the ipk and `STATE/vplane/clips`; one key unit (250-400 KB) is read to feed | `payload/vplane/clips` |
| Floor | the player will not start under 100 MB `MemAvailable` (`-m`) | `MemAvailable` 684-735 MB at Home (TV: hardware-census.md) |

`CmaFree` is not a guard: CMA is lent to movable pages and the kernel migrates them out when the decoder asks (memory-result.md, CMA); it reads 4-9 MB while 4K video plays and allocations made beforehand hold (opennow-result.md). The step list reads it before, during and after.

## Fallback

No player, an error from the player, no answer in 6 s, a heartbeat gone stale for 12 s, a restart of the shell: the saver draws the art itself and asks the player to stop. The owner's switch is the only way to the video path, and it is off. Turning it off while the picture is up gives the art to the GPU from black.

## Moving wallpaper (not built)

Needs files that are not this slice's. Under LG's compositor the Home stub (`payload/homestub/index.html`, a transparent web page) plays an HTML5 `<video loop>` from a variant page that a TV step installs. Under LumaTV's compositor a helper window sits beneath the shell, with a hole contract in `LumaHome.qml` (a transparent wallpaper layer where the video shows). The loop would replicate `liveWall`'s warp (`LumaHome.qml`, 960x540 at 30 fps, 10-20 s, seamless); `vplane_clips.py` writes any such clip. Cost to the GPU is the hole's blend instead of 10 % Mali at 10 fps (hardware-census.md); blur-behind over the video is not possible.

## Packaging

Wired by CutoverWatchdog (`7e36a87`, `cutover-result.md`; `tests/test_lumatv_vplane.py`, `tests/test_package_tools.py`):

- `tools/build-ipk.sh` requires and stages `vplane/build/arm/lumatv-vplane` (`sh vplane/build.sh`, zig), `vplane.sh`, `role.json` and the clips under `payload/vplane/`.
- `lumatv vplane install|start|stop|remove|status` runs `vplane.sh` (`start STATE /tmp/<shortId>-vplane <appId>.vplane`). `apply` starts the player only where `status` says installed and role written, and never installs; `revert` removes it as it does the colour reader.
- After a package update the staged program stays the old one until the owner runs `lumatv vplane install` again.

The way back is `lumatv vplane remove` (the role, the manifest, the binary and the clips) and `lumatv theme set '{"screensaver":{"video":false}}'`.

## Checks run (offline)

- `tests/test_vplane_player.py` (57): the real player on the host against a stand-in decoder library and a stand-in compositor (call order, feed rate, loop, window types, veil, caption, clock and drift pixel for pixel, page turns, wake, ttl, refusals, teardown).
- `tests/test_vplane_clips.py` (24) and `tools/vplane_clips.py check --deep`: formats, geometry, QML numbers, contrast.
- `tests/test_vplane_saver.py`, `tests/test_saver_deck.py`, `tests/artdeck.test.js`: the saver against a stand-in player on the QML host, and against the real player (the request the QML writes is the one the C reads); the shuffle bag across restarts.
- `tests/test_vplane_install.py` (12), `tests/test_vplane_measure.py` (9).
- `compositor/crates/lumatv-compositor/tests/vplane_host.rs` (4): the player against the real wayland-server and compositor.

## TV steps

None has run. Receiver muted through Home Assistant before any playback; Filmmaker picture mode; nothing written outside `/tmp` until step 4; abort on a failed unit. The TV is in a slot (Main's release). The sampler: `scp vplane/tools/measure.py` to `/tmp/`, then `python3 /tmp/measure.py run LABEL 30 --proc lumatv-vplane`; `table` puts the runs side by side.

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

## Not verified on the TV

Everything above. In particular: that LG's compositor accepts a screensaver-type window from this app id; the role's names; a still at 60 fps through the pipeline; the loop seam; the exported window moving without a glitch; the output rate; the CMA figures; `TextFile` `exists` read straight after a `name` change (the deck's file check; the host stand-in does it); the key and pointer path to a screensaver window.

## Needs Nitin

- The role file (an LS2 permission grant) and the new root process: `lumatv vplane install` when he says so.
- The switch: `lumatv theme set '{"screensaver":{"video":true}}'`, only after steps 1-6.
