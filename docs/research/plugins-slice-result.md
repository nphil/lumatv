# Plugins slice: notification drawer, plugin cards, the helper's own plugins, device facts, screensaver and art mode, ambient light

Surfaces: `panels/Drawer.qml`, `panels/Screensaver.qml`, `panels/ArtPiece.qml`, `widgets/HomeAssistantWidget.qml` (the At-home
lane), `kit/{PluginHub,NoteLog}.qml`, the topic filter in `kit/Helper.qml`, an optional `icon` on `kit/LuButton.qml`, `art/`, the
burn-in tokens in `payload/theme/theme.json` (`screensaver`). `kit/PluginCard.qml` is deleted (nothing drew it). Helper side: `helper/app/src/{device,ambient}.rs`,
`plugins/{local,firstparty}.rs`, `ambient/src/plan.rs`. Scenes: `tests/qmlhost/scenes_plugins.py` (13 x2, 21 x4, 36 x2). Tests:
`tests/test_plugins_surfaces.py` (run from `tests/`: `python3 -m unittest test_plugins_surfaces`), `tests/test_ambient_slot.py`
(`python3 -m unittest tests.test_ambient_slot`), `helper/app/tests/{firstparty,plugins}.rs`, unit tests in the modules.
Probe slot: `ambient/capture/tv_probe_slot.py`. Nothing here has run on the set.

## Resume here

State at 2026-10-25, offline only.

- **Notification Centre** (mock 13): `panels/Drawer.qml`, one column grouped by who said it: AI access (the helper's audit, a
  paused assistant's denial has a Resume button that asks the helper, which asks on the TV), each plugin (the broker's
  notifications with the plugin's buttons, each drawn with its channel's icon: the helper now publishes `channelTitle` and
  `channelIcon` with every notification), the TV (`NoteLog`: updates, what LG's notification service toasted). D-pad down the
  items, Left/Right between an item's buttons, OK runs the button or clears an item without one, Up from the first item is
  Clear all, Back closes. Opening it marks everything seen. **Wired by ShellSlice-2** (8fd5c0e): the bell, or Enter on it, is
  `root.openLayer("drawer")`, Resident z 67, `open()` on show, Back returns the focus to the bell, the bell's badge is
  `NoteLog.unread` (Home sets `NoteLog.seenAt` at start, so only what arrives after the shell started counts). The **gear hold is
  not wired**: LG's 1 s Settings hold arrives as `Key_Option` and launches LG Settings, so a hold needs the key filter to cancel LG's
  long-press timer as the Home hold does, which cannot be tried offline. My scenes and tests still build the panel with
  `Qt.createComponent`; `root.openLayer('drawer')` works now.
- **Screensaver and art mode** (mock 21): `panels/Screensaver.qml`, up on the helper's `screen.state === "Screen Saver"` while it is
  shown; options are SettingsSlice's `theme.screensaver` (`style` clock|art|off, `dim`, `glance`, `art.everyMin|order|caption`;
  `afterMin` is LG's power manager, nothing here counts idle time). Clock: the block sits at 60 % brightness on black, moves every
  minute while faded out (2 s each way, the PLAN's 4 s black fade), dims to 60 % of that over the first hour, the glance line is
  the weather plugin's card and the first card of any other plugin. After 4 hours in the saver the TV is put in standby
  (`power/powerOff`, PLAN: "standby after a set number of hours"; `screensaver.standbyHours`, 0 = never; Settings has no row for
  it yet). All the numbers are tokens in `theme.json` `screensaver` (`clockLevel .6`, `artLevel .78`, `dimTo .6`, `dimOverMin 60`,
  `moveEverySec 60`, `fadeMs 2000`, `crossfadeMs 1400`, `preloadMs 6000`, `graceMs 300`, `shiftEverySec 4`, `standbyHours 4`). Art: six public-domain prints in `art/` (licence checked by `tools/screensaver_art.py`,
  `--check` runs offline), matte and frame, A/B crossfade 1.4 s, the next print preloaded 6 s before its turn and passed over if it
  is not loaded 300 ms after (in order mode the next one after it, not the same again), a pixel step every 4 s. The PLAN's quick
  bar (Previous, Like, Hide, Next) is not built: any key wakes the TV. It uses the dark material's inks in either material. No timer runs and no
  image is kept while it is hidden. **Wired by ShellSlice-2**: Resident "saver" z 95, shown while it is `active`; Home swallows every
  key (press and release) while it is up, calls `item.key()` once per press (asks `tvpower/power/turnOnScreen`), never while the AI
  prompt is up. Settings has the row for it: "Then go to standby" 0|2|4|8 h writes `screensaver.standbyHours`.
- **Plugin cards on Home**: the At-home lane draws `PluginHub.homeCards` as chips (icon disc, title, first fact) when there are
  any, and one chip saying Home Assistant isn't linked yet (OK opens Settings at its page) when there are none: no invented house.
  The weather, news and clock plugins are Home's own widgets and are left out of the lane. D-pad moves between chips, OK enters,
  OK on a chip runs its first action (`POST /v1/plugins/<id>/actions/<action>`).
- **The helper's own plugins** (weather, news, clock, ambient light): `plugins/local.rs` runs them through the real broker session
  in-process (no socket; validation, limits, expiry and the action route are the same as any plugin's), listed in the `plugins`
  topic under `firstParty`. `plugins/firstparty.rs`: cards derived from the `weather`/`news` topics (so the saved-first cache
  behaviour of the widgets holds, and the clock only ticks while the screen is on), a Refresh action, and the ambient light tile
  (`plugin.ambient.tiles`, action `toggle` flips `base.ambient.enabled`). An external plugin that claims one of those ids is
  inert ("that name belongs to a plugin of the helper's own"). `kit/Helper.qml` now subscribes to their topics too.
- **Device facts** (`helper/app/src/device.rs`): `GET /v1/device` and an additive `device` object in every plugin's `welcome`:
  `{"screen": {"class", "width", "height"}, "input": ["touch"|"remote"|"pointer"|"controller"]}`. Config `helper.json`
  `device` (screenClass tv, 1920x1080, touch false, remote true, pointer true, detectControllers true); a controller is seen
  when an input device reports BTN_GAMEPAD. Pointer means the TV has one, not that LG's remote-pointer toggle is on.
- **Ambient light, helper side** (`ambient/src/plan.rs`, `helper/app/src/ambient.rs`): Settings' `base.ambient` values become the
  engine config, 58 lights clockwise from the top left, and a still colour (warm|cool|rose|iris|foam). `GET /v1/ambient`,
  `POST /v1/ambient/test {run}` (120 ms per section, two passes, 60 s at most), `POST /v1/ambient/context {covered?, game?}`.
  Output is DDP to the WLED strip named in `helper.json` `ambient.wled {host, port}` (LAN IPv4 only; with none nothing is sent;
  never black if nothing was lit before). The capture side talks over a unix socket (`ambient.sock` in the data dir, or
  `ambient.captureSocket`) that exists only while a picture is wanted: it reads `wire` frames and answers one byte, the wanted
  hardware preset. Hue is not driven (see Open).
  The `sound` setting adds the TV's own sound mix (`ambient.sound.capture` in `helper.json`) to the lights; see ambient-result.md,
  "Sound-reactive".
- **The colour reader** (Nitin's decision, 2026-10-25): `lumatv-capture`, a 32-bit native service the package ships with the one LS2 role
  it needs (below). Built, tested on the host against the fake library, packaged, wired into `tools/lumatv`; **not run on the set**.
- **Light-sync probe slot**: `ambient/capture/tv_probe_slot.py`, now installs the reader and its role as the package does and captures
  through it (below). It ran once before the reader existed (12:35, see below).

## The colour reader: `lumatv-capture`

`ambient/capture/service.c`, built by `ambient/capture/build.sh` (`build/arm/lumatv-capture`, 16 KB, ELF32 ARM, glibc 2.28; the same
build gives `build/host/lumatv-capture` for the tests). Idle it is one `connect()` every 2 s: no library loaded, no bus client, no
`/dev/video60`. The helper (`helper/app/src/ambient.rs`) listens on `ambient.sock` in its data dir only while a picture is wanted; the
reader connects, is told the hardware preset (one byte; more bytes later), takes the capture (`initHistogram`, `setPresetHistogram`),
polls at 30 Hz and writes one `AMB1` frame per poll (`ambient/src/wire.rs`, 204 bytes). It gives the capture back (finalize) when the
helper closes, when LG's resource manager takes it (policy callback), after two seconds of failing polls, and on SIGTERM; a refused
init is logged with the lines the library and ls-hubd left in `/var/log/messages` and retried after 5 s, never in a tight loop.

- **Wiring**: `tools/lumatv` `capture start|stop|remove|status`; `apply` installs and starts it (`capture_start`, not fatal), `revert` removes
  it with the role (`capture_remove`), so nothing of it stays once LumaTV is off. `tools/build-ipk.sh` requires and packages
  `payload/capture/{lumatv-capture,capture.sh,role.json}` (the build says to run `ambient/capture/build.sh` if the binary is missing).
  `payload/capture/capture.sh` is the one installer, used by the tool and by the probe slot.
- **The role** (`payload/capture/role.json`, `@EXE@` filled in with the staged path `/var/lib/lumatv/bin/lumatv-capture`, written to
  `/var/luna-service2-dev/roles.d/lumatv-capture.json`, then `ls-control scan-volatile-dirs` and `scan-services`): it allows only LG's
  histogram capture. `allowedNames` are the library's client names (`com.webos.service.capture.client`, `.*`), `inbound` is empty (nothing is
  offered), `outbound` is `com.webos.service.capture`, `com.webos.service.capturepermission` and `com.palm.bus`. A test pins that set.
- **Sources**: the slot of 12:35 (ls-hubd's `LSHUB_NO_ROLE_FILE {"EXE": ...}` shows a role is matched by executable path, and the
  library's own strings give the bus names: `ambient-result.md`, "Probe slot (2026-10-25 12:35 EDT)"); `strings` of the workstation copy of
  `libvtcapture.so.1.0.0` (`com.webos.service.capture.client` is followed by `/dev/urandom`, so the registered name may carry a suffix: hence
  the `.*` form); `docs/research/home-launcher-replacements.md` (a rooted launcher writes its Luna ACG file under `/var/luna-service2-dev`,
  PlainHome's `autostart.sh`); `compositor/tools/tv-inspect.sh` (the three trees searched for roles: `/usr/share/luna-service2`,
  `/var/luna-service2`, `/etc/luna-service2`) and `compositor-c2-result.md` ("ls-hubd matches a role file by the executable path
  (`exeName`)", marked there as inference); the slot's note that `appInstallService/dev/install` writes a role for a service declared in an
  ipk (not used: the tool, which runs as root, writes it, as the launcher precedent does).
- **Not exposed**: the frames go to the helper's socket and nowhere else. The helper publishes the ambient light's status (`GET /v1/ambient`:
  enabled, mode, state, problem; no colours) and sends DDP to the strip; no topic, route, notification or AI-access/MCP tool carries a picture
  reading, and the reader offers no bus service at all.
- **The developer flag stays the owner's**: LG's permission gate still wants `/var/luna/preferences/vt_devmode` (or Hue Sync's command
  line). The reader never creates it; without it the init is refused, logged, and the ambient light shows no picture (a still colour
  works). Only the probe slot creates it, per capture. Keeping it for everyday light sync is the separate decision PLAN records.
- **Tests**: `tests/test_capture_service.py` (9: idle until the socket exists, frames carry what the library said, the preset byte, EAGAIN,
  finalized exactly once on close and on SIGTERM, policy callback, refused init not retried in a tight loop, dry run),
  `ambient/tests/capture_service.rs` (the crate's own `wire::decode` reads the C service's frames for presets 0 and 3),
  `tests/test_capture_install.py` (role written for the staged path with exactly the allowed names, no rescan when unchanged, a binary
  swap, remove, start/stop, and the started service really sending frames to a socket).
- **Verified on the TV (2026-10-25 16:27, `ambient-result.md` "Probe slots")**: ls-hubd reads the role under `/var/luna-service2-dev/roles.d`
  only through a manifest in `manifests.d` (`capture.sh` writes `org.nphil.lumatv.capture.json`), after `ls-control scan-volatile-dirs`;
  `type` `regular` suffices; the library registers `com.webos.rm.client.<session>` (uMediaServer's resource-manager client) and the
  capture client, which the template names with outbound `com.webos.media`, `com.webos.service.capture` and `...capturepermission`;
  LG's own client-permission files already cover those names. The reader took 5.2-5.6 % of a core at 30 polls a second, over the
  plan's 1 %; the zones change about 8 times a second, so 10 polls is the next setting to measure.

## The probe slot (Nitin's `vt_devmode` approval, 2026-10-25)

```
python3 ambient/capture/tv_probe_slot.py --plan          # the steps and the script that runs on the set; touches nothing
python3 ambient/capture/tv_probe_slot.py --go            # from the repository root, with Main's go
python3 ambient/capture/tv_probe_slot.py --go --scenarios home,hdmi2 --secs 30 --no-mute
```

Ask Main for a slot of 10 minutes, "measure". Needs `ambient/capture/build.sh` run first (`build/arm/lumatv-capture`), paramiko, the
HA token (`/data/home/tmp/ha-token-fetch.sh`). It reuses `compositor/tools/{tv,tv_gate}.py` (wake, power state, receiver mute).

**Since the first run (12:35) the slot captures through the package's reader**: it uploads `lumatv-capture`, `role.json`, `capture.sh` and
`slot_collect.py` (a Python 3 stand-in for the helper's socket, stdlib only), runs `capture.sh install` (the package's own installer, so
the role and path are what the ipk will have), runs one window per scenario (flag created, the staged reader started on a socket of the
slot's own, the collector reads its frames for 60 s, the reader gives the capture back, flag removed), and puts the package back as it
found it (`capture.sh remove` if it was not installed, `capture.sh start` if it had been running). `--role-type privileged` is the one
knob if ls-hubd refuses the `regular` role (`<scenario>.log.txt` has the `LSHUB_` lines, `<scenario>.service.log` what the reader said).

- The flag exists only while a capture runs: created straight before the reader starts for a window and removed once it has given the
  capture back (about 1 s after the 60 s collected), never while an app launches or the receiver is muted. Create and remove time (set clock, epoch seconds) and who removed it (`end`, `trap`,
  `deadman`, `finally`) are in `slot-log.json` under `flag.windows`, plus `createdAt`/`removedAt` for the whole slot.
- Removal has three independent paths: the on-set script's EXIT/INT/TERM/HUP trap (it carries on without the workstation), a
  dead-man timer per window (window length + 45 s, survives a SIGKILL of the script), and the workstation script's own `finally`,
  which stops the on-set script first and then checks `test ! -e`. The collector runs in the background and the script `wait`s for it, so a
  stop reaches the trap at once instead of after the window. A flag that is there at the end exits non-zero and says so.
- It does not start if the flag exists already (`--adopt-flag` overrides and removes it at the end), if the library is not the
  recorded one (md5 `35d3fec6`), if someone holds `/dev/video60`, or if the set is not Active. A new failed unit ends the captures.
- Home, HDMI 2 (`com.webos.app.hdmi2`), Netflix (the 4K title by deep link, then ENTER through Home Assistant; the last attempt on
  24 Oct failed with `tvq-pb-101` on the ARC audio format, so the capture may show a title page or an error: the probe reports
  what the zones did either way and `netflix-foreground.txt` says what was on screen). Receiver muted first and put back as it
  was; Picture Mode read before and after, never set; set left in Active Standby.
- **It ran once** (TvSlots-2, 2026-10-25 12:35, `docs/research/ambient/probe-2026-10-25-1234/`): the slot did what it is built to do (flag
  created and removed per window and seen by the probe, receiver muted and put back, Netflix's Play pressed, set back in standby), but
  the probe aborted in all three windows. With the flag present `libvtcapture` logs "dev mode on", then dies in `LSRegisterPubPriv`
  (`std::runtime_error`, exit 134): ls-hubd has no role file for `/tmp/.../ambient-probe` (`LSHUB_NO_ROLE_FILE`), and the library
  registers on the bus (`com.webos.service.capture.client`). So the flag is not enough: the capture helper has to run as a native
  service with an LS2 role, installed with the ipk (a role file is a permission grant on the TV: Nitin's decision). The set's busybox
  `date` has no `%N`, so the first run reported "COULD NOT CONFIRM THE FLAG IS GONE" although the trap had removed it (checked by hand);
  times are whole seconds now and a literal `%N` reads back as seconds (`tests/test_ambient_slot.py`).
- Results: `docs/research/ambient/probe-<date>-<time>/`: `<scenario>.json|txt|service.log|log.txt`, `interrupts-*`, `slot-log.json`,
  `summary.txt` (frames and changes a second, interval percentiles, the reader's CPU, the last colour of each area, a verdict per scenario). Then fill "Probe results" in `ambient-result.md`.
- `tests/test_ambient_slot.py` (23) runs the generated on-set script for real (stand-in `luna-send`, the host reader on its fake library, the
  real collector): the flag is there when each reader starts and gone between windows, a stopped script and a killed script both lose it,
  frames come through; and the orchestration against a stand-in set (flag already there, failure mid-way, an install that fails, the package
  put back as found or running again, receiver already muted, the role type asked for).

## Checks run (2026-10-25, offline)

- `cd helper && ./build.sh test`: all 22 suites ok (306 lib tests) before the channel-icon change; after it `cargo test -p lumatv-helper
  --lib plugins::` 17 ok, `--test plugins` 14 ok, `--test firstparty` 5 ok; `cargo clippy --workspace --all-targets -- -D warnings` clean;
  `cargo fmt --all --check` clean for these files (it still flags SettingsSlice's `assert_eq!(owners, ...)` lines in `api.rs` and `settings.rs`).
- `cd ambient && ./build.sh test`: 94 unit + 11 interop tests ok; `./build.sh check` (fmt, clippy, aarch64 release build) ok.
- `python3 -m unittest tests.test_ambient_slot`: 16 ok. `cd tests && python3 -m unittest test_plugins_surfaces`: Drawer, Lane, Saver x3,
  Art check ok (run one by one with the other slices' work in the tree).
- Scenes, dark and light, in `/tmp/plugins-shots`: `13-notifications`, `13-notifications-empty`, `21-screensaver-clock`,
  `21-screensaver-clock-bare`, `21-screensaver-art`, `21-screensaver-art-portrait` (`21-screensaver-art-starry`), `36-home-plugin-cards`,
  `36-home-plugin-cards-focus`; looked at against `docs/mocks/shots/13-notifications.png` and `21-screensaver-*`. The last renders of the
  drawer, clock and art are after the final QML; the others are one change older (portrait art, clock-bare, the two Home lane scenes).

## Contracts

- `plugin.<id>.notifications` items: `{id, channel, channelTitle, channelIcon, revision, receivedAt, expiresAt, post}`; the two
  channel fields are what the plugin declared at `register`.
- `GET /v1/plugins` and the `plugins` topic: `plugins` (the owner's) and `firstParty` (the helper's own), both
  `{id, name, version, class, state, registered}`, first-party with `firstParty: true`.
- `welcome.device` (above). **Proposal for the shared crate** (`/data/home/LumaShow/lumashow/crates/plugin-api`, not edited here):
  add `device: Option<DeviceFacts>` to `wire::Welcome` with `#[serde(default, skip_serializing_if = "Option::is_none")]`, where
  `DeviceFacts { screen: Screen { class: String, width: u32, height: u32 }, input: Vec<String> }`, a paragraph in
  `docs/plugin-api.md` ("what the device is, so a plugin picks a layout without asking") and the same object in its welcome
  example. The helper writes it additively until then; a client that ignores unknown fields is unaffected.
- `PluginHub` lists (`list`, `connected`, `cards`, `tiles`, `notes`, `homeCards`) are assigned only when their content changes;
  `tileRows()`/`tileRow()` are PanelsSlice's contract and unchanged.
- `NoteLog.post({source, icon, title, body, time, actions})`; `NoteLog.unread`; `NoteLog.markSeen()`.

## Decisions for Nitin

- The weather and news widgets on Home keep reading the helper's `weather`/`news` topics and the saved cache: a plugin card has at
  most four 64-character facts, so a five-day forecast with icons and a news story with a picture cannot be one. The plugin
  cards for them exist (the clock too) as the contract surface other plugins and the saver's glance line use. Replacing the rich
  widgets with plain cards would lose the forecast rows.
- Hue light sync is not wired: starting a Hue entertainment stream needs an HTTPS `PUT` to the bridge (self-signed certificate),
  which the helper's fetch layer cannot do; WLED over DDP is the only output. Say if Hue is wanted before the TV slot.

## Not verified on the TV

- The saver over LG's own: it is drawn when the power state says `Screen Saver`; whether LG's saver also draws underneath, and
  whether `turnOnScreen` from the key wakes the panel as the remote's power key does.
- Every timing (first frame under 50 ms, p90 under 3 ms, the art's 1.4 s crossfade at 60 fps): `gate.py --surfaces drawer,saver`
  once the surfaces are wired.
- The gear hold (LG's 1 s Settings hold arrives as `Key_Option`), the bell, the toast subscription (`getToastNotification` is
  subscribe-only on this firmware) and the dedupe against LG's own copy of a plugin toast.
- WLED/DDP against the real strip; the capture socket against a real 32-bit capture helper (not built: `probe.c` is its core,
  `ambient-result.md` "Next, 2"; it needs an LS2 role, see the probe slot); the pointer toggle. The probe slot itself has run (above).

## Open

- The gear hold (above). The host tests of these surfaces decide on warnings of their own files only, so other files' warnings in the
  tree do not fail them.
- A cards sheet (the full plugin card from the plugin API) has no mock; chips and the drawer are what exists, `PluginCard.qml` is gone.
- Plugin settings pages (a plugin's own page in Settings, `kit/PluginHub` data) are SettingsSlice's.
