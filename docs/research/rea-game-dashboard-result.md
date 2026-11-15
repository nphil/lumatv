# LG Game Dashboard and Game Optimizer: sources, the HDMI gate, what LumaGame can use

Read-only session 2026-11-15 14:01-14:20 on the OLED65C4 (O22, webOS 10.2.1, firmware 33.22.52), TV in Active
Standby throughout: no keys, no settings writes. Settings receipts in `rea/`; LG files and the prettified Game
Optimizer bundle under `/data/home/tmp/lumagame-rea-20261115/` (kept out of git). **Measured** = read from the TV or
from LG's code; **Inferred** = not observed.

## Who draws what (measured)

- Game Optimizer menu: `com.webos.app.gameoptimizer`, an Enact web app.
- Game Dashboard (the in-game strip and quick panel): system QML in QuickSettings (`com.webos.app.quicksettings`,
  `QuickGameSettingsMainView.qml`; copy in `/data/home/lgtv/uiinv/qml/qml/QuickSettings/`).
- FPS counter and crosshair: inputcommon `BgViewLayer/GameFpsItem.qml`, monitor profile only. `fpsCounter`,
  `crosshair`, `hgig` and `truMotionMode` are absent on this TV (settingsservice `errorKey`).

## Where the numbers come from (measured)

Everything comes from `luna://com.webos.service.utp.extinputs` on the HDMI input's pipeline:

- `bind {subscribe:true}` returns `broadcastId`, used as `pipelineId`; nothing else is queried without it.
- `getVRRInfo {pipelineId, mode:"GameOptimizer", interval:1000}`: `vrrInfo.frameRate` (the fps figure) and
  `vrrInfo.vrrType`.
- `getVideoInfo {pipelineId}`: `video.width`, `video.height`, `video.hdrType`.
- `getCurrentAudioCodecType {pipelineId}`: the audio codec.

The strip shows up to four items from `other.gameDashboardStatusList` (now `fps`, `vrr_aiGameSound_whiteStabilizer`,
`blackStabilizer`, `lowLatency`). "Low Latency" is on when `picture.memcDelayLevel` is `none`; "Input Lag" reads
`other.inputOptimization` (`auto`/`standard` show Standard, anything else Boost). With an app in front, `getVideoInfo`
and `getVRRInfo` return -106 and `bind` returns -101 (`docs/AGENTS.md`).

## The HDMI gate (measured)

QuickSettings enters game mode only when `gameMode[gameInput]` is `on`, the picture mode is a game mode (`game`,
`hdrGame`, `dolbyHdrGame`) or the sound mode is `game`, `enableQuickGame` is on and `Utils.isAvailableGameMode` passes
(`QuickSettingsMain.qml:136`, `Utils.js:416`). That check maps `gameInput` through `hdmiAppIdMap`, which knows only
`hdmi1`-`hdmi4` (USB-C and DP on the monitor profile), requires that `com.webos.app.hdmiN` is the foreground app, and
refuses PIP/SXS multiview. `gameMode` stores only `hdmi1`-`hdmi4` keys, and `dimensionInfo.gameInput` read `none`
with Home in front.

## Verdict for LumaGame

- Showing LG's dashboard over a LumaGame stream: no. The gate wants an HDMI app in front, and the data source is the
  HDMI pipeline.
- Feeding LumaGame's stats into it: no. The dashboard only reads extinputs getters; there is no setter or injection
  method.
- Reading the same sources: no for an app's pipeline (-106/-101 above). LumaGame's own overlay reads the engine's
  stream stats (decode rate, bitrate, codec, size, HDR) and, through the helper, the output side:
  `/proc/lg/de/win0/status` (`vFreq`, `lowdelaymode`) and the `DE2VSYNC`/`osd_irq` rates
  (`docs/lumagame-gaming-features.md`, "Measuring the scan rate"). VRR state comes from the VRR research, not from
  extinputs.

## Game Optimizer settings and an internal app

| Setting | Keyed by | For an NDL app |
|---|---|---|
| Genre, Black/White Stabilizer, `inputOptimization` | `gameInput` (stabilizers also by dynamic range and genre) | inactive: `gameInput` is `none` (measured) |
| `enableALLM`, `freesync`, `gameOptimization`, `enableQuickGame` | `gameInput` | readable with `gameInput` `none`; they switch HDMI behaviour (inferred) |
| Game picture mode | `input`, `dynamicRange` | applies: an NDL game load of an HDR stream switched the picture dimension from `filmMaker`/`sdr` to `hdrGame`/`hdrALLM` (measured) |
| `memcDelayLevel` (`none`/`mid`/`max`) | descriptor says global | `mid` at Home, `none` during that NDL game playback (measured): the dashboard's "Low Latency" state comes with the game load |

The play receipts are from the 2026-11-15 display session (`/data/home/tmp/lumagame-display-20261115/`,
`picture-idle-before.json` against `picture-play-before.json`); the same load also turned `energySaving` and
`realCinema` off. Game Optimizer offers nothing more for the helper to write: the game load already gives the game
picture mode and `memcDelayLevel: none`, and the remaining low-latency knobs are the gaming lease's driver controls.

Untested: an SDR stream's picture dimension; whether the ALLM and FreeSync switches affect app pipelines at all.
