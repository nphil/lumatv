# Settings slice: curated rows on LG's own keys, deep links, Ambient light, art and logo options

## Resume here

Done (2026-10-25, offline; nothing was run on the set):

- Every Settings page audited against its mock (`docs/mocks/shots/01..04, 24, 30-*`) and `docs/mocks/settings-map.md`. Gaps filled, wrong
  keys fixed (below), Ambient light added as a ninth rail section after Network.
- Every row that writes LG's settings service now uses the category, key and value names LG's own Settings app uses, taken from
  `research/lgsettings/main.js` (its `getSystemSettings` key lists and the value tables behind its labels). That list lives in
  `tests/qmlhost/stubs/WebOSServices/LgSettings.js`; the stub service refuses a write to a key outside it or a value outside its table.
- "More in LG settings" rows land on the LG screen their text names (`R.lg(page, {target})`, launch parameters `{target: "Brightness"}`);
  the targets are LG's router table (`rV` in `main.js`, in `LgSettings.js`). Game Optimizer launches `com.webos.app.gameoptimizer`.
- Tab bar: Right/Left changed the tab and then dropped the focus into the first row, so a second Right acted on that row and the
  third tab could not be reached by D-pad. The focus now stays on the bar. (`PageList.qml`)
- Tests: `python3 -m unittest tests.test_settings` (see "Verified"). Scenes: `tests/qmlhost/scenes_settings.py` (one per tab, dark and light).

Needs the TV (not verified there): the aspect-ratio write with `app_id`, `time.noOperationAutoOff` values, `picture.logoLuminanceAdjust`
`off|light|strong`, `sound.avSync` and `sound.audioBalance` ranges, whether `getSystemSettingDesc` answers `ui.active/visible` the way LG's
app reads it, every `target`. Slot script: read each key with `getSystemSettings` and `getSystemSettingDesc`, write each value back to
what it was.

Not built (and why):

- Remote & pointer: per-key app assignment for the remote's app keys (PLAN: "open any installed app, or do nothing"). It needs an app
  chooser and the key filter's config contract, neither exists. `remote.numberKeys` (theme.json, default true) is written by the
  "Number keys open dock tiles" switch; nothing reads it yet.
- Home & look: "Glass depth" (mock 30-settings-home) needs a Lucent token for panel depth; "Wallpaper" has no chooser (slideshow not built).
- System: panel hours (`panelcontroller/getPanelUsageTime`): the reply shape is not in any capture; "LumaTV update: Up to date" has no
  update source, so the About tab shows the helper's state and a link to LG's Software update instead.
- Light theme: toggles show a dark thumb when on and the chosen card is washed out (kit/LuToggle.qml, LuCard.qml tokens, not this slice's files).
- Sound: the mock's "HDMI input audio format" is per HDMI port in LG (`inputAudioFormatHDMI1..4`, values not recovered); the page has
  "Digital sound output" (`sound.soundOutputDigital`: auto, pcm, passThrough) instead, which is the one that governs eARC.
- Remote: the mock's "Hold Home for LG's Home" switch is a read-only row ("Always on"): it is the escape if LumaTV misbehaves.

## What each page writes

| Page | Row | Where |
|---|---|---|
| Picture | mode, OLED pixel brightness, Filmmaker ambient light, Eye comfort, contrast, brightness, colour, tint, sharpness, TruMotion, energy saving | `picture.pictureMode backlight filmmakerAmbientLight eyeComfortMode contrast brightness color tint sharpness truMotionMode energySaving` |
| Picture | AI Picture Pro, AI genre, AI brightness | `aiPicture.ai_Picture ai_Genre ai_Brightness` |
| Picture | Aspect ratio (16:9, Original, 4:3), for the input in front | `aspectRatio.arcPerApp` with `app_id` |
| Sound | digital sound output, match screen and sound, balance, automatic volume, TV speakers too | `sound.soundOutputDigital avSync audioBalance autoVolume`, `soundOutput` (external_arc or tv_speaker_external_arc) |
| Sound | sound mode (dimmed while the receiver plays): AI Sound Pro, Standard, Cinema, Clear voice, Sports, Music, Game | `sound.soundMode` aiSoundPlus standard movie news sports music game |
| Inputs | HDMI-CEC | `other.simplinkEnable`; Auto/Always/Hidden per input stay in theme.json `inputs.*` |
| Network | Wake from the network | `network.wolwowlOnOff` ("true"/"false") |
| Remote | pointer, speed, size | `general.remotePointer`, `option.pointerSpeed`, `option.pointerSize` |
| System | turn off when idle, no-signal off, Quick Start+, screen shift, logo dimming, audio guidance, captions | `time.noOperationAutoOff`, `time.autoOff15Min`, `option.quickStartMode`, `picture.screenShift`, `picture.logoLuminanceAdjust`, `option.audioGuidance`, `caption.captionEnable` |
| System | language, time zone (read, change in LG's screen) | `localeInfo.locales`, `time.timeZone` |

Dropped because LG has no such key or LG shows it only on monitors: `general.autoInput` (LG: `other.autoInputSwitch`, monitors only),
`sound.avSyncAdjustment`, `sound.balance`, `sound.digitalSoundOut`, `general.autoOff2Hours/4Hours`, and two sound modes the service
does not know (`cinema`, `clearVoice`: LG's are `movie` and `news`). `general.pointerSpeed/Size`, `general.simplinkEnable`,
`general.quickStartMode`, `general.wolwowlOnOff`, `general.deviceName`, `general.language/timeZone*` were in the wrong category.

## Keys agreed with the other slices

theme.json (user file, written through the host's `patch`, one level deep: a nested block is sent whole):

- NotifySlice (logos): `logo.on`, `logo.off`: 1, 2 or 3, default 1. Titles on the Standby tab: 1 Panes, 2 Arc, 3 Name only.
- PluginsSlice (screensaver): `screensaver` = `{style: clock|art|off, afterMin: 5|10|30, dim: bool, glance: bool, art: {everyMin: 5|15|60, order: shuffle|sequence, caption: bool}}`.
  Defaults are in `payload/theme/theme.json`. The art rows show only with style Art, the glance row only with style Clock.
- Remote: `remote.numberKeys` bool (default true).
- PluginsSlice also reads `screensaver.standbyHours` (0|2|4|8), row "Then go to standby" on the Screensaver tab; the burn-in tokens in the same block are not edited here.

Helper settings layer, owner `base.ambient` (`helper/app/src/schemas/base.ambient.json`, registered in `SettingsStore::register_builtin`;
the page reads `GET /v1/settings/base.ambient` and writes `POST` one change at a time): `enabled`, `mode` (picture, colour,
picture-off-home), `brightness` 0..100 step 5, `still_colour` (warm, cool, rose, iris, foam), `saturation` 50..150 step 5, `dark_hold`,
`skip_bars`, `game_edges`, `delay_ms` 0..2000 step 10, `top`, `right`, `bottom`, `left` (0..120). The page also reads `GET /v1/ambient`
(state, strip) and posts `POST /v1/ambient/test {run}` from the "Test pattern" row on the Light tab; a 409 from the helper shows its words as a note (PluginsSlice serves both).

## Wiring asked of others

- ShellSlice: `Settings.launchApp(appId, params)` (Layers.qml must pass `params` in the `/launch` call, and the launch redirect must let a
  launch of `com.palm.app.settings` from Settings through).
- PanelsSlice uses `PageList` (`valueText` on sliders was added for them); PageReceiver and PluginTiles are theirs.

## Test and harness changes

- `tests/qmlhost/stubs/WebOSServices/{LgSettings.js,FakeSettings.js,LS.qml}`: keys in LG's categories, refusals recorded as `error` on the
  luna call, `getSystemSettingDesc` answered (`ui.active/visible`).
- `tests/qmlhost/fakehelper.py`: `GET/POST /v1/settings/base.ambient` validated against the real schema file, `GET /v1/ambient`,
  `POST /v1/ambient/test`.
- `tests/test_settings.py`: rail has Ambient light; the build/summary test and the prompt tests assert behaviour through `--eval`
  (rail summaries; the prompt's drawn texts); new tests for ambient (helper writes, ordering, refusal), art block, logos, number keys, aspect
  ratio with `app_id`, the tab bar, every row against the stub's refusals, every LG deep link.
- `Settings.qml` run alone has no theme (`Lu.theme` is empty), so tests that focus a slider go through Home; the logic-only tests use `surface="Settings.qml"`.
