# LG-process restyles: input banners, on-screen keyboard (LgRestyles)

## Resume here

Built and tested offline, **not verified on the TV**:

- `tools/inputcommon_patch.py`: LG's input pill, OK banner (with its options key), INFO detail card and software-update bar in the
  Lucent tokens, as anchored edits to a staged copy of `com.webos.app.inputcommon`'s QML.
- `tools/inputcommon.sh`: stages that copy, mounts only the rewritten files over LG's QML directory (overlay mount, nothing in the
  root file system) and drops inputcommon's CRIU snapshot. Verbs `on`, `off`, `stage`, `status`.
- `tools/keyboard_patch.py`: LG's 600 ms "pudding" for the on-screen keyboard's window replaced by the theme's layer motion, in the
  compositor view that hosts the IME.

**The keyboard's keys are not restyled: the source is not in anything copied from the set.** maliit's key caps, language bar and
candidate list are drawn by the IME process (`/usr/sbin/MaliitServer`, plugin `libplugin-global.so` per `/var/lib/maliit/server.conf`),
and the 85 QML files PLAN.md names for it are in no copy in `lgtv/uiinv`, `lgtv/tmp-research`, `/tmp/tvsrc` or `/tmp/ose` (those hold the
compositor side only, `StarfishKeyboardView.qml`). The upstream maliit and webOS-ports keyboards on GitHub are different code. So the
look of the keys needs a capture first ("Needs the TV" 5 below); the patch for it then follows `inputcommon_patch.py`'s shape.

On the TV (2026-10-25 19:43, "On the TV" below): the QML directory is the guessed one and matches the census; the ipk does not ship
`tools/inputcommon.sh` or `tools/inputcommon_patch.py` (`lumatv inputcommon` answers "did not answer" until `tools/build-ipk.sh` lists
them); with the files put on the set by hand, stage, mount and unmount work and nothing crashes; the restyled look was not seen because
inputcommon kept running from its boot-time checkpoint and LG restored that checkpoint again after a SAM close, marker or no marker.

Next, in order:

1. `tools/build-ipk.sh`: ship the two tools (and `tools/keyboard_patch.py`, which `build_shadow` runs).
2. A start of inputcommon without its checkpoint (the images under `/mnt/lg/cmn_data/checkpoint/com.webos.app.inputcommon/`, or a
   reboot with the mount made at boot), then the pill, OK, the options key and INFO with the physical remote.
3. Capture maliit's tree ("Needs the TV" 5), so the keys can be done.

CHANGELOG line for the main agent: *Restyles of LG's own processes: the input banners (`com.webos.app.inputcommon`) in the Lucent look through an
anchored patch and an overlay mount (`tools/inputcommon_patch.py`, `tools/inputcommon.sh`), and the on-screen keyboard's open and close in the
theme's layer motion (`tools/keyboard_patch.py`); both fail loudly when LG changes the code they patch.*

## Where LG's sources are

| What | Copy | Notes |
|---|---|---|
| inputcommon QML, 273 files, 11 MB, with its images | `/data/home/lgtv/uiinv/inputcommon/qml` | made durable here from `/tmp/tvsrc/inputcommon/qml` (the only copy there was in `/tmp`). The tests and the host use the first of the two that exists. |
| `StarfishKeyboardView.qml` (342 lines), `BaseView.qml`, `KeyboardView.qml` | `/data/home/lgtv/uiinv/qml/qml/WebOSCompositor[Base]/views` | the compositor side of the keyboard; OSE's open `luna-surfacemanager` (`/tmp/ose`) has a near-identical `KeyboardView.qml` |
| maliit's QML | none | see above |

Facts found on the way (kept so they are not rediscovered):

- inputcommon is restored from a CRIU snapshot (`preload.json` lists it, `/mnt/lg/cmn_data/checkpoint` holds only it, `com.webos.app.inputcommon_criu_*.log.tgz`
  in `/var/log`). A restored process holds the QML it was dumped with: a mount alone changes nothing until the snapshot is dropped (`invalid_img`, PLAN.md
  platform findings). `inputcommon.sh` does that on `on` and on `off`, and does **not** close a running inputcommon: `hdmiN` and `livetv` run the same binary as
  their `main`, so closing the one on screen leaves the input. An input that is already up keeps its old banners until it is next launched.
- The input pill, OK banner and detail card are `SimpleBanner`, `InputBanner` and `DetailInfoBanner` under `UserInterfaceLayer/Containers/MainViewLayer/InputBanner`;
  the options key is `BannerItems/OptionButton.qml` on `Components/ImageButton.qml`; every colour, font and size is in `UserInterfaceLayer/GlobalStyle.qml`'s
  `banner` object and every font in its seven font functions, which take LG Smart UI from the font manager.
- LG's banner is 156 px high (a flat PNG bar) at `mBannerTopMargin` 562 in the banner list; the Lucent pill is 88. The list position, the owner's banner
  position setting and the detail card's place under the pill are LG's and unchanged: the mock's bottom-left is **not** done (the list stacks the detail card
  below the pill, so moving the pill down pushes the card off the screen).
- LG's own bugs, both present on the set and left alone: `ImageButton` reads `bFocusScaleEffect`, which nothing defines (its focus-scale state never runs);
  `StarfishKeyboardView.qml:230` binds `visible` to `draggableMask.isTransitioning`, a property that Item has not (the patch removes the mesh it sat on).
- The stage works on an unmounted tree: a copy of an already-overlaid directory is a copy of our own files, which the patch refuses ("already patched").

## What the input patch does

Seven files change (`GlobalStyle.qml`, `SimpleBanner.qml`, `InputBanner.qml`, `DetailInfoBanner.qml`, `OptionButton.qml`, `BannerIcon.qml`, `NsuProgressBar.qml`), and
nothing else. Only style is edited: every line of LG's behaviour (handlers, keys, interface reads and calls, focus) is the same in the patched tree
(`test_only_style_is_removed_and_every_line_of_LGs_behaviour_is_still_there`), and the host runs the same keys through both trees and gets the same luna calls and
the same focus.

- **Fonts**: Manrope in all seven font functions (so also the popups, no-signal page and the channel banner, which draw from the same sheet). The four faces are
  loaded by `FontLoader`s in the style sheet from `$STATE/fonts`, because inputcommon does not share the compositor's fonts. A missing face fails the patch (a banner in a
  fallback face is worse than LG's). LG's dingbat face stays (Dolby marks are glyphs). Text in a script Manrope lacks falls to whatever Qt picks (LG picks a face per
  locale; this set is English).
- **Pill and OK banner**: a glass pill (palette `sheetBottom`, `hairline` edge) 88 px high, 64 px off the screen edge, drawn by a rectangle that takes the pictures' own
  `visible` rules; the four PNG pieces stay, with no source. The focused input name is a lit inset (`focus` wash, `focusEdge` ring), not LG's white pill with dark text.
  The input name is a small dim line over the device label (LG sets the name small and the CEC label large; kept, sizes from the type scale).
- **Options key**: a round key with a lit ring on focus and one glyph in both states; its `DropShadow` is off.
- **Detail card**: 1100x184, sheet radius, hairline, chips as lifted tiles with ink text, the clock in ink.
- **Update bar**: sheet, light track and accent fill; its `DropShadow` is off.

Shared sheet values move the channel banner with them (fonts, ink, chips, info card colours, the progress bar); its background PNGs and layout are left to
its own restyle (SURFACES: "Channel banner", mock 20).

## Verified offline

Run from the repo root:

- `python3 -m unittest tests.test_inputcommon_patch` (20 tests: the patch on the real tree, seven files and no others, style only, no new shadow, anchors missing or doubled
  fatal for every file and all-or-nothing, twice refused, palette and fonts checked, and in the host every scene loads clean stock and patched in both materials, pill
  88 against 156, Manrope in the name and label, the same luna calls and focus for OK, Right, Left as the stock tree).
- `python3 -m unittest tests.test_inputcommon_stage` (10: staging keeps only the rewritten files and leaves LG's tree alone, a refused stage keeps the last good one, `on`
  unmounts then stages then mounts then drops only inputcommon's snapshot, a failed stage or mount leaves LG's banners and the snapshot, an overlay that will not unmount
  stops everything, `off`, status, no QML directory, usage; `mount` and `umount` are stand-in scripts).
- `python3 -m unittest tests.test_keyboard_patch` (11: the patch, the motion read from the theme and bad motion refused, only the animation changes, anchors fatal, and in the
  host on LG's own `BaseView`: it loads clean where LG's has one warning, the panel is reported the same way and the window ends in the same place, it no longer overshoots
  (stock 42 px past its place), it is quicker, and the duration follows the theme).
- Scenes, dark and light, looked at against `docs/mocks/shots/25-input-*`: `python3 tests/qmlhost/scenes.py --only 25-lg-input-pill,25-lg-input-ok,25-lg-input-options,25-lg-input-info,25-lg-input-update --out /tmp/ic-shots`.
  The renders show LG's layout in the Lucent look; they do not match the mock's pill (no lead icon tile, a one-line "HDMI 3 · PlayStation 5" title, buttons Details/Inputs/Picture): that is
  LumaHome's own banner (`panels/Banner.qml`), not LG's.
- The host (`tests/qmlhost/inputcommon.py`, `keyboard.py`) is Qt 5.15 on software GL: layout, colour, states and load errors, no timing. LG's own fonts are not on the workstation, so stock
  renders use a fallback face.

## Needs the TV (not verified on the TV)

1. **Where the QML is.** `ls -d /usr/palm/applications/com.webos.app.inputcommon/qml` (the script's first guess, [INFERENCE] from the layout of LG's other native apps); else `find / -name RootWindow.qml 2>/dev/null`;
   else `grep -i qml /proc/$(pidof com.webos.app.inputcommon | cut -d' ' -f1)/maps`. Then `find <dir> -name '*.qml' | wc -l` (273) and `md5sum <dir>/UserInterfaceLayer/GlobalStyle.qml` against the census copy.
   If the QML is compiled into the binary (a `qrc`), no mount reaches it: stop and report. Override with `INPUTCOMMON_QML=<dir>`.
2. **The snapshot.** `ls -la /mnt/lg/cmn_data/checkpoint/*`: the script touches `invalid_img` in every directory whose name contains `inputcommon`. Confirm that is the right name and that LG takes a cold start after it
   (`/var/log/inputcommon`, `com.webos.app.inputcommon_criu_restore.log.tgz` stop growing).
3. **Stage, mount, look.** `sh tools/inputcommon.sh stage`, `sh tools/inputcommon.sh on`, `grep inputcommon /proc/mounts`; from LumaHome switch to HDMI 2 (the Denon): the pill, OK (the inset on the name, Right to the options key, OK opens LG's
   options), INFO (the detail card), and a screenshot of each. Check `faultmanager` for an inputcommon crash (the compositor-restart segfault in HANDOFF is a different path). The mount does not survive a reboot: `apply`'s boot path must run `on` again (as `webglass` does), or the banners go back to LG's.
4. **Revert.** `sh tools/inputcommon.sh off`, switch input, LG's look is back. A reboot also clears the mount.
5. **maliit.** `find / -iname '*maliit*' 2>/dev/null`, `cat /var/lib/maliit/server.conf`, `ls -R /usr/share/maliit* /usr/lib/maliit* 2>/dev/null`, `strings $(find / -name 'libplugin-global.so' 2>/dev/null | head -1) | grep -i 'qrc:\|\.qml' | head`. Copy
   whatever QML there is to `/data/home/lgtv/uiinv/maliit/`. If the keys are compiled into the plugin (a `qrc`), the patch has to be a rebuilt resource, which is a bigger decision for Nitin; report what is there.
6. **Frame cost.** The pill and banners are over video drawn by inputcommon: no live shadow is added and two were removed, so none is expected; the frame gate does not cover a banner over HDMI, so this stays unmeasured.

## On the TV (2026-10-25 19:43-19:48)

Set on Home, LG's compositor, receiver muted through Home Assistant, `filmMaker`; code `4ee268a`, the ipk of that commit installed
at 19:28. Files in `docs/research/round7-2026-10-25/slot6/` and `slot6b/` (`log.txt`, captures at 960x540).

1. The QML is where the script guesses: `/usr/palm/applications/com.webos.app.inputcommon/qml`, 273 `.qml` files, and
   `UserInterfaceLayer/GlobalStyle.qml` has the census copy's md5 (`d34b51f5cc71621b2cae13a635469a8b`). The checkpoint directory is
   `/mnt/lg/cmn_data/checkpoint/com.webos.app.inputcommon/` (CRIU images `core-2513.img` and so on, dated Sep 14).
2. **The ipk does not ship the tool.** `tools/build-ipk.sh` lists neither `tools/inputcommon.sh` nor `tools/inputcommon_patch.py`, so
   `lumatv inputcommon status` on a set installed from it answers `{"ok":false,"error":"tools/inputcommon.sh did not answer"}`
   (`sh: can't open '.../tools/inputcommon.sh'` in the tool's log). For the slot the two committed files were put into the installed
   app's `tools/` and removed again at the end. With them: `status` `{"ok":true,"qml":...,"staged":false,"mounted":false}`; `lumatv
   inputcommon on` `{"ok":true,"inputcommon":"on","snapshotsDropped":1}` in 0.6 s, the overlay in `/proc/mounts` over the QML directory
   (and a second one under the `lg.thinqai.adapter` jail's view of the same path); `off` unmounted both; no crash report in
   `/tmp/var/log/reports/librdx/`, inputcommon alive throughout, `faultmanager` with nothing new.
3. **The look did not change, because the process did not restart.** `com.webos.app.inputcommon` was pid 2513, restored from the
   checkpoint at boot, long before the mount; a QML engine does not re-read files it has loaded, so the pill on the switch to HDMI 2
   (`slot6/pill.png`: "HDMI 2" over the device name, no pill box) is LG's own, the same as with the mount off (`slot7/own-pill.png`).
   Closing the app through SAM (`closeByAppId com.webos.app.hdmi2`) and launching HDMI 2 again brought **the same pid 2513 back in
   2.2 s**: LG restored the checkpoint again, with the marker the tool had left in the directory (`invalid_img`, directory mtime 19:43).
   So `snapshotsDropped 1` is not a cold start; the marker is not what LG reads, or not when the images are restored by SAM's launch.
   What would show the rewritten files is a start with no checkpoint: the images removed (LG's own `criu` tooling, or `rm` of the
   directory's `.img` files, which is a change to `/mnt/lg/cmn_data`, not the rootfs) or a reboot after the mount is made persistent
   through `apply`. Neither was done in this slot.
4. OK and INFO on HDMI 2 reach the source over CEC: with the receiver and the Apple TV behind it, OK brought the Apple TV's profile
   screen and the Denon's "SOURCE Apple TV / INPUT HDMI / SOUND Multi Ch Stereo" bar (`slot6b/ok-banner-fresh.png`), INFO showed the
   resolution chips over LG's name line (`slot6/info.png`: "16:9 2160p PCM" drawn over "HDMI 2"). LG's OK banner with the options key
   was not captured; whether SIMPLINK forwards the key before the banner or the capture missed it is open, and the captures are LG's
   look either way (3).
5. maliit: not looked at (no time in the round).

Next: ship the two tools in the ipk (`tools/build-ipk.sh`), then a slot that restarts inputcommon without its checkpoint and reads the
pill, OK and INFO with the physical remote.

## Wiring left to others (files I do not own)

- `tools/lumatv`: a verb `inputcommon on|off|status` that runs `sh "$APPDIR/tools/inputcommon.sh" "$@"` under `take_mutation_lock` and prints its JSON, an entry in the `surfaces` registry (revert: `inputcommon off`), and `inputcommon on` in the boot apply
  when the switch was on (the mount does not survive a reboot). CutoverWatchdog owns the verbs.
- The keyboard patch belongs where the shadow module is built: after the stock `views/StarfishKeyboardView.qml` is copied into `$SHADOW_MODULE`, run
  `python3 tools/keyboard_patch.py "$SHADOW_MODULE/views/StarfishKeyboardView.qml" "$THEME_SRC/theme.json"` and keep LG's file when it exits non-zero. It only matters while LG's compositor is the one running; LumaTV's own compositor
  draws the keyboard window's entry itself (compositor-c2-result.md, "IME routing").
- Both switches off by default, like `webglass`: owner switches.

## Not done

- maliit's keys (above): no source.
- The rest of inputcommon's own screens: the no-signal page and its photos (`BgViewLayer/Screensaver.qml`), the banner-position editor, the alert popups ("Input Locked", set-top box) and the Live TV channel banner share the font and the sheet's colours now, but their
  layouts and backgrounds are LG's. SURFACES.md rows 89-92 and 164-165 still stand.
- The mock's bottom-left pill with a lead icon and Details/Inputs/Picture buttons (LumaHome's `Banner.qml` has that; LG's `launchMoreActions` opens LG's own options and is kept).
