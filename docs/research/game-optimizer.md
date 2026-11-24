# Game Optimizer on the OLED65C4 (webOS 10.2.1-5202)

LG's Game Optimizer (`com.webos.app.gameoptimizer`, a web app) is a front end for keys of `com.webos.service.settings`. The only
part that is not a setting is the live strip (frame rate, VRR range, signal), which comes from `utp.extinputs` and is not on the
compositor's allowlist. Inputs & devices › Devices launches LG's app for it; this map is what a native page has to call.

Sources: the app's bundle decoded from the TV (rules marked *LG*), `/etc/palm/description.json` and `defaultSettings.json`
(*DB*), read-only `luna-send` probes (*probed*).

## Storage and dimensions

The settings DB keeps one value per dimension. A key must be sent with exactly the dimensions it declares, otherwise the answer
carries `errorKey`. `getSystemSettingDesc` refuses a `dimension`; `getSystemSettings` takes one and echoes it.

| Dimension | Values |
|---|---|
| `gameInput` | none, hdmi1-4, dp1, dp2, usbc1, usbc2 |
| `gameGenre` | Standard, FPS, RPG, RTS, Sports, USER |
| `dynamicRange` | sdr, sdrALLM, hdr, hdrALLM, dolbyHdr, dolbyHdrALLM, technicolorHdr, technicolorHdrALLM |
| picture: `input`, `pictureMode` | hdmi1-4; the game modes are `game`, `hdrGame`, `dolbyHdrGame` |
| sound: `audioInput` | hdmi1-4 and hdmi1_game-hdmi4_game |

`other` is stored as `other$<gameInput>.<gameGenre>.<dynamicRange>.<uhdDeepColor>`. Three groups of keys:

- Per input, `{gameInput}`: gameGenre, gameOptimization, freesync, enableALLM, inputOptimization, enableQuickGame,
  enable144HzBooster, enableQFT, darkMode, blueLight, gameUIColor, gameScreenRatio, gameScreenPosition, gameSettingModified,
  qmsVrr, uhdDeepColor.
- Per genre, `{gameInput, dynamicRange, gameGenre}`: blackStabilizer, whiteStabilizer, lowLevelAdjustment, gameAdjustContrast,
  gameBlackLevel, gameSharpness, gameColorDepth. The range is the live signal's, so an ALLM signal is a different row
  (`sdrALLM`). With no game input in front the context is `gameInput: none`, whose genre rows are not the ones a game uses.
- No dimension: `gameMode` (one object, `{hdmi1..hdmi4: "on"|"off"}`; the whole object is written), `gameDashboardStatusList`
  (an array of four tile ids), `oledCareMode`.

Picture keys of the game inputs: `picture$<input>.<pictureMode>`, dimension `{input, pictureMode}` (`motionPro`,
`hdrDynamicToneMapping`, `DtmProfessional`). `eyeComfortMode` is global; `truMotionMode` is per app (`app_id`).
Sound keys: `sound$<audioInput>`; `{audioInput: "hdmi2_game"}` and `{audioInput: "hdmi2"}` both read, with equal defaults.
The `other` answer in the `none` context is `{gameInput: none, gameGenre: Standard, dynamicRange: sdr}`; its numbers come back
as numbers (`gameBlackLevel: 50`).

With `gameInput: none` the service answers `ui.active: false` for the genre, the stabilizers and adjusters, `blueLight`,
`darkMode`, `inputOptimization`, `gameScreenPosition` and `uhdDeepColor`; it answers `active: true` for `gameMode`,
`gameOptimization`, `freesync`, `enableALLM`, `enableQuickGame`, `enable144HzBooster`, `lowLevelAdjustment`, `gameUIColor` and
`gameScreenRatio`.

## The features

*Selected* means the input's value in `gameMode` is `on`; LG dims everything below while it is off.

| Feature (LG's name) | Key | Dimension | Values | Greyed when (*LG*) |
|---|---|---|---|---|
| Game Optimizer | `other.gameMode` | none | per input on/off | never. Turning on also turns on Dynamic Action Sync and HDMI Deep Color |
| Game Genre | `other.gameGenre` | gameInput | Standard, FPS, RPG, RTS, Sports, USER | not selected; the picture and sound mode must be Game for a genre to apply. "(User Settings)" after a genre whose `gameSettingModified` is true |
| Prevent Input Delay | `other.inputOptimization` | gameInput | `auto` = Standard, `on` = Boost | not selected. Returns to Standard while Game Optimizer is off |
| VRR / VRR & G-Sync | `other.gameOptimization` | gameInput | on, off | not selected. Title "VRR & G-Sync" when `tv.model.supportVRR` is `Gsync` |
| AMD FreeSync Premium | `other.freesync` | gameInput | on, off (low, high, wide on some models) | not selected. Title "AMD FreeSync" when `tv.hw.turnkeyVersion` is `T2.0` |
| Fine Tune Dark Areas | `other.lowLevelAdjustment` | genre | -30..30 (OLED) | not selected, or neither VRR nor FreeSync is on |
| Instant Game Response (ALLM) | `other.enableALLM` | gameInput | on, off | not selected |
| Quick Frame Transport | `other.enableQFT` | gameInput | on, off | `ui.visible` false on this TV |
| 144Hz Mode | `other.enable144HzBooster` | gameInput | on, off | not selected. LG asks to keep or revert after a switch |
| Black Stabilizer | `other.blackStabilizer` | genre | 0..20 | not selected |
| White Stabilizer | `other.whiteStabilizer` | genre | 0..20 | not selected |
| Game Contrast | `other.gameAdjustContrast` | genre | 0..100 | not selected |
| Game Black Level | `other.gameBlackLevel` | genre | 0..100 | not selected |
| Game Sharpness | `other.gameSharpness` | genre | 0..50 | not selected |
| Game Color Depth | `other.gameColorDepth` | genre | 0..100 | not selected |
| Reset | `gameSettingModified[genre]` = false, then `resetSystemSettings {category: other, keys: [stabilizers + the four adjusters]}` | genre | | not selected |
| Dark Room Mode | `other.darkMode` | gameInput | off, level1, level2 | not selected |
| Reduce Blue Light | `other.blueLight` | gameInput | off, level1, level2 | not selected, or `picture.eyeComfortMode` is not off |
| OLED Motion | `picture.motionPro` | input, pictureMode | on, off | not selected; the value counts only while `picture.truMotionMode` is `user`; off while the signal is ALLM |
| TruMotion | `picture.truMotionMode` | `app_id` | off, cinemaClear, natural, smooth, user | |
| Dynamic Tone Mapping (HGiG) | `picture.hdrDynamicToneMapping` | input, pictureMode | on, off, HGIG | not selected. `DtmProfessional` and `DtmMastering1k/4k/10k` apply only while it is off |
| Wide Aspect Ratio | `other.gameScreenRatio` | gameInput | 16:9, 21:9, 32:9 | not selected |
| Screen Position | `other.gameScreenPosition` | gameInput | top, middle, bottom | not selected |
| Menu Color | `other.gameUIColor` | gameInput | violet, orange, green | not selected |
| Game Dashboard | `other.enableQuickGame` | gameInput | on, off | not selected, or neither a game picture mode nor sound mode `game` is in use |
| Dashboard tiles | `other.gameDashboardStatusList` | none | four of: fps, vrr_aiGameSound_whiteStabilizer, blackStabilizer, lowLatency, aiGameSound, hdr, resolution, dolbyAtmos, inputLag | |
| AI Game Sound | `sound.aigamesound` | audioInput | on, off | not selected, or sound mode is not `game` |
| Game Equalizer | `sound.gameTVEqualizer`, `gameTVEqualizer10band` | audioInput | on, off; ten bands, -10..10 | AI Game Sound is on (they exclude each other), personalized sound is on |

The frame-rate counter of this firmware is the `fps` dashboard tile; it shows over the game while the Game Dashboard is on.
Tile names: FPS, VRR (falls back to AI Game Sound or White Stabilizer by what is on), Black Stabilizer, Low Latency, AI Game
Sound, HDR, Resolution, Dolby Atmos (only when `tv.model.soundModeType` is `Dolby Atmos`), Input Lag (Standard or Boost).

## Config gates

`com.webos.service.config/getConfigs` on this TV: `tv.hw.turnkeyVersion` = `LG`, `tv.model.supportHighFrameRateMode` = `off`,
`tv.model.motionProMode` = `OLED Motion`, `tv.model.supportVRR` = `Gsync`. Not set: `tv.model.supportDolbyTVATMOS`,
`tv.model.supportGameEdit`, `tv.model.supportQFT`, `tv.model.supportFreeSync`, `tv.conf.supportGameOptimizer`, every
`profile.gameOptimizer*`, `audio.supportPSW`. LG's own gates: the 144Hz title takes the number from
`supportHighFrameRateMode` (144 when it has none); the Atmos tile follows `tv.model.soundModeType` (not probed).

## Not on this firmware

- Auto genre detection: no key (`autoGenre`, `autoGameGenre`, `enableAutoGenre`, `genreDetection`, `gameGenreAuto`,
  `dynamicActionSync` all answer `errorKey`). The genre is the owner's pick.
- FPS Counter and Crosshair: LG's app has rows for them (`fpsCounter`: off and the four corners), but the TV's description has
  no such key and `profile.gameOptimizer.supportFpsCounter` / `supportCrossHair` are not set.
- `picture.hgig`: hidden; HGiG is a value of `hdrDynamicToneMapping`.
- Quick Frame Transport: the desc is `visible: false`.
- `qmsVrr` and `uhdDeepColor` are on the HDMI tab of Inputs & devices.

## Rules for a client

- Per-input keys, picture keys of the game modes and the sound keys can be read and written from Home by naming the dimension
  (`{gameInput}`, `{input, pictureMode}`, `{audioInput}`).
- The genre keys follow the live signal's `dynamicRange`, which Home cannot name, so they are changed through the live context
  of the input in front: `other`'s answered dimension must carry that `gameInput`. A write that changes one marks its genre in
  `gameSettingModified` (an object value, written whole).
- `gameMode` and `gameDashboardStatusList` are global objects/arrays: write them whole.
- OLED Motion needs TruMotion on User (`app_id` of the input). `motionPro` is written for `game`, `hdrGame` and `dolbyHdrGame`.
- Dynamic Tone Mapping and HGiG are one key of the `hdrGame` mode. AI Game Sound and the Game Equalizer are keys of
  `hdmiN_game`; the two exclude each other.
- A write answers `returnValue` and, when a key was refused, `errorKey`.

## Not verified on the set

The set was read-only so far and has no game signal on an HDMI input. Untested: every write, `resetSystemSettings` with a
genre dimension, the live context of an HDMI input in front (its `dynamicRange`, the ALLM variants, `ui.active` there), the
side effects of turning Game Optimizer on (`uhdDeepColor`), and LG's keep-or-revert prompt for 144Hz Mode.
