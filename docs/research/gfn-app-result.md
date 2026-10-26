# The GeForce NOW app for the TV: build state, TV probe list

OpenNOW's stream engine and application core, unmodified and pinned, with a webOS platform crate that puts the stream on the video plane through LG's NDL DirectMedia (a 32-bit feeder) and reads the controller from evdev. Research and decisions: `opennow-result.md` (decode path), `gfn-444-vrr-result.md` (the ceiling: 4K120 HEVC Main10 HDR10 4:2:0 at a fixed 120 Hz, low-delay cloud-game mode), `gfn-input-result.md` (the receiver and the input design). Everything here was built and tested offline on the workstation; nothing has run on the set.

Evidence tags as in those notes: `TV:` measured on this set, `PUB:` public source, `[INFERENCE]` not run.

## Resume here

Done (2026-10-25/26, offline, host-tested; counts under "Checks run"):

1. **Probe streams** (`gfn/tools/gfn_streams.py`, output in `gfn/build/streams/`, not in git, 715 MB): the Phase 0 matrix, 13 files, all `check` ok (the 4K120 at a declared level 5.1 is encoded at 5.2 and has its VPS/SPS level byte rewritten; x265 refuses 4K120 at 5.1 outright).
2. **The feeder** `lumatv-gfn-feeder` (`gfn/feeder/`, C, ARM and host builds): NDL v2 with Opus passthrough (the audio type NDL has; no Opus decoder in the engine), HDR10 static metadata, the frame-drop threshold, flush, render-queue readings; `--play` for the probe (one JSON line a second: queue min/max/mean/p99, waits on a full buffer, events 44/45/46, CmaFree, MemAvailable, RSS) and `--serve` for the engine (`gf_proto.h`). Its window is vplane's bare Wayland client or an id exported by the UI process (`--window-id`).
3. **The engine** (`gfn/engine/`, Rust): `opennow-streamer` built from OpenNOW `a49b124` (2026-10-18) with `[patch]` replacing `opennow-streamer-platform` by the webOS crate (`crates/opennow-streamer-platform`: the feeder client, the capability report, the pad bridge) and `crates/lumatv-gfn-pad` (evdev discovery, reader thread per pad, grab, rumble). Builds for the host and for the set (`cargo zigbuild --release --target aarch64-unknown-linux-gnu.2.36 -p lumatv-gfn-streamer`: aarch64, glibc 2.35 or older symbols, 61 MB with line tables). `hello` answers with backend `ndl`: H.265 8/10-bit 4:2:0 with HDR10, H.264 8-bit, AV1 refused with its reason.
4. **The core** `opennow-core` builds from the same revision through `gfn/engine/build-core.sh` with `keyring` replaced by `crates/keyring` (no OS credential store on the set: the core keeps sessions in its own 0600 JSON files, its documented `local-file` path). Host build done; the aarch64 build is the next command to run (`sh gfn/engine/build-core.sh`).

Open, in order:

5. The UI (the 64-bit card app over the core protocol: sign-in with the device code and QR, library, launch, in-stream overlay that hides after inactivity) and the pad reader for the library screens. Not started; design under "The app" below.
6. Packaging: `gfn/build-ipk.sh` for `org.nphil.lumatv.gfn` (own ipk, never touching LumaTV's install), with the feeder as the app's `main` (the role the installer writes is keyed to `main`'s path and is what NDL needs) starting the 64-bit UI through `gfx64/launch.sh`. Not written; plan below.
7. Phase 1 on the set (E1-E3, approved; the slot is Main's to grant): the list at the end.

Commits: `b62692b` (streams, feeder, tests), then the engine. Builds needed after a fresh checkout: `sh gfn/feeder/build.sh`, `cd gfn/engine && cargo build && cargo zigbuild --release --target aarch64-unknown-linux-gnu.2.36 -p lumatv-gfn-streamer`, `sh gfn/engine/build-core.sh [--host]`, `python3 gfn/tools/gfn_streams.py gen` (15 minutes).

## What was built, and why this shape

| Piece | Where | Runs as | Talks to |
|---|---|---|---|
| Feeder (NDL DirectMedia, 32-bit) | `gfn/feeder/` | the app's `main` (SAM's jail uid, groups `input` and `compositor`) | the engine over `/tmp/lumatv-gfn/feed.sock` (`gf_proto.h`); LG's compositor for its own window in probe mode |
| Engine (OpenNOW streamer + webOS platform crate, 64-bit) | `gfn/engine/crates/{opennow-streamer-platform,lumatv-gfn-pad,lumatv-gfn-streamer}` | a child of the core, through the 64-bit runtime (`gfx64/launch.sh`'s loader) | the core over JSON lines on stdio; the feeder; `/dev/input/event*` |
| Core (OpenNOW application core, 64-bit) | built by `gfn/engine/build-core.sh` | a child of the UI | the UI over JSON lines on stdio (`docs/core-protocol.md` in OpenNOW); NVIDIA's services |
| UI (64-bit card app, Rust toolkit) | not yet | SAM's card window, started by the feeder | the core; the pad side channel (`LUMATV_GFN_PAD_SIDE`) while the overlay is up |

Why the feeder is `main`: a native app's LS2 role is generated at install time for `main`'s exact path and names `com.webos.media.client.*` and `com.webos.rm.client.*`, which is what NDL registers (PUB jellyfin-webos-native `docs/ndl.md`, "It needs a Luna role"); any other binary would need a role written as root (what `lumatv vplane install` does for the saver). The 64-bit UI cannot be `main` because the loader and the Mali runtime are the launcher's job (`rustui/app/stub/lumatv-card.c`, `gfx64/launch.sh`), so the feeder takes the launcher's part.

Why the engine does not decode: LG's library is 32-bit and the engine is 64-bit; the socket carries access units (12.5 MB/s at 100 Mbit/s) and the feeder's replies. The PTS both carry is the arrival time on the engine's monotonic clock, nanoseconds by default (jellyfin feeds nanoseconds; SS4S feeds milliseconds; the ES player presents on arrival and uses the PTS for A/V alignment only), `--pts-unit` and `LUMATV_GFN_PTS_UNIT` switch it for the probe.

Audio: NDL's load takes `NDL_AUDIO_TYPE_OPUS` with the channel count and the rate in kHz (SS4S `ndl_audio.c`), so GFN's Opus packets pass straight through; the set ships FFmpeg's Opus decoder inside the pipeline (`gfn-444-vrr-result.md`). PCM S16LE is in the protocol as the fallback if the TV refuses Opus; it would need a decoder in the feeder (the set's own `libavcodec.so.59`, 32-bit).

What the TV path says no to, and the engine reports as such: recording and replay (the frames never pass through the process), the microphone, pointer lock, screenshots. Capability report (`crates/opennow-streamer-platform/src/lib.rs`): the core's resolver (`opennow-core/src/streamer.rs`) reads `hdrColorQualities` for HDR and `colorQualities` for the rest, so Auto with HDR on resolves to H.265 10-bit 4:2:0 HDR10, Auto without to H.265 8-bit, and 4:4:4 falls back to 4:2:0 with the core's own diagnostic.

### Controllers

`lumatv-gfn-pad` (the design of `gfn-input-result.md`): sysfs facts for discovery (`capabilities/ev` with KEY and ABS, `capabilities/key` with BTN_SOUTH), the node opened read-write, one blocking reader thread per pad, a snapshot in XInput form at every `SYN_REPORT` that changed it (DS4 through `hid-sony`: `BTN_SOUTH/EAST/WEST/NORTH` to A/B/X/Y, `ABS_X/Y/RX/RY` 0..255 resting at 128 to -32767..32767 with up positive, `ABS_Z/RZ` to the triggers, the hat to the d-pad; OpenNOW's radial dead zone, 8 %), `EVIOCGRAB` while a session is live, inotify plus a 2 s rescan for hotplug, `EVIOCSFF` + `EV_FF` for rumble (the engine's `controller-rumble` event is applied in the streamer process, which holds the pad, before it goes up the pipe). The PS button is `Guide` (the engine asks the UI for its overlay); Select+Start held a second is the exit chord (`StopStream`). While input is paused (the overlay is up) the reports go to the UI's datagram socket instead of the game.

Latency added by this path is the engine's own 250 µs poll of its input queue on top of the ~2 ms of the receiver (`gfn-input-result.md`); the Phase 4 measurement stands.

## The app (not yet built)

The 64-bit card app of `cardapp-result.md` (Wayland window, every seat's keyboard, SAM's lifecycle, proven on the set 2026-10-25 19:39) with these screens, Lucent TV profile (`skill://design-language`, `languages/lucent/LANGUAGE.md`; minimal text, a row is name and value, explanations behind one help mark):

1. Sign-in: `auth.device.start` gives the code, the URL and `qrRows`; the screen shows the QR and the code, polls `auth.device.poll` at the interval it names, `auth.device.complete`. Nitin signs in himself; the app never sees a password and the core never shows a token.
2. Library: `catalog.library.list` pages (box art through `artwork.*`), a search, a game page with `catalog.launch.inspect`'s decision (`ready`, `ownership_required`, ...).
3. Launch: `streamer.detect` once (the engine's `hello`), `session.create` with `runtimeCapabilities` = that report plus `nativeHdrSupported: true` and `nativeHdrDisplay {minimumNits 0, maximumNits 1000}` (the HDR path is only resolved against the embedded capabilities the shell passes; without them the core refuses HDR), `session.poll` until status 2 or 3, `streamer.start {session}`; the core spawns `opennow-streamer` beside itself.
4. In-stream: a transparent window over the video (the compositor draws it above the punch-through, as the saver's window is drawn over its picture), chrome that leaves after inactivity, the overlay on `overlay-request` (Guide) or the remote's Back: `streamer.input.pause` while it is up, resume, `streamer.stop` + `session.stop` to leave; stats from `streamer.changed` (bitrate, fps, drops, the feeder's queue in `DecodeTimings`).

Process start: SAM starts `main` (the feeder) which starts `gfx64/launch.sh ... lumatv-gfn-app`; the app opens the card window, exports the video element (`wl_webos_foreign.export_element`, as `vplane/src/vpl_wl.c` does; the Rust window needs the same request) and tells the feeder the id over the feeder's control socket (a `--window-id` at start is the probe's way); the app spawns the core with `nativeStreamerExecutablePath` pointing at the engine and the environment the engine reads (`LUMATV_GFN_FEEDER_SOCKET`, `LUMATV_GFN_PAD_SIDE`).

Package plan (`gfn/build-ipk.sh`, after `tools/build-ipk.sh`'s card package): app id `org.nphil.lumatv.gfn`, title "GeForce NOW", `main` = `bin/lumatv-gfn-feeder`, `bin64/{lumatv-gfn-app,opennow-core,opennow-streamer}`, the 64-bit runtime bundle (`gfx64/`, 37 MB, its own copy), `appinfo.json` with `requiredMemory` for a 4K stream, no `requiredPermissions` beyond the defaults until the TV says a name is denied. Installed and removed with `appInstallService/dev/install` like the card app (`tools/cardapp.sh`); LumaTV's package is not touched.

## Checks run (offline)

- `gfn/feeder/build/host/split_test` (4): the access-unit splitter on AUD and bare HEVC, two slices per picture, H.264, AV1 OBUs.
- `tests/test_gfn_feeder.py` (14): the feeder against the NDL stand-in (`gfn/feeder/tests/fake_ndl.c`) and the fake compositor (`vplane/tests/fake_wayland.py`): call order, every picture fed at the clip's rate with a rising PTS, HDR and drop-threshold calls after the load, Opus beside the video, a full buffer retried then dropped, a pipeline error stopping the run, a given window id meaning no compositor, loop and seconds, refusals reported, the socket protocol end to end (HELLO, LOADED, 150 ACCEPTs, STATUS, EVENT, PONG, FLUSH, STATE, UNLOAD), the ARM binary's facts (ELF32 ARM, soft-float, libc and libdl only), the generated streams splitting into exactly their pictures.
- `gfn/engine`: `cargo test --workspace` (9): the keyring stand-in, the pad's capability parsing, DS4 to XInput mapping and dead zone, the ff_effect layout and ioctl numbers, the feeder frames' decoding, HDR10 constants, the pad bridge's gating; plus `tests/session.rs`: the runtime against the real feeder (host build) with a pad on a pseudo-terminal (result below under "Resume here" once run).
- `gfn/tools/gfn_streams.py check`: 13 of 13 ok.

## TV probe list (Phase 1; E1-E3 approved, the slot is Main's)

Read-only first, then the probe app. Receiver muted through Home Assistant before any playback; Filmmaker picture mode noted; nothing written outside `/tmp` until the install; abort on a failed step.

Upload: `gfn/feeder/build/arm/lumatv-gfn-feeder`, the clips from `gfn/build/streams/` (start with the 10 s ones under 65 MB; the 100 Mbit/s one is 125 MB), `tone_48k_stereo_20ms.opuspkts`, `vplane/tools/measure.py`.

1. Read-only facts: `/etc/umediaserver/device_codec_capability_config.json` and `umediaserver_resource_config.txt` from this set; `luna-send -n 1 luna://com.webos.service.videooutput/...` is not needed yet.
2. `lumatv-gfn-feeder --probe` from `/tmp` (no app, no role): expected `"library":true` with the five optional calls (`features` 31) since the library sits in `/usr/lib`; a `LSRegisterPubPriv` abort is expected later at Init, not here.
3. Install the probe app (Nitin's OK): the ipk of step 6 or, before it exists, a minimal native app whose `main` is the feeder (`appinfo.json`: id `org.nphil.lumatv.gfn`, type `native`, `main` `lumatv-gfn-feeder`), through `appInstallService/dev/install`. Then run the installed binary over SSH exactly as `zig build play` does for `ndlplay` (PUB jellyfin-webos-native): `cd /media/developer/apps/usr/palm/applications/org.nphil.lumatv.gfn && ./lumatv-gfn-feeder --play /tmp/gfn/1080p60_high_sdr_l41_20m.h264 --size 1920x1080 --fps 60 --seconds 20 -w card`. Pass: a picture (the test card under noise), `"playing":1` in the per-second lines within 2 s, `fed` 60 a second, `dropped` 0. Fail: `LSHUB`/`Invalid permissions` lines in `/var/log/messages` name the role to widen; `windowPolicy` lines name the window type to change (`-w overlay`, `default`).
4. The HEVC tiers, each 20 s with `--loop --hdr`: `1080p120`, `1440p120`, `4k60`, then `4k120_main10_hdr_l52_50m_ref1`, `_80m_ref1`, `_100m_ref1`, `_80m_ref4`, `4k120_main_sdr_l52_80m_ref1`, and `4k120_main10_hdr_l51_80m_ref1` (the declared-5.1 question). Record per clip: the summary line, p99 queue, `wait_max_us`, `buffer_full`, `playing_ms`, CmaFree before/during/after, and `python3 /tmp/measure.py run <clip> 20 --proc lumatv-gfn-feeder` for flips/s, vsync/s (120 expected at 120 fps), Mali, CPU, temperature. Gate G1: the 80 Mbit/s 4K120 clip for 5 minutes (`--seconds 300`) with no `buffer_full` loop, no black frames, p99 queue at most 2.
5. E1 extras during the 4K120 HDR clip: `/proc/lg/de/win0/status` (`hdr_type`, `lowdelaymode`), `luna-send -n 1 luna://com.webos.settingsservice/getSystemSettings '{"category":"picture","keys":["pictureMode"]}'` (does `isGameMode` select `hdrGame`), `cat /proc/interrupts | grep -i vsync` twice 10 s apart. Then the same with `--drop-threshold 0`, `2`, `4` and `--pts-unit ms`.
6. E2: `4k120_main444_10_hdr_l52_80m_negative` for 10 s: expected black or garbage and a colour-domain line from the decoder (`dmesg`, `/var/log/messages`); the AV1 clip (`--codec av1 --fps 120`): expected refusal or 60 fps.
7. E3: `nice -n 19 ffmpeg -benchmark -threads 3 -i /tmp/gfn/4k120_main444... -f null -` with a static 64-bit FFmpeg if one is on hand; otherwise skip (the verdict in `gfn-444-vrr-result.md` stands on the source).
8. Audio: step 3's clip with `--audio /tmp/gfn/tone_48k_stereo_20ms.opuspkts` (receiver unmuted briefly): a tone; `adropped` 0. If NDL refuses the Opus load, the PCM fallback becomes the next feeder change.
9. The cross-process window: with the probe app's window up in one process (`--serve` mode with its own window), a second feeder `--play ... --window-id <the id from the first's stdout>`; expected: the picture shows in the first process's window [INFERENCE: what the installed app relies on].
10. Pads: `ls -l /dev/input/event* /sys/class/input/event*/device/name`, then the engine's pad crate on the set is Phase 4; nothing tonight.

Each step's lines go to `docs/research/gfn/tv-<date>/`.

## Not verified on the TV

Everything above. In particular: that the installer's role covers NDL's names for this app id; the window type the compositor gives the probe; that 4K120 Main10 is sustained at 80 Mbit/s; the level 5.1 declaration; HDR10 engaging through `SetHDRInfo` and the picture mode chosen; Opus through NDL on this firmware; a window id exported by another process; the PTS unit's effect on A/V alignment; the engine's socket throughput on the set's CPU; the pad's grab under SAM's jail.

## Needs Nitin

- Installing the probe app and the GeForce NOW app (Principle 0): the `appInstallService/dev/install` calls.
- Signing in: he enters the device code on his phone or PC; the app never stores a credential it was handed.
- GFN's terms: OpenNOW is a third-party client; his decision stands as in `opennow-result.md`.
