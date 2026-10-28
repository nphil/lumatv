# Screenshots and screen recordings of what LumaTV composes

## Resume here

Built and tested offline (2026-10-28); nothing has run on the TV. The switch is `capture.enabled` in `/var/lib/lumatv/helper.json`, off by default; Main switches it on after the TV check below.

Needs before the check: the encoder's second slot (`docs/research/venc-result.md`, steps 1 to 10, `synth-720` above all) must have passed. The first slot never got a stream out of `/dev/venc` (a call-order mistake of the prototype, fixed since), so the hardware encoder is unproven on the set. Screenshots do not depend on it and can be checked first (steps 1 to 5).

## What it does

- **Screenshot**: a PNG (RGB, 1920x1080) of the last page the compositor composed, in `/var/lib/lumatv/captures/shot-YYYYMMDD-HHMMSS.png`.
- **Recording**: an H.264 MP4 (fragmented, 1280x720, 30 fps, 4 Mbit/s by default) of the same pages, `rec-YYYYMMDD-HHMMSS.mp4`, through the hardware encoder.
- **Triggers**: Settings > System > Screenshots & recording (a screenshot now, a screenshot in 5 seconds, a recording switch), two optional quick-panel tiles (Screenshot, Screen recording; offered under Add a tile only while capture is on), and the helper: `POST /v1/capture/screenshot`, `POST /v1/capture/record/start`, `POST /v1/capture/record/stop`, `GET /v1/capture`. The helper forwards to the compositor's control socket: `lumatv-compositor ctl screencap status|shot|record start|record stop`.
- **Indicator**: a red dot in the top left corner of the system layer for as long as a recording runs (a note, "Screenshot saved" or "Recording saved", for 2.6 s after a capture; it is not in the picture it announces).
- **Files**: under a size cap (`capture.maxBytes`, 256 MiB; the oldest of its own files go first, nothing else in the folder is ever touched), a free-space floor of 128 MiB that stops a recording, one recording at most 5 minutes (`capture.maxRecordSeconds`) and half the cap.

## Decisions and why

- **The source is the compositor's own pages.** Video is on a plane no page holds, and the hole left for it is black in the picture (`Output::grab` makes the alpha opaque; the compositor's pixels are premultiplied, so that is compositing over black). Protected video is black by construction, not by a check. Nothing in this feature opens the video tap (`/dev/video60`), LG's capture service, or any device of the video path. Unprotected video is black too: a capture of a film is a capture of LumaTV's controls over a black window.
- **Composed, not scanned out, while capturing.** A window that covers the screen is normally scanned out directly and no page holds it; a screenshot at Home at rest would read a stale page. While a capture is pending or a recording runs, `Output::set_compose_only` stops direct scan-out (`kms/mod.rs`), one full frame is forced (`Scene::redraw_all`), and the picture is read from the page that frame went into. Back to direct scan-out when the capture ends. The picture is read on the GPU: one textured quad from the page into a scratch target of the size asked (the GPU scales), then `glReadPixels` (`Gl::grab_target`); the scratch target is freed when the capture ends. No page, texture or damage state of the frame path is touched.
- **The encoder is a child process** (`lumatv-venc feed`, `venc/probe.c`, wire in `venc/feed_proto.h`), not a library in the compositor: it shares the video MCU with the decoder, and a call that never returns costs a killed child, not the frame loop. Every wait on it has a deadline (3 s a frame, 5 s to open); the child has the encoder's own watchdog and slow-encode stop besides. NV12 frames go over a pipe (about 1.4 MB at 720p30, a copy each way), one frame in flight.
- **The pipeline never blocks the loop.** The loop reads a picture when a frame was drawn and 33 ms have passed since the last one, and hands it to a thread (nice 10) through a channel that drops, counted, rather than waits. The thread converts to NV12 (BT.709, limited range, integer), feeds the encoder and writes the file. A screen that does not change is kept alive: after a second with no picture the last one is encoded again (a few dozen bytes), so the file has a sample at least every second.
- **Fragmented MP4**, because a recording on a TV can end at any moment: every fragment (a keyframe, at most two seconds) is whole in itself and the file plays up to the last one, with nothing rewritten at the end. Timestamps are the compositor's clock.
- **No recording without the dot, structurally.** The compositor publishes the `capture` topic on the shell's UI channel; the shell draws the dot and says so in its state (`capture.dot`); a recording starts only once that arrives (4 s), and ends if it stops arriving for 3 s (the shell gone, restarted, or its layer not drawing). Screenshots need the shell connected (something that can say a capture happened). So a process on the TV that calls the API cannot record in secret.
- **One switch, one file.** The helper checks `capture.enabled` before it asks the compositor anything; the compositor reads the same `helper.json` again on every command, so a change needs no compositor restart (the helper reads it at start, as for `cameras.enabled`). The helper's config checks the whole `capture` section at start (a typo fails there, as the cameras' does); the compositor clamps.
- **Idle costs nothing.** No thread, no GPU target, no timer until a command: `State::tick` tests four fields and returns; the topic is compared without building JSON while unchanged.

## Interfaces

| Where | What |
|---|---|
| `helper.json` | `{"capture": {"enabled": false, "maxBytes": 268435456, "maxRecordSeconds": 300, "record": {"height": 720, "fps": 30, "kbps": 4000}, "encoder": "", "control": "/tmp/xdg/lumatv-control"}}`; `height` 360, 540, 720 or 1080; `encoder` empty is `<data>/bin/lumatv-venc` |
| Control socket | `screencap status`, `screencap shot`, `screencap record start`, `screencap record stop`: one JSON line back; `ok: false` carries `code` (`disabled`, `no-indicator`, `busy`, `not-recording`, `no-encoder`, `disk-full`, `timeout`, `grab`, `write`, `encoder`) and `error` |
| Helper | the four routes above; 403 `capture_disabled`, 409 `no_indicator`, `busy`, `not_recording`, 501 `no_encoder`, 503 `compositor_unavailable`, 507 `disk_full`, 504 `timeout`, 500 `capture_failed`. The shell's QML tells all but the 409 group by status alone: a 409 reaches it as status 0 in the workstation host, so the 409 group is told from the state it reads afterwards |
| UI channel | topic `capture` `{recording, stage, since, n, last {kind, name, ok, note, at}}` out; `state.capture.dot` back |
| Files | `/var/lib/lumatv/captures/` (`.part` while a file is written), the encoder's log `/var/lib/lumatv/compositor/venc.log` |

## Sharing to Home Assistant or BeastNAS (design only, not built)

- Always the owner's action on the TV: a Share row on a capture in Settings, answered with a confirmation that names the destination. Never automatic, never from the API, never from an assistant: a capture is the screen, and sending it out is disclosure.
- Home Assistant: the helper already holds the saved link (`ha_store`); a screenshot or recording goes to HA's media upload (`POST /api/media_source/local_source/upload`, into a `captures` folder of its media), where a phone, a dashboard or an automation finds it. Outbound only, to the address the owner linked.
- BeastNAS: one destination in `helper.json` (`capture.share`: an HTTPS URL or an SSH target, set by the owner); the file is pushed, not pulled, so the TV opens no port. Recordings up to 128 MiB stream; the TV deletes nothing after sending.
- Each send is logged on the TV with its destination and shown in Settings, as the AI access calls are.

## Not done

- `lumatv-venc` is not in the ipk: stage it like `lumatv-jpeg` (`jpeg_stage` in `tools/lumatv`, `tools/build-ipk.sh`) once the TV check passes. For the check it is copied by hand.
- No audio. A recording is silent; the TV's own mix is a separate decision (`venc-result.md`, Needs Nitin 4).
- A delete-all row and a list of files in Settings (the page shows the count and size; Main or Cody read the folder).
- The capture is 1920x1080 at most (the output), whatever the panel resolution; 1080p recording works only if the encoder takes 1080 (the second slot's `--full`).

## TV check (needs Main; Nitin's OK for the encoder steps)

Preconditions: forge/main with this commit installed through the usual Face ID route, LumaTV's compositor live, receiver muted, Home in front. For steps 6 to 12 the encoder's second slot has passed and `venc/build/arm64/lumatv-venc` is in `/var/lib/lumatv/bin/lumatv-venc` (mode 755). Helper restarted after each edit of `helper.json`.

1. **Off by default.** `curl -s -XPOST 127.0.0.1:18731/v1/capture/screenshot` answers 403 `capture_disabled`; `lumatv-compositor ctl screencap shot` answers `disabled`; Settings > System > Screenshots & recording says it is off, rows dimmed; `ls /var/lib/lumatv/captures` has nothing; the quick panel's Add a tile does not offer the two tiles.
2. **Switch on**: `capture.enabled: true`, helper restarted. `GET /v1/capture`: `enabled true, available true`.
3. **Screenshot at Home at rest** (Home alone, so the window is scanned out directly): `POST /v1/capture/screenshot` returns a name in under 1 s (`ms` is the PNG's write); `scp` the file: 1920x1080, Home as it is, upright, not black, not stale. "Screenshot saved" shows top left for 2.6 s and is not in the picture. During the call `ctl status` shows no `tick:` warning over 8 ms in the log beyond the one frame; Home's frame gate numbers do not move.
4. **Screenshot over the composed case**: with the volume display or a toast up, and again with the quick panel open: the picture shows it. With the cursor visible the sprite is in it.
5. **Video is black**: Netflix (protected), then YouTube or an HDMI input: the PNG has the app's controls and a black window where the video plays; no pixel of the film. Compare with LG's own capture (`luna-send ... executeOneShot`), which shows the video: the difference is the point.
6. **Recording**: Settings > System > Screenshots & recording > Screen recording on. The dot is in the corner within a second; `ctl screencap status` says `stage recording`, `dot true`. Navigate Home for 30 s, switch off. The file is whole (`ffprobe`: h264, 1280x720, about 30 s, `ffmpeg -i f.mp4 -f null -` with no error) and shows Home moving. Read: the child's `venc.log` (its last JSON line: frames, `ioctl_ms_avg`, `hw_ms_avg`, `cpu_pct`), the loop's `ctl status` (`renderUsMax`), `ctl wake` and the log's `tick:` lines during the recording, `top -H` for the `record` thread and `lumatv-venc`.
7. **Cost**: the frame gate over Home while a recording runs (`docs/research/perf/gate.py`): p90 render and late frames against the same run with no recording; the loop's thread must not be the one that moves. The recording's own stutter is the pipeline's to answer: `status` shows `dropped` and `keptAlive`.
8. **Quick-panel tiles**: Edit tiles > Add a tile > Screenshot and Screen recording; the screenshot taken from the tile shows what was under the panel (it slides away first); the recording tile starts a recording as the panel closes.
9. **The dot is the rule**: start a recording, `lumatv-compositor ctl shell restart`: the recording ends within about 4 s, its file plays, `last.note` names the dot. With the shell not running (`ctl shell` says so), `record start` and `shot` answer `no-indicator`.
10. **Limits**: `maxRecordSeconds: 5` (the least), record: it ends by itself with the time limit, a whole file. `maxBytes` 16 MiB with a few files in the folder: the oldest go, a foreign file in the folder stays.
11. **Encoder failure**: during a recording `kill -9` the `lumatv-venc` child: the recording ends with the reason in `last.note`, the file up to then plays, the compositor's frames do not move, Netflix started afterwards plays (the encoder shares the video MCU). `dmesg | grep -i venc`: only the `open::ok`/`close::ok` pairs. With the program missing, `record start` answers `no-encoder` and names the path.
12. **Off again**: `capture.enabled: false`, helper restarted: 403 again, `pidof lumatv-venc` empty, no `record` or `shot` thread in `ls /proc/$(pidof lumatv-compositor)/task`, the compositor's idle wake-ups as before.

Pass marks: steps 1 to 5 on their words; step 6 a decodable file of about the recording's length with a timeline that matches the clock; step 7 no loop-side change beyond the readback (a few ms on the frames drawn for a picture, not a missed refresh over Home); steps 9 to 12 on their words.

## Tests

- Compositor: `cargo test` (unit: colour conversion against the standard matrix, the folder and its cap, the MP4 writer against ffmpeg decoding a real x264 stream, the recording pipeline against a fake encoder; `tests/screencap.rs`: off by default, no capture without the shell, the PNG with a black hole and premultiplied pixels over black, the cap, a recording through a fake encoder into a whole file, no start without the dot, a recording that ends when the dot goes, a missing encoder; `tests/screencap_gl.rs`: Mesa's software GL, the page read back upright, at 1:1 equal to the CPU readback, scaled by the GPU, the scratch target freed; `tests/screencap_venc.rs`: the recorder against the real child built for the host and against scripts that die, hang or answer nonsense). `cargo test --features ui`, `cargo clippy --all-targets` with and without it.
- `venc/test.sh`: the `feed` round trip against the fake kernel (ready record, a unit per frame with the frame's pts, a keyframe every GOP starting with its SPS, a clean end when stdin ends, nothing but the protocol on stdout, a wrong-sized frame refused).
- Helper: `capture::tests` (off refuses before the compositor is asked, the routes relay, every reason has a status, a compositor that is not there or talks nonsense), the config section.
- QML: `python3 tests/qmlhost/compile_all.py`; `bun test tests/capture.test.js`; `tests/test_capture_ui.py` (the dot, its report and the note against the shell client's host; the page against the fake helper).

## Changelog line (for Main)

Screenshots and screen recordings of what LumaTV composes (video stays black): PNG of the compositor's last page, fragmented MP4 through the hardware encoder's program (child process, deadlines), red recording dot required from the shell, files under a size cap; `POST /v1/capture/{screenshot,record/start,record/stop}`, Settings > System > Screenshots & recording and two quick-panel tiles; off by default behind `capture.enabled`, TV check in `docs/research/capture-result.md`.
