# Ambient light, phase 4: capture probe and helper-side modules

## Resume here

Done (2026-10-25, the probe slot of 16:27 captured; "Probe results" below):

- Offline recovery of the four `vtCapture_*Histogram` signatures, data layout, preset geometry, return codes and the
  process allow-list (see "ABI", "Permission gate"). Header `ambient/capture/vthist.h`.
- 32-bit probe `ambient/capture/probe.c` (`./build.sh`, `./test.sh`), host tests pass against a fake library.
  Built for the set as `ambient/capture/build/arm/ambient-probe` (ignored by git; rebuild with `build.sh`).
- Read-only TV slot (03:25 EDT): `libvtcapture` on the set is byte-identical to the workstation copy (md5 35d3fec6...);
  `/var/luna/preferences/vt_devmode` did not exist; nobody held `/dev/video60`; `arecord` is ALSA 1.0 with
  `libasound.so.2`; `aud_dsp_reset_count` is at `/sys/module/aud/parameters/`.
- Second slot (05:17 to 05:28 EDT): the `vt_devmode` reader search finished (3 readers, see "Permission gate"), the
  flag was NOT created (decision for Nitin below), the audio captures were done ("Audio results"). Set left in Active
  Standby, receiver unmuted at volume 0.355.
- Rust crate `ambient/` (`lumatv-ambient`, `./build.sh check` green at 05:19: fmt, clippy -D warnings, 87 unit tests,
  11 interop tests, aarch64-musl release build): zones, layout, bars, delay, tuning with dark gate, smoothing, engine,
  HueStream v2 over a DTLS 1.2 PSK client, WLED DDP sender. See "Helper-side design". Nothing was sent to any real
  Hue or WLED device.

Decision for Nitin (the only thing blocking the picture side):

- The capture probe needs the file `/var/luna/preferences/vt_devmode` to exist, because LG's library refuses
  `initHistogram` to any process that is not LG's Hue Sync service. The same file is read by two more programs of LG's
  screen-capture package: `/usr/sbin/captureservice` (the running `com.webos.service.capture`, which owns screenshots
  and the video-capture resource) and the test tool `/usr/bin/vtCaptureTestSuite`. The flag is therefore a
  development switch for LG's whole capture stack, not just one library check; with it set, LG's capture service
  most likely skips its caller permission check too (the exact effect in `captureservice` was not recovered: the path
  string sits in `captureService.cpp` and its use was not traced).
- Question: may the flag exist permanently (a file in `/var`, nothing on the read-only parts), so ambient light can
  run; or only for short supervised tests (create, probe, remove); or not at all, in which case the alternative is
  to run the capture helper as a process whose command line contains Hue Sync's service path, which is impersonation
  and is not proposed.
- With "short tests": the probe plan is 3 runs of 60 s (Home, HDMI 2 if it has signal, muted Netflix), flag removed
  straight after each batch. Nitin allowed the flag for probe slots (PLAN.md, Decided).
- Probe slot run 12:35 ("Probe slot" below): the flag passes the gate, then LG's library aborts the process because
  it registers on the luna bus and the probe has no LS2 role file. Probe slot run 16:21 with the package's reader and
  its role: the hub still said no role file, because on this firmware a volatile role is read only through a manifest
  (`capture.sh` writes one since `4888500`) and the first name the library registers is a resource-manager client,
  `com.webos.rm.client.<session>`. Probe slot run 16:27 with both: 60 s of zone data in all three scenarios.

Next, in order:

1. Tune `engine::Config` defaults from the dumps (`probe-2026-10-25-1627/<scenario>.json`, 16 areas each; replay
   through `zones::parse_dump`).
2. Done offline (not verified on the set): the reader's default is 10 polls a second (`-r`, 1..120), it logs a `cost:`
   line when a capture ends, `tv_probe_slot.py` runs one minute of Home at 10/s (`--hz`), and the helper's ambient
   engine, fed by the reader's own host build over the capture socket, drives a fake WLED socket (test
   `the_capture_reader_at_its_default_rate_drives_the_strip_end_to_end`). Hue streaming is not wired: see "Hue
   streaming: Nitin's steps" under "Probe slots".
3. Next TV slot: `python3 ambient/capture/tv_probe_slot.py --go --no-standby`, then read "Reader CPU at 10 polls a
   second" under "Probe slots". `tools/lumatv apply` installs the reader as the package does (`capture.sh install`).

## ABI

Everything below is `libvtcapture.so.1.0.0` on the set's webOS 10.2.1 (32-bit ARM, Thumb-2, built from LG's
`lib32-screen-capture-webos 1.0.0-56`); the workstation copy is `research/zones/libvtcapture.so.1.0.0`
(byte-identical to the census copy). The kernel side is documented in LG's header
`v4l2-ext-vt.h` (`research/lgsrc/.../v4l2-ext-renderer-header/linux/`), which matches the binary field for field.

```c
typedef bool  (*vth_policy_cb)(void);                         // LG's resource manager taking the capture back
void *vtCapture_initHistogram(vth_policy_cb cb);              // handle (a VtHistogram*) or NULL
int   vtCapture_setPresetHistogram(void *h, int preset);      // 0..3; must precede the first get
int   vtCapture_getHistogram(void *h, struct vth_zone *out);  // out: N entries, 16 always enough
int   vtCapture_finalizeHistogram(void *h);

struct vth_zone { uint32_t area_id; uint16_t r[256], g[256], b[256]; };   // 0x604 bytes
```

- Data: per area, one 256-bin histogram per channel (8-bit code values, `u16` counts), `area_id` 0..N-1. The mean of
  a channel is `sum(i * bin[i]) / sum(bin)`. Whether the counts are raw pixel counts or scaled is for the probe to
  show; Hue Sync copies the three arrays verbatim into the same layout its own software fallback fills.
- Areas are rectangles in the picture (0,0 top left, fractions of width and height), not a grid, and overlap.
  Hardware presets (from LG's header, copied into the Rust zone table):
  - 0 video, no bars: 16 areas. 1 top/bottom bars skipped (picture rows 0.1265..0.8735). 2 left/right bars skipped
    (columns 0.13..0.87). 3 games: 14 areas, more weight on the edges.
  - The black-bar handling HyperHDR does in software is therefore a preset choice here: 1 for letterboxed films, 2 for
    pillarboxed, 0 otherwise; the helper still needs a detector to pick (see the mapping design).
- Return codes (from the ioctl errno): get 0 ok, 2 not started, 3 null out, 7 EOPNOTSUPP, 11 EAGAIN, 13 ENODATA,
  17 EACCES, 1 other. Init: NULL only; the reason is a PmLog line in context `capture` (`/var/log/messages`), which
  the probe prints after a failure.
- Control ids on `/dev/video60`: START 0x982a14, PRESET 0x982a15 (value 0..3), DATA 0x982a16 (size N*0x604),
  STOP 0x982a17, through `VIDIOC_S/G_EXT_CTRLS`. The header's own text says the driver captures SVP and DRM video
  inside the secure world and returns histogram data from it, which is what makes the API work under Netflix.
- Hue Sync's own sequence: `init(cb)`, `setPreset(h, mode)` with mode 0 unless it is 1..3, then a poll thread that
  calls `get` and hands the 16 areas to its Hue stream; `finalize` on stop.
- `cb` is called from the library's thread; Hue's callback logs and returns false. The library does not use the
  return value.

## Permission gate in initHistogram

`vtCapture_initHistogram` refuses the call (returns NULL, "your process has no permissions" in PmLog) unless one of
these holds, read from `/proc/<pid>/cmdline` and the file system:

- the command line contains `/com.lighting.huesync.service/hue_sync_service`, or
- the file `/var/luna/preferences/vt_devmode` exists ("dev mode on" is logged and the check passes).

The second is LG's own development switch, the first is what LG's Hue Sync service satisfies. Naming a process after
Hue Sync would pass the check by impersonation and is not used. Creating the flag file writes outside the set's
approved write list and relaxes the gate for every user of the library, so it needs Main's decision; the probe
reports whether it already exists.

Readers of the string `vt_devmode` on the set (read-only search of `/usr/bin`, `/usr/sbin`, `/usr/lib`, `/usr/palm`,
05:18 EDT; `/etc`, `/lib` and the other trees were not searched): `/usr/lib/libvtcapture.so.1(.0.0)`,
`/usr/bin/vtCaptureTestSuite`, `/usr/sbin/captureservice`. In `captureservice` the path is next to the service name
`com.webos.service.capture` and the source file name `captureService.cpp`; the code that reads it was not traced
(its caller-permission use is an inference). No script, app or JS file mentions it.

Other facts from the same code: `initHistogram` also fails with code 11 when the resource manager's `acquire` is
refused, which is the "second `vtCapture_init` fails with 11" seen before (one VT consumer at a time).

## Helper-side design (`ambient/`, crate `lumatv-ambient`)

Gate: `ambient/build.sh check` (rustfmt, clippy `-D warnings`, 87 unit tests, 11 interop tests, aarch64-musl release
build of the library). Pure std plus RustCrypto (`aes-gcm`, `hmac`, `sha2`) and `getrandom`; `#![forbid(unsafe_code)]`
like the other helper crates. No traffic to any real device was sent; the Hue and WLED sides are tested against
loopback fakes, and the DTLS client against OpenSSL's `s_server`.

Process split (unchanged from PLAN): the 32-bit capture helper polls LG's library and sends reduced frames
(`wire`: 204 bytes per poll, 6 KB/s at 30 Hz) over a local socket; the 64-bit helper owns everything below.

```text
 capture thread                              output thread (30..60 Hz)
 wire frame -> ZoneFrame -> push_frame ->  bars ------------------------------+
                                           delay line --tick--> mapping (per preset matrix)
                                              -> dark gate -> shape (sat, gain, white balance, gamma) -> smoother
                                              -> hue_channels() -> huestream::Session::send  (DTLS, UDP 2100)
                                              -> wled_leds()    -> ddp::Sender::send         (UDP 4048)
```

The 32-bit capture helper (not built yet; `probe.c` is its core) has this lifecycle:

- Runs only while the screen is on and ambient light is enabled; initialises on a start message from the helper, and
  finalizes on stop, on standby, on a failed poll streak, and when LG's policy callback fires (LG's resource manager
  wants the capture: release at once, retry after 2 s).
- `initHistogram` returning NULL: retry with a 5 s back-off and tell the helper why (the PmLog `capture` lines say
  whether it is the permission gate, a busy scaler, or no driver). While LG's own Hue Sync service or LG's capture
  service holds the scaler, this cannot run; that is a state to show in Settings, not an error to fight.
- Poll 20 to 30 times a second, `setPresetHistogram` whenever the helper's control message names a new preset
  (`Engine::wanted_preset`), reduce each poll to a `wire` frame, send it. A return of 11 (EAGAIN) is "nothing new".
- Cost per poll: one ioctl, a 24 KB copy, 12 K multiply-adds for the means: the probe's CPU line is the number that
  confirms it stays under the plan's 1 %.

HyperHDR features and where they are:

- Zone to light mapping (`layout`): lights are points around the picture; area weight = `exp(-(d/spread)^2) *
  area^-locality`, `d` the distance to the area's rectangle; one normalised matrix per hardware preset, built at
  start (`spread` 0.18, `locality` 0.6). A strip is `layout::strip(start, end, n, first_led)`.
- Time smoothing (`smooth`): HyperHDR's linear smoothing (a run to each new target over `settle_ms`, 180 ms; a new
  target continues from where the output is) driven by timestamps, so the result does not depend on frame or tick rate;
  plus its anti-flicker (`threshold` 0.012, `step` 0.004, `timeout_ms` 1500).
- Brightness and saturation gain, per-light white balance, gamma (`tuning`): saturation in code space keeping luma,
  gain, global then per-light white balance, output gamma last (1.0 for Hue, about 2.2 for a bare strip unless
  WLED's own gamma is on).
- Dark threshold (`tuning::DarkGate`): when every area's largest channel stays under `threshold * 0.8` (threshold
  0.06) for 600 ms the lights hold `floor` (0.02) of the tint of the last bright scene and ignore the noise of a black
  picture; they reopen at once above `threshold * 1.4`. A hold, not off: a Hue lamp at about 2 % is steady, at 0 it
  flickers or drops out.
- Black bars (`bars` + hardware presets): letterbox and pillarbox are seen through three fixed areas (14 top strip,
  15 left strip, 1 bottom fifth) and answered by switching the hardware preset (1 or 2), not by cropping. Entered
  after 1.5 s of evidence, left after 0.8 s, black frames decide nothing. Games mode (preset 3) is a setting.
- Output delay (`delay`): captured frames wait `delay_ms` (0..2000) before they reach the mapping; the
  `engine::tick` clock is the caller's.
- Off on Home and menus: capture stops there, no frames arrive, and after `stale_ms` (1.5 s) the engine fades out;
  `set_enabled(false)` does the same at once.

Outputs:

- `huestream`: HueStream v2 packets (16-byte header, config UUID, 7 bytes per channel, up to 20 channels), RGB or xy
  colour space; `Session` = `dtls::PskClient` + sequence counter. REST start/stop requests are builders only
  (`start_request`, `stop_request`); the helper sends them over HTTPS with the application key.
- `dtls`: DTLS 1.2, `TLS_PSK_WITH_AES_128_GCM_SHA256` only, with the cookie round, fragment reassembly, flight
  retransmission (500 ms, doubling, 6 s total), early encrypted records held for the ChangeCipherSpec, close_notify
  both ways. Interop with OpenSSL `s_server` passes plain and with `-listen` (cookie), wrong key and identity are
  covered; known-answer tests for the PRF, master secret, key block, Finished and the GCM record come from Python's
  hashlib and `cryptography`.
- `ddp`: WLED DDP packets (flags 0x40 or 0x41 with push on the last, sequence 1..15, type 0x0B RGB or 0x1B RGBW,
  1440-byte chunks on whole LEDs); send every output tick, WLED leaves realtime mode about 2.5 s after the last packet.

Not exercised, because it needs the real things: a Hue bridge or Bifrost (Bifrost's entertainment support is the part
of the plan still to confirm; the handshake is what OpenSSL accepts, which is what Bifrost uses), a real WLED strip,
audio-driven effects. Constants for these (`spread`, `settle_ms`, dark `floor`) are first guesses to tune with
`examples/replay.rs` on probe dumps and then on the lights.

## Audio results (2026-10-25, 05:19 to 05:28 EDT)

Source played: a 12 s stereo sine file (440 Hz left, 660 Hz right, 8000/32768 = 0.244 of full scale) through the
default ALSA device (pulseaudio sink `pdefaultapp`), receiver muted through Home Assistant, TV on Home.

- `/dev/snd/pcmC1D*c` are the capture devices of card 1 (`LGSIC`): D0 (WoV PDM mic) offers S24_LE only, D1
  (SpeakerFeedback) S32_LE only; D2 (MixerCapture), D3 (SoundEngineCapture) and D4 (BluetoothCapture) offer S16_LE and
  S32_LE, 2 or 8 channels, 48 kHz only, periods 64 to 512 frames.
- `hw:1,2` (MixerCapture, S16_LE, 48 kHz, 2 ch): works. During the tone: peak 0.2441, rms 0.1221 over 5 s, so the mix
  arrives at its source level, before the TV volume (TV volume reads 0 with output `external_arc`) and before the
  receiver's mute. In standby, with nothing playing, and with the muted Netflix error screen, it records exact
  silence (peak 0.0000). Opening it logs `mix_cap ... free mc done` on close only.
- `hw:1,3` (SoundEngineCapture): opens, but every read stalls until the 10 s `timeout` kills it ("Input/output
  error"), at S16_LE or S32_LE, 2 or 8 channels, in standby, during Netflix and during the tone. The kernel logs
  `aud_bm ERR bm_get_block_tail: block_info is NULL` and `auimc ERR ausrimc_post_imc: no such processor number` when
  it closes: that stream has no source block wired. `aud_dsp_reset_count` stayed 0.
- Conclusion: `hw:1,2` is the audio source for sound-reactive effects (mono mix of both channels at line level, no
  volume dependence); `hw:1,3` is not usable.
- Nothing in the playback path changed: arecord only reads, and no mixer or routing was touched.

Netflix did not play. The 4K title (`80141336`, deep link and ENTER as in HANDOFF) opened its detail page and the
first Play, made before any capture, ended in Netflix error `tvq-pb-101 (5.2.8)`; the log shows `audio codec not
supported` on the `audio-app-es` stream (`GST_STREAM_ERROR`, `netflix.player PF_EVENT_TYPE_INT_ERROR 18`) with sound
output `external_arc` to the AVR-S760H, and a second Play after Netflix's own "Restart" failed the same way. Netflix
was closed afterwards. Open: whether this is the receiver's ARC audio format list (Dolby Atmos or DD+ stream
refused), or something that changed since the 24 Oct Dolby Vision pass; an audio-output/ARC check is the first thing
to do before the next playback test. No Netflix picture was therefore captured.

## Probe slot (2026-10-25 12:35 EDT): the flag passes, the bus registration does not

`python3 ambient/capture/tv_probe_slot.py --go` from a clean worktree at `c91148d`, probe built there by
`ambient/capture/build.sh` (byte-identical to the committed build's output). Files in
`docs/research/ambient/probe-2026-10-25-1234/` (`slot-log.json`, `slot.sh`, the three `.txt` reports, launch and
foreground replies, interrupt snapshots; the `.csv` and `.bin` are empty). Preflight: library md5 `35d3fec6...` as
before, nobody held `/dev/video60`, the flag absent, failed units as found (`broadcast-configd`, `f2io-on`,
`faultmanager`, `gibbs`, `nudge`, `service-logger`, `voiceinput`; none new during the slot), Picture Mode `filmMaker`.
Receiver muted at 12:35:09 (was on, unmuted, 0.355) and unmuted at 12:35:59.

Every capture ended the same way within a second of the flag's creation: the probe prints `dev flag ... present`,
LG's library logs `webos_vt_histogram.cpp.vtCapture_initHistogram():36 dev mode on!!` (the gate passed, 12:35:14.284
for Home), 1.6 ms later ls-hubd logs `LSHUB_NO_ROLE_FILE {"EXE":"/tmp/ambient-slot-202610251234/ambient-probe"} No
role file for executable`, and the library throws `std::runtime_error: LSRegisterPubPriv FAILED`, which ends the
process (`terminate called`, exit 134). The foreground was `com.webos.app.home`, `com.webos.app.hdmi2` and `netflix`
at the three starts; Netflix played the 4K title this time (Picture Mode `dolbyHdrCinema` at 12:36:00, back to
`filmMaker` on Home). So the histogram API needs two things of its caller: the Hue Sync command line or the flag
(known), and a luna-service2 role for the executable, because after the gate the library registers a bus client
(its strings: `com.webos.service.capture.client`, `com.webos.service.capture`, `com.webos.service.capturepermission`,
`com.palm.bus/signal`, "rmc regist pipeline fail": the resource manager and the capture service's permission check
are reached over the bus). A role is what `appInstallService/dev/install` writes for a service declared in an ipk, so
the capture helper should be a native service of LumaTV's package; the probe, built as a bare binary under `/tmp`,
can never pass this step. `/usr/sbin/captureservice` reacted to nothing (its PERMISSION lines at 12:35:15 are its
own pipeline poll).

The flag: created by the on-set script straight before each probe run and removed by its `finish` straight after the
abort (three windows of about a second each at 12:35:14, 12:35:23 and 12:35:52, from the launch times and settles in
`slot-log.json`; the per-window times in `flag-events.txt` were lost because busybox `date +%s.%N` prints a literal
`%N` on the set, so the script's own final check raised before it could record them and printed "COULD NOT
CONFIRM"). The file was confirmed absent by hand at 12:36:13 EDT (`ls: No such file or directory`) and again at
12:50. Nothing was sent to any Hue or WLED device; nothing else on the set was changed; the set was left in Active
Standby by the script.

## Probe slots (2026-10-25 16:21 and 16:27 EDT): the role through a manifest, then 60 s of zones in each scenario

Both from a clean worktree, `python3 ambient/capture/tv_probe_slot.py --go --no-standby`, the reader built there by
`ambient/capture/build.sh` (md5 `cd324c4e`, the committed build's), the package not installed on the set before either
run (`capture.sh install` staged `/var/lib/lumatv/bin/lumatv-capture` and `capture.sh remove` took it away after).
Files in `docs/research/ambient/probe-2026-10-25-1621/` and `probe-2026-10-25-1627/`.

16:21 (`8108c43`, role template of `plugins-slice-result.md`): the flag passed the gate (`dev mode on!!`), then ls-hubd
`LSHUB_NO_ROLE_FILE {"EXE":"/var/lib/lumatv/bin/lumatv-capture"}` and `LSRegisterPubPriv FAILED` for the name
`com.webos.rm.client._fd2ItCC7IvCIO1`, exit 134, in each of the three windows (flag windows of 4-5 s). Two things were
wrong, read on the set: ls-hubd's configuration (`/etc/luna-service2/ls-hubd.conf`) lists `ManifestsVolatileDirectories`
(`/var/luna-service2/manifests.d`, `/var/luna-service2-dev/manifests.d`) and no volatile roles directory, so a file in
`roles.d` is read only when a manifest names it (every dev-installed app has one: `{"id", "version", "roleFiles",
"clientPermissionFiles", ...}`, in the flat `exeName`/`type`/`allowedNames`/`permissions` form); and the first name
LG's library registers is uMediaServer's resource-manager client, `com.webos.rm.client.<session>`, not the capture
client. LG's own role for `/usr/bin/vtCaptureTestSuite` (`/usr/share/luna-service2/roles.d/`) names
`com.webos.service.capture.client*` and `com.webos.rm.client.*` with outbound `com.webos.media`; LG's
`compat.perm.json` already gives `com.webos.rm.client.*` the `private` group and `com.webos.service.capture.perm.json`
gives the capture client `all`, so no client-permissions file is needed. `4888500`: the template names those two,
`capture.sh` writes `manifests.d/org.nphil.lumatv.capture.json` beside the role and removes both.

16:27 (`4888500`): every window captured for its 60 s. Flag windows (set clock): home 16:27:55-16:28:55, hdmi2
16:29:04-16:30:04, netflix 16:30:35-16:31:35, each removed by the script's own `end`; receiver muted 16:27:50 (was on,
unmuted, 0.355) and unmuted 16:31:44; Picture Mode `filmMaker` before, `dolbyHdrCinema` after (Netflix was still in
front: see the problem below). The reader: `capturing: preset 0, 16 areas, 30 polls a second (init 32 ms)`, `finalize
rc=0`; LG's library logged the resource manager registered, acquired and released each time
(`resourceManagerClient.cpp`, one `release(): return error ('m_resources.size()')` warning at every finalize, harmless).

| Scenario | Frames in 60 s | Zone changes a second | Interval p50 / p90 / p99 / max (ms) | Reader CPU (of a core) | Mean R G B of the 16 areas | Black fraction |
|---|---:|---:|---|---:|---|---:|
| Home (LumaHome's wallpaper) | 1799 | 7.9 | 33.1 / 34.1 / 35.3 / 39.5 | 5.55 % | 58 61 81 | 0.062 |
| HDMI 2 (the receiver's menu, blue) | 1799 | 7.9 | 33.1 / 34.0 / 34.8 / 37.2 | 5.17 % | 40 70 100 | 0.023 |
| Netflix (Meridian at its 15 % loading screen) | 1799 | 7.5 | 33.2 / 34.0 / 35.3 / 38.7 | 5.23 % | 14 14 13 | 0.813 |

The areas' pixel counts are fixed per preset (7200, 2304, 1728, 1584, 1504, 1440, 1380, 864, 720, 704 for preset 0),
so the histogram is a subsample, not a full count. 30 frames a second is the reader's poll rate; the zones changed
7.5-7.9 times a second in every scenario, Home's slow wallpaper included, so about 8 Hz is the hardware's update rate
and 10 polls a second would lose nothing (reader default since this note). The Netflix window read non-zero colour from protected video (area 3: 41 35 28,
luma up to 88), the point of the kernel's secure-world histogram.

Problem found in the room: the slot unmuted the receiver at 16:31:44 while Netflix was still playing (`--no-standby`
leaves the set on, and Home was launched only after the unmute), so the title was heard; Nitin stopped the session at
16:31. Fixed in the script: the scenario app is closed and Home launched before the receiver is restored.

### Reader CPU at 10 polls a second (offline; not verified on the set)

Measured at 30/s: 5.17-5.55 % of a core, i.e. 1.7-1.85 ms of CPU per poll, if the cost is all per poll. On the host
(fake library) a poll's own work in the reader is 0.022 ms reducing 16 areas x 3 x 256 bins plus 0.008 ms sending, about
0.03 ms (a few times that on the ARM core), so nearly all of the 1.8 ms is inside `getHistogram` (library and kernel).
Model: CPU % = hz x per-poll ms / 10 + fixed. At 10/s that predicts **about 1.8 %, still over 1 %**; 1 % needs under
1 ms a poll, or about 5 polls a second (0.9 %). The hardware changes the zones about 8 times a second, so 5/s samples
every second change at best. The slot's service log now ends each capture with a `cost:` line (process CPU, % of a
core, ms a poll, and CPU per step: `getHistogram`, reduce, send; wall time of `getHistogram`) so the next slot shows
where the 1.8 ms goes. If it is over 1 % at 10/s: rerun with `--hz 5` (the two points give the model's slope and fixed
part), and decide between 5/s and accepting a number between.

### Hue streaming: Nitin's steps (a key action, so not done by an AI)

The helper cannot start an entertainment stream by itself: the bridge needs an application key with a client key, an
entertainment area, and an HTTPS call to start it. Steps for Nitin:

1. Press the bridge's link button, then create an application key that also returns a client key: `POST
   https://<bridge>/api` with `{"devicetype":"lumatv#tv","generateclientkey":true}`; keep `username` (the application
   key) and `clientkey`.
2. In the Hue app create an Entertainment area with the lights wanted, placed where they stand; note its id (`GET
   https://<bridge>/clip/v2/resource/entertainment_configuration`, header `hue-application-key`).
3. Give the helper the bridge address, application key, client key, application id (`GET /auth/v1`, response header
   `hue-application-id`) and the area id in its config; the helper will start the stream with `PUT
   .../entertainment_configuration/<id>` (`{"action":"start"}`) over HTTPS, then DTLS on UDP 2100.
4. Bifrost instead of a bridge: the area must exist in Bifrost; the client key is its fixed `BifrostHueTlsKey`.

None of the config keys in step 3 exist yet; adding them is the next code step once Nitin has the key action done.

## Open

- Latency of the histogram against the picture (a flash on the screen to the zone change) and the reader at 10 polls a
  second: the next slot, with the helper side on the socket.
- Netflix playback error of 05:19: the title played at 12:35 (`dolbyHdrCinema`) and failed at 12:23 under LumaTV's
  compositor with a video `INT_ERROR 18` (compositor-c2-result.md, third gate); not reproducible at will. At 16:30 the
  title stayed at 15 % of its loading for the whole window.
- Home Assistant showed the receiver `off` while the TV was in Active Standby, and `on`, unmuted, 0.355 once the screen
  was on.
