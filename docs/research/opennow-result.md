# OpenNOW on the C4: decoder limits, decode path, plan

Research only; nothing built, nothing run on the set. "After LumaTV" item in PLAN.md.

## Resume here

Done (2026-10-25, offline): this note. Evidence is the repo's hardware notes, public sources (URLs inline) and OpenNOW's own docs and source.
Open: Phase 0 below (offline test streams and the 32-bit NDL feeder), then one TV slot for the decode probe, which needs Nitin's approval to install a probe app. No TV step has run. Nothing here is proof that a mode works on the set until Phase 1 records it.

Evidence tags: `TV:` measured on this set, `SRC:` LG's published GPL tree, `PUB:` public source (URL), `[INFERENCE]` needs the probe named next to it.

## Verdict

Feasible for the headline mode, with three of the five wishes lost by hardware, not by effort.

| Wish | Verdict | Why |
|---|---|---|
| 4K, 120 fps | **Yes, one probe from certain**: HEVC Main10 through NDL DirectMedia onto the video plane | Moonlight forks run 4K120 HDR HEVC on C1-C5 over NDL; NVIDIA's own LG app streams 4K120 HDR10 on 2024+ OLEDs. Not yet measured on this set. |
| HDR | **Yes, HDR10 (PQ, BT.2020)** via `NDL_DirectVideoSetHDRInfo` | GFN sends HDR10, not Dolby Vision. |
| 4:4:4 chroma | **No** | LG lists HEVC Main/Main10 only; the kernel decoder's output maps are 4:2:0. OpenNOW already falls back to 4:2:0 at preflight, so the stream still starts. |
| VRR / G-Sync compatible | **No for decoded video** [INFERENCE] | VRR is an HDMI-input feature; on a decoded plane the panel follows the content's fixed rate. |
| ALLM | **Not by itself** [INFERENCE] | Signalled by HDMI sources; NDL's load payload carries an `isGameMode` flag whose effect is unmeasured. |

AV1 is the codec to avoid at 4K120 on this path: LG's capability file caps it at 4K60 and Aurora measured 8-12 ms decode against about 1 ms for HEVC. HEVC is also what OpenNOW would negotiate anyway for HDR, and H.264 is capped by OpenNOW itself below 4,953,600 pixels.

If true 4:4:4 or VRR matters, the C4's HDMI inputs already do both from a PC (see the last section); that is a different product from a port.

## What the C4's decoder accepts

| Item | Finding | Source |
|---|---|---|
| Public spec, webOS TV 24 | HEVC 3840x2160@60 Main/Main10 L5.1, 60 Mbps; AV1 4K@60, 50 Mbps; H.264 4K@30 (some models 60); VP9 4K@60 | PUB https://webostv.developer.lge.com/develop/specifications/video-audio-240 |
| Device capability file (another webOS 10 set, `o22n2`) | H.265 4096x2304 **120 fps**, 50 Mbit/s; H.264 60; VP9 60; AV1 60, 60 Mbit/s; VVC 60. H.265 at 120 is the only 120 in the file | PUB https://github.com/HookedBehemoth/jellyfin-webos-native/blob/master/docs/codecs.md and `device-codec-capability.json` beside it |
| Hardware | `/dev/video21 lgsoc-vdec-decoder`, `video22 lgsoc-vdec-sink`, `/dev/lg/vdec0-7`; H.264, HEVC, VP8, VP9, AV1, MPEG-2/4; outputs NV12/LFBC; decoder output maps are tiled HEVC 8/10-bit and AV1 FBC 8/10-bit, none for 4:4:4. The display side (`vo.c` `VO_SET_COLOR_SUBSAMPLE`) does accept a 4:4:4 frame format, so only the decoder lacks a map [INFERENCE: not proof it refuses the stream] | TV (census, `hardware-census.md`), SRC `kdriver/core/vdec/include/vdec_kapi.h`, `vo.c` |
| Decode memory | Frames sit in 137 MB of secure memory on the display engine's windows (54 + 83 MB regions); CmaFree reads 4-9 MB while 4K Dolby Vision plays, UI buffers allocated beforehand held | TV (PLAN, Rendering stack) |
| AV1 hardware decode on this set | YouTube's player loads `"video":"AV1"` through uMediaServer `BUFFERSTREAM`, `adaptiveStreaming` 4096x2304 at **60** fps | TV (`research/arch/x4/all.jsonl`, line 9587) |
| 4K Dolby Vision | Netflix 3840x2160 at 59.94 Hz plays; the panel stayed at 60 Hz | TV (PLAN; `compositor-c2-result.md`) |
| Level | 4K120 is 995,328,000 luma samples/s; HEVC level 5.1 allows 534,773,760 and 5.2 allows 1,069,547,520, so the stream needs **level 5.2**. webOS declares 5.1 and accepts 5.2 in practice; some models return no frames and no error above their declared level. OpenNOW's ANNOUNCE says `maxCodecLevel:51` | PUB https://www.webosbrew.org/develop/guides/media-basics/ ; OpenNOW `nvst_rtsp.rs` |
| Bitrate | Capability file 50 Mbit/s; Moonlight FAQ 65 max, trouble over 45; SS4S allows 95 on high-end SoCs; Aurora ships 80 default, 300 selectable (tested on a C1). NVIDIA streams AV1 to LG TVs "up to 100 Mbps"; OpenNOW's official-client reference session used 100 Mbps | PUB https://github.com/mariotaku/moonlight-tv/wiki/FAQs ; SS4S `ndl_video.c` ; https://newreleases.io/project/github/GuiDev1994/aurora-tv/release/v1.0.0 ; https://www.flatpanelshd.com/news.php?id=1755583202&subaction=showfull |
| Decode time | Aurora: HEVC with `maxNumReferenceFrames=1` cut 4K120 HDR decode from about 13 ms to about 8 ms; 1 slice per frame; 4 MB buffers so large IDRs do not hit BufferFull | PUB Aurora v1.0 notes (link above) |
| Official app | LG lists OLED65C4PUA among 2024 models with 4K HDR GeForce NOW and "120Hz on select OLED models"; NVIDIA says the Blackwell upgrade brings 4K120 HDR to LG OLEDs; a G3 owner reports 4K 120 fps HDR10 10-bit. The set already has `com.geforcenow.play` 1.0.18 (a web app). Whether the C4 is on NVIDIA's 4K120 list was not readable (support page returned 403) | PUB https://www.lg.com/us/support/help-library/compatible-lg-tvs-with-nvidia-geforce-now-beta-CT10000020-20152438861512 ; https://www.techradar.com/televisions/lg-tvs-are-the-first-to-get-4k-120hz-nvidia-geforce-now-game-streaming-anyone-want-rtx-5080-power-without-the-cost ; https://www.reddit.com/r/GeForceNOW/comments/1o05225/lg_tv_120fps |
| GFN side | HDR10 and 10-bit need Ultimate/Performance; YUV 4:4:4 is not offered with AV1 | PUB https://nvidia.custhelp.com/app/answers/detail/a_id/5390 (search excerpt) |

## Panel, HDR, VRR

- HDMI inputs: four HDMI 2.1 ports at 48 Gbps, 4K/120 and 4K/144, VRR 40-144 Hz, ALLM, G-Sync Compatible, FreeSync. LG: "144Hz of C4 only works with games or PC inputs". PUB https://www.lg.com/uk/tvs-soundbars/oled-evo/oled48c45la ; https://www.whathifi.com/reviews/lg-c4.
- The panel option list has a 144 Hz entry (`ovi/status` `panelFramerate 3`) beside a 60 Hz CRTC. TV: `hardware-census.md`.
- Decoded video is not on that path. The UI plane takes its refresh from the video pipeline: a browser page put the output at 50 Hz, Netflix and HDMI 2 held 60 Hz (TV: `perf-tweaks.md`, `compositor-c2-result.md`). So a 120 fps stream should lock the output near 120 Hz, and no variable-rate lever is seen anywhere in NDL's API [INFERENCE: read `de/status` and flip timing during a 120 fps clip].
- OpenNOW's request bag has `cloudGsync`; it must stay false here. A fixed 120 fps stream on a 120 Hz output needs no VRR.
- HDR: `NDL_DirectVideoSetHDRInfo` takes mastering primaries, white point, min/max luminance, MaxCLL/MaxFALL and transfer/primaries/matrix (SS4S `SetHDRInfo`). OpenNOW already holds those fields (display data in the session request, decoded colour metadata). The TV's picture-mode switch on HDR10 under the own compositor was seen only for Netflix (`dolbyHdrCinema`); GFN's mode is unmeasured.
- HEVC level, 4:2:0 and 10-bit all map to one NDL load: `NDL_VIDEO_TYPE_H265` (enum H264=1, H265=2, VP9=3, AV1=4); there is no field for bit depth or chroma, so a 4:4:4 stream can only be tried, not declared (PUB https://github.com/webosbrew/webos-userland, `NDL_directmedia_types.h`).

## Which media API to use

Order to try; all are LG's, none gives the app the decoded frame (every webOS SS4S backend reports output-direct).

| # | Path | Use | Notes |
|---|---|---|---|
| 1 | **NDL DirectMedia v2** (`libNDL_directmedia.so.1`, a 9.6 KB stub that dlopens its impl; `NDL_DirectMediaInit/SetWindowId/Load`, `NDL_DirectVideoPlay(buf,size,pts µs)`, `GetRenderBufferLength`, `FlushRenderBuffer`, `SetFrameDropThreshold`, `SetHDRInfo`, `NDL_DirectAudioPlay`) | **First.** Feed H.264/HEVC access units and PCM; decode, HDR and the video plane are LG's | Verified on hardware on a 2025 OLED77C5 (webOS 10.3.1): PUB https://github.com/webosbrew/samples. Moonlight, Aurora and Jellyfin's `ndlplay` use it; `ndlplay` decoded 1920x1080p120 H.265, 960 frames in 7 s, with a translucent GL overlay above the video (PUB jellyfin-webos-native `docs/ndl.md`). The library is in Netflix's jail on this set (`libNDL_directmedia.so.1.0.0`, 9,620 B, `research/arch/app-libs-01.txt`). |
| 2 | **Starfish media pipeline** (`libplayerAPIs`, `StarfishMediaAPIs`, load payload `BUFFERSTREAM`/`DIRECTMEDIA-ES-PLAYER`) | If NDL refuses a mode (SS4S lists H.264, H.265, AV1 for it) | Same pipeline with more knobs (`pauseAtDecodeTime`, PTS, buffer levels). Moonlight's FAQ: NDL performs better; SMP sometimes handles what NDL cannot. Process on the set: `starfish-media-pipeline pipeline_process` under `umediaserver` (TV: `compositor/probe-2026-10-25.txt:117`). |
| 3 | uMediaServer `BUFFERSTREAM` as Chrome and Cobalt use it | Reference only | What the official GFN web app very likely goes through [INFERENCE]; capture its load payload once as the known-good recipe (see Phase 1). |
| 4 | GStreamer `decproxy`/`lxvideodec`/`lxvideosink`, 1.24 on the set | Not first | 131 plugins; `lxvideodec` advertises AV1, H.265 and VVC. No documented entry for a custom app; a way to read caps. |
| 5 | Raw V4L2 (`video21`) | No | LG's nodes are plumbing for the pipeline, root-only, and a raw decode skips the secure memory, HDR metadata and plane routing [INFERENCE]. |
| 6 | `libNDL_vt` (video into a GL texture) | No | The GPU cannot sample a video plane; imported memory costs 5-20x (PLAN, Rendering stack). |

Facts the NDL path imposes (PUB jellyfin-webos-native `docs/ndl.md`, verified there on webOS 10):

- The app must be installed with a luna role keyed to its exact binary path, or `LSRegisterPubPriv` fails and the process aborts; the app id given to `NDL_DirectMediaInit` must match. The repo already solved this for LG's media-resource client in `lumatv-capture` (role plus `manifests.d` file, names `com.webos.rm.client.*`, `com.webos.media.client.*`, written by `payload/capture/capture.sh`; `docs/research/ambient-result.md`, 16:21 and 16:27 runs).
- Window id before `NDL_DirectMediaInit`, from `wl_webos_foreign.export_element(surface, type 0)`, then `set_exported_window(src, dst)`; `NDL_DirectMediaSetAppState(FOREGROUND)` or every frame is dropped silently.
- The ES player has no clock: it presents on arrival. The app paces the feed (Aurora: monotonic PTS plus `pauseAtDecodeTime`). That matches OpenNOW, whose `PresentationClock` ignores GFN's RTP timestamps.
- Feed whole access units, not chunks. Video-size changes tear the pipeline down.
- The libraries are 32-bit; a 64-bit process cannot load them. Same split the repo already uses (32-bit launcher plus 64-bit app, `lumatv-capture`). The transport and engine stay 64-bit; a small 32-bit C feeder takes compressed access units and Opus-decoded PCM over a Unix socket. At 100 Mbps that is 12.5 MB/s.
- No pull side and no decoded-frame callback: flow control is `GetRenderBufferLength` plus buffer events 44/45/46 (BufferLow/Full). There is no corruption feedback, so loss recovery stays on OpenNOW's NACK/invalidate/IDR logic.

The LumaTV compositor already places apps' video: Netflix `setDisplayWindow` 1920x1080 from a 3840x2160 source through `wl_webos_exported`, protocol errors 0 (`compositor-c2-result.md`). Only PlxNative's capture is still outstanding.

## OpenNOW's architecture and what changes

Source: https://github.com/OpenCloudGaming/OpenNOW (MIT), `README.md`, `docs/hdr.md`, `docs/linux-video-backends.md`, `native/opennow-streamer/README.md`, `docs/native-gamepad-input.md`, `docs/core-protocol.md`.

| Layer | What it is | On the C4 |
|---|---|---|
| `opennow-qt/` | Qt 6.8 Quick UI (desktop and console layouts), SDL3 gamepads, Vulkan `StreamVideoItem` | Do not port (below) |
| `native/opennow-core/` | Rust process: accounts, settings, catalog, CloudMatch session, versioned JSON protocol to the UI (`docs/core-protocol.md`, 122 KB); sign-in by device code with QR (`auth.device.start`); tokens in the OS credential store with a memory-only fallback | Reuse as is, run headless. Needs a file store under `/var/lib` (mode 0600) in place of Secret Service |
| `opennow-streamer-{core,transport,protocol}` | RTSPS negotiation, Mjolnir SRTP video socket, ICE/DTLS/SCTP for audio/input, NACK/FEC, Opus, gamepad packets, MKV recording | Reuse. Pure Rust (str0m, rustls, vendored libopus); the README cross-checks aarch64 for the Linux platform crate only (`cargo check`), so the full aarch64 build is Phase 0 work |
| `opennow-streamer-platform-{linux,windows,macos}` | Decoders (Vulkan Video, VA-API, CUDA, V4L2, FFmpeg) that publish **GPU frames** into a one-frame mailbox; Qt samples them. Pi V4L2 and Android MediaCodec backends already exist as precedent for new backends. The shared `opennow-streamer-platform` crate also pulls bundled `sdl2` and `x11-dl` on Linux | **New crate** `…-platform-webos`: submit access units to the feeder; publish no frames. A cargo feature split keeps SDL and X11 out of the webOS build |
| `opennow-streamer-ffi` | C ABI built around GPU-frame tokens for QRhi | Not used; the standalone `opennow-streamer` JSON-lines host is the non-Qt entry point and needs a "direct plane" presenter in place of its SDL window |
| `opennow-streamer-hid` | DS4/DualSense reports, generic HID | Optional later |

What the decode swap loses (all rest on frames the app never sees): screenshots of the stream, GPU frame generation, stream-frame overlays blended into the video (the HUD becomes a transparent plane above it), decode-duration stats. Source-stream recording and replay capture work, because they record the encoded units.

What the swap costs in code: OpenNOW's negotiation is already capability-driven. `hello` reports per-codec `colorQualities`; core checks Auto and manual choices against them before CloudMatch allocates a seat; HDR 4:4:4 requires HEVC and a supported path, "unsupported HDR falls back to SDR before allocation", and chroma is reduced to 4:2:0 with a diagnostic (`docs/hdr.md`). By those docs a webOS backend that reports H.265 8/10-bit 4:2:0 and H.264 8-bit gets the right session with no change to negotiation. One-line knob to evaluate: ANNOUNCE sets `x-nv-video[0].maxNumReferenceFrames:0` and `refPicInvalidation:1`; Aurora's gain came from forcing one reference frame (the GFN server may ignore it).

UI choice, Qt 6 against a LumaTV-native UI over the unchanged core: **LumaTV-native.**

- LG's compositor has no `xdg_wm_base` (PUB jellyfin-webos-native `docs/display.md`, 23 globals listed); Qt 6's Wayland client expects it. LumaTV's own compositor could add it, but nothing else needs it.
- LG's `libmali` has a Vulkan 1.3.260 ICD but no Wayland surface extension and no loader (PUB `docs/vulkan.md`), so OpenNOW's Vulkan presenter has no window system on this set; the Rockchip blob the repo ships is GLES (reports ES 3.2). A Qt 6.8 build would run on GLES [INFERENCE: untested] and be a second UI toolkit beside LumaTV's.
- The 64-bit LumaTV toolkit already runs as a card app at about 60 fps (`cardapp-result.md`, `gfx64-result.md`), and a transparent overlay over an NDL plane is proven by `ndlplay`.
- Cost: screens to write: sign-in (QR from `auth.device.start`), library grid, game detail, settings, in-stream menu. Small against the stream engine, large against all of OpenNOW's Qt.

## Input

- Evidence on webOS: `/dev/input/js0-5` are mode 0664 root:input (world readable); `/dev/input/event0-31` are 0660 root:composit; three `wl_seat`s, remote keys on seat 1; Bluetooth pairing is LG's `bthidmanager` (PUB jellyfin-webos-native `docs/input.md`; SURFACES.md row on pairing prompts, which LumaTV replaces with its own alert).
- Moonlight TV (SDL-webOS, up to four pads): USB works for LG's listed pads (Logitech F310/F510/F710, Xbox 360 wired, PS3); Bluetooth works on webOS 5+ for Xbox One, DualShock 4 and Switch Pro on one tested model, Xbox Series and Stadia not; adapters (Mayflash Magic NS 2, Magic-S Pro 2) are the safe route (PUB https://github.com/mariotaku/moonlight-tv/wiki/Gamepad-Setup).
- This set: a `LiteEMF Hid Vendor` dongle on `hidraw0` and a Sony `054c:09cc` (DS4 v2) on OHCI bus 8 were enumerated (TV: census). Held by nobody: `hidraw0`.
- OpenNOW's side is a 38-byte XInput-style event inside a 54-byte NVST envelope (`docs/native-gamepad-input.md`), built from SDL3 snapshots. The port replaces SDL3 with the Linux joystick or evdev API in the 64-bit engine: map to the XInput layout, 4 slots, hotplug, send neutral state on disconnect. Rumble back to the pad is unverified (SDL-webOS behaviour not documented).
- Which pads enumerate under which node is a probe (Phase 1), not a given. The set's pad keys are also meaningful to LG's key chain (gamepad Menu/Settings/Xbox, SURFACES.md line 343); the in-stream app must own them while in front.

## Risks

| Risk | Weight | Mitigation |
|---|---|---|
| 4K120 HEVC 10-bit at 60-100 Mbps not sustained by this set's pipeline (capability file says 50 Mbit/s) | High, decides the headline | Phase 1 test streams at 50/80/100 Mbps, level 5.2, 1 and 4 reference frames; fallback tiers 4K60, 1440p120 |
| Level above 5.1 refused silently | High | Probe level 5.1 and 5.2 on the same content; GFN's encoder level must then be known [INFERENCE: read from the session's DESCRIBE/SDP] |
| Secure and contiguous memory with the UI card present | Medium | Allocate UI buffers before the video opens (as `cmahold.c`); watch `CmaFree` |
| Output rate follows the stream; the UI plane then runs at the stream's rate | Medium | Overlay redraws damage only; HUD at 60 or 120 chosen from the measured flips |
| NDL ABI differs between firmware releases (it changed between webOS 4 and 5 under one SONAME) | Medium | Gate on firmware fingerprint as the compositor does; the feeder `dlsym`s per symbol |
| App install and role: installing apps, and any sign-in, are Nitin's (Principle 0) | Process | Probe app installs behind `--approved`; Nitin enters the GFN device code himself; the AI never reads the token file |
| GFN terms: a third-party client against NVIDIA's service | Not technical | OpenNOW states it is unaffiliated; Nitin's decision |
| Reverse-engineered transport changes server-side | Medium | Track OpenNOW upstream; keep the port to one new platform crate plus the feeder |
| AV1 unusable at 4K120 | Low | Not required; HEVC is the path |

## Phased plan

**Phase 0, offline (no TV).**
1. Generate probe streams on the workstation: x265 Main10 HDR10 3840x2160 at 120, 60, level 5.1 and 5.2, 50/80/100 Mbps, 1 and 4 reference frames, 1 slice, no B-frames; one 1080p120; x265 Main 4:4:4 10 as the negative test; SVT-AV1 4K120 10-bit.
2. Write the 32-bit C feeder (`lumatv-ndl`): NDL v2 from `webos-userland` headers, SS4S's `ndl_video.c` as the reference; reads Annex-B files or a socket, splits access units, feeds with PTS, logs render-buffer length and events per frame. Role and manifest from `capture.sh`'s template.
3. Read OpenNOW's seams: `opennow-streamer-platform/src/lib.rs` (backend trait), `linux_backend.rs`, `opennow-streamer-ffi` against the standalone host; list what the host needs of SDL.
4. Confirm the aarch64 glibc 2.36 build of core and streamer (`cargo zigbuild`, as `compositor/.cargo/config.toml`).

**Phase 1, one TV slot (needs Nitin's approval to install the probe app).** Read-only first: `/etc/umediaserver/device_codec_capability_config.json` and `/etc/umediaserver/umediaserver_resource_config.txt` from this set, `MRCGetMaxVideoResolution`. Then the probe matrix over NDL with the offline streams: pass/fail, frames shown, BufferFull events, render-buffer length p99, `CmaFree`, flip rate (UI output Hz), SoC temperature, Mali busy. Record the 4:4:4 and AV1 outcomes. Pad enumeration with Nitin's controllers (`/dev/input/js*`, event names). With Nitin signed in himself, once: the official GFN app at 4K120 with its stats, and the media load payload and picture mode captured with the X4 tooling (`research/arch/x4/x4-monitor.sh`) as the known-good recipe.
Gate G1: 4K120 Main10 HDR10 at 80 Mbps for 5 minutes, no BufferFull loop, no black frames, p99 render buffer at most 2 frames. Otherwise choose the highest tier that passes and record it.

**Phase 2, headless stream.** `opennow-streamer-platform-webos` plus the feeder; the standalone host runs one real GFN session (Nitin signs in with the device code); Opus decoded in the engine, PCM through `NDL_DirectAudioPlay`; HDR info from the session. Gate G2: 10 minutes at the G1 tier, input working from a pad.

**Phase 3, UI.** LumaTV card app over the core protocol: sign-in QR, library, launch, in-stream menu; HUD with ping, loss, bitrate, render-buffer depth. Gate: 60 fps UI, back and Home behave as the compositor's rules.

**Phase 4, controllers.** Joystick/evdev to NVST events, hotplug, a pairing path through LumaTV's alert.

**Phase 5, finish.** HLG/HDR10 switching, reconnect and resume, standby and Home-return, ipk packaging beside the card app, update path for the pinned OpenNOW revision.

## If 4:4:4 or VRR is required

Both exist today on the C4's HDMI inputs: a small PC on HDMI running OpenNOW or the official GFN client gets 4:4:4 (RTINGS' C4 review states a setting requirement for chroma 4:4:4 on PC inputs) and G-Sync Compatible VRR (PUB https://www.rtings.com/tv/reviews/lg/c4-oled). The port cannot reach either, because decoded video never touches the HDMI path.

## Not found

Which codec the official app uses for 4K120 on the C4; whether HEVC 4:4:4 is refused or silently mis-decoded by this set's decoder (no public report found); NDL behaviour above level 5.1 on webOS 10.2.1 build 5202; rumble over the TV's USB/Bluetooth stacks.
