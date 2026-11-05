# Live TV capture: the set's antenna feed into Plex

Nitin asked on 2026-11-05 for the TV's own broadcast feed to reach Plex (or something like it) without switching the TV to it and without new hardware (no HDHomeRun). This note holds the feasibility result, the path, the risks and the probes still to run. Nothing was tuned, recorded or changed on the set and no byte of transport stream has been read; every TV read was read-only, under the lease.

## Verdict

**Probably feasible, not yet proven.** The route is LG's kernel DVB/PVR interface, read through a second demux channel, with the tuner steered by LG's own windowless "scan" pipeline. Three assumptions decide it and each takes one probe. LG's own DVR is not a route: it is off on this US set and writes LG's encrypted format.

| Gate | Assumption | Why it may hold | How it fails |
|---|---|---|---|
| 1 Tune without a window | a caller that is not LG's Channel Tuning app can open a `purpose: SCAN` broadcast pipeline, retune with `setAtscManualTuning` and keep the lock | LG's app does exactly this with `windowId ""`; the demod is locked now with no pipeline open | `utp` refuses the caller, or drops the pipeline after a timeout |
| 2 A second demux channel | `demux2` can take the demod's port while LG's stack owns `demux0` | the demod's one port feeds any channel, and the driver leaves a port alone when its config is unchanged; LG's own record-while-watching has to share it [INFERENCE] | `EBUSY`, or LG's channel is reset |
| 3 PVR download | a root process outside LG's DVR service can pass the PVR cert and read `dvr2` | the cert is `seed XOR table`, documented in LG's GPL header; the table is in `ts-record` and `libhal_lg115x`; the seed is one HAL call | the TEE or the TE MCU refuses a caller that is not LG's. Fallback: PES taps (no cert), remuxed in software |

One tuner means one RF channel at a time. The 57 antenna channels sit on 10 RF channels, so Plex sees `TunerCount 1`, and the TV's own Live TV outranks Plex.

## Resume here

Done: the tuner, demux and PVR inventory from LG's tree and from the set; LG's DVR ruled out; the cert recipe found in LG's code; the windowless tune call found in LG's Channel Tuning app; the Plex side sized.

Open: Gates 1 to 3. **Next: P1a, then P1b (below); both need Nitin's OK** (they open the TE's kernel nodes, and a kernel wedge there ends in a cold boot, as the encoder slot did: `venc-result.md`).

## Evidence key

`TV:` read on this set (OLED65C4PUA, webOS 10.2.1, kernel 5.4.268-329.ptl4tv.2, `tv.hw.SoCChipType O22N2`) on 2026-11-05, receipts in `live-tv-capture/`. `SRC:` LG's O22 GPL tree (`/data/home/lgtv/research/lgsrc/k/soc_GO/linux-and-kdriver_paluma-5/kdriver`, paths relative to it; webOS 25, newer than the set). `BIN:` strings or disassembly of files copied off the set (kept outside git: `libkadaptor.so.2.0.1` sha256 `ae0e91f0`, `ts-record` `47756a93`, `libhal_lg115x.so.2.0.1` `8a95eed0`). `PUB:` public page. `[INFERENCE]` is untested. Ioctl numbers are computed from the headers, not read off the set.

## What the set has

| Item | Answer | Evidence |
|---|---|---|
| RF tuners | **1**: a Silicon Labs Si2178B front end and the SoC's internal ATSC demod, one `/dev/dvb/adapter0/frontend0`. No second tuner, no ATSC 3.0 | TV: `/proc/lxdvb/demux/status` `NUM_OF_TUNER: 1`; modules `si2178b`, `si2178b_atv`, `dvb_si2178b_chip`, `atsc_dtv_soc`, `demod_khal`, `webos_tv`; configd `tv.hw.supportDualTuner`, `supportTripleTuner`, `support2RF`, `supportATSC3`, `tunerless` all false, `supportInternalDTVDemod` true (`20-config-dvr-tuner.txt`, `11-devices-modules.txt`) |
| Demux channels | 6 SDEC channels: 4 reach `/dev/dvb` as `demux0..3` (= `sdec0c0..c3`, 64 PID and 64 section filters each); the 2 sub channels (one filter each) exist only as `/dev/lg/sdec0c4` and `c5`. 256 PID filters in all. Each `demuxN` pairs with `dvrN`, `caN`, `deliveryN` | SRC: `core/te/sdec/sdec_cfg.c`; TV: `10-nodes.txt` |
| `delivery0..3` | ATSC 3.0, Japan 4K and CI+ 2.0 baseband only: no use for ATSC 1.0 | SRC: `include/linux/dvbv5-ext/dvbv5-ext-delivery.h` |
| PVR | 2 download engines (`pvr0dn0/1`) and 2 upload engines (`pvr0up0/1`); a download writes 192-byte packets (188 plus a 4-byte timestamp) into a ring of about 10 MB from the TE's reserved 43 MB shared region | SRC: `core/te/pvr/pvr_cfg.c`, `core/te/te_cfg.c`, `core/te/dvb-core/lxdvb_dvr.c` (`DVR_DN_SHARED_ONLY_BUF_SIZE`) |
| Demod input | one port, `IN_DEMOD0`, selectable by every channel | SRC: `lxdvb_demux.c` `dmx_ext_source_to_input_cfg` |
| Who holds what | `legacy-broadcast-atsc` (pid 2255) holds `frontend0` open read-write and `/dev/video2`; nothing holds a demux, dvr, pvr, sdec or te node. All four demux inputs read `None / DISABLE`, no filter, every PVR channel idle, `TE_HW` interrupts 0 since boot | TV: `12-holders.txt`, `32-frontend-holder.txt` (fdinfo flags `02`), `30-lxdvb-demux.txt`, `51-dvr-props.txt`, `14-interrupts.txt` |
| Channels | 57 antenna channels on RF 18, 19, 21, 22, 25, 27, 29, 32, 35, 36; the TV scans RF 2 to 69; last channel `1_32_2_4_0_0_0` (RF 32, 581 MHz, 2.4) | TV: `channels-by-rf.txt`, `63-atsc-scan-info.txt`, `40-last-channel.txt` |
| Load to share with | 3 cores online, 79 % idle; `MemAvailable` 754 MB; `CmaFree` 84 MB; `eth0` 100 Mb/s; `legacy-broadcast-atsc` 12.6 MB resident | TV: `59-baseline.txt` |

## Does the tuner stay locked while the display shows something else

**The hardware does, and does now.** The demod reads `LOCK`, `VSB`, packet errors 0, BER 0 on three reads (08:29, 08:33, 08:35 EST) with the set in Active Standby, Home in front, no media pipeline (`getActivePipelines`: "pipeline empty") and the TE idle; its IPC interrupt kept counting (458,207 to 475,034 in 12 minutes, about 22 a second). The status does not name the RF channel; the last channel (RF 32) is the likely one [INFERENCE]. LG's stack leaves the frontend tuned after Live TV closes (`56-demod-signal.txt`, `58-demod-signal-2.txt`, `65-final-state.txt`). [INFERENCE: how long it stays so, and whether deeper standby powers it down; not measured.]

**LG offers no product feature for it on this set.** Live TV's `tv` pipeline belongs to the Live TV window (it claimed `ADTU`, `SDEC`, `VTP` and two `VDEC` in a captured session: `docs/research/multiview/tv-2026-10-30/luna-lgmv-both-playing/ums-pipelines.json`), and "record while watching" is the DVR, which is off (next section).

**LG's middleware does have a windowless tune.** The Channel Tuning app (`com.webos.app.channelsetting`, `main.js` on the set) opens its pipeline with `utp.broadcast/open {"options":{"appId":"com.webos.app.channelsetting","windowId":"","purpose":"SCAN"}}`, tunes with `utp.channel/scan/setAtscManualTuning {"broadcastId":…,"tune":{"sourceType":"INPUTSOURCE_TERRESTRIAL_DIGITAL","physicalNumber":N}}` and ends with `utp.broadcast/close` (BIN: `61-chan-app-luna.txt`). `legacy-broadcast-atsc` lists scan purposes AUTO_SCAN, MANUAL_SCAN, EIT_TOUR, SLU and watch purposes that include "EPG gather" and "section parse" (`strings-legacy-broadcast-atsc.txt`). The manual-tuning screen shows signal bars after the call, so the call locks the tuner [INFERENCE]; whether it keeps the lock with no screen is Gate 1.

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
| R1 PVR download: `demuxN` PID filters with `DMX_OUT_TS_TAP`, read `dvrN` | the program's own transport packets, ready for a TS server | **Primary.** Gated by the cert (Gate 3). SRC: `lxdvb_demux.c` `set_dvb_filter_attr`, `lxdvb_dvr.c` `dvr_read` |
| R2 PES taps: `DMX_OUT_TAP` filters read from `demuxN` | PES of each PID, no PVR, no cert | **Fallback.** Needs a software muxer (PES to TS, PCR made from PTS or read from the STC); [INFERENCE: PES headers come through intact] |
| R3 SDEC input dump (`SDEC_IOC_DUMP_START`, the TE debug dumps) | the whole input TS | No. The MCU grants dumps by "authority" (`te_get_authority`) and the set has no `te`, `lxdvb`, `pvr` or `sdec` entry in debugfs (`34-debugfs-names.txt`) |
| R4 `delivery0..3` | ATSC 3.0, Japan 4K and CI+ 2.0 baseband | No: not ATSC 1.0 |
| R5 the decoded picture (video tap, `/dev/venc`) | pixels | No: needs the display path, which is what Nitin does not want to switch. The encoder route is in `venc-result.md` |
| R6 `pvr0up0/1`, or `dvrN` opened write-only | the upload side: it feeds a stream into a demux channel for playback and has its own cert | No: wrong direction |
| R7 uMediaServer | pipeline types `media`, `advr`, `record`, `tv`, `tv_bg`, `camera`, `vsm` and more; none hands a stream to a caller, and `record` is the DVR's | No: `docs/research/gfn/tv-2026-10-26/umediaserver_resource_config.txt` |

## The path

On the TV, as root, in the helper (it already shells out to `luna-send`):

1. **Key, once.** `HAL_PVR_CRYPTO_GetSecureKey(NULL, buf16)` from `libhal_lg115x.so.2` in a 32-bit process: the set's `python3` 3.10.13 is 32-bit with `_ctypes` (`66-python-ctypes.txt`), so no compiler is needed. `libkadaptor` forwards the call to a secure-element client (`ca_if_func_for_hal_drm_crypto`, command 0xC; the library also carries `TEEC_*` strings) [INFERENCE: a TEE call]. The key is a device secret: kept in memory or a 0600 file under `/var/lib/lumatv`, never in git, logs or receipts.
2. **Tune.** `utp.broadcast/open` with `purpose "SCAN"` for a `broadcastId`; `utp.channel/scan/setAtscManualTuning` with the RF number from the channel's id (`1_<rf>_<major>_<minor>_…`); `utp.broadcast/getSignalStatus` until strong. Never `saveAtscManualTuning` or `deleteAtscManualTuning`: they edit the channel map.
3. **Tap.** Open a free `demuxN` (not `demux0`, which LG's pipeline uses) read-write and send `DMX_EXT_S_CTL` (`0x40106f35`) with id 7 and `dmx_ext_source {IN_DEMOD 0, port 0, PARALLEL 1}`, the values of LG's own example. The same TPI config as LG's means the driver does not reset the port (SRC: `te_io.c` `config_tpi` returns 0 when unchanged).
4. **Learn the program.** Section filters (`DMX_SET_FILTER`, `0x403c6f2b`) on PID 0 for the PAT and the PMT's PID for the PMT; PSIP is on PID `0x1FFB` if names are wanted.
5. **Open the download.** `/dev/dvb/adapter0/dvrN` read-only (same N), then `PVR_EXT_S_CTL` (`0x40107000`) with id 3, size 32 and the cert `cert[i] = seed[i % 16] XOR table[i]` (table: `include/linux/dvbv5-ext/dvbv5-ext-pvr.h` lines 208-214). LG's `HAL_PVR_OpenDownloadDevice` calls `open`, then `HAL_PVR_CRYPTO_GetSecureKey`, then an `ioctl` (BIN, disassembly). Before the cert, `dvr_read` returns `EIO`.
6. **Start the PIDs.** One demux fd per PID: `DMX_SET_PES_FILTER` (`0x40146f2c`) with `DMX_IN_FRONTEND`, `DMX_OUT_TS_TAP`, `DMX_PES_OTHER`, `DMX_IMMEDIATE_START`, for PAT, PMT, PCR, video, each audio and, if wanted, `0x1FFB`. Read `dvrN`: 192-byte packets; keep the 188 bytes that begin with `0x47` [INFERENCE: the 4 timestamp bytes come first; the sync byte's offset settles it].
7. **Serve** the packets as `video/mp2t` over HTTP, to as many clients as ask for programs on that RF channel (one tap, fan out in software).
8. **Let go.** Close the fds (the driver frees the filters and the download), then `utp.broadcast/close`.

Owner first: the helper refuses (HTTP 503) and releases when `com.webos.app.livetv` is in front or a `tv` pipeline of Live TV appears, and while Live TV runs a stream may share its RF channel without tuning at all (steps 3 to 7 need no tune call).

## Plex

Plex's DVR takes an HDHomeRun-style network tuner. It finds one by IP (Settings, Live TV & DVR, "specify the device's location"), scans its lineup, matches the lineup to Plex's own guide by country and postal code, and then records or streams from the lineup's stream URLs. Recording needs a Plex Pass (`PUB: support.plex.tv/articles/225877347-live-tv-dvr`). The facade is five small endpoints, the set tvhProxy and the other emulators serve (`PUB: github.com/PREngineer/TVHProxy`):

| Endpoint | Content |
|---|---|
| `/discover.json` | `FriendlyName`, `DeviceID`, `BaseURL`, `LineupURL`, `TunerCount 1` |
| `/lineup_status.json` | `ScanInProgress 0`, `ScanPossible 1`, `Source "Antenna"` |
| `/lineup.json` | one entry per channel: `GuideNumber` "2.1", `GuideName`, `URL` to the stream. The source is `iepg/getChannelList` (57 entries, no scan on the TV). Plex refuses a lineup that repeats a channel id (PUB: the tvhProxy README), so the numbers must be unique: the names repeat on RF 29, the numbers do not |
| the stream URL | `/auto/v<GuideNumber>`: the helper's TS of that program, `503` when the tuner is on another RF channel or Live TV has it |
| `/device.xml`, SSDP | optional: adding the device by IP does not need them |

The facade lives in the helper: Plex (on BeastNAS) reaches the TV on the LAN and the stream goes straight from the TV to Plex. A 19.4 Mb/s RF channel is a fifth of the TV's 100 Mb/s link. The same stream URL serves Jellyfin, Emby or Tvheadend [INFERENCE: not tried]. Guide data comes from Plex; the TV's own listings (sparse) are not used.

## Risks

- **Contention.** One tuner, and LG's own use wins. A retune for Plex drops whatever the owner had locked, and LG's Live TV may find the last channel changed [INFERENCE: whether `setAtscManualTuning` moves `getLastChannelId`; P2 reads it before and after]. The UMS would evict a scan pipeline when Live TV opens its `tv` pipeline [INFERENCE]; the helper must notice and stop. Plex sees the stream end and keeps a partial recording.
- **Wedge.** The TE, SDEC and PVR nodes are kernel interfaces LG's own services use; a bad sequence can hang the engine. The encoder slot wedged the kernel once (`venc-result.md`). Each probe runs with Nitin at the set, the panel off, the lease held, one cert write at most, and fds closed on any `EIO` or `ENODEV`.
- **DRM and copy flags.** US ATSC 1.0 is sent in the clear; the set has no CableCARD and no ATSC 3.0, whose streams can be encrypted. The cert is LG's check on who may download, not content protection, and the seed comes from LG's own library on the TV. Not yet seen: whether any of the 57 channels' PMTs carries a redistribution-control descriptor (tag `0xAA`); P1b reads them. The seed is device-specific.
- **CPU, memory, network.** The stream is 2.4 MB/s at most, a few percent of one core [INFERENCE: measure]; the set idles at 79 % with 754 MB free; the TE ring comes from the TE's reserved memory, not the UI's CMA [INFERENCE]; the link is 100 Mb/s. LG's stack already spends about 2 % of a core on the idle tuner (11:41 of CPU time in 9.5 hours, 22 demod interrupts a second); the helper adds nothing while idle.
- **Power.** The tuner is already running. A stream must keep the set in Active Standby and the panel off: the helper must not wake the screen, and `tvpowerd` must not suspend mid-recording [INFERENCE: `warmOwner` and `registerActiveStandbyRequest` in `HANDOFF.md` are the place to hold it].
- **Firmware.** The interface is LG's and may change with an update (the set runs webOS 10.2.1, firmware 33.22.52): run P1b again after one.
- **Coverage.** The tuner scans RF 2 to 69; stations that move to ATSC 3.0 disappear from the set for good (`supportATSC3` false).

## Effort

Probes P1a to P3: a short slot each (S). The build after they pass: the helper's tuner module (luna calls, the DVB ioctls, a fake DVB device for tests, the HTTP stream, the contention rules) and the facade, with Plex's setup on BeastNAS (M, a feature of the helper's size, not a trial). L if Gate 3 fails and the PES muxer (R2) is needed. The TV side needs the 32-bit key call (python ctypes, nothing to build).

## Next probes (each needs Nitin's OK)

| Probe | Does | Touches | Passes when |
|---|---|---|---|
| **P1a** key | in a 32-bit `python3`, `ctypes.CDLL("libhal_lg115x.so.2").HAL_PVR_CRYPTO_GetSecureKey(None, buf)`; prints the key's SHA-256 prefix only | the secure world; no device node | returns 0 and 16 bytes that are not all zero. Fails: the TEE refuses a caller outside LG's services, so R2 |
| **P1b** tap (no tune, the RF channel locked now: RF 32 if the last channel is it) | `demux2` input, PAT, PMT, then `dvr2` with the cert and TS taps for one program (2.4 if RF 32), 10 s into RAM, then close | `demux2`, `dvr2`, a PVR download, TPI port | PAT and PMT parse; every 188-byte packet starts with `0x47`; no continuity gap on any PID for 10 s; `/proc/lxdvb/demux/status` shows the filters while open and none after; the demod stays `LOCK`; Gates 2 and 3 pass |
| **P2** retune | `open` `SCAN`, `setAtscManualTuning` to RF 27, `getSignalStatus`, P1b's tap, hold 10 minutes, `close`; read `getLastChannelId` and the demod before and after | the tuner's channel | the demod locks on 27 with no window or sound; P1b passes on 27; the lock holds; `lastChannelId` is unchanged (or is restored); Gate 1 passes |
| **P3** contention | with P2's pipeline open, Nitin launches Live TV | the screen | Live TV plays; the helper's tap ends cleanly; the pipeline's state is read from `getBroadcastState` |
| **P4** only if P1a fails | R2: PES taps for 10 s, remuxed on the host, played | `demux2` | `ffprobe` finds no error; audio and video stay in step |

Before P1b: the lease; the panel off; `ls -l /proc/*/fd` shows only LG's `frontend0` holder; the receipts of P1a. Stop at the first `EIO`, `ENODEV` or kernel message from `te`, `sdec`, `pvr` or `lxdvb`.

Decisions for Nitin: OK for P1a and P1b; that the TV's own Live TV outranks Plex; that a Plex Pass is in place.
