# Live TV capture: the set's antenna feed into Plex

Nitin asked on 2026-11-05 for the TV's own broadcast feed to reach Plex (or something like it) without switching the TV to it and without new hardware (no HDHomeRun). This note holds the path, what the probes P1a to P3 measured on the set the same day, the risks and what is still unmeasured. The probes ran under the TV lease with Nitin's OK, in Active Standby, with no picture or receiver write.

## Verdict

**Feasible, and measured: a transport stream of one antenna program was read from the set's kernel while the panel was off.** LG's windowless pipeline retunes the frontend, a second demux channel and a PVR download carry the program, and LG's own HAL supplies the certificate. The TV's own Live TV outranks it and takes the tuner at once. The facade is the `lumatuner` engine (`helper/tuner`), served by the helper's Live TV Tuner plugin, which is off by default (`docs/plugins/live-tv-tuner.md`); switched on, it served Plex on 2026-11-05 (below, Plex).

| Gate | Result | Evidence |
|---|---|---|
| 1 Tune without a window | **Passes, with two conditions.** A caller that is not LG's app can open a `SCAN` pipeline, but `utp` frees the pipeline when the opener's connection goes, so the opener must stay connected (`luna-send -i`). `setAtscManualTuning` alone does not move the frontend; `changeChannel` on the pipeline does, and it moves the TV's last channel, which has to be put back. The helper now uses the legacy service beneath `utp`, where neither condition holds (The path, step 2) | `p2-open-plain-luna-send.json`, `p2-changechannel.json`, `p2-hold-and-live-tv.json` |
| 2 A second demux channel | **Passes.** `demux2` took the demod's port with `IN_DEMOD 0, PARALLEL`, the values LG's own channel 0 showed while its pipeline ran; the driver left the port alone, both streamed together, no `EBUSY`, no reset | `p2-changechannel.json`, `p2-hold-and-live-tv.json` |
| 3 PVR download | **Passes.** The certificate built from the seed key of LG's HAL (called from the set's 32-bit python) was accepted by a root process outside LG's DVR service; `dvr2` then read | `p1a-key.txt`, `p1b-tap.txt` |

One tuner means one RF channel at a time. The 57 antenna channels sit on 10 RF channels, so Plex sees `TunerCount 1`, and the TV's own Live TV outranks Plex.

## Results

| Probe | What ran | Result |
|---|---|---|
| P1a key | `HAL_PVR_CRYPTO_GetSecureKey(NULL, buf)` from `libhal_lg115x.so.2`, twice | return 0, 16 bytes written and nothing beyond, not all zero, the same both times and in P1b (SHA-256 prefix `28129764`): a stable device secret |
| P1b tap | `demux2` input, PAT, virtual channel table, all six PMTs, then `dvr2` with the cert and TS taps on the PMT, the clock and picture PID and the audio, 10 s, on the channel locked at the time | the demod's channel was RF 32 (PAT `tsid 0x02e3`; the table lists 2.1, 2.2, 2.3, 17.3, 2.4, 2.5). Program 2.4: 9,303 units in 10.0 s, first data at once, every sync byte in place, no scrambled packet, no transport error, no continuity gap; `ffprobe` reads MPEG-2 video 720x480 and AC-3 stereo, and the video decodes. No `0x09` or `0xAA` descriptor in any of the six PMTs and no access-controlled channel. The demod stayed `LOCK`; the TE clock-gated again on close |
| P2 retune | held `SCAN` pipeline, `changeChannel` to RF 27 (`1_27_5_1_0_0_0`, 5.1 WAGA-HD), the tap held 723 s | the demod locked on RF 27 (strength 66 to 71, quality 36 to 99 over three reads), the PAT and table were RF 27's. 562 MB in 723 s (6 Mb/s on average, windows to 10 Mb/s), no scrambled packet, no transport error, no read error, demod `LOCK` throughout. 1,886 continuity gaps in 2.93 million units (0.06 %), in bursts that coincide with the demod's and the TE's packet-error messages and LG's own video decoder counting macroblock errors: RF 27 reaches the set badly at times; RF 32 had none in its 10 s. The tap's python used 3.8 % of one core. The last channel moved to RF 27's and was put back by a `changeChannel`; the channel list (57, same digest) and the panel (Active Standby) were unchanged |
| P3 contention | `launch com.webos.app.livetv` while the pipeline streamed RF 27, then Live TV changed to RF 32's 2.4, then `closeByAppId` | Live TV came to the front within one poll (2.4 s) **with the panel still in Active Standby**, with its own `tv` and `avconnector` pipelines (level 4). The helper's pipeline left UMS's list and `getBroadcastState` went from `BROADCAST_STATE_RENDERING` to `BROADCAST_STATE_UNREALIZED`, with no error. The tap kept reading, with no error: first RF 27's packets, then RF 32's **on the same PIDs** (WAGA-HD and WSB-HD both use `0x30`, `0x31`, `0x34`, `0x35`), so silence of a PID cannot tell that the tuner moved. On close LG released its filters on `demux0` and the demod went to low power (`UNLOCK`); the tap saw no data for 5.2 s and ended by itself, closing everything. No oops, BUG or call trace in the kernel log |

The end state after the probes: Active Standby, Home in front, no pipeline, last channel `1_32_2_4_0_0_0` (RF 32), `legacy-broadcast-atsc` alive with `frontend0` open, the demod **`UNLOCK`** (it read `LOCK` on RF 32 before the probes; `closeByAppId` on Live TV ended that, and the helper tunes for each stream). Receipts: `live-tv-capture/p1a-key.txt`, `p1b-tap.txt`, `p2-*.json`; the probe is `tap.py` (run on the set with python3: `prefer=<major.minor> require=1 hold=<s> tag=<name>`), the held opener `open-hold.sh`. The 10-second captures stay out of git.

## Evidence key

`TV:` read on this set (OLED65C4PUA, webOS 10.2.1, kernel 5.4.268-329.ptl4tv.2, `tv.hw.SoCChipType O22N2`) on 2026-11-05, receipts in `live-tv-capture/`. `SRC:` LG's O22 GPL tree (`/data/home/lgtv/research/lgsrc/k/soc_GO/linux-and-kdriver_paluma-5/kdriver`, paths relative to it; webOS 25, newer than the set). `BIN:` strings or disassembly of files copied off the set (kept outside git: `libkadaptor.so.2.0.1` sha256 `ae0e91f0`, `ts-record` `47756a93`, `libhal_lg115x.so.2.0.1` `8a95eed0`). `PUB:` public page. `[INFERENCE]` is untested. Ioctl numbers are computed from the headers; P1b and P2 used them on the set.

## What the set has

| Item | Answer | Evidence |
|---|---|---|
| RF tuners | **1**: a Silicon Labs Si2178B front end and the SoC's internal ATSC demod, one `/dev/dvb/adapter0/frontend0`. No second tuner, no ATSC 3.0 | TV: `/proc/lxdvb/demux/status` `NUM_OF_TUNER: 1`; modules `si2178b`, `si2178b_atv`, `dvb_si2178b_chip`, `atsc_dtv_soc`, `demod_khal`, `webos_tv`; configd `tv.hw.supportDualTuner`, `supportTripleTuner`, `support2RF`, `supportATSC3`, `tunerless` all false, `supportInternalDTVDemod` true (`20-config-dvr-tuner.txt`, `11-devices-modules.txt`) |
| Demux channels | 6 SDEC channels: 4 reach `/dev/dvb` as `demux0..3` (= `sdec0c0..c3`, 64 PID and 64 section filters each); the 2 sub channels (one filter each) exist only as `/dev/lg/sdec0c4` and `c5`. 256 PID filters in all. Each `demuxN` pairs with `dvrN`, `caN`, `deliveryN` | SRC: `core/te/sdec/sdec_cfg.c`; TV: `10-nodes.txt` |
| `delivery0..3` | ATSC 3.0, Japan 4K and CI+ 2.0 baseband only: no use for ATSC 1.0 | SRC: `include/linux/dvbv5-ext/dvbv5-ext-delivery.h` |
| PVR | 2 download engines (`pvr0dn0/1`) and 2 upload engines (`pvr0up0/1`); a download writes 192-byte units (a 4-byte big-endian 27 MHz timestamp, then the 188-byte packet) into a ring of 9.99 MB | SRC: `core/te/pvr/pvr_cfg.c`, `core/te/dvb-core/lxdvb_dvr.c`; TV: `p1b-tap.txt` (`/proc/lxdvb/dvr/status`) |
| Demod input | one port, `IN_DEMOD0`, selectable by every channel; LG's channel 0 sets `IN_DEMOD, 0, Parallel` | SRC: `lxdvb_demux.c` `dmx_ext_source_to_input_cfg`; TV: `p2-changechannel.json` (the input rows) |
| Who holds what | at rest `legacy-broadcast-atsc` (pid 2255) holds `frontend0` open read-write and `/dev/video2`; nothing holds a demux, dvr, pvr, sdec or te node. All four demux inputs read `None / DISABLE`, no filter, every PVR channel idle, `TE_HW` interrupts 0 since boot. While its pipeline runs it also holds `demux0` with video and audio filters | TV: `12-holders.txt`, `32-frontend-holder.txt`, `30-lxdvb-demux.txt`, `51-dvr-props.txt`, `14-interrupts.txt`, `p2-changechannel.json` |
| Channels | 57 antenna channels on RF 18, 19, 21, 22, 25, 27, 29, 32, 35, 36; the TV scans RF 2 to 69; last channel `1_32_2_4_0_0_0` (RF 32, 581 MHz, 2.4) | TV: `channels-by-rf.txt`, `63-atsc-scan-info.txt`, `40-last-channel.txt` |
| Load to share with | 3 cores online, 79 % idle; `MemAvailable` 754 MB; `CmaFree` 84 MB; `eth0` 100 Mb/s; `legacy-broadcast-atsc` 12.6 MB resident | TV: `59-baseline.txt` |

## Does the tuner stay locked while the display shows something else

**At rest it did before the probes, not after them.** The demod read `LOCK`, `VSB`, packet errors 0, BER 0 on three reads (08:29, 08:33, 08:35 EST) with the set in Active Standby, Home in front, no media pipeline and the TE idle, on RF 32 (the table P1b read). After P3 force-closed Live TV it read `UNLOCK` in low power; the lock at rest is LG's leftover from earlier use, not something to rely on, so the helper tunes for each stream.

**LG offers no product feature for it on this set.** Live TV's `tv` pipeline belongs to the Live TV window (it claimed `ADTU`, `SDEC`, `VTP` and two `VDEC` in a captured session: `docs/research/multiview/tv-2026-10-30/luna-lgmv-both-playing/ums-pipelines.json`), and "record while watching" is the DVR, which is off (next section).

**LG's middleware does have a windowless tune, and it is a full `tv` pipeline.** The Channel Tuning app (`com.webos.app.channelsetting`) opens `utp.broadcast/open {"options":{"appId":"com.webos.app.channelsetting","windowId":"","purpose":"SCAN"}}`, calls `utp.channel/scan/setAtscManualTuning` and, when the RF channel has channels, `utp.broadcast/changeChannel` to one of them (BIN: `61-chan-app-luna.txt`, the app's `main.js`). Run from a root shell: the pipeline lives only as long as the `luna-send` that opened it (`-i` keeps it); `setAtscManualTuning` answers with the channel id and a lock but leaves the frontend where it was; `changeChannel` retunes it, turns the pipeline `WATCH_NORMAL` and `BROADCAST_STATE_RENDERING`, and UMS lists it as a `tv` pipeline of `com.webos.app.channelsetting` at level 6 that claims the TV's decode chain (`ADTU`, `SDEC`, `VTP`, two `VDEC`, `G1_H264`, `G1_MSVC`). The panel does not wake and the volume status does not change.

## LG's own DVR: no

| Fact | Evidence |
|---|---|
| `tv.rmm.dvrReady` false and `dvr.advancedDvrReady` false, read live; the US region file says the same | TV: `20-config-dvr-tuner.txt`; `/etc/configd/data/rmm/US/USA/tv.rmm.json` (copied in the first census) |
| The region file is what turns it off. Another broadcast country would factory-reset the set and delete every channel; US owners report "DVR Ready" greyed out in the service menu | `settings-coverage.md` (Broadcast Country row); PUB: AVS Forum thread 3281124 |
| A recording needs a source pipeline (`utp.dvr/record/startTimeshiftBuffering {sourcePipelineId, storagePath}`) and goes to `lg_dvr` encrypted: the HAL has `HAL_PVR_CRYPTO_EncryptData`, `GetCipherKeys`, `SetCipherKeys`; `ts-record` calls `DILE_CRYPTO_ReadDVRDeviceSecret` | BIN: `/data/home/lgtv/uiinv/inputcommon/qml/Interfaces/DvrInterfaces/AppServices.js`; `strings-ts-record.txt`; `libhal_lg115x` exports |
| `ts-record` (`com.webos.service.tsrecord`, dynamic, not running) is LG's recorder on the same kernel PVR path with encoder elements; its API is not known and it is not needed | BIN: `strings-ts-record.txt` |

## Routes to the raw stream

| Route | What it gives | Verdict |
|---|---|---|
| R1 PVR download: `demuxN` PID filters with `DMX_OUT_TS_TAP`, read `dvrN` | the program's own transport packets, ready for a TS server | **Used, and measured** (P1b, P2). SRC: `lxdvb_demux.c` `set_dvb_filter_attr`, `lxdvb_dvr.c` `dvr_read` |
| R2 PES taps: `DMX_OUT_TAP` filters read from `demuxN` | PES of each PID, no PVR, no cert | Not needed: the cert passed |
| R3 SDEC input dump (`SDEC_IOC_DUMP_START`, the TE debug dumps) | the whole input TS | No. The MCU grants dumps by "authority" (`te_get_authority`) and the set has no `te`, `lxdvb`, `pvr` or `sdec` entry in debugfs (`34-debugfs-names.txt`) |
| R4 `delivery0..3` | ATSC 3.0, Japan 4K and CI+ 2.0 baseband | No: not ATSC 1.0 |
| R5 the decoded picture (video tap, `/dev/venc`) | pixels | No: needs the display path, which is what Nitin does not want to switch. The encoder route is in `venc-result.md` |
| R6 `pvr0up0/1`, or `dvrN` opened write-only | the upload side: it feeds a stream into a demux channel for playback and has its own cert | No: wrong direction |
| R7 uMediaServer | pipeline types `media`, `advr`, `record`, `tv`, `tv_bg`, `camera`, `vsm` and more; none hands a stream to a caller, and `record` is the DVR's | No: `docs/research/gfn/tv-2026-10-26/umediaserver_resource_config.txt` |

## The path

In the helper (a static 64-bit binary; `luna-send` and the set's 32-bit python are its two helpers):

1. **Key, once per run.** `HAL_PVR_CRYPTO_GetSecureKey(NULL, buf16)` from `libhal_lg115x.so.2` in the set's `python3` (3.10.13, 32-bit, `_ctypes`; the helper is 64-bit and cannot load the library). The key is a device secret: kept in memory, never in git, logs, receipts or a file.
2. **Tune.** `luna://com.webos.service.legacybroadcast.broadcast/open {"options":{"appId":"com.webos.app.channelsetting","windowId":"","purpose":"SCAN"}}` (LG's legacy broadcast service, which `utp`'s drives); `playSignaling {broadcastId, channelId}`; `getSignalStatus` until `signal.locked`. `playSignaling` asks UMS for the tuner and the section decoder alone (`ADTU`, `SDEC`; purpose `WATCH_SECTION_PARSE`) and leaves the last channel as it was. A pipeline already playing takes the next `playSignaling` (96 to 140 ms on the set, the lock 0.29 to 0.31 s after it, 2026-11-11), so the pipeline a stream lets go of plays on for 1 s and the next tune plays its channel on it. The pipeline outlives its opener, so its id is kept in a file in the temp directory and the next tune stops and closes the one a run left open. `utp`'s `changeChannel` asks for the whole watch chain (video and audio decoders): UMS found no candidate while Netflix in 4K held all four `VDEC` (`POLCAND_FIND_ERR`), so with the TV on and a video app in front every tune failed (2026-11-08). Never `saveAtscManualTuning` or `deleteAtscManualTuning`: they edit the channel map.
3. **Tap.** Open a free `demuxN` (not `demux0`, which LG's pipeline uses) read-write and send `DMX_EXT_S_CTL` (`0x40106f35`) with id 7 and `dmx_ext_source {IN_DEMOD 0, port 0, PARALLEL 1}`.
4. **Learn the program.** Section filters (`DMX_SET_FILTER`, `0x403c6f2b`, `CHECK_CRC | ONESHOT | IMMEDIATE_START`; the section number is the fifth filter byte) on `0x1FFB` for the virtual channel table (every section), on PID 0 for the PAT, on the PMT's PID for the PMT. Refuse the program when a PMT descriptor is `0x09` or `0xAA` or the table marks the channel access-controlled.
5. **Open the download.** `/dev/dvb/adapter0/dvrN` read-only (same N), then `PVR_EXT_S_CTL` (`0x40107000`) with id 3, size 32 and the cert `cert[i] = seed[i % 16] XOR table[i]` (table: `include/linux/dvbv5-ext/dvbv5-ext-pvr.h` lines 208-214). Before the cert, `dvr_read` returns `EIO`.
6. **Start the PIDs.** One demux fd per PID: `DMX_SET_PES_FILTER` (`0x40146f2c`) with `DMX_IN_FRONTEND`, `DMX_OUT_TS_TAP`, `DMX_PES_OTHER`, `DMX_IMMEDIATE_START`, for the PMT, the clock PID and every elementary stream; not for PID 0. Read `dvrN` (non-blocking, `poll`): 192-byte units, the sync byte at offset 4. For a channel streamed before, the PIDs its last stream started are started as soon as the tuner locks, alongside the section filters of step 4, and started again if the tables name others.
7. **Serve** the 188-byte packets as `video/mp2t` over HTTP: a PAT the helper writes (one program, ten a second) first, then the packets. One tap, one client; a second tuner request is refused (503).
8. **Let go**, in this order: close the PID fds, the download, set the channel's input to `DMX_EXT_SRC_TYPE_NULL`, `stop`, `close`, and the record; a stream's pipeline is stopped and closed 1 s after the stream ends unless the next tune has taken it. A pipeline LG unrealized (Live TV opened) is closed the same way: `stop` and `close` left Live TV playing (2026-11-05).

Owner first: the helper refuses (503) and ends the stream when `com.webos.app.livetv` is in front or has a pipeline in UMS, or when its own pipeline is no longer `BROADCAST_STATE_RENDERING` (polled every 5 s).

## Plex

Plex's DVR takes an HDHomeRun-style network tuner. It finds one by IP (Settings, Live TV & DVR, "specify the device's location"), scans its lineup, matches the lineup to Plex's own guide by country and postal code, and then records or streams from the lineup's stream URLs. Recording needs a Plex Pass (`PUB: support.plex.tv/articles/225877347-live-tv-dvr`). The facade is the five small endpoints the set tvhProxy and the other emulators serve (`PUB: github.com/PREngineer/TVHProxy`); `lumatuner` serves them:

| Endpoint | Content |
|---|---|
| `/discover.json` | `FriendlyName`, `DeviceID` (from the TV's MAC), `ModelNumber HDTC-2US`, `BaseURL` (the host the client used), `LineupURL`, `TunerCount 1` |
| `/lineup_status.json` | `ScanInProgress 0`, `ScanPossible 1`, `Source "Antenna"` |
| `/lineup.json` | one entry per antenna channel of `iepg/getChannelList` (57 on the set, none skipped or hidden; a repeated number is listed once): `GuideNumber` "2.4", `GuideName`, `URL` |
| `/lineup.m3u` | the lineup as an M3U playlist: `#EXTM3U url-tvg="<base>/xmltv.xml"`, then per channel `#EXTINF:-1 tvg-id="2.4" tvg-chno="2.4" tvg-name="<name>",2.4 <name>` and the `/auto/v2.4` URL |
| `/xmltv.xml` | XMLTV from the TV's own broadcast guide (`iepg/getEventList`, one read-only call per channel for a day back to six days ahead, kept 30 minutes); channel ids are the guide numbers, times UTC; a channel with no listings has none |
| `/auto/v<GuideNumber>` | the stream above from the program's first whole picture, answered once that has come; 404 for no such number, 503 while another stream runs or the TV is in use, 503 with `X-HDHomeRun-Error: 807 No Video Data` for a channel that cannot start: no signal (no lock in 3 s), weak signal (tables not whole in 8 s, no whole picture in 5 s, or a multiplex the demodulator counts packet errors on), not on air (a clean multiplex whose whole tables do not list it, or with none of its picture); nothing of a refusal is kept; 403 for a protected program, 502 when the hardware fails |
| `/lineup.post` | 200, nothing to start |

Plex (on BeastNAS) reaches the TV on the LAN and the stream goes straight from the TV to Plex; a 19.4 Mb/s RF channel is a fifth of the TV's 100 Mb/s link. Guide data comes from Plex; the TV's own listings (sparse) are not used. The same stream URL should serve Jellyfin, Emby or Tvheadend [INFERENCE: not tried].

To add it: Settings, Inputs & devices, Live TV Tuner, switch "Share the antenna tuner" on and approve its words (it is off by default; the listener binds the TV's private address, port 5004, private networks only); in Plex, Live TV & DVR, set up, enter `<the TV's address>:5004`, and pick the channels. `hdhr` in `helper.json` holds the rest, all optional: `port` (5004), `bind`, `allowedNets`, `python` (the 32-bit one), `dvbDir`, `channel` (the demux channel, 2: LG's own stack keeps 0), `macFile`.

The lineup also serves players that are not DVR tuners; with `<tv>` the TV's address and port, `http://<tv>:5004`: VLC, Media, Open Network Stream, `<tv>/lineup.m3u` (or one channel, `<tv>/auto/v2.4`); Jellyfin, Live TV, add a tuner: HDHomeRun at `<tv>` or M3U Tuner with `<tv>/lineup.m3u`, and an XMLTV guide provider with `<tv>/xmltv.xml`; Emby, Live TV, the same two choices; Channels DVR, Add Source, HDHomeRun at `<tv>`, or Custom Channels with the playlist and `<tv>/xmltv.xml` as its guide; TiviMate, Add Playlist, M3U with `<tv>/lineup.m3u`, then `<tv>/xmltv.xml` as the EPG; Kodi, PVR IPTV Simple Client, M3U play list URL `<tv>/lineup.m3u` and XMLTV URL `<tv>/xmltv.xml`. The TV's own listings are sparse (a handful of channels have any), so a player with its own guide source may do better with that.

Measured on 2026-11-05 with Plex 1.43.5 on BeastNAS and a559ccb3 on the set: Plex added the device by address and read the 57 channels; its 30311 antenna guide ("Local Broadcast Listings") matched 53, all with listings, and not 35.2, 35.3, 36.4 or 66.1. Plex takes `HDTC-2US` for a transcoding tuner and offers "Transcoder Quality"; the facade cannot transcode and sends the broadcast as is, so the device is set to "Original format". A tune of 2.4 answered in 4.6 s and streamed 82 s: 2.5 MB left the TV in 10 s and Plex cut one HLS segment a second (MPEG-2 720x480, AC-3 stereo, 2.4 Mb/s). Cancelling the grab closed the connection and the helper's pipeline went. Plex's transcoder must be able to write its session folder: no Live TV starts without it.

Measured on 2026-11-05 with 7e8b8f43 over the 59 channels: the first byte came a median 1.81 s after the request (0.93 to 4.27 s; 3.27 s with the 2 s settle the tune had before). Through Plex, from the tune request to a client's first HLS segment, 3.3 to 4.3 s.

## Risks

- **Contention.** One tuner, and LG's own use wins, at once: opening Live TV unrealized the helper's `utp` pipeline (P3), in standby with the panel off too [INFERENCE for the legacy section-parse pipeline: not measured]. A stream leaves the TV's last channel as it was. A retune for Plex drops whatever the owner had locked. The helper notices through the foreground app, UMS's list and its own pipeline's state, not through the packets (P3). Plex sees the stream end and keeps a partial recording.
- **Decoders and sound.** The helper's pipeline holds `ADTU` and `SDEC` alone in UMS's list; with Netflix in 4K in front, Netflix kept its four `VDEC`, its sound and scaler and the front while the pipeline locked RF 27 (2026-11-08). Sound through the TV or the Denon's ARC during a stream was not checked by ear.
- **Wedge.** The TE, SDEC and PVR nodes are kernel interfaces LG's own services use; a bad sequence can hang the engine. The encoder slot wedged the kernel once (`venc-result.md`). Nothing wedged in the probes: the kernel log held driver trace lines only (no oops, BUG or call trace), and the demux input was put back to `None` after each run. A change of firmware can change this: run `tap.py` again after an update (the set runs webOS 10.2.1, firmware 33.22.52).
- **DRM and copy flags.** US ATSC 1.0 is sent in the clear; the set has no CableCARD and no ATSC 3.0, whose streams can be encrypted. The cert is LG's check on who may download, not content protection. None of the six programs on RF 32 carries a CA or redistribution-control descriptor, nor is any access-controlled; the other RF channels are not read, and the helper refuses a program that carries one.
- **Reception.** Continuity gaps follow the antenna (RF 27: 0.06 % of the packets over twelve minutes). The helper does not repair them.
- **CPU, memory, network.** The tap's python used 3.8 % of one core at 6 Mb/s on average; the ring is the kernel's (9.99 MB, about 8 s at 10 Mb/s); the link is 100 Mb/s. The helper adds nothing while the switch is off.
- **Power.** The probes kept the set in Active Standby and the panel off; a stream does not wake the screen [INFERENCE for the helper: `warmOwner` and `registerActiveStandbyRequest` in `HANDOFF.md` are the place to hold it if `tvpowerd` suspends mid-recording].
- **Coverage.** The tuner scans RF 2 to 69; stations that move to ATSC 3.0 disappear from the set for good (`supportATSC3` false).

## Not yet measured

- Plex: playback in a Plex app (the run drove Plex's API), a recording, and Plex's handling of the facade's 503 and 403.
- Sound during a stream, by ear (above).
- Two clients on one RF channel: not built (the second is refused).
- `/lineup.m3u` and `/xmltv.xml` on the set, and in VLC, Jellyfin, Emby, Channels DVR, TiviMate and Kodi: built and tested against the fake only (the `getEventList` shape was read on the set, `channel-guide.md`).
