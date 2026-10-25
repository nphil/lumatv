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
2. Done on the set (17:46-17:49, "Reader CPU at 10 and 5 polls a second"): the reader's default of 10 polls a second
   costs 1.87 % of a core on Home, 1.77-1.87 ms a poll with nothing fixed, 1.1 ms of it inside `getHistogram` and
   0.6-0.7 ms in the reader's own reduce; 5 polls a second costs 0.88 %. The helper's ambient engine, fed by the
   reader's own host build over the capture socket, drives a fake WLED socket (test
   `the_capture_reader_at_its_default_rate_drives_the_strip_end_to_end`). Hue backlights through Bifrost are wired in the helper
   (offline, against a fake bridge): see "Hue backlights through Bifrost" under "Probe slots" for the design, the read of
   Bifrost's source with its gaps, Nitin's steps (install Bifrost, make the area, `hue-link --pair`) and the TV checklist.
3. Rate decided (Nitin, 2026-10-25 18:05): 10 polls a second, all 7.6 of the hardware's ~8 zone changes a second, for good.
   The reader's reduce is integer-only and NEON (`ambient/capture/reduce*.c`, "Reduce in NEON" under "Probe slots"): 15 200
   instructions a poll against 1 920 000, bit-identical output on 4.3 M made-up and constructed zones; expected cost about
   1.2 % of a core at 10/s (1.1 ms in `getHistogram`, about 0.01 ms in the reduce), to be measured on the set at the next
   slot (checklist in that section). The service is not installed on the set (the slot removed it); `tools/lumatv apply`
   installs the reader as the package does (`capture.sh install`) when it ships.

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

### Reader CPU at 10 and 5 polls a second (2026-10-25 17:46-17:49 EDT)

Two slots from a clean worktree at `f3ed5a6` (the reader of `3045dec`, md5 `02da86dc`), `tv_probe_slot.py --go
--no-standby` and then `--hz 5`, one minute of Home each (LumaHome's wallpaper, the set woken from Active Standby, LG's
compositor); the service staged by `capture.sh install`, its role through the manifest, and removed by the slot after
each run (`capture.sh status` `installed false, role false, running false` before and after); flag windows 17:46:12-17:47:12
and 17:47:45-17:48:45 (set clock), each removed by the script's `end`; receiver muted 17:46:07 and 17:47:40 (was on,
unmuted, 0.355) and unmuted after Home was confirmed in front; Picture Mode `filmMaker` before and after. Files in
`docs/research/ambient/probe-2026-10-25-1746/` (10/s) and `probe-2026-10-25-1747/` (5/s).

| Rate | Frames in 60 s | Zone changes a second | Interval p50 / p90 / p99 / max (ms) | Reader CPU (of a core) | Per poll (ms) | `getHistogram` CPU / wall (ms) | reduce (ms) | send (ms) |
|---|---:|---:|---|---:|---:|---|---:|---:|
| 10/s | 600 | 7.6 | 100.0 / 100.2 / 101.0 / 102.7 | 1.87 % (1.9 % by the collector) | 1.866 | 1.102 / 1.136 | 0.716 | 0.015 |
| 5/s | 300 | 4.9 | 200.0 / 200.5 / 202.6 / 204.8 | 0.88 % (0.92 %) | 1.769 | 1.123 / 1.163 | 0.597 | 0.014 |

The model's two points: slope 0.198 % per poll a second, fixed part about zero (1.87 - 10 x 0.198 = -0.11), so the
cost is all per poll, 1.77-1.87 ms: `getHistogram` 1.1 ms of CPU with no waiting (wall 1.14-1.16 ms: the library and
the kernel's secure-world read), the reader's reduce 0.6-0.7 ms (0.022 ms on the host; the ARM core and a bin walk with
no vector code), send 0.015 ms. Against the offline estimate: the 1.8 ms held, but a third of it is the reader's own
reduce, not the library. At 10/s the zones changed 7.6 times a second (all of the hardware's ~8); at 5/s 4.9 (every
change is a new sample, every second one of the hardware's is missed). 1 % at 10/s is out of the reader's reach alone
(`getHistogram` is 1.1 % at that rate); the reduce is what the reader can shrink (NEON, or sum fewer bins), to about
1.2 % at 10/s. The rate is settled at 10/s (Resume here); the reduce follows.

### Reduce in NEON (2026-10-25, offline; the cost is not yet measured on the set)

The reader's reduce (16 areas x 3 channels x 256 bins to a mean per channel, a black share and a pixel count) was double
precision on the soft-float ARM build: 1.92 M instructions a poll, 0.60-0.72 ms on the A76 (about 2 instructions a cycle at
1.4 GHz). It is now integer-only (`ambient/capture/reduce_core.h`, `reduce.c`, `reduce_neon.c`).

- **Sums.** All 256 bins of every channel are needed (the mean and the count depend on each), so nothing is skipped; the
  saving is the sweep itself. Count, sum of code x bin and the dark count fit `uint32_t` for any `uint16_t` bins
  (n <= 16.8 M, sum <= 4.28 G). NEON: per 8 bins one load, one pairwise add-accumulate (count) and two widening
  multiply-accumulates against the code values (sum); the black count is two vectors and a masked third. The 24.6 KB the
  library has just written is read once, in address order, three contiguous 512-byte channels an area.
- **Rounding.** `q16(sum / n / 255)` is floor((514 sum + n) / (2 n)) in exact arithmetic (65535 / 255 = 257), done with
  one 32-bit divide for the whole part and one for the remainder (hardware `udiv` in the NEON file, 64-bit only past 8 M
  pixels); the black share is the same formula on 255 x dark, the smallest of the three ratios found by cross-multiplication.
  The double-precision formula agrees with it everywhere except where the exact value is half way between two codes (its
  three roundings stay within about 1e-11, any other value is at least 1 / (2 n) >= 3e-8 from a boundary). There it
  disagrees in 4.4 % of cases (583 600 of 13.3 M half-way means with n up to 3 M), so those few values, detected by a zero
  remainder, are computed with the old double formulas. The output is therefore bit-identical to the old reduce, not just
  close.
- **Build.** zig cannot select NEON code for the soft-float target, so `reduce_neon.c` alone is built hard-float for
  `cortex_a15` (ARMv7-VE, NEON, divide) and linked into the soft-float program; nothing passed or returned is a float.
  `reduce.c` calls it only when `getauxval(AT_HWCAP)` reports NEON and divide in both instruction sets; otherwise the
  scalar integer path runs. The service logs which one: `capturing: ... reduce neon`.
- **Identical output** (`ambient/capture/reduce_test.c`, run by `test.sh`): the old code verbatim as reference, on picture-like
  and corner-case histograms (bumps, flat colours, spikes, letterbox bars, every bin random, every bin 65535, empty channels,
  n from 0 to 16.8 M) and on zones built to land exactly half way in the mean and in the black share, every field compared
  for equality. Host (scalar path): 2 000 000 zones and 2 000 000 half-way zones (5.0 M half-way values, 387 342 of them
  where plain rounding differs from the old code), 0 mismatches. `qemu-arm -cpu max` (scalar and NEON paths on the ARM
  build): 180 000 zones and 120 000 half-way zones, 0 mismatches. No dump recorded on the set exists (the three `.bin`
  files of probe-2026-10-25-1234 are empty), so the zones are made up; `reduce_test -f DUMP` replays any `ambient-probe -d`
  dump the same way.
- **Cost.** Executed instructions a poll of 16 areas on the ARM build under `qemu-arm` (`-d exec -singlestep`, polls 3
  minus polls 1): reference 1 923 631, scalar integer 84 690, NEON 15 201. qemu's emulation time is not a cycle count, so
  the time on the A76 is an estimate [INFERENCE]: NEON 15.2 k instructions at 1 to 2.5 a cycle at 1.4 GHz is 4-11 us
  (0.004-0.011 ms), the scalar fallback 0.025-0.06 ms, against the old 0.60-0.72 ms. On the x86-64 host (-O1, a busy
  machine) a poll took 11.3 us with the reference and 4.3 us scalar. A poll then costs about 1.10 (`getHistogram`) + 0.01
  (reduce) + 0.015 (send) + 0.03 (rest of the loop) = 1.16 ms, 1.16 % of a core at 10/s; the target was 1.2 %.

TV check list for the next slot (`tv_probe_slot.py --go --no-standby`, 10/s is the default):

1. The service log's `capturing:` line ends `reduce neon`. `reduce scalar` means the kernel did not report NEON and divide
   (the reader still works, the reduce then costs 0.03-0.06 ms); no crash (SIGILL) and frames in every window.
2. The `cost:` line: reduce 0.01 ms or less (was 0.716), per poll about 1.15-1.2 ms (was 1.866), `getHistogram` unchanged at
   1.10 ms CPU, process CPU about 1.2 % of a core (was 1.87 %). The slot's verdict says `not above` at an expected 1.2 %.
3. The zones behave as before: about 600 frames a minute, 7-8 zone changes a second on Home, colours of the same order as
   the 17:46 run (`home.json`).
4. Optional, exact: `ambient-probe -d dump.bin` on the set (same flag window rules as the slot), then
   `reduce_test -f dump.bin` on the workstation: 0 mismatches on real histograms.

### Hue backlights through Bifrost (wired offline; not verified on the TV)

Nitin has no Hue bridge: the target is Bifrost (a Hue bridge emulator that drives the bulbs through Zigbee2MQTT; PLAN, Surfaces).
A real bridge speaks the same API and works the same way; the helper treats both alike. Built 2026-10-25, all against a fake
bridge on the loopback (HTTPS from python's `ssl`, DTLS-PSK from OpenSSL's `s_server`, `helper/app/tests/hue.rs` and
`tests/support/fake_hue_bridge.py`); no traffic to any bridge, Bifrost or lamp.

```text
 ambient (engine, 30 Hz) --set(demand, frame)--> hue::Output thread --HTTPS--> GET /auth/v1, GET area, PUT area {"action":"start"|"stop"}
                                                                    --DTLS--> UDP 2100, HueStream v2, PSK identity = application id,
                                                                              key = client key
```

- **Modules** (`helper/app/src/`): `hue_store.rs` (the saved link: `<data dir>/ambient/hue.json`, mode 0600, and the pinned
  certificate `hue-cert.pem`), `hue_rest.rs` (HTTPS through the image's `openssl s_client`, the same child `fetch::TlsFetch` and
  Home Assistant use; `fetch` itself only does `GET`/`HEAD` and checks public authorities, so the shared crate is untouched),
  `hue.rs` (the thread that owns the stream), `ambient.rs` (demand, lights, status), `cli.rs` (`hue-link`, `hue-area`,
  `hue-unlink`), `api.rs` (`POST /v1/ambient/refresh`, what the command line calls to make a running helper read the link
  again). In `ambient/`: `huestream::channel_light` (an area's channel position as a light) and `is_uuid` made public.
- **Trust**: Bifrost generates its own certificate (from the MAC in its config), so `hue-link` pins the certificate the bridge
  shows at link time and every later connection must present exactly that one (`-CAfile <pin> -partial_chain`, no host name check:
  the pin is the identity). Another host on the address, or Bifrost with a regenerated `cert.pem` (it regenerates when the file is
  moved away), fails the handshake and is sent nothing, not even a key; `hue-link` again re-pins.
- **Keys**: read from the bridge's reply (`--pair`) or from standard input, kept in a 0600 file, sent only after the pinned
  handshake and on the TLS child's standard input, never on a command line, never in a log, a topic, an API answer or an error
  (`Debug` of the link says `<redacted>`; tests grep every output for both keys). Bifrost has no authentication ("Everybody has
  full access", its implementation status) and its keys are fixed constants, so for Bifrost they protect nothing; they matter
  only for a real bridge.
- **Sequence** (one stream): read the area (channels, positions, status), `PUT start`, open DTLS (handshake up to 6 s), then the
  latest frame at most `fps` times a second and again at least once a second (the bridge ends a stream after about 10 s of
  silence). `fps` is 20 by default, Bifrost's own `streaming_fps` default; `hue-link --fps <1..60>` changes it, to match a
  different `streaming_fps` in Bifrost's `config.yaml`. A stream the bridge ends is noticed within 250 ms, the area is stopped to
  put it right, and it is started again after 5 s (then 10, up to 60; the words are the topic's `problem`, prefixed `Hue:`). An
  area streaming for another application id is left alone (the thread waits and looks again). An area already active under this
  application's own id is the helper's own earlier stream (it was killed before it could stop the area; Bifrost never clears an
  area when a stream ends): it is stopped and started afresh.
- **Stop**: the stream is open exactly while the lights are lit. Screen off, ambient light switched off, Home covering the picture
  in the mode that goes dark there, or the lights faded out: black is sent twice, the DTLS session is closed with its alert, `PUT
  stop` hands the area back. The helper ending does the same (at most 4 s on the stop call). Tests: `the_still_colour_streams_...`
  (real screen watcher, fake `luna-send`), `the_pictures_colours_reach_each_channel...` (picture frames on the capture socket,
  then none), `the_helpers_own_stale_stream_is_stopped_and_started_afresh`, `the_stream_is_paced_to_bifrosts_default...`.
- **Lights**: one engine light per channel of the area, placed by its position: `x` from -1 (left) to 1 (right) is the picture's
  left to right and `z` from -1 (floor) to 1 (ceiling) is bottom to top; the depth `y` does not move it (a first guess, see the
  list for the TV). With no strip configured, Hue alone is enough; the strip and Hue are driven from the same engine.
- **Topic** `ambient`: `hue` (linked with an area), `hueState` (`idle`, `connecting`, `streaming`, `error`), `available` is a
  strip or Hue, and `problem` carries the Hue output's words, or "linked but no area chosen: run `lumatv-helper hue-area`".

#### Against Bifrost's source (read-only, github.com/chrivers/bifrost, master, 2026-10-25)

What matches what the helper speaks:

- `src/routes/api.rs` `post_api`: `POST /api` answers at once with `username` = the fixed application id
  (`01010101-0202-0303-0404-050505050505`) and, when `generateclientkey` is true, `clientkey` = hex of `BifrostHueTlsKey` in upper
  case. No link button, so `hue-link --pair` succeeds on its first try. `src/routes/auth.rs`: `GET /auth/v1` returns that same
  application id in the `hue-application-id` header.
- `src/routes/clip/entertainment_configuration.rs`: `GET .../entertainment_configuration` lists areas with `status`,
  `active_streamer`, `channels[].channel_id` and `channels[].position` (what the helper reads, `crates/hue/src/api/entertainment_config.rs`);
  `PUT .../<id>` with `{"action":"start"}` or `{"action":"stop"}` sets the first light service to streaming or normal mode and
  the area's `status` and `active_streamer` (the fixed application id on start, none on stop). Entertainment Zones are GET, POST
  and PUT in its implementation status.
- `src/server/entertainment.rs`: DTLS server on UDP `entm_port` (2100) from OpenSSL's `dtls_server` with a PSK callback that
  ignores the identity and always answers `BifrostHueTlsKey` (16 bytes), so any identity works and the client key from the
  registration is the right key. 5 s for the handshake, 10 s read timeout, datagrams read into a 1024-byte buffer (the largest
  packet the helper sends is 192 bytes). The first frame of a session is parsed only to find the area and is not forwarded (the
  helper streams continuously, so one frame of 50 ms is lost). The area UUID in the packet must exist and must not change
  mid-stream; the colour mode must not change (the helper sends RGB only).
- `crates/hue/src/stream.rs`: HueStream v1 and v2; the v2 header is 16 bytes (`HueStream`, version, sequence, colour mode),
  then the 36-byte ASCII area UUID, then 7 bytes per channel (id, three big-endian u16), which is what `huestream::encode`
  writes (byte offsets agree; `huestream::parse` is the same layout).
- `config-reference.md`: `streaming_fps` (default 20) is the most updates a second Bifrost forwards to Zigbee; it is a z2m
  server setting that no API shows a client, hence the helper's own `fps` (default 20). Faster input is dropped and the smoothing
  assumes the rate it sees, so equal is best.

Gaps and risks found (none changes the code beyond what is already in; each is for the TV slot):

1. **Bifrost never clears an area itself.** When a stream ends (client close, 10 s of silence, an error) `EntertainmentService::run`
   sends `EntertainmentStop` to the backend but leaves the area `status: active` with its `active_streamer` until a `PUT stop`.
   The helper handles it: its own stale stream is stopped and started again (test above).
2. **Black is not off.** `backend/z2m/entertainment.rs` `generate_frame` clamps brightness to at least 1 of 2047, so a black
   frame leaves a Hue lamp on at its dimmest, not off. The engine's dark gate already holds about 2 % in the dark (PLAN), so
   this is a floor, not a regression; what the lamp does after `PUT stop` (Bifrost sets the light's mode to normal and sends the
   Zigbee stream reset) is for the TV to show.
3. **Hue lamps only.** The frames are Philips' Zigbee entertainment records (`hue::zigbee::EntertainmentZigbeeStream`, vendor
   code 0x100b). Bifrost lists an Entertainment service on every light ("FIXME: This should be feature-detected, not always
   enabled", `bridge_import.rs`), so an area with a non-Hue lamp (IKEA and others) is accepted and does nothing on that channel.
   The area should hold the Hue backlights only.
4. **Areas have to be made through the API.** Bifrost's `config.yaml` has no entertainment section (`config-reference.md`). Areas
   are created by `POST /clip/v2/resource/entertainment_configuration` (the Hue app does this when pointed at Bifrost, which it
   finds like a real bridge). Channel positions come from the request for ordinary lamps; for a gradient light each segment gets
   one of seven fixed positions (`POSITIONS`, "hard-coded values fitting for an LCX005 gradient light chain").
5. **`PUT start` has no conflict check** (a second start of an active area succeeds) and indexes `light_services[0]` without a
   check, so an area without a light service would panic that request; areas made from Bifrost's lights always have one
   (`renderer_reference` is set).
6. **One stream at a time**: the DTLS loop serves one session before accepting the next, so a second client (the Hue app's sync,
   LG's own Hue Sync app on the TV) waits or fails while the helper streams.
7. **Not run against Bifrost itself**: only against OpenSSL's server with the same cipher (`PSK-AES128-GCM-SHA256`) and PSK
   callback shape. Bifrost's context is a default `dtls_server` with the PSK callback, so the cipher should be offered; to be seen.

Nitin's steps (key actions and an install on his network, so none is done by an AI). Bifrost first:

1. Install Bifrost on a host with its own LAN address (it answers on ports 80 and 443 like a bridge, so it cannot share the
   address of anything that serves those): the Home Assistant add-on (`github.com/chrivers/bifrost-hassio`), Docker
   (`doc/docker-compose-install.md` in `github.com/chrivers/bifrost`) or from source. Its `config.yaml` needs the interface's MAC,
   the address, netmask, gateway, timezone and the Zigbee2MQTT websocket (`z2m: server1: url: ws://<z2m>:8080/api?token=...`);
   `doc/config-reference.md` lists the rest, including `streaming_fps` (leave the default 20, or pass the same number to
   `hue-link --fps`). Start it and check its log lists the Hue lamps ("Adding light ...").
2. Make the entertainment area in Bifrost: point the Hue app at it (it is found like a bridge; Bifrost has no authentication, so
   there is nothing to press) and make an Entertainment area with the Hue backlights only, placed where they stand and the TV at
   the front. Without the Hue app: `GET https://<bifrost>/clip/v2/resource/entertainment` lists each lamp's entertainment service
   id, and `POST https://<bifrost>/clip/v2/resource/entertainment_configuration` with `{"metadata":{"name":"TV"},
   "configuration_type":"screen","locations":{"service_locations":[{"service":{"rid":"<id>","rtype":"entertainment"},
   "positions":[{"x":-0.8,"y":1.0,"z":0.5}]}, ...]}}` makes one (the body shape is from `EntertainmentConfigurationNew`, not run).
3. On the TV, as root over the Homebrew Channel's ssh, `H=/var/lib/lumatv/bin/lumatv-helper`:
   `$H hue-link --data-dir /var/lib/lumatv --host <bifrost ip> --pair`. No button: Bifrost answers the registration at once. It pins
   the certificate Bifrost shows, makes the two keys, proves the key by using it, saves everything (mode 0600) and prints the
   areas, never a key. If Bifrost has exactly one area it is chosen.
4. With more than one area: `$H hue-area --data-dir /var/lib/lumatv` lists them and `... --area <id>` chooses one.
5. Switch ambient light on in Settings, Ambient light. A running helper is told by the command itself. `$H hue-unlink --data-dir
   /var/lib/lumatv` removes the link, the keys and the pin.
6. Only with a real Hue bridge (Nitin has none): the same commands work with the bridge's address, after pressing its round link
   button within 30 s of `--pair`; without `--pair` the registration's answer is piped in on standard input
   (`curl -k -X POST https://<bridge>/api -d '{"devicetype":"lumatv#tv","generateclientkey":true}' | ssh <tv> $H hue-link
   --data-dir /var/lib/lumatv --host <bridge>`), and the area is made in the Hue app.

Not verified on the TV (next slot, in this order):

- `hue-link` on the set: the set's `openssl` accepts `-partial_chain` and `-verify_return_error` (`check-config` shows the
  program), the pinned certificate can be read by the TLS child (it runs as `fetch.user`, default `nobody`; `<data
  dir>/ambient/` is made 0755 for that; if the data dir above it is closed, set `fetch.user` to `null`).
- Against Bifrost (once installed): the DTLS handshake (cipher and PSK callback as in point 7), `PUT start` answered without
  errors, the first-frame loss, the lamps' state after `PUT stop`, and Bifrost's log line "Incoming entertainment fps: N" equal
  to `fps`.
- The channel placement on the picture (`x`, `z`, depth ignored) against where the lamps really are.
- Helper CPU with the stream open (DTLS AES-GCM at 20 Hz plus the 30 Hz engine) and the screen-off stop within a second.

## Open

- Latency of the histogram against the picture (a flash on the screen to the zone change): the next slot, with the
  helper side on the socket. The reader at 10 and 5 polls a second is measured ("Reader CPU at 10 and 5 polls a second").
- Netflix playback error of 05:19: the title played at 12:35 (`dolbyHdrCinema`) and failed at 12:23 under LumaTV's
  compositor with a video `INT_ERROR 18` (compositor-c2-result.md, third gate); not reproducible at will. At 16:30 the
  title stayed at 15 % of its loading for the whole window.
- Home Assistant showed the receiver `off` while the TV was in Active Standby, and `on`, unmuted, 0.355 once the screen
  was on.
