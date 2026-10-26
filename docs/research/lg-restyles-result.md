# LG-process restyles: input banners, on-screen keyboard (LgRestyles)

## Resume here

Built and tested offline, **not verified on the TV**:

- `tools/inputcommon_patch.py`: LG's input pill, OK banner (with its options key), INFO detail card and software-update bar in the
  Lucent tokens, as anchored edits to a staged copy of `com.webos.app.inputcommon`'s QML.
- `tools/inputcommon.sh`: stages that copy, mounts only the rewritten files over LG's QML directory (overlay mount, nothing in the
  root file system) and makes the input apps start cold, so that the rewritten files are read: LG's CRIU dump of the app is set
  aside in CRIU's working directory and the idle instances are ended through SAM ("Cold start" below). Verbs `on`, `off`, `stage`,
  `status`; wired into `tools/lumatv` as `lumatv inputcommon on|off|status` (off by default, mounted again by `apply` at boot).
- `tools/keyboard_patch.py`: LG's 600 ms "pudding" for the on-screen keyboard's window replaced by the theme's layer motion, in the
  compositor view that hosts the IME.

**The keyboard's keys are not restyled: the source is not in anything copied from the set.** maliit's key caps, language bar and
candidate list are drawn by the IME process (`/usr/sbin/MaliitServer`, plugin `libplugin-global.so` per `/var/lib/maliit/server.conf`),
and the 85 QML files PLAN.md names for it are in no copy in `lgtv/uiinv`, `lgtv/tmp-research`, `/tmp/tvsrc` or `/tmp/ose` (those hold the
compositor side only, `StarfishKeyboardView.qml`). The upstream maliit and webOS-ports keyboards on GitHub are different code. So the
look of the keys needs a capture first ("Needs the TV" 5 below); the patch for it then follows `inputcommon_patch.py`'s shape.

On the TV (2026-10-25 19:43, "On the TV" below): the QML directory is the guessed one and matches the census; the ipk did not ship
`tools/inputcommon.sh` or `tools/inputcommon_patch.py` (`lumatv inputcommon` answered "did not answer"); with the files put on the set by
hand, stage, mount and unmount worked and nothing crashed; the restyled look was not seen because inputcommon kept running from its
boot-time checkpoint and LG restored that checkpoint again after a SAM close, marker or no marker.

Since then (offline, not verified on the TV): the three scripts are in the ipk, with a test that everything the tool calls is shipped
(`tests/test_package_tools.py`); `inputcommon on` now does what LG's own scripts say is needed for a cold start ("Cold start" below, with the
TV check); the verb, the registry row, the boot re-mount and the keyboard patch's place in the shadow build are wired.

Next, in order:

1. The TV check of "Cold start" (pid, command line and banners after `on`; the ladder if the pid is still 2513).
2. The pill, OK, the options key and INFO with the physical remote on that cold instance.
3. Capture maliit's tree ("Needs the TV" 5), so the keys can be done.

CHANGELOG line for the main agent: *Restyles of LG's own processes: the input banners (`com.webos.app.inputcommon`) in the Lucent look through an
anchored patch and an overlay mount (`tools/inputcommon_patch.py`, `tools/inputcommon.sh`; `lumatv inputcommon on` sets LG's CRIU dump of the input
app aside and ends the idle instances so they start cold and read the patched files), and the on-screen keyboard's open and close in the theme's layer
motion (`tools/keyboard_patch.py`); the three scripts are in the ipk; both patches fail loudly when LG changes the code they patch.*


## Where LG's sources are

| What | Copy | Notes |
|---|---|---|
| inputcommon QML, 273 files, 11 MB, with its images | `/data/home/lgtv/uiinv/inputcommon/qml` | made durable here from `/tmp/tvsrc/inputcommon/qml` (the only copy there was in `/tmp`). The tests and the host use the first of the two that exists. |
| `StarfishKeyboardView.qml` (342 lines), `BaseView.qml`, `KeyboardView.qml` | `/data/home/lgtv/uiinv/qml/qml/WebOSCompositor[Base]/views` | the compositor side of the keyboard; OSE's open `luna-surfacemanager` (`/tmp/ose`) has a near-identical `KeyboardView.qml` |
| maliit's QML | none | see above |

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
- `python3 -m unittest tests.test_inputcommon_stage` (26: `Staging` on the census tree, 10: staging keeps only the rewritten files and leaves LG's tree alone, a refused stage keeps the last
  good one, `on` unmounts then stages then mounts then sets only inputcommon's dump aside, a failed stage or mount leaves LG's banners and the dump, an overlay that will not unmount
  stops everything, `off`, status, no QML directory, usage; `ColdStart` on a stand-in set, 16: the dump renamed with every file and symlink as it was, the persisted copy byte for byte
  unchanged, the idle input apps closed through a stand-in SAM only after the dump is aside, the one on screen and look-alike ids left, a refused close or a SAM that hangs costs nothing,
  a copy already aside kept and a newer dump dropped, `off` putting it back, a restore that is still running waited for; `mount`, `umount` and `luna-send` are stand-in scripts).
- `python3 -m unittest tests.test_package_tools` (7: everything the tool and the boot hook call or import is copied by `tools/build-ipk.sh` and listed in its REQUIRED_FILES, the checker
  finds a script that is not, and the dry run lists the three restyle scripts as present) and `tests.test_lumatv_inputcommon` (37: the verb, the flag, the boot re-mount that leaves a
  mounted overlay alone, revert, the registry row, the keyboard patch in `build_shadow` on LG's own view, the shadow's patch fingerprint and boot-bind's rebuild below).
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
2. **The cold start.** Superseded by "Cold start" below: the marker in `/mnt/lg/cmn_data/checkpoint` was tried (round 7) and is not what makes LG start the app cold.
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
- `build_shadow` runs `python3 tools/keyboard_patch.py` on the shadow module's copy of `views/StarfishKeyboardView.qml` with the shipped theme's motion;
  LG's file stays when the patch refuses (an anchor missing, no usable motion), with a line in the log. Not behind a switch: the shadow is LumaTV's own
  copy of the module. It only matters while LG's compositor is the live one.
- The shadow module records `.shadow-patch`, a fingerprint of `SHADOW_PATCH_FILES` (the patch script, the two modules it imports and the theme), and `shadow_current`
  and `apply`'s `compositor_changed` compare it. Before, a set that already had a shadow kept it through an update that changed only a patch or the theme's motion, so
  the keyboard patch reached a set only when LumaHome's QML changed in the same update. The first `boot-bind` or `apply` after this update rebuilds the shadow once and
  restarts LG's compositor once so that it loads it; under LumaTV's compositor (it draws none of the module) `boot-bind` rebuilds without a restart. A patch added to
  `build_shadow` appends its files to the list, and a test fails when one is missing.
- `tools/build-ipk.sh` copies `inputcommon.sh`, `inputcommon_patch.py` and `keyboard_patch.py` (and lists them, with `tileicons.js`, `systemui.py` and
  `systemui_patch.py`, which were copied and not listed, in REQUIRED_FILES). `tests/test_package_tools.py` follows what the tool, the boot hook and
  every shipped script call or import and fails when any of it is not copied and listed.

## Not done

- maliit's keys (above): no source.
- The rest of inputcommon's own screens: the no-signal page and its photos (`BgViewLayer/Screensaver.qml`), the banner-position editor, the alert popups ("Input Locked", set-top box) and the Live TV channel banner share the font and the sheet's colours now, but their
  layouts and backgrounds are LG's. SURFACES.md rows 89-92 and 164-165 still stand.
- The mock's bottom-left pill with a lead icon and Details/Inputs/Picture buttons (LumaHome's `Banner.qml` has that; LG's `launchMoreActions` opens LG's own options and is kept).
