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
- App keys (leftovers round): Remote & pointer has a row per app key (Netflix, Prime Video, Disney+, LG Channels) showing what it does; OK opens the
  chooser (`settings/PageAppKey.qml`, a child page of Remote): LG's default, Does nothing, then every app the TV launches. The choice is `remote.appKeys`
  in theme.json (below). LG Channels is a hidden system app, so "installed" is read from every launch point, not from the library's list.
- Glass depth: Lucent token `theme.depth` (`level` flat|normal|deep, `steps` 0, 1, 1.5), `Lu.depth`, applied in `Glass.qml` to the shadow's strength, offset and blur
  and to the edge's weight (flat 0.7, deep 1.15 of the designed edge); the quad's margin grows with it (60 px, 90 px deep) so a deeper shadow is not clipped. Row on
  Home & look › Look.
- Sleep timer (`kit/Power.qml`): a stepper on System › Power and a tile in the quick panel (`panels/QuickPanel.qml`, between the receiver and Wi-Fi, as in mock 10).
  Panel hours: a read-only row on System › OLED care.
- Tests: `python3 -m unittest tests.test_settings` (see "Verified"). Scenes: `tests/qmlhost/scenes_settings.py` (one per tab, dark and light).

Needs the TV (not verified there): the aspect-ratio write with `app_id`, `time.noOperationAutoOff` values, `picture.logoLuminanceAdjust`
`off|light|strong`, the `sound.avSync*` limits (LG reads `values.range` min, max and interval from `getSystemSettingDesc`; the row is dimmed until the
TV answers one) and `sound.audioBalance` range, whether `getSystemSettingDesc` answers `ui.active/visible` the way LG's
app reads it, every `target`. New this round, all from LG's own Settings app (`research/lgsettings/main.js`) and read on no TV yet: the sleep timer
(`tvReservationAgent/timer/sleep/getRemainTime` answers `{sleepTimeMinute, sleepTimeSecond}`; whether `com.webos.surfacemanager` may call it), panel hours
(`panelcontroller/getPanelUsageTime` `{subscribe: true}` answers `{panelUsageTime}`; `config/getConfigs` `{configNames: ["tv.conti.supportUsedTime"]}`), and the
app keys on the remote (that the key filter gets `Key_webOS_Netflix` and the others, and that a launch of an app id from the key works). Slot script: read each
key with `getSystemSettings` and `getSystemSettingDesc`, write each value back to what it was; call the three luna reads above and put the replies in this
note; press each app key with a choice of "Does nothing", of another app and of LG's default.

Dead rows removed: "Wallpaper" (an info row that said a slideshow is not built) and the `sound.avSync` slider. The census of the owner's set
(`lgtv/tmp-research/census1/dc-census/settings_all.json`) answers `sound.avSync: "on"`, a switch: the old row wrote milliseconds to it. LG's own delay slider is the
per-output key (`avSyncSpdif` for eARC), which the row now uses.

Not built (and why):

- Home & look: a wallpaper chooser (slideshow not built).
- System: "LumaTV update: Up to date" has no update source, so the About tab shows the helper's state and a link to LG's Software update instead.
- Light theme: toggles show a dark thumb when on and the chosen card is washed out (kit/LuToggle.qml, LuCard.qml tokens, not this slice's files).
- Sound: the mock's "HDMI input audio format" is per HDMI port in LG (`inputAudioFormatHDMI1..4`, values not recovered); the page has
  "Digital sound output" (`sound.soundOutputDigital`: auto, pcm, passThrough) instead, which is the one that governs eARC.
- Remote: the mock's "Hold Home for LG's Home" switch is a read-only row ("Always on"): it is the escape if LumaTV misbehaves.

## Verified (offline host, 2026-10-25)

`python3 -m unittest tests.test_settings tests.test_panels tests.test_scenes_loader tests.test_shell_surfaces tests.test_tokens`: 86 tests, OK. Scenes for the
leftovers, dark and light, looked at: `03-settings-remote-appkeys`, `03-settings-appkey-chooser`, `03-settings-appkey-chosen`, `30-settings-home-glass-{normal,flat,deep}`,
`30-settings-system-power` (sleep timer), `30-settings-system-oled-care` (panel hours), `10-quick-panel` (sleep timer tile), `02-settings-sound` (delay row).
Not on the TV: nothing above has run on the set.

## What each page writes

| Page | Row | Where |
|---|---|---|
| Picture | mode, OLED pixel brightness, Filmmaker ambient light, Eye comfort, contrast, brightness, colour, tint, sharpness, TruMotion, energy saving | `picture.pictureMode backlight filmmakerAmbientLight eyeComfortMode contrast brightness color tint sharpness truMotionMode energySaving` |
| Picture | AI Picture Pro, AI genre, AI brightness | `aiPicture.ai_Picture ai_Genre ai_Brightness` |
| Picture | Aspect ratio (16:9, Original, 4:3), for the input in front | `aspectRatio.arcPerApp` with `app_id` |
| Sound | digital sound output, match screen and sound, balance, automatic volume, TV speakers too | `sound.soundOutputDigital audioBalance autoVolume`, the delay key of the way out (`avSyncSpdif` for eARC and optical, `avSyncSpeaker`, `avSyncWisa`, `avSyncBtSurround`), `soundOutput` (external_arc or tv_speaker_external_arc) |
| Sound | sound mode (dimmed while the receiver plays): AI Sound Pro, Standard, Cinema, Clear voice, Sports, Music, Game | `sound.soundMode` aiSoundPlus standard movie news sports music game |
| Inputs | HDMI-CEC | `other.simplinkEnable`; Auto/Always/Hidden per input stay in theme.json `inputs.*` |
| Network | Wake from the network | `network.wolwowlOnOff` ("true"/"false") |
| Remote | pointer, speed, size | `general.remotePointer`, `option.pointerSpeed`, `option.pointerSize` |
| Remote | the four app keys (Netflix, Prime Video, Disney+, LG Channels), one chooser page for them | theme.json `remote.appKeys` |
| Home & look | Glass depth (Flat, Normal, Deep) | theme.json `depth.level` |
| System | sleep timer (Off, 10, 20, 30, 60, 90, 120, 180, 240 min), also a quick panel tile | luna `tvReservationAgent` `timer/sleep/add {sleepTimeMinute: "30"}`, read with `timer/sleep/getRemainTime` |
| System | panel hours (read only, OLED care tab) | luna `panelcontroller/getPanelUsageTime` `{panelUsageTime}`, hours = floor(ticks / 6); shown only where `config/getConfigs` says `tv.conti.supportUsedTime` |
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
- Remote: `remote.numberKeys` bool (default true); `remote.appKeys` = `{netflix, amazon, disneyplus, lgchannels}`, each an app id, `""` for do nothing, absent for
  LG's own target where that app is installed and nothing where it is not (never LG's auto-install). Agreed with ShellHost, who reads it in the key path
  (`payload/keyfilter`, LumaHome). Qt keys: `Key_webOS_Netflix`, `Key_webOS_Amazon`, `Key_webOS_Disneyplus`, `Key_webOS_LGChannels`
  (`docs/research/surfaces/KeyEntryPoints.json`); LG's apps: `netflix`, `amazon`, `com.disney.disneyplus-prod`, `com.webos.app.lgchannels`. The page
  sends the whole `appKeys` object each time (a nested block replaces whole), so concurrent writes cannot lose an entry.
- Home & look: `depth.level` = `flat|normal|deep`, `depth.steps` = `{flat: 0, normal: 1, deep: 1.5}` (Lucent token, `kit/Lu.qml` `depth`, read by `Glass.qml`).
- PluginsSlice also reads `screensaver.standbyHours` (0|2|4|8), row "Then go to standby" on the Screensaver tab; the burn-in tokens in the same block are not edited here.

Helper settings layer, owner `base.ambient` (`helper/app/src/schemas/base.ambient.json`, registered in `SettingsStore::register_builtin`;
the page reads `GET /v1/settings/base.ambient` and writes `POST` one change at a time): `enabled`, `mode` (picture, colour,
picture-off-home), `brightness` 0..100 step 5, `still_colour` (warm, cool, rose, iris, foam), `saturation` 50..150 step 5, `dark_hold`,
`skip_bars`, `game_edges`, `delay_ms` 0..2000 step 10, `top`, `right`, `bottom`, `left` (0..120). The page also reads `GET /v1/ambient`
(state, strip) and posts `POST /v1/ambient/test {run}` from the "Test pattern" row on the Light tab; a 409 from the helper shows its words as a note (PluginsSlice serves both).

## Wiring asked of others

- ShellSlice: `Settings.launchApp(appId, params)` (Layers.qml must pass `params` in the `/launch` call, and the launch redirect must let a
  launch of `com.palm.app.settings` from Settings through).
- ShellHost: reads `remote.appKeys` and `remote.numberKeys` in the key path (agreed in messages 2026-10-25); `Layers.settingsPages()` skips pages with `search: false`
  (the chooser).
- PanelsSlice uses `PageList` (`valueText` on sliders was added for them); PageReceiver and PluginTiles are theirs.

## Test and harness changes

- `tests/qmlhost/stubs/WebOSServices/{LgSettings.js,FakeSettings.js,LS.qml}`: keys in LG's categories, refusals recorded as `error` on the
  luna call, `getSystemSettingDesc` answered (`ui.active/visible`).
- `tests/qmlhost/fakehelper.py`: `GET/POST /v1/settings/base.ambient` validated against the real schema file, `GET /v1/ambient`,
  `POST /v1/ambient/test`.
- `tests/test_settings.py`: rail has Ambient light; the build/summary test and the prompt tests assert behaviour through `--eval`
  (rail summaries; the prompt's drawn texts); new tests for ambient (helper writes, ordering, refusal), art block, logos, number keys, aspect
  ratio with `app_id`, the tab bar, every row against the stub's refusals, every LG deep link.
- Leftovers round (2026-10-25): app-key chooser (rows, writes, removal of an entry, an app id), Glass depth (token values, the margin that holds
  the shadow, the row), sleep timer (set, read back, LG's rounding), panel hours (counter, the config that hides it), sound delay (key by output,
  limits from the description), and that the dead rows are gone. The host's expressions see `root` and the module's types but not the names a QML
  file imports, so `HostCase.probe` imports the staged `kit` directory (`--data`) to reach `Power`, `Lu` and `Apps`.
- `Settings.qml` run alone has no theme (`Lu.theme` is empty), so tests that focus a slider go through Home; the logic-only tests use `surface="Settings.qml"`
  (a probe can set `Lu.theme` to give a page a theme).
- Stubs: `FakeSettings.js` answers `panelcontroller/getPanelUsageTime` (10230 ticks), `config/getConfigs` (`tv.conti.supportUsedTime` true) and the sleep
  timer (`tvReservationAgent` `timer/sleep/getRemainTime` and `add`, kept in `sleepSeconds`); `getSystemSettingDesc` carries `values.range` for the four
  `sound.avSync*` delays (a fixture: 0..400 step 10; the TV's own limits are read).
