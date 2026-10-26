# LG-process restyles: input banners, Live TV, no-signal, keyboard (LgRestyles)

## Resume here

Built and tested offline, **not verified on the TV**:

- `tools/inputcommon_patch.py`: the restyle of `com.webos.app.inputcommon`, the one process behind every input and Live TV, as anchored edits to
  a staged copy of its QML (17 files, listed in "What the patches do"): the input pill, the OK banner with its options key, the INFO card and
  the update bar; the Live TV channel banner, its OK state and its INFO card; the no-signal screen; the banner position editor; the
  prompts ("set-top box?", "Input Locked"). Lucent tokens, Manrope, no live shadow.
- `tools/inputcommon.sh`: stages that copy, mounts only the rewritten files over LG's QML directory (overlay mount, nothing in the root file
  system) and makes the input apps start cold, so that the rewritten files are read: LG's CRIU dump of the app is set aside and the idle
  instances are ended through SAM ("Cold start" below). Wired as `lumatv inputcommon on|off|status` (off by default, mounted again by `apply` at
  boot). All three scripts are in the ipk (`tests/test_package_tools.py`).
- `tools/keyboard_patch.py`: LG's 600 ms "pudding" for the on-screen keyboard's window replaced by the theme's layer motion, in the compositor
  view that hosts the IME. Switch: `lumatv keyboard on|off|status` (off by default; a change is one `apply`, which rebuilds the shadow module and,
  under LG's compositor, restarts it once). The flag is part of the shadow's fingerprint, so the first `apply` after this update rebuilds the
  shadow once on every set.
- `tools/voice_patch.py`, `tools/voice.sh`: the restyle of LG's Search app (`com.webos.app.voice`, 202 QML), same shape as inputcommon's ("LG's Search app" below).
  Switch: `lumatv voice on|off|status` (off by default, mounted again by `apply` at boot). The app is resident (SAM keeps it alive, round 11), so `on` and `off` also
  end its process and the next open starts it from the files then mounted ("Search restyle: hosting"). Verified on the TV 2026-10-26: Search shows the patched look after `on` and LG's after `off`.
- `tools/maliit_patch.py`, `tools/maliit.sh`: the restyle of LG's on-screen keyboard keys (maliit: key caps, function keys, focus, candidate rows, number pad; 85 QML in
  `/usr/lib/maliit/plugins/qml`), same shape as inputcommon's ("The keyboard's keys (maliit)" below). The server reads its QML once, when it starts, so `on` and `off` restart
  `maliit-server.service` (`systemctl try-restart`, the restart LG's own `lsm-ready.service` does at every compositor start; never SAM), and a server that does not come back, or
  logs a file it could not load, gets the mount taken away again. Switch: `lumatv keys on|off|status` (off by default, mounted again by `apply` at boot), separate from `keyboard`.

**Not done:** LG's voice bar (`VoiceAgent`, source on disk) is not restyled: the `lean` profile stops the voice services that open it and LumaHome takes the mic key
(`voice-result.md`), so it cannot appear; with `lean`'s `voice` group turned back on it shows in LG's look. maliit's dictation card (`view-stt`, what the keyboard's mic
key opens) is left for the same reason.

Renders, dark and light, 960x540, in `docs/research/lg-restyles/` (`25-lg-input-*`, `20-lg-channel-*`, `18-lg-no-signal*`, `25-lg-banner-position`,
`35-lg-prompt`); regenerate with `python3 tests/qmlhost/scenes.py --only <names> --out DIR --theme both`.

Next, in order: (1) "TV check, overnight round" below; (2) "TV check, keyboard and search" below, then "TV check, keyboard keys";
(3) the owner's pick on the optional items ("Not done").

CHANGELOG line for the main agent: *Restyles of LG's own processes in the Lucent look: the input banners, Live TV's channel banner and INFO card, the
no-signal screen, the banner position editor and the set-top box / input lock prompts (`com.webos.app.inputcommon`, an anchored patch and an overlay
mount: `tools/inputcommon_patch.py`, `tools/inputcommon.sh`; `lumatv inputcommon on|off` also sets LG's CRIU dump of the input app aside so the apps
start cold), and the on-screen keyboard's open and close in the theme's layer motion (`tools/keyboard_patch.py`, `lumatv keyboard on|off`); layouts hold
at 200 % text; both patches fail loudly when LG changes the code they patch.*

## Where LG's sources are

| What | Copy | Notes |
|---|---|---|
| inputcommon QML, 273 files, 11 MB, with its images | `/data/home/lgtv/uiinv/inputcommon/qml` | made durable here from `/tmp/tvsrc/inputcommon/qml` (the only copy there was in `/tmp`). The tests and the host use the first of the two that exists. |
| `StarfishKeyboardView.qml` (342 lines), `BaseView.qml`, `KeyboardView.qml` | `/data/home/lgtv/uiinv/qml/qml/WebOSCompositor[Base]/views` | the compositor side of the keyboard; OSE's open `luna-surfacemanager` (`/tmp/ose`) has a near-identical `KeyboardView.qml` |
| maliit's QML, 85 `.qml` (164 files with its pictures and style lists) | `/data/home/lgtv/uiinv/maliit/qml` | round 11, from `/usr/lib/maliit/plugins/qml` on the set; the archive `docs/research/round9/r11-2026-10-26/maliit-qml.tgz` is in the repository, and the tests and the host use the first of the two that exists |

Facts found on the way (kept so they are not rediscovered):

- inputcommon is not started by its binary: LG's snapshot boot launched it once with `{"snapshot":true}` and dumped it with CRIU, the instances in
  the boot snapshot keep running, and every launch after a close is a CRIU restore from that dump (`/tmp/checkpoint/com.webos.app.inputcommon`, the
  persisted copy in `/mnt/lg/cmn_data/checkpoint`, which holds only it). A restored process holds the QML it was dumped with, and gets the dumped pid
  (2513) back. `invalid_img` in the persisted copy is not what stops the restore (round 7: it did not). "Cold start" below has LG's scripts, the
  processes and what `inputcommon.sh` does about it; an input that is on screen keeps its old banners until it is next launched.
- The input pill, OK banner and detail card are `SimpleBanner`, `InputBanner` and `DetailInfoBanner` under `UserInterfaceLayer/Containers/MainViewLayer/InputBanner`;
  the options key is `BannerItems/OptionButton.qml` on `Components/ImageButton.qml`; every colour, font and size is in `UserInterfaceLayer/GlobalStyle.qml`'s
  `banner` object and every font in its seven font functions, which take LG Smart UI from the font manager.
- LG's banner is 156 px high (a flat PNG bar) at `mBannerTopMargin` 562 in the banner list; the Lucent pill is 88. The list position, the owner's banner
  position setting and the detail card's place under the pill are LG's and unchanged: the mock's bottom-left is **not** done (the list stacks the detail card
  below the pill, so moving the pill down pushes the card off the screen).
- LG's own bugs, both present on the set and left alone: `ImageButton` reads `bFocusScaleEffect`, which nothing defines (its focus-scale state never runs);
  `StarfishKeyboardView.qml:230` binds `visible` to `draggableMask.isTransitioning`, a property that Item has not (the patch removes the mesh it sat on).
- The stage works on an unmounted tree: a copy of an already-overlaid directory is a copy of our own files, which the patch refuses ("already patched").

## What the patches do

Seventeen files of inputcommon's tree change (`tests/test_inputcommon_patch.py` names them; the patch prints them and `inputcommon.sh` mounts exactly those):
`GlobalStyle.qml`; `InputBanner/{SimpleBanner,InputBanner,DetailInfoBanner}.qml`; `BannerItems/{OptionButton,BannerIcon,NsuProgressBar,BannerEditItem}.qml`;
`ChannelBanner/{ZappingBanner,ChannelBanner,DetailInfoBanner}.qml`; `BannerEditView.qml`; `Components/SimpleButton.qml`; `PopupLayer/AlertPopupView.qml`;
`BgViewLayer/{Screensaver,ScreensaverMainText,ScreensaverSubText}.qml`. Only style is edited: every line of LG's behaviour (handlers, keys, interface reads
and calls, focus) is the same in the patched tree (`test_only_style_is_removed_and_every_line_of_LGs_behaviour_is_still_there`), and the host runs the same keys
through both trees and gets the same luna calls and the same focus (input OK banner, channel OK banner, the prompt's two answers, the four banner places).

- **Fonts**: Manrope in all seven font functions (so in every screen of the process). The four faces are loaded by `FontLoader`s in the style sheet from
  `$STATE/fonts`, because inputcommon does not share the compositor's fonts. A missing face fails the patch (a banner in a fallback face is worse than LG's).
  LG's dingbat face stays (Dolby marks and the source-type glyphs are glyphs). Text in a script Manrope lacks falls to whatever Qt picks (this set is English).
- **Pill and OK banner**: a glass pill (palette `sheetBottom`, `hairline` edge) 88 px high, 64 px off the screen edge, drawn by a rectangle that takes the pictures' own
  `visible` rules; the four PNG pieces stay, with no source. The focused input name is a lit inset (`focus` wash, `focusEdge` ring), not LG's white pill with dark text.
  The input name is a small dim line over the device label (LG sets the name small and the CEC label large; kept, sizes from the type scale).
- **Options key**: a round key with a lit ring on focus and one glyph in both states; its `DropShadow` is off.
- **Detail card**: 1100x184 (taller when the type is: the clock and the chips never meet), sheet radius, hairline, chips as lifted tiles with ink text, the clock in ink.
- **Update bar**: sheet, light track and accent fill; its `DropShadow` is off.
- **Channel banner** (Live TV, mock 20): the same sheet and the same lit inset on the strip and on the OK banner behind it, 64 px off the edge; the number at 84 (LG's 105),
  the station name regular 28 over the programme in semibold 34, the progress bar in the accent. The strip is 156 high at LG's type size and grows with the type (name
  above programme above bar never meet). Channel colours that carry meaning (blue for invisible, yellow for fine-tuned, orange for internet) are LG's.
- **Channel INFO card**: LG's size and layout (programme, times, synopsis, chips), the sheet, a hairline, 64 px off the edge, the programme and description rows grown with the type.
- **No-signal screen** (mock 18): LG's moving block on black, which hops every 10 s against burn-in and ends in LG's silent power-off after 860 s (both untouched), with its 12-frame
  animation removed, the header at 40 and the line under it at 26 in the palette's ink at 66 % and 55 % over black (7.9:1 and 5.6:1; a light theme draws its light sheet colour, so the
  words are light on black in both). With LG's photo setting on (`general.noSignalScreenSaver`) the words sit in a sheet on the photo, hairline and sheet radius.
- **Banner position editor**: four sheets in the pill's shape, the current one ticked, the focused one lit; the upper two off the top edge by the Home margin.
- **Prompts** (set-top box question, input lock): the sheet, ink, hairline, the sheet radius; the buttons are rows on the sheet with a lit ring when focused and no shadow; LG's image shadow is off.

Shared sheet values reach every screen of the process (fonts, ink, chips, the progress bar). What is left of LG's look: the Teletext, CI, EWS, DVR, LG Channels and region-specific
popups, none of which can appear on this set (SURFACES.md), and the red colour-key glyph in the no-signal photo box (the remote's key colour).

## Verified offline

Run from the repo root:

- `python3 -m unittest tests.test_inputcommon_patch` (the patch on the real tree, the 17 files and no others, style only, no new shadow, anchors missing or doubled
  fatal for every file and all-or-nothing, twice refused, palette and fonts checked; the no-signal words' contrast on black in both materials; and in the host every scene loads
  clean stock and patched, **no two runs of words overlap at 100 % and at 200 % text in dark and light** for every screen, the channel strip is 156 high and grows with the type, the
  keys do what LG's do (input OK banner, channel OK banner, the prompt's two answers, the four places of the banner), the no-signal block still hops in eleven seconds and its animation
  is gone, the words are light on black in both materials).
- `python3 -m unittest tests.test_inputcommon_stage` (26: `Staging` on the census tree, 10: staging keeps only the rewritten files and leaves LG's tree alone, a refused stage keeps the last
  good one, `on` unmounts then stages then mounts then sets only inputcommon's dump aside, a failed stage or mount leaves LG's banners and the dump, an overlay that will not unmount
  stops everything, `off`, status, no QML directory, usage; `ColdStart` on a stand-in set, 16: the dump renamed with every file and symlink as it was, the persisted copy byte for byte
  unchanged, the idle input apps closed through a stand-in SAM only after the dump is aside, the one on screen and look-alike ids left, a refused close or a SAM that hangs costs nothing,
  a copy already aside kept and a newer dump dropped, `off` putting it back, a restore that is still running waited for; `mount`, `umount` and `luna-send` are stand-in scripts).
- `python3 -m unittest tests.test_package_tools` (everything the tool and the boot hook call or import is copied by `tools/build-ipk.sh` and listed in its REQUIRED_FILES, the checker
  finds a script that is not), `tests.test_lumatv_inputcommon` (the `inputcommon` verb, the flag, the boot re-mount that leaves a mounted overlay alone, revert, the registry rows; the
  `keyboard` verb: `on` and `off` flip the flag and run one `apply`, a failed or silent `apply` puts the flag back, a set with no shadow module changes only the flag, status, usage; the
  keyboard patch in `build_shadow` only while the flag is on; the flag is in the shadow's fingerprint) and `tests.test_surfaces_registry` (`keyboard` is a registry row, off with no flag).
- `python3 -m unittest tests.test_keyboard_patch` (11: the patch, the motion read from the theme and bad motion refused, only the animation changes, anchors fatal, and in the
  host on LG's own `BaseView`: it loads clean where LG's has one warning, the panel is reported the same way and the window ends in the same place, it no longer overshoots
  (stock 42 px past its place), it is quicker, and the duration follows the theme).
- Compile gate on a clean checkout: `python3 tests/qmlhost/compile_all.py` says 117 compiled, 0 failed (the patches touch no file of `payload/compositor/lumatv`).
- Scenes, dark and light, 26 renders in `docs/research/lg-restyles/`, looked at against `docs/mocks/shots/{25-input-*,20-channel-banner,18-no-signal}`:
  `python3 tests/qmlhost/scenes.py --only 25-lg-input-pill,25-lg-input-ok,25-lg-input-options,25-lg-input-info,25-lg-input-update,20-lg-channel-banner,20-lg-channel-ok,20-lg-channel-options,20-lg-channel-info,18-lg-no-signal,18-lg-no-signal-photo,25-lg-banner-position,35-lg-prompt --out /tmp/ic-shots --theme both`.
  The renders show LG's layout in the Lucent look; they do not match the mocks' pill and no-signal page (a lead icon tile, a title and buttons): those are LumaHome's own banner and
  no-signal block (`panels/Banner.qml`), not LG's. LG's no-signal text in the host is a stand-in ("No Signal" and a hint); the set's strings are LG's.
- The host (`tests/qmlhost/inputcommon.py`; `--text 200` doubles every size of LG's style sheet, `--audit overlap` runs `layoutaudit.py` on the picture; `keyboard.py`) is Qt 5.15 on software GL:
  layout, colour, states and load errors, no timing. LG's own fonts are not on the workstation, so stock renders use a fallback face.

## Needs the TV (not verified on the TV)

1. **Where the QML is.** `ls -d /usr/palm/applications/com.webos.app.inputcommon/qml` (the script's first guess, [INFERENCE] from the layout of LG's other native apps); else `find / -name RootWindow.qml 2>/dev/null`;
   else `grep -i qml /proc/$(pidof com.webos.app.inputcommon | cut -d' ' -f1)/maps`. Then `find <dir> -name '*.qml' | wc -l` (273) and `md5sum <dir>/UserInterfaceLayer/GlobalStyle.qml` against the census copy.
   If the QML is compiled into the binary (a `qrc`), no mount reaches it: stop and report. Override with `INPUTCOMMON_QML=<dir>`.
2. **The cold start.** Superseded by "Cold start" below: the marker in `/mnt/lg/cmn_data/checkpoint` was tried (round 7) and is not what makes LG start the app cold.
3. **Stage, mount, look.** `sh tools/inputcommon.sh stage`, `sh tools/inputcommon.sh on`, `grep inputcommon /proc/mounts`; from LumaHome switch to HDMI 2 (the Denon): the pill, OK (the inset on the name, Right to the options key, OK opens LG's
   options), INFO (the detail card), and a screenshot of each. Check `faultmanager` for an inputcommon crash (the compositor-restart segfault in HANDOFF is a different path). The mount does not survive a reboot: `apply`'s boot path must run `on` again (as `webglass` does), or the banners go back to LG's.
4. **Revert.** `sh tools/inputcommon.sh off`, switch input, LG's look is back. A reboot also clears the mount.
5. **maliit.** Done in round 11 (the tree is copied, it is plain QML on disk and the keys are not a compiled resource): "The keyboard's keys (maliit)" below.
6. **Frame cost.** The pill and banners are over video drawn by inputcommon: no live shadow is added and two were removed, so none is expected; the frame gate does not cover a banner over HDMI, so this stays unmeasured.

## TV check, overnight round (not verified on the TV)

One slot of about 25 minutes, code = the ipk of the commit that carries this section, applied (`lumatv apply`). Set on Home, receiver muted through Home Assistant first (unmuted
only after Home is back and the quiet is confirmed), Active Standby afterwards. Commands as in round 7's `slots.py`: `luna-send` over SSH needs a PTY; `TOOL` is the app's
`tools/lumatv`, `APP` its directory, `shot NAME` is `luna-send -n 1 luna://com.webos.service.capture/executeOneShot '{"path":"/tmp/x-NAME.png","method":"DISPLAY","format":"PNG","width":960,"height":540}'`
copied back, keys are `python3 /tmp/kb.py OK|RIGHT|DOWN|BACK|358` (358 is INFO). Evidence to `docs/research/lg-restyles/tv/` (`log.txt` first, one line per step with PASS, FAIL or INFO).
Abort the slot, undoing the step in hand, on a new `failed` unit in `systemctl --failed`, a new file in `/tmp/var/log/reports/librdx/`, or a moved `aud_dsp_reset_count`.
Renders to compare with are the files named in each step (`docs/research/lg-restyles/`, 960x540: every size in the capture is half of the 1080-line size quoted).

1. **Before.** All read-only. `$TOOL version` is the commit's; `ls $APP/tools/inputcommon.sh $APP/tools/inputcommon_patch.py $APP/tools/keyboard_patch.py` lists three files;
   `$TOOL inputcommon status` and `$TOOL keyboard status` answer `"ok":true`, the first with `"mounted":false`, the second with `"on":false`; `find /usr/palm/applications/com.webos.app.inputcommon/qml -name '*.qml' | wc -l`
   is 273 and the `GlobalStyle.qml` md5 is `d34b51f5cc71621b2cae13a635469a8b`. Keep `ls -la /mnt/lg/cmn_data/checkpoint/com.webos.app.inputcommon`; an `invalid_img` with the 19:43 mtime of round 7 is
   removed (the one file the first version of the tool made). Then, with the switch off, LG's own look for comparison: launch `com.webos.app.hdmi2`, `shot own-pill` after 1.6 s; launch an input with nothing
   connected (HDMI 4, or the one the input picker shows empty), `shot own-nosignal`; Home. PASS: every item as written; any other md5 or count is a firmware change: stop and report.
2. **Stage and mount.** `$TOOL inputcommon on`. PASS: `"ok":true`, `"checkpointAside":1`, `"closed"` the idle input apps, `grep inputcommon /proc/mounts` shows an overlay over
   `/usr/palm/applications/com.webos.app.inputcommon/qml`, `find /var/lib/lumatv/inputcommon/upper -name '*.qml' | wc -l` is 17 and includes `UserInterfaceLayer/Containers/BgViewLayer/Screensaver.qml` and
   `UserInterfaceLayer/Containers/MainViewLayer/ChannelBanner/ZappingBanner.qml`. FAIL with `anchor found` in the error: LG's tree differs from the census in that file; copy the line and stop (nothing was mounted).
3. **The instance.** Ten seconds later `luna-send -n 1 luna://com.webos.applicationManager/running '{}'`, and for each input app's `processid` `ps -o pid,ppid,lstart,args -p <pid>`. PASS: no input app has pid 2513 or 2517 and
   none has `"snapshot":true` in its command line. FAIL: pid 2513 with `"snapshot":true` still running: run the ladder of "Cold start" step 6 (a, b, c in that order, each undone first) and report which one changed the pid.
4. **Input banners.** Launch `com.webos.app.hdmi2`, `shot pill` at 1.6 s and the seconds from the launch to the first frame; after 4 s `kb OK`, `shot ok`; `kb RIGHT`, `shot options`; `kb BACK`; `kb 358`, `shot info`; `kb BACK`.
   PASS against `25-lg-input-pill|ok|options|info.jpg`: a dark glass pill about 44 px high and 32 px from the left edge in the capture, Manrope (no LG grey bar, no white focus pill); on OK the name is inside a
   lit inset; after RIGHT the ring is on the round options key and the inset is gone; the INFO card is about 550 px wide, a hairline edge, the chips as small tiles. OK on the name still reaches the
   Apple TV over CEC as before (round 7: the Apple TV's profile screen). INFO: record the seconds from key to card.
5. **No-signal.** `luna-send -n 1 luna://com.webos.service.settings/getSystemSettings '{"category":"general","keys":["noSignalScreenSaver","noSignalGuideArea"]}'` (read-only) says which of the two
   looks to expect. Launch the input with nothing connected; `shot nosignal-0` at 3 s and `shot nosignal-12` at 15 s. PASS with the photo setting off: a black screen, the words "No Signal" and one hint line in dim light Manrope
   (compare `18-lg-no-signal.jpg`), no animated picture, and the block of words at another place in the second shot (LG's hop every 10 s; the same place twice is a FAIL: report whether the process was the cold one).
   PASS with it on: LG's picture with the words in a dark rounded sheet over it (`18-lg-no-signal-photo.jpg`). Either way the words are lighter than the black behind and nothing sits in LG's old grey box.
6. **Live TV and the prompt.** Launch `com.webos.app.livetv` and `shot livetv` at 3 s. If LG asks to tune channels ("Start Channel Tuning now?"), PASS is the sheet-coloured prompt with two answers and a lit ring on the
   focused one (`35-lg-prompt.jpg`); answer with `kb DOWN`, `kb OK` for the second answer (never the first: it starts tuning) or `kb BACK`. If channels exist, `kb OK` gives the OK banner (`20-lg-channel-ok.jpg`: the
   strip with the number, the station, the programme in white, a thin accent progress bar, the round options key beside it) and `kb RIGHT` the ring on that key; `kb 358` the INFO card (`20-lg-channel-info.jpg`). INFO when neither shows
   within 10 s: record what did. Then Home.
7. **Banner position editor (skip after two minutes if it cannot be reached).** It opens from LG's Settings (General) or from LG's on-screen remote ("Change Banner Location") [INFERENCE, SURFACES.md row 90]. Read
   `luna-send -n 1 luna://com.webos.service.settings/getSystemSettings '{"category":"option","keys":["bannerPosition"]}'`, open the editor, `shot banner-edit`, **`kb BACK` only** (OK would save a position), read the setting
   again. PASS: four pills as in `25-lg-banner-position.jpg`, the upper two off the top edge by about 32 px in the capture, one ticked, one lit; the setting is the same before and after.
8. **Cost.** `$TOOL fps on` before steps 4 and 5 and `$TOOL fps off` after: `grep -c FPSJANK /var/log/messages` before and after. INFO only (the frame gate does not cover a banner over HDMI): record the counts and any `FPSJANK` line from the pill or the no-signal block.
9. **Back.** `$TOOL inputcommon off`. PASS: `"checkpointBack":1`, `ls -A /tmp/checkpoint` has `com.webos.app.inputcommon` and no `.lumatv-aside.*`, the next switch to HDMI 2 shows LG's own pill (compare `own-pill`) and pid 2513 comes back.
   Then `$TOOL inputcommon on` once more and leave it on only if Nitin wants the banners; the switch survives a reboot (`apply` mounts again) but only a restart with his OK proves that (as in "Cold start" step 5).
10. **Keyboard switch.** `$TOOL keyboard status` is `"on":false,"patched":false`. `$TOOL keyboard on` (PTY, up to a minute). PASS: `{"ok":true,"keyboard":"on","applied":true,"patched":true}`,
    `grep -c 'Luma: patched' /var/lib/lumatv/qml/WebOSCompositor/views/StarfishKeyboardView.qml` is 1, `$TOOL surfaces` has `{"id":"keyboard","on":true`. Under LG's compositor `apply` restarts it once (a few seconds of
    black, Home back after); under LumaTV's compositor there is no restart and no visible change to expect: INFO, say which one was live. With LG's compositor live, open a text field of an LG app (LG Settings, Network, a
    Wi-Fi password field [INFERENCE]), `shot keyboard` at 1 s: PASS the panel stands flush at the bottom with no squashed copy of itself, and open and close take well under half a second with no overshoot (by eye).
    `kb BACK`. Then `$TOOL keyboard off`: `"keyboard":"off","patched":false`, the grep is 0, `$TOOL surfaces` has `"id":"keyboard","on":false`. Leave it as found (off).
11. **The two trees** were copied in round 11 (`/data/home/lgtv/uiinv/voice/`, `/data/home/lgtv/uiinv/maliit/`): nothing to copy.
12. **End.** Home in front, `$TOOL inputcommon status`, `$TOOL keyboard status`, `$TOOL surfaces` saved in the log, the receiver as found.

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
5. maliit: not looked at in the round; copied in round 11 (85 QML).

Next: the TV check of "Cold start" below (the scripts are in the ipk now, and `inputcommon on` does the cold start).

## Cold start of inputcommon (LG's scripts, what the tool does, the TV check)

What round 7 left open: after `inputcommon on`, SAM's `closeByAppId` and a new launch of HDMI 2 brought **the same pid 2513** back in 2.2 s with
the QML it had been dumped with. The answer is in LG's own units and scripts, copied from the set into
`/data/home/lgtv/tmp-research/census1/dc-census/x/etc/systemd/system/` (`criu.service`, `scripts/criu.sh`, `scripts/criu_copy_img.sh`,
`scripts/criu_remove_img.sh`, `snapshot-boot-components.service`), and in the census's process lists (`lgtv/uiinv/ps.txt`, `running.json`,
`listApps.json`, `docs/research/memory/before/procs.tsv`).

**What LG does**

- *Which directory.* `criu.service` runs `/usr/sbin/criu service` (`criu.sh`, after `sam.service`) with `CRIU_IMAGE_DIR=/tmp/checkpoint`: that is
  the directory a restore reads. `/mnt/lg/cmn_data/checkpoint` (`CRIU_IMAGE_SECOND_DIR`) is the **persisted second copy**: at every boot
  `criu_copy_img.sh` copies each `/tmp/checkpoint/<app>` that has a `pgid` file into it (`pgid` last, so its presence means a complete copy)
  and then turns the working copy's `pages-N.img` into symlinks to the persisted `pages-N.img.ori`, "reducing dump size of first storage(tmp)".
  `/tmp/criu_restoring` exists while a restore runs (the script waits up to 5 s for it); `criu.sh` makes `/tmp/criu_enable` only when the kernel
  command line has `criu_enabled` and `/sys/power/mode` is not `cold`.
- *`invalid_img`.* LG's scripts only write it, never read it: in the persisted directory when `md5sum -c MD5SUM` fails, in the working
  directory when the two `MD5SUM` files differ; each time the log says "will be deleted" and the `rm` is commented out. So some other
  component reads it, and nothing copied from the set says which, or in which directory [INFERENCE: SAM or the criu client]. On the set the
  marker in the **persisted** directory did not stop the relaunch of the **working** copy: the restore came back with the dumped pid (round 7,
  slots 6 and 6b; the directory's mtime was still the 19:43 of the `on` at 19:47, so nothing had consumed the marker). In snapshot-making mode
  `criu_remove_img.sh` invalidates the persisted copies by deleting their `pgid`.
- *Who restores.* The restored process is a child of SAM (pid 2500) in `procs.tsv` and in the cgroup of `sam.service`; `criu service` (2509) is
  its daemon [INFERENCE: SAM asks, `criu service` restores].
- *What is restored.* `snapshot-boot-components.service` (the snapshot-making boot) runs `luna-send luna://com.webos.applicationManager/launch
  '{"id": "com.webos.app.inputcommon", "params": {"snapshot": true}}'`. At boot the census shows two instances of
  `/usr/bin/com.webos.app.inputcommon {... "appId":"com.webos.app.inputcommon","parameters":{"snapshot":true} ...}`, pids 2513 and 2517, which
  SAM's running list gives to `com.webos.app.livetv` and `com.webos.app.hdmi2`: every app whose `main` is `file:///usr/bin/com.webos.app.inputcommon`
  (fifteen in `listApps.json`: HDMI 1-4, Live TV, `livetvopapp`, DisplayPort 1-2, AV 1-2, component, SCART, USB-C 1-2, `inputcommon`) is served by
  such an instance. [INFERENCE: they come out of the boot snapshot, since their pids are the lowest of the boot and their command line is the
  snapshot launch's; the TV resumes a prebuilt hibernation image at every cold power-on (`snapshot resume=... criu_enabled`, community notes),
  so neither whether the hibernation image or a restore at boot brought them back, nor whether `/tmp` is the image's again after a restart, was
  read on the set.] A launch after a close restores the dump with the dumped pid (`core-2513.img`, 2515, 2516, 2524 are its tasks: observed in
  slot 6b). The command line of a restored instance keeps `"snapshot":true` whatever input it is serving: that is how to tell it from a cold start.
- *Boot.* If `/tmp` is the boot snapshot's again after a restart [INFERENCE, as above], whatever is done to `/tmp/checkpoint` is undone by it, and
  so is the overlay mount: `apply` at boot has to do both again, and does.

**What `tools/inputcommon.sh on` does** (tests: `tests/test_inputcommon_stage.py`, class `ColdStart`, 16 tests on a stand-in set, and `Staging` on the census tree)

1. Unmount any overlay, stage the patched copy from LG's tree, mount it (as before; a failure leaves everything as it was and goes no further).
2. **Rename** `/tmp/checkpoint/com.webos.app.inputcommon` to `/tmp/checkpoint/.lumatv-aside.com.webos.app.inputcommon`: one atomic rename inside one
   directory, nothing copied or deleted, the page files stay the symlinks they are; LG's scripts list the directory with plain `ls` and a
   dot-name is not in it, and `*inputcommon*` does not match it. A launch finds no dump and starts the binary. (It waits up to 5 s for
   `/tmp/criu_restoring`, as LG's script does.) A dump found at the original path while a copy is already aside was made with the overlay mounted
   and is removed; LG's own stays aside.
3. **Close the idle instances**: SAM's `running` list is read, and each of the fifteen apps that is running and is not the foreground app gets
   `closeByAppId`. The preload manager loads a closed preloaded app again at once, now cold; the app on screen is left (closing it would leave
   the input) and keeps LG's banners until it is next launched. This is after step 2, so the reload cannot be a restore. The reply is
   `{"ok":true,"inputcommon":"on","checkpointAside":1,"closed":[...],"onScreen":"..."}`.
4. `off`: unmount, put the dump back (a dump made meanwhile is removed first), close the idle instances again (the reload restores LG's);
   only when there was something to undo. `status` adds `"checkpoint":"present|aside|none"` and `"criuEnabled"`.

What it never does: touch `/mnt/lg/cmn_data/checkpoint` (LG's persisted copy; the working copy's page files point into it, and a missing target
would break every restore until the next making boot) or write `invalid_img` anywhere. The marker the first version left in that directory
(the directory's mtime is 19:43 on 2026-10-25) is probably still there on the set; nothing known reads it, and step 1 below removes it.

**TV check (not verified on the TV).** Set on Home, LG's compositor, receiver muted, Active Standby afterwards; one slot.

1. *Before.* `ls -la /mnt/lg/cmn_data/checkpoint/com.webos.app.inputcommon | head -20` and keep the listing; if `invalid_img` is there with the
   19:43 mtime of round 7 remove it (`rm`; it is the one file the first version of the tool made); if its time is another, leave it and report.
   `ls -la /tmp/criu_enable /tmp/checkpoint /tmp/checkpoint/com.webos.app.inputcommon | head -30` (expected: the enable file, the dump with
   `pgid`, `core-*.img` and `pages-*.img` symlinks into the persisted copy). `luna-send -n 1 luna://com.webos.applicationManager/running '{}'`
   for the input apps and their `processid`; for each `ps -o pid,ppid,lstart,args -p <pid>` (expected for a snapshot instance: ppid 2500,
   `"snapshot":true`).
2. `lumatv inputcommon status` then `lumatv inputcommon on` (with a PTY; `luna-send` needs one over SSH). Expected: `"checkpointAside":1`,
   `closed` the idle input apps found in 1, `onScreen` empty; `ls -A /tmp/checkpoint` shows `.lumatv-aside.com.webos.app.inputcommon` and no
   `com.webos.app.inputcommon`; `status` says `"checkpoint":"aside","criuEnabled":true`; `/proc/mounts` has the overlay; the persisted directory's
   listing is byte for byte the one of 1.
3. *The instance.* Ten seconds later `running` again: if HDMI 2 is preloaded it is back with a **new pid** (neither 2513 nor 2517) and a `ps` command
   line without `"snapshot":true`, start time now. Then switch to HDMI 2 from Home and read: the pill (Lucent glass, 88 px, Manrope), OK with the
   options key, INFO, the three captures; pid and command line of the instance behind it (`"appId":"com.webos.app.hdmi2"` in it is a cold start);
   seconds from the key to the first frame (a cold start of a Qt app is slower than the 2.2 s of a restore; record it, Nitin keeps the inputs
   preloaded for speed); `/tmp/criu_restoring` absent; `grep -i 'criu\|checkpoint' /var/log/messages | tail -20` for SAM's side.
4. *Both ways.* `lumatv inputcommon off`: `"checkpointBack":1`, `ls -A /tmp/checkpoint` has the original name again and no aside, the idle
   instances closed; the next switch to HDMI 2 gives LG's banners and **pid 2513 again** (the restore of LG's dump). Then `on` and the cold start once more.
5. *Boot.* With the switch on (`lumatv inputcommon on` keeps `.inputcommon-on`), after Nitin's OK for a restart (the 2026-10-25 approval covered the
   logo check's): the tool's log has `inputcommon on: {...}` from the boot `apply` within a minute, `/tmp/checkpoint` has the aside again (the
   boot snapshot brought the dump back) and the first switch to HDMI 2 shows the Lucent banners.
6. *If step 3 still shows pid 2513 with `"snapshot":true`:* the restore does not read the working directory by that name, or SAM remembered
   the dump. In this order, each undone before the next, none across a restart: (a) read SAM's lines in `/var/log/messages` and the process's
   `ppid`; (b) rename the persisted directory away for one launch (`mv /mnt/lg/cmn_data/checkpoint/com.webos.app.inputcommon
   /mnt/lg/cmn_data/checkpoint/.aside`, switch to HDMI 2, `mv` it **straight back**: until then the working copy's page files dangle and a restart
   would copy from dangling links); (c) `rm /tmp/criu_enable` for one launch (CRIU off for every app) and `touch` it back. Report which of them
   changed the pid; the tool then does that one instead (or as well).

Open: whether SAM dumps the app again after a cold launch (then a dump taken with the overlay mounted would be restored later, patched:
fine, and the tool removes it at `off`); whether the preload manager reloads a closed `hdmi2` at once or on the next screen-on; whether ending an
idle Live TV instance has side effects; the 5 s wait for `/tmp/criu_restoring` may be too short on a loaded boot.

## Wiring (done in `tools/lumatv`, `tools/build-ipk.sh`)

- `lumatv inputcommon on|off|status` runs `sh "$APPDIR/tools/inputcommon.sh" "$@"` under the mutation lock and prints its JSON (`status` with `"on"` in
  front); the flag `.inputcommon-on`, off by default and changed only when the tool says ok. The `surfaces` registry has the row `inputcommon` (way back:
  `inputcommon off`). `apply` mounts again at boot while the flag is on and the overlay is not in place, and leaves one that is (`on` ends the idle input
  apps, which `apply` must not do at every launch); `revert` turns it off.
- `build_shadow` runs `python3 tools/keyboard_patch.py` on the shadow module's copy of `views/StarfishKeyboardView.qml` with the shipped theme's motion, while the flag
  `.keyboard-on` is there (`lumatv keyboard on|off|status`; off by default; the flag is also in the registry as `keyboard` and `revert` removes it); LG's file stays when the
  patch refuses (an anchor missing, no usable motion), with a line in the log. `keyboard on|off` flips the flag and runs one `apply` when the shadow module exists (a set that was never
  applied only gets the flag), puts the flag back when that `apply` fails, and answers `{"ok":true,"keyboard":"on","applied":true,"patched":true}`. It matters only while LG's compositor is the live one.
- The shadow module records `.shadow-patch`, a fingerprint of `SHADOW_PATCH_FILES` (the patch script, the two modules it imports and the theme), and `shadow_current`
  and `apply`'s `compositor_changed` compare it. Before, a set that already had a shadow kept it through an update that changed only a patch or the theme's motion, so
  the keyboard patch reached a set only when LumaHome's QML changed in the same update. The first `boot-bind` or `apply` after this update rebuilds the shadow once and
  restarts LG's compositor once so that it loads it; under LumaTV's compositor (it draws none of the module) `boot-bind` rebuilds without a restart. A patch added to
  `build_shadow` appends its files to the list, and a test fails when one is missing.
- `tools/build-ipk.sh` copies `inputcommon.sh`, `inputcommon_patch.py` and `keyboard_patch.py` (and lists them, with `tileicons.js`, `systemui.py` and
  `systemui_patch.py`, which were copied and not listed, in REQUIRED_FILES). `tests/test_package_tools.py` follows what the tool, the boot hook and
  every shipped script call or import and fails when any of it is not copied and listed.

## LG's Search app (`com.webos.app.voice`)

Source: `/data/home/lgtv/uiinv/voice/qml` (202 `.qml`, copied from `/tmp/tvsrc/apps/com.webos.app.voice/qml`; the app's pictures, `Views/images/`, are not in it).
The app is one native process: `Main.qml` is its root, `Style.qml` its style sheet (about 190 colours, in objects by screen), `Views/` its screens. Under `lean`
LG's voice services and cloud search are stopped, so the screens a user can reach are the search field with its keyboard, the keyword rows, the option popup,
the confirm and notice popups; result cards and the voice screens (Weather, IoT, Voice ID, Sports) are only restyled through the shared style sheet.

`tools/voice_patch.py` (14 files: `Style.qml`, `Main.qml`, `Views/{SearchView,MainView,UtteranceUIView}.qml`, `Views/SearchView/{SearchInputBox,NotiView}.qml`,
`Views/ContentsList/KeywordsRec.qml`, `Views/Components/{ButtonFlex,ButtonIcon,FocusableText,FocusableSwitch,FocusableSwitchWithText,MiniBg}.qml`,
`Views/PopupView/optionPopup.qml`), style only:

- **A role for every colour of the style sheet.** `voice_patch.py` holds a table (sheet, ink, dim ink, hairline, lit fill, ring, accent, danger) and rewrites every
  quoted colour of `Style.qml` by it. A colour with no role, or a role for a colour that is gone, is fatal: a firmware update that adds a colour must be looked at.
  Kept on purpose: the black behind thumbnails, the IP channel's orange, the web tile's blue, the select pink and `transparent`.
- **Manrope for Latin locales**: `reloadFontFamily` takes the four families from LG's language table; each goes through `lumaFamily`, which gives Manrope when the
  locale's class is Latin and LG's family otherwise (this set is English). The faces are loaded by `FontLoader`s in the style sheet; `Main.qml`'s four defaults are Manrope too.
- **One sheet under everything.** LG draws white words over 60-85 % black; with ink that follows the theme the sheet is whole, on every theme (the full-screen field, the
  drawer's dim, the option popup's backdrop). The drawer's black gradient picture is dropped.
- **Focus is a lit fill with a ring**, not LG's white box with dark text: the keyword chips, the buttons (`ButtonFlex`), the popup rows, the picture buttons (round, ring on focus; the
  glyph picture that reads on the sheet is used in both states, as in inputcommon), the search field (a pill: a wash at rest, the lit fill and a ring focused). The grey
  `7D848C` chips are the hairline wash.
- **No live shadow**: the seven `DropShadow`s of the files above are off; the pads that LG only drew as the source of a shadow (`visible: false`) are drawn.
- The AI keyword dot and its text are the accent; the "related" highlight and the utterance's highlight colour are the accent.

Host: `tests/qmlhost/voice.py` (`--scene search|search-focus|auto|keywords|noti|option|confirm`, `--patched`, `--light`, `--text 200`, `--audit`) loads LG's own `Style.qml`,
`StringSet.qml`, `GlobalVars.qml`, every `Services/*.qml` wrapper and the views, with `main`, `mainView`, `models`, `inputManager`, `voiceHintManager`, `ttsManager`, `util`
and `Service` as stand-ins (`Service` answers LG's search server with four popular keywords and four auto keywords). Pictures are not drawn (not in the copy), so the three
picture buttons show nothing; the close, more and profile buttons' places are visible by their rings only. Scenes: `tests/qmlhost/scenes_voice.py`
(`09-lg-search*`, dark and light in `docs/research/lg-restyles/`; `*-200` are 200 % text). LG's own bugs in the host, left alone: `SearchView.qml:287` (`KeyNavigation.up` returns
nothing while the dim is down) and the auto-keyword delegates reading their `ListView` while the model is cleared.

`tools/voice.sh` is `tools/inputcommon.sh` with another tree's names (the script reads `IC_NAME`, `IC_ID`, `IC_ROOT_FILE` (`Main.qml`), `IC_STYLE_FILE` (`Style.qml`), `IC_PATCH_TOOL`,
`IC_ASIDE` (`.lumatv-aside-voice.`), `IC_DUMP_GLOBS`, `IC_APPS`, `IC_BINARY`, `VOICE_QML`, `VOICE_BINARY`): the overlay is named `voice`, the stage directory is `$STATE/voice`, a dump named
`com.webos.app.voice` would be set aside and nothing else (`voiceweb` and `voiceagent` are not matched, nor is inputcommon's dump; the set has none), and the app's process is ended after the
mount or the unmount (next section). The QML is at `/usr/palm/applications/com.webos.app.voice/qml` (round 11: 203 `.qml` files, `Style.qml` and `Main.qml` with the census copy's md5s);
`VOICE_QML` overrides.

## Search restyle: hosting

Round 11 found `lumatv voice on` without effect: `closeByAppId` left pid 8321 where it was and a later launch showed LG's look. The app is not hosted in another process and not restored from a CRIU
dump. It is one resident native process that SAM keeps alive, and it reads its QML once, when that process starts; a mount shows only in a process started after it.
Evidence: `docs/research/round9/r11-2026-10-26/` (`log.txt`, `voice-host.txt`, `search-*.png`).

- **Resident.** `keepAliveApps` in `com.webos.applicationManager` lists `com.webos.app.voice` in every layer of the census. SAM answers `closeByAppId` with `returnValue true` and [INFERENCE: SAM's own lines for a close
  were not captured] sends `close` through `registerApp`; the app's `Main.qml` answers that event with `requestDeactivateApp("close")`, `clearApp(true)` and the C++ `VoiceView::hideView(bool killProcess)` (names from the
  binary's strings), which hides the window and leaves the process. Pid 8321 stood through every close of the round (04:05:09 to 04:06:11). The memory round (2026-10-25, `memory-result.md`) found that SIGTERM ends it.
- **No shared host.** The process runs `/usr/bin/com.webos.app.voice` (`ps` shows `com.webos.app.v`, the name cut to 15 characters), SAM lists it as `native` with that pid, and the mount table has no jail
  for it. `RelaunchAsCommon` in SAM's log is, by the lines around it (`relaunching` with `processId` 8321, then the app's own `voice_app_registerApp ... "event":"relaunch"`), the handler for a native app whose process
  is already there: it sends the `relaunch` event through `registerApp` and starts nothing.
- **No dump.** `/tmp/checkpoint` held only inputcommon's dump (04:23) and `voice status` says `"checkpoint":"none"`. LG's preload list has the app as `oneshot` and `partial`, not `criu`; with `lumatv preload on`
  it is `isEnabled:false`, so the process of round 11 was started by the round's own first launch (04:05:03; LG's Search on screen within 3 s).
- **The relaunch after a close is dropped.** `SystemSettings.qml` ignores any `registerApp` event except `pause` and `close` that comes less than 300 ms after the previous one (`_IGNORED_KEY_INTERVAL`); the log's
  `{"key interval":266}` is that. The round launched Search straight after the closes of `voice on` and `voice off`, so `search-on.png`, `search-down.png` and `search-restored.png` show Home, not Search in either look.
  `search-recovery.png` (a launch six seconds after a close, the mount already off) shows LG's look from the same process.
- **Launch class.** `type native`, `privilegedJail false`, `trustLevel default`, lifecycle version 2, `main` a binary in `/usr/bin`: the class of the input apps, whose cold start showed the overlay in the same
  round (`restyle-pill.png`, `restyle-ok.png`). A process started after the mount is therefore expected to read the patched files.

`voice on` unmounts any overlay, stages, mounts, closes the app through SAM (it hides and merges its history), waits a second, ends its process with SIGTERM (SIGKILL after three seconds if ignored) and waits up
to three seconds for SAM to stop listing the pid. The next open, or LG's preload manager when its policy is on, starts the app from the patched files: a cold start. `voice off` unmounts first and ends the process
again, so that LG's files are what the next start reads. An overlay that is busy (the process holds a file of it) is let go by ending the process and unmounting again. Processes are found by the first word of
`/proc/<pid>/cmdline`, which has to be the binary: `comm` is shared by any `com.webos.app.voice*` binary and only picks candidates. Nothing else is signalled, SAM is not touched and nothing is written under
`/usr`. An app SAM names as the foreground app is left, as for the input apps; Search is an overlay, which `getForegroundAppInfo` never names (`home` with Search open, round 11), so it is ended wherever it is.
An app that is not running costs nothing: the mount is enough. `status` lists the pids, `on` and `off` say `ended`. `tests/test_voice_stage.py`, class `VoiceResident`, runs all of it on a fake `/proc` and once
on real processes.

Verified on the TV (2026-10-26 04:53, ipk of commit 6afef85, which holds this commit's scripts; `voice-check.txt` and `voice-own`, `voice-search`, `voice-down`, `voice-reopen`, `voice-restored.png` in the round 11 directory): Search opened the old way
was pid 13642 (parent pid 2500, SAM's in the census; `SigCgt` 0x84e8, so SIGTERM is not caught). `voice on` took 1.9 s and answered `"closed":["com.webos.app.voice"],...,"ended":["com.webos.app.voice"]`, and SAM then said
`launchable`. The next launch was pid 14126 and drew the patched look (pill field, hairline chips, dark sheet, Manrope; maliit's keyboard unchanged); its mountinfo has the `voice` overlay over the QML directory, and a second open
after a close was the same pid, patched, at once. `voice off` took 1.4 s and the next launch (pid 14543) was LG's look again. Failed units, crash reports and DSP resets were as before. The binary is a 30 kB launcher that builds
`applicationDirPath + "/qml/Main.qml"` (no resource shadows the overlay). With the keyboard up `kb DOWN` moves the keyboard's key focus (g to v), not the chips: the lit chip, the typed auto keywords and the option popup are not yet looked at on the TV.

## TV check, keyboard and search (search: done in round 11, see above; keyboard: not verified on the TV)

One short slot, after "TV check, overnight round". Set on Home, receiver muted through Home Assistant, evidence to `docs/research/lg-restyles/tv/` (`log.txt` first, PASS/FAIL/INFO a line).
Renders to compare with: `09-lg-search*` (960x540).

1. **Where the QML is.** Found in round 11: `/usr/palm/applications/com.webos.app.voice/qml`, 203 `.qml` files (the census copy has 202), `Style.qml` and `Main.qml` with the census copy's md5s
   (`9319a9d40bde81ea919d713f3fdd9e50`, `5b49a4d9e6f1309799814623ee3c09dd`), and the patch applied. After a firmware update: `find / -name SearchInputBox.qml 2>/dev/null` and `VOICE_QML=<dir>`; `ls <dir>/Views/images | head`.
2. **Before.** `$TOOL voice status` answers `"ok":true,"on":false,"mounted":false,"checkpoint":"none"` and `"pids"` (empty until Search has been opened since boot); `$TOOL surfaces` has `"id":"voice","on":false`;
   `ls $APP/tools/voice.sh $APP/tools/voice_patch.py` lists both. LG's own Search for comparison, which also starts the resident process whose end step 3 checks: launch it
   (`luna-send -n 1 luna://com.webos.applicationManager/launch '{"id":"com.webos.app.voice"}'`, PTY), `shot own-search` at 4 s, `kb OK` on the field for LG's keyboard, `shot own-keyboard`, `kb BACK` twice, `closeByAppId` it and
   wait 2 s (the app drops a launch that comes less than 300 ms after a close). `$TOOL voice status` now has its pid in `pids`; `ps -o pid,ppid,lstart,args -p <pid>` for the record.
3. **On.** `$TOOL voice on` (PTY; about 3 s when Search's process has to be ended). PASS: `{"ok":true,"voice":"on",...,"closed":["com.webos.app.voice"],"onScreen":"","ended":["com.webos.app.voice"]}` (`"ended":[]` when Search was not open),
   `grep ' overlay ' /proc/mounts` shows the overlay over that directory, `$TOOL voice status` has `"pids":[]`, no new `failed` unit and no new file in `/tmp/var/log/reports/librdx/`. FAIL with the old pid still in `pids`: read the tool's
   log (`ignored SIGTERM`, `still running`). Launch Search and `shot voice-search` at 5 s (a cold start, a new pid in `running`):
   the field is a pill (hairline, `09-lg-search.jpg`), Manrope, the sheet dark with nothing white. `kb DOWN` moves the keyboard's key focus while it is up (round 11: g to v): the lit chip is INFO. Type through LG's keyboard (it is maliit's, unchanged): the auto keywords
   appear under the field as hairline chips (`09-lg-search-auto.jpg`). The microphone/account buttons show LG's pictures on the sheet: INFO what they look like (the pictures were not in the copy).
   Right to the more key, OK: the option popup (`09-lg-search-option.jpg`): a sheet with a hairline, the switch in the accent. Quit with its last row or `kb BACK`. Close the app (`closeByAppId`), wait 2 s and launch it again: the same
   pid opens at once, still in the patched look.
4. **Light.** None needed on the set: the sheet is always the dark material's.
5. **Off.** `$TOOL voice off` (PTY): `"voice":"off"`, `"ended":["com.webos.app.voice"]` after step 3, the overlay gone, `pids` empty. The next launch (a cold start, `shot own-search-restored` at 5 s) shows LG's look (compare `own-search`) in a process with another
   new pid; `$TOOL surfaces` has `"id":"voice","on":false`. Close it. Leave it as found (off).
6. **Keyboard switch** is step 10 of "TV check, overnight round"; the keys' own switch is "TV check, keyboard keys".
7. **End.** Home in front, `$TOOL voice status`, `$TOOL inputcommon status`, `$TOOL surfaces` in the log, the receiver as found.

Offline checks that back the steps: `python3 -m unittest tests.test_voice_patch tests.test_voice_stage tests.test_lumatv_inputcommon tests.test_package_tools tests.test_surfaces_registry`.

## The keyboard's keys (maliit)

Source: `/data/home/lgtv/uiinv/maliit/qml` (85 `.qml`, round 11, from `/usr/lib/maliit/plugins/qml`). `MaliitServer` (`maliit-server.service`) runs LG's plugins: the Latin one (`libplugin-global.so`, every language this set types) runs
`view-global/main.qml`, which imports the shared `keyboard-common/` (the keys, the three function areas, the focus box, the style sheets); `view-chinese/` and `view-japanese/` are the other two plugins, `view-stt/` the dictation card.

Facts found on the way (kept so they are not rediscovered):

- **The keys are plain QML on disk.** Every view's `main.qml` imports `../keyboard-common`, which a compiled resource cannot give it; the only resource is the pictures (`qrc:///images-2k/...`, listed by the tree's `.qrc` files; `common.qrc` and `japanese.qrc`
  are build leftovers). The patch needs no rebuilt resource.
- **The server loads its QML once and keeps it.** Its own line (`MaliitServer ... IME default ... @@@@ root.foregroundAppId <app>`, written by `main.qml`) is in the logs of rounds 9 and 10 at every change of the foreground app, with no keyboard on screen: the server runs
  all the time. A mount is read after a restart of the server. LG restarts it itself: `lsm-ready.service` stops and starts `maliit-server.service` every time the compositor comes up (the unit is in the census), so `systemctl try-restart` of that one unit is
  LG's own move, and `maliit.sh` never speaks to SAM. Under LumaTV's compositor the census found the unit inactive and maliit unrouted (phase 6): `keys on` then mounts and restarts nothing, and the keys are read when the unit next starts.
- **Sizes.** `Screen` picks the resolution (4K 1, 2K 2, HD 3) and every number of the sheets is `N / resolution`: at 1080p a letter cell is 87 x 66 px with a 39 px label, the panel 1920 x 438 (and 90 more for the candidate row). Every label is
  `fontSizeMode: Text.Fit`, so a larger type shrinks into its key; the grid is not touched.
- **Focus is one rectangle.** `FocusPanel` is moved over the focused key (15 x 12 px larger than it), draws the key's words and glyph again in its own fill (opaque white with dark words, over a DropShadow layer), and the key itself looks the same focused or not.
  The pictures come in pairs, a light one and a dark `_f` one drawn for the white box.
- LG's own faults in the stock tree, on the set too and left alone (the host counts them, `[known] 18`): `ButtonBase.radius` reads a style that has none, the language key's TTS string and the focus box's `focusedItem` are assigned `undefined`, the page-count row reads
  `fontfamily`, which nothing defines. The hardware-keyboard bar's chip put its F-key label over its word (LG's Chinese chip already fits the word): fixed here.

**`tools/maliit_patch.py`** (38 files, `tests/test_maliit_patch.py` names them; style only):

- *Colour roles.* Every quoted colour of the 33 style sheets is looked up in a table by its object path (`normal.bgColor`, `hid.hidPredItem.normal.fgColor`); a colour with no role, or a role whose colour is gone, refuses the whole tree. Each colour written is an opaque composite of
  the palette: the panel is the opaque sheet (a keyboard is words over whatever is behind, and an opaque sheet makes each contrast a number the picture cannot change), the character keys the lifted tile (the hairline wash on the sheet) with a hairline edge, the function keys the
  recessed one (the row wash), the candidate row a well with tile chips, Search and Enter the accent with its ink, and the shift and accent-letter marks the accent when on (taken toward the ink until it reads 3:1 on the focus fill, which the light material needs).
- *Focus.* A lit tile (the focus-edge wash over the sheet), a 3 px ring in the ink, the same ink, no shadow (`layer.enabled: false`). The glyph pictures are the one of each pair that reads on the sheet, in both states (`getImage`: the dark twins on a light sheet).
- *Type.* Manrope Medium (57) on the keys; `setDefaultFont` goes through LG's language table (`languageClass === "latin"`) and other scripts keep LG's face; the four faces are loaded by FontLoaders in the style sheet (`$STATE/fonts`); the literals that name LG Smart UI are Manrope.
- *Shape.* The corner is the palette's row radius, the edge one pixel (LG's is two); LG's sizes and offsets stay, so keys are readable at 10 feet as they were (39 px labels) and the focus box is lifted 15 x 12 px as before.
- *Not touched:* anything LG's code does (every handler, key, signal, service read and focus line is the same in the patched tree), the plugins' layouts, `main.qml` files and `view-stt`.

**`tools/maliit.sh`** is `inputcommon.sh`'s overlay (stage in `$STATE/maliit`, mount over `/usr/lib/maliit/plugins/qml`, `MALIIT_QML` overrides) with the start of a service in place of the cold start of an app: `on` unmounts, stages, mounts, runs `systemctl try-restart maliit-server.service`,
watches the unit for five seconds and reads what `MaliitServer` logged since the restart. A unit that is not active afterwards, or a log line that says a file of the keys did not load (`Type X unavailable`, `is not a type`, a syntax error), takes the mount away again, brings the unit up on LG's QML
(`reset-failed` and `restart` for a failed one), answers `ok:false` and leaves `lumatv`'s flag off: a keyboard that cannot open is the one fault that leaves a TV with no way to type. `off` unmounts and restarts, only when there was a mount. A unit that is not running stays not running.
Wiring: `lumatv keys on|off|status`, the flag `.keys-on` (off by default), the registry row `keys`, `apply` mounts again at boot while the flag is on and the overlay is not, `revert` turns it off; `keyboard` (the window's motion: a shadow-module rebuild and one compositor restart) is a separate switch.
`tools/build-ipk.sh` copies both files and lists them in REQUIRED_FILES.

Verified offline, from the repo root:

- `python3 -m unittest tests.test_maliit_patch` (22 on the census tree: the files named and no others, every colour a role of the palette, behaviour lines identical, no live shadow, every pair reads 4.5:1 (words) and 3:1 (ring, marks) in both materials, anchors missing or doubled fatal and all-or-nothing,
  a colour or a role that LG adds or loses fatal, twice refused, palette and fonts checked; and 12 in the host: every scene loads clean stock and patched in both materials with the same count of LG's own warnings, all 85 files of the three plugins compile as they did stock, Manrope in the sheet,
  the focus box lit and ringed and opaque and without a shadow layer, Search in the accent,
  **the keys send what LG's send** (a letter, a shifted letter, Search, delete, Clear all, space, a symbol, the number pad: the same lines stock and patched), the shift mark, **no two runs of words overlap or are cut at 100 % and 200 % text, dark and light**, and every run reads 4.5:1 in the picture's own pixels).
- `python3 -m unittest tests.test_maliit_stage` (15: stage keeps only the rewritten files, a refused patch leaves the last stage and restarts nothing, `on` unmounts then stages then mounts then restarts the one unit, only that unit is ever called and SAM never, an inactive unit is left inactive,
  a unit that does not come back or a log line about a file that did not load rolls back and brings the unit up, LG's own warnings and lines from before the restart are not a failure, `off` restarts only after a mount, status, no QML directory, usage; `mount`, `umount`, `systemctl` are stand-in scripts),
  `tests.test_lumatv_keys` (the verb, the flag, apply's boot re-mount, revert, dispatch, the verb never touching a unit itself), `tests.test_surfaces_registry` (`keys` is a registry row), `tests.test_package_tools`.
- The host `tests/qmlhost/maliit.py` (`--scene letters|shift|symbols|numbers|numbers-symbols|candidates|candidates-focus|hid|focus-*`, `--patched`, `--light`, `--text 200`, `--audit`) runs LG's `view-global/main.qml` the way the plugin does, with stand-ins for `PluginProxy`, `Prediction`, `PmLog`, the luna `Service`
  (the five calls `main.qml` makes), the `com.meego.maliit` enums and LG's language library; the library's absolute path and the `qrc:///images-2k/` pictures are redirected in the staged copy as text edits. Every time of a run counts from the moment the keyboard stands (the focus box has landed), not from the start, so a loaded machine cannot
  move a key press before the keyboard is up. Qt is 5.15 on software GL: layout, colour and states, no timing.
- Renders, dark and light, 100 % and 200 % text, 960x540, in `docs/research/lg-restyles/` (`09-lg-keys`, `-focus-search`, `-focus-shift`, `-symbols`, `-numbers`, `-candidates`, each with `-200` and `-light`), looked at against `docs/mocks/shots/09-search-keyboard.png` and `keyboard-own.png` (LG's, round 11):
  `python3 tests/qmlhost/scenes.py --only 09-lg-keys,09-lg-keys-focus-search,09-lg-keys-focus-shift,09-lg-keys-symbols,09-lg-keys-numbers,09-lg-keys-candidates --out DIR --theme both` (and the `-200` names).

## TV check, keyboard keys (not verified on the TV)

One short slot, after "TV check, keyboard and search"; it needs Nitin's go like the keyboard switch's compositor restart (`keys on` restarts `maliit-server.service`, the keyboard's own process: no app, no SAM). Set on Home, receiver muted through Home Assistant, evidence to
`docs/research/lg-restyles/tv/` (`log.txt` first, PASS/FAIL/INFO a line), `TOOL`, `APP`, `shot` and `kb` as in "TV check, overnight round". The keys are drawn by LG's compositor's keyboard view: `$TOOL compositor status` says which compositor is live; under LumaTV's own (maliit unrouted, phase 6)
there is nothing to see and only steps 1, 2 and 8 apply. Renders to compare with: `09-lg-keys*` (960x540). Abort the slot, undoing the step in hand (`$TOOL keys off`), on a new `failed` unit in `systemctl --failed`, a new file in `/tmp/var/log/reports/librdx/` or a moved `aud_dsp_reset_count`.

1. **Before.** All read-only. `$TOOL version` is the commit's; `ls $APP/tools/maliit.sh $APP/tools/maliit_patch.py` lists both; `$TOOL keys status` answers `"ok":true,"on":false` with `"mounted":false` and the unit's state; `systemctl is-active maliit-server.service`, `systemctl show -p MainPID maliit-server.service`
   (keep the pid), `pidof MaliitServer`; `find /usr/lib/maliit/plugins/qml -name '*.qml' | wc -l` is 85 and `md5sum` of `keyboard-common/style/Style.qml` is `5dc6e9bbd84af0b846b0c0b16fb2b866`, of `view-global/main.qml` `5b52f8f4826d140611a201a9d506062e` (any other count or md5 is a
   firmware change: stop and report). If the keyboard is up and the unit is not `active`, the server is not that unit: INFO `ps -ef | grep -i [m]aliit` and stop. LG's own look for comparison: launch the Search app (`luna-send -n 1 luna://com.webos.applicationManager/launch '{"id":"com.webos.app.voice"}'`, PTY),
   `kb OK` on the field, `shot keys-own` at 1 s (`keyboard-own.png` of round 11 is what to expect), `kb BACK` twice, Home.
2. **On.** `$TOOL keys on` (PTY; about 8 s: the restart and five seconds of watching). PASS: `{"ok":true,"keys":"on","restarted":true,"unit":"active"}`; `grep ' overlay ' /proc/mounts` has `maliit ... /usr/lib/maliit/plugins/qml`; `find /var/lib/lumatv/maliit/upper -name '*.qml' | wc -l` is 38;
   `systemctl show -p MainPID maliit-server.service` is a new pid; `grep MaliitServer /var/log/messages | tail -5` has a `root.foregroundAppId` line from after the restart (the patched `main.qml` is running) and none with `unavailable`, `is not a type` or `Syntax`; nothing new in `systemctl --failed`.
   FAIL with `"ok":false`: the tool already took the mount away and brought the server up on LG's QML; copy `error` and the log lines, `$TOOL keys status` must say `"mounted":false`, stop. FAIL with `refused` in `/var/lib/lumatv/log` (an `anchor found 0 times` line): LG's QML differs from the census in that file; copy the line, stop (nothing was mounted or restarted).
3. **The keys.** Search app, `kb OK` on the field, `shot keys-letters` at 1 s; record the seconds from the key to the first frame (INFO: the server was just restarted). PASS against `09-lg-keys.jpg`: an opaque dark panel (no LG grey-blue), four rows of rounded tiles in Manrope Medium, the function keys darker with a hairline,
   **`g` lit with a white ring and no white box and no shadow under it**, Search in the accent with dark words (LG's is green). FAIL, LG's look after an `ok:true`: the keys are not read from that directory: INFO `strings $(find /usr/lib/maliit -name 'libplugin-global.so*' | head -1) | grep -E 'qrc:|plugins/qml|\.qml' | sort -u | head -30`
   and `grep -c 'plugins/qml' /proc/$(pidof MaliitServer)/maps`; stop (then the keys are a compiled resource after all and a rebuild of it is Nitin's call).
4. **Focus and keys.** The D-pad walks the focus as with LG's keys and `kb OK` types the letter into the field. `kb RIGHT` to the end of the row and `kb UP`: `shot keys-focus-search` (`09-lg-keys-focus-search.jpg`: the lit box with its ring over Search, the word in the ink). Shift: `kb DOWN`, `kb LEFT` x 5 from `g`, `kb OK`,
   `shot keys-shift`: capitals, and the mark in the key's corner is the accent. `$%^` (`kb LEFT` x 5 from `g`, `kb OK`): the symbol grid and the key reads `ABC` (`09-lg-keys-symbols.jpg`). Delete, the space bar, Clear all and Search do what they did. PASS: every key as with LG's look.
5. **Candidates and the number pad.** Two letters in the Search field: the candidate row above the keys (`09-lg-keys-candidates.jpg`) if LG's prediction offers words, INFO if it does not. A number field (LG Settings, Network, an IP address field [INFERENCE]; skip after two minutes): `shot keys-numbers` (`09-lg-keys-numbers.jpg`).
6. **Cost.** `$TOOL fps on` before step 3 and `$TOOL fps off` after: `grep -c FPSJANK /var/log/messages` before and after. INFO only (the focus box lost its shadow layer).
7. **Off.** `$TOOL keys off` (PTY): `"keys":"off","restarted":true`; the overlay is gone from `/proc/mounts`; the next open of the keyboard is LG's (`shot keys-own-again` equals `keys-own`); `$TOOL surfaces` has `"id":"keys","on":false`; a second `keys off` says `"restarted":false`.
   Leave it as found (off) unless Nitin wants the keys: `keys on` survives a reboot (`apply` mounts again).
8. **Boot** (only with Nitin's OK for a restart, after the steps above, with `keys on` kept): after the restart `$TOOL keys status` has `"on":true,"mounted":true` within a minute and the first keyboard shows the Lucent keys. INFO whether `maliit-server` was started before the mount
   (the first open LG's, then Lucent once `apply`'s `keys on` has restarted it): `apply` runs `keys on` at boot only while the overlay is not there.
9. **End.** Home in front, `$TOOL keys status`, `$TOOL surfaces` in the log, the receiver as found.

Offline checks that back the steps: `python3 -m unittest tests.test_maliit_patch tests.test_maliit_stage tests.test_lumatv_keys tests.test_package_tools tests.test_surfaces_registry`.

## Not done

- maliit's dictation card (`view-stt`) and the screens that only the Chinese and Japanese plugins draw (their stroke and kana layouts): those two plugins take the shared style sheets' colours, faces and focus, and nothing else is restyled in them.
- LG's voice bar (`VoiceAgent`): its source is on disk, but it cannot open while `lean` is on and LumaHome holds the mic key; the plan's one arc in place of the 485-frame animation is
  not built (SURFACES.md row 183).
- Live TV screens that cannot appear on this set (Teletext, CI, EWS, DVR, LG Channels, region popups) keep LG's look apart from the shared fonts and colours (SURFACES.md rows 167-173).
- The mock's bottom-left pill with a lead icon and Details/Inputs/Picture buttons (LumaHome's `Banner.qml` has that; LG's `launchMoreActions` opens LG's own options and is kept).
- No-signal buttons (mock 18's "Inputs" and "Home"): the screen takes no key focus in LG's code (`needKeyFocus: false`), and buttons there would need LG's key routing changed.
