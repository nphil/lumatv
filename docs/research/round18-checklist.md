# Round 18: the TV checklist

Every TV check still pending after rounds 14 to 17, in run order, each with the reading that passes it, the way back, and whether Nitin
has to be at the set. Built from the result docs on forge/main of 2026-10-29 (named per step); a step whose doc has changed since reads the
doc's text. Nothing here was run. No step is a decision: each is a reading.

Marks: **[G]** goes through Nitin's Face ID gate (an install, a reboot, an app on or off the set). **[N]** Nitin at the set (eyes on
the panel, the remote in hand, a real event, or a display change he has to allow). **[-]** neither: the round's driver runs it with the
receiver muted through Home Assistant's MCP.

Names: `R` = `sh compositor/tools/tv-first-light.sh`; `T` = `python3 docs/research/arch/tv.py`; `ctl` on the set =
`/tmp/lumatv/lib/ld-linux-aarch64.so.1 --library-path /tmp/lumatv/lib /tmp/lumatv/bin/lumatv-compositor ctl --runtime-dir /tmp/xdg`;
`LT` = `/media/developer/apps/usr/palm/applications/org.nphil.lumatv/tools/lumatv`. Evidence goes to
`docs/research/compositor/slot-2026-10-<day>-r18/` (README first, one directory a step group, `log.txt` with PASS/FAIL/MINOR/INFO lines).

## 0. Before the set (host, no TV)

Build order for the package (the gfn app is now inside it): `gfn/feeder/build.sh`; the three `cargo zigbuild` calls and
`gfn/engine/build-core.sh`; `rustui/tools/build-gfn.sh`; `gfx64/fetch-blob.sh` and `gfx64/build.sh`; `rustui/tools/build-card.sh`;
`sh gfn/build-ipk.sh`; then `sh tools/build-ipk.sh` (it stops, naming the missing build, when the gfn ipk is absent). Bundle:
`sh compositor/tools/build-tv.sh` (now `ui` by default: `FEATURES` says `ui`, `bin/lumatv-ui` is in it). `python3 tests/qmlhost/compile_all.py`
must say 0 failed. Round 18's bundle, built on the host from forge/main `b65ef9a` with `LUMATV_FEATURES=ui` (2 min 10 s on a warm cache): 93.9 MiB in `/tmp/lumatv` (37.9 MB as the deploy tarball): `bin/` 32.3 MiB, of which `lumatv-ui` 27.6 MiB (line tables kept) and `lumatv-compositor` 3.7 MiB; `lib/` 59.1 MiB (the Mali blob is 53.7 MiB); `sm/` 2.3 MiB. The package grows by the gfn ipk (the 10-26 build is 37.7 MB; this round's is measured when it is packed); a set short of memory (`MemAvailable` under about 400 MB) is read first, since `/tmp` is RAM.

## 1. Install and read-back [G]

1. The installer (`tv-install.sh`): ipk, `lumatv apply`, `R deploy`, `R cutover install`. Read back: `LT version`; `LT gfn status` says
   `"installed":true,"current":true` with `build` equal to `shipped` (a set that had no GeForce NOW app says `installed:false`: apply does not
   put it on, `LT gfn install` is Nitin's own command); `cat /tmp/lumatv/FEATURES` says `ui`; `ls -l /var/lib/lumatv/compositor/bin/lumatv-ui`
   exists (the supervisor runs the compositor from there, which looks for it beside itself); `LT compositor check` answers JSON.
   Revert: `LT revert`; the app alone: `LT gfn remove` (the sign-in under `/var/lib/lumatv-gfn` stays).
   **Expected:** the app at the set before this round was the 2026-10-26 build (`lumatv-gfn-app` md5 `db62520227a0…`): `build` was empty, so
   apply updates it. Nitin does not sign in again.
2. Start state, written to the slot's `start-state.json`: power, foreground, picture mode, `lumatv inputs status`, `md5sum` of `theme.json`,
   `layout.json`, `helper.json`, `Tokens.js`, `lumatv keyboard`, `lumatv transitions status`, `lumatv wallpaper status`, the receiver's level and
   input, `ctl shell` (`rust.kinds` empty), `getPixelRefresherInfoList` and `getPanelUsageTime '{"subscribe":false}'`.
   The end state (section 11) is compared with it.

## 2. Read-only, LG's compositor live, nothing changes [-]

3. Refresh guard (`refresh-guard-result.md`, steps 1 to 3). `luna-send -n 1 -f luna://com.webos.service.oledepl/getPixelRefresherInfoList '{}'`
   and `…/panelcontroller/getPanelUsageTime '{"subscribe":false}'`; `curl -s 127.0.0.1:18731/v1/state/panel`. Expected: `read` true,
   `offrsCount`, `lastRunHours`, `intervalHours` equal to LG's, `usageHours` = ticks / 6, `due` = (usage >= last + interval). `read` false
   with an `error` names the key or unit that differs; that is a finding, not a failure of the tooling. Settings > System > OLED care shows
   "Panel refresh: last run N h ago, M runs". Revert: none.
4. Early-boot privacy, before any reboot (`privacy-menu-result.md`, "Next" 1 and the boot-timeline preconditions). Read, press nothing:
   the first command of each of `/etc/systemd/system/scripts/{bam,rec-info-adapter,rec-listmgr,nn-proxy,kdump}.sh`; `LT status` has `persist`
   and `early` true; `sh /var/lib/lumatv/dc-dev/tools/declutter.sh status` has `early.hook` true; `cat /sys/power/mode`. Record whether a
   script calls a bare command first (only then is a hook before kdump's +13.4 s possible, 1.6 to 2.7 s earlier). Revert: none.
5. Off-screen scan-out proof, check (a) (`scanout-research.md`): `R deploy`; `R scanout-proof stock afbc0 big`; read
   `/tmp/scanout-proof/out/*/verdict.txt`; `T 'sh /tmp/lumatv/scanout-proof.sh clean'`. Expected (the doc's table): `stock` AB24 `…151`,
   import ok; `afbc0` linear, import ok (route B open) or still `…151` (the library ignores the hint); `big`: CmaFree falls by no more than the
   buffers (3 x 9 MB). `Invalid argument` on import means scattered pages. Touches `/tmp/scanout-proof` and a GL context only. Revert: `clean`.
6. Display-chip read-only slot: done 2026-10-29 03:46 EDT (`compositor/layerdeep-2026-10-29/`); not repeated. Open questions 3 to 8
   of `display-chip-direct.md` need a display change: section 7.

## 3. Gate session on LumaTV's compositor, receiver muted [-]

All through `python3 compositor/tools/tv_gate.py` (one session each, `enable --session`, the gate's own clean-up ends it); AVR muted through
Home Assistant first and put back at the end. Stop after any FAIL that moves a guard value (failed units, `librdx` files, `aud_dsp_reset_count`).

7. PlxNative's close, fifteen times (AppCompat; round 17: 9 of 10 crashed, libSDL2 `+0x875c6`, a 1 s stall in its EGL unload; `112d9cc`
   sends every client's callbacks at its frame's draw): `python3 compositor/tools/tv_gate.py --only apps --apps-repeat 15` (about 12 min).
   Expected: `GATE apps: PASS`, 30 launches of `[pass N of 15]` rows, every one `alive-at-10s yes connection-lost 0 closed-in-15s yes`, no
   `exit_status`, no `crash-reports +N`. Keep `/tmp/plxnative-crash.log` whole beside the rows (the gate counts `*** SIGNAL` reports 3 s after each close).
   A crash is read with `apps-com.beb.plxnative-rN.txt` and the app's own report; the A/B is `ctl callbacks clients landing` (the flip, as in round 17),
   repeated with `--apps-repeat 10`. GeForce NOW's
   browser step (Home over it stays in front 6 s later) runs once at the end. Revert: `ctl callbacks clients early` (the default).
   **Pass:** 15 of 15 clean. A pass here, with step 8, unpauses persistent go-live.
8. x1, three configurations, each three times (`x1-analysis.md`, "Next round"; the bundle's `x1client` changed, so only a fresh deploy counts):
   (1) `tv_gate.py --only x1` (callbacks at the draw, the default); (2) `--only x1 --x1-callbacks landing` (callbacks at the flip, round 17's mode);
   (3) `--only x1 --x1-rt 10` (the client SCHED_FIFO 10). Read per run `x1-result.txt` and `x1-kwait.txt`: the first lines are `RAW STRICT` and
   `RAW RESULT` (the PLAN's rule, untraced reading kept), then the attribution. Expected: the compositor's flips on time (longest interval under
   17 ms, `pacer late` unchanged, `loop stalls 0`); every client gap paired with a client reaction 3.3 ms or more over the one before
   (`x1 log: N gap(s) …; the trace implies N`); no gap that pairs with a late flip, a loop wait or a late callback (`x1 gaps:` fails the gate for those).
   `x1 attribution: INCONCLUSIVE` is not a pass: rerun. Also in the trace: file writes belong to `lumatv-log`, never the FIFO loop; no loop
   D-state sleep from file logging; read `compositor.log` for drop notes. `--x1-strict` once for the comparison with the 8 of 25 earlier passes.
   Revert: the switches end with the run.
9. Frames at 60 and 120 Hz (`tv_gate.py --only frames --frames-quick` then `--only frames --refresh 120`; the full set with `--only frames`):
   0 frames skipped, `burst_over20` 0 on home-dpad/quick/picker/toast, `gl errors 0`; at 120 Hz round 16's quick set (quick 14, picker 10, toast 3,
   home-dpad 2) is the bar to beat; the `slides:` line reads 13 or more steps for 110 ms. Revert: the gate puts the divider back (60) and reads it.
10. The rest of the gate, once, for the whole picture: `tv_gate.py --skip volume,x1,apps,frames` (firmware, homeheld, netflix, hdmi, keys, ime, shell,
   standby, homekey, wd-hang, wd-crash, escape). `volume` reads the receiver over REST and stays skipped. Standby and wake here obey the refresh
   guard (section 10). **Not** `--enable-after`: go-live is Nitin's word.

## 4. The Rust kinds, one at a time [-] (the first look of each on the panel is [N])

Each kind is opt-in per kind and the QML stays the fallback until a kind passes this list twice on the set. Gate for each step:
`ctl shell` shows the kind in `rust.ui.kinds` and `rust.connected`; if a kind is not in the installed build, the step is `SKIPPED (not built)`.
`tv_gate.py`'s `shell` and frame checks do not read `rust.ui` yet: the readings below are `ctl` ones, and the frame gate over a Rust kind is
`python3 docs/research/perf/gate.py --surfaces <surface> --tag r18-rust-<kind>` run twice. Common off switch, no deploy: `ctl shell rust -<kind>`.

11. Core: `ctl shell rust +osd +toast` (`rust-shell-plan.md` section 6, steps 3 to 5). `ctl shell`: `rust.connected`, `rust.engine` true,
    `rust.ui.kinds ["osd","toast"]`; journal `shell-rust: client started`, `…the client is on the engine`. Volume keys over Home and an app: the Rust
    bar (`rust.ui.osd` true, `ctl status` `ui frames … gl errors 0`; `ctl capture /tmp/osd.png`). Toast: `luna-send -n 1 luna://com.webos.notification/createToast
    '{"message":"Remote battery low"}'`: top right, `rust.ui.toasts` 1, `shell-rust: toast delivered` against `…system layer live` under 50 ms.
    Volume hold (`volume-osd-result.md` 3): hold Volume+ 3 s, receiver low: 3 a second rising to 8, the Denon keeps up, no step back at release. [N] eyes.
12. Home (`rust-home-result.md`, "The TV round" 2 to 5): `ctl shell rust +home`; journal `Home is the Rust shell's: the QML Home window is left out`,
    `rust.ui.home.home` true, `rust.wants` true. D-pad over status bar, cards, dock with the lift flowing; OK launches (`[log] launch`); Back from an
    app returns to the Rust Home; a digit opens a dock tile; hold OK on a card opens the editor, on a tile its options; bell opens the drawer and the
    badge counts and clears. `tv_gate.py --only shell,frames` with `home-dpad` at 120 Hz: 0 `burst_over20`. Profile chip opens "Who's watching?"; a switch
    shows the veil and the new Home in under 300 ms (`profiles: switch to … asked` against `switchedAt`). Cameras card: stills once a second, none while hidden. [N] eyes + remote.
13. Panels (`rust-panels-result.md`, "TV checks"): `ctl shell rust +quick +picker +drawer`. Gear tap opens the quick panel at x 1320, 560 wide, under
    150 ms; Back and a second tap close; Input opens the picker, again goes to the next input, OK launches; arrows move the lift, OK acts on the sleep
    timer and Wi-Fi/receiver; hold OK picks up a tile with `remote.quickPanelEdit` on; two profiles: the strip first, a PIN profile asks the pad in place;
    All settings opens Settings; gear hold opens the drawer (Clear all, an item's buttons); 45 s idle closes any panel; hover is a cue, a click acts, a
    click beside a panel closes it. Over Netflix and over HDMI: `quick`/`picker` surfaces of `gate.py` (round 16: end 146-166 ms, p90 under 3 ms).
    The context menu, confirm dialog and switcher chrome (`+menu +confirm +switcher`, on the compositor's kind list) are read the same way once
    `rust-panels-result.md` lists them as built; until then the QML ones stay.
14. Settings (`rust-settings-result.md`): `ctl shell rust +settings`; `ctl shell show settings`, then each ported page by `show settings PAGE`. The kind is
    all-or-nothing and stays off until every section is ported: run the pages that exist (Privacy, Profiles, Home & look, System, Picture, Network,
    Ambient, Remote) for fit, focus flow and writes; confirm one value changed and put back (a Picture setting is read before and after). Text entry and
    the Layout/Cameras hand-offs are listed as open in the doc; their TV step is `SKIPPED` until the doc says built. [N] eyes.
15. Cameras (`rust-cameras-result.md`, "TV checks" 1 to 9): `ctl shell rust +cameras`, `ctl shell show cameras`. Grid up at once, stills about 5 a
    second a tile; OK starts the stream over Home (a hole over the video plane), chrome fades after 2.5 s, Left/Right switch, Back stops; `appFront`
    stays false while the feeder window is up; over an app stills only; Up scrubs the strip (thumbnail 250 ms after the knob settles, playback 450 ms
    after the last step, a gap toasts "No recording at that time"); Down opens events/clips; hold OK opens options and a switch writes `notify.<id>`.
    Switch `popup` (RustSystem) together with it and look for a double draw. Cost: harness `RESULT … cpu mean` with the grid streaming, at most 73 layers. [N] eyes.
16. System layer (`system-layer-result.md`, "TV checks"; kinds `alert`, `saver`, `capture` and later `cards`, `popup`, `layout`, each when `rust.ui.kinds` has it):
    `ctl shell rust +alert`: a PIN prompt and an LG alert are the Rust ones, OK and Back by name. `+saver` and `+capture` are read with the QML twin's
    steps (section 6 for the saver, step 23 for the recording dot); a kind not in the installed build is `SKIPPED (not built)`.

## 5. Display-state features carried from rounds 14 to 17 [-] unless marked

Each doc's list, quoted by number; timings are the short ones named there. Put the theme back with the byte-for-byte md5 of the start state.

17. OLED care's own screens (`oledcare-result.md` 1 to 7): `LT theme set '{"oledcare":{"enabled":true,"stepSec":10,"dimAfterMin":1,"dimOverMin":2}}'`.
    Orbit: one pixel on one axis a beat, never past 3 (round 14 passed); `dim[0]` falls from 1 to 0.85 only when Home is still (Home runs the moving
    wallpaper: use `wallpaper gpu` for this step); over video the bar follows the offset; idle tick near 10 a second; saver up: offset and dim stand still.
18. App dim and shift (`oledcare-apps-result.md` 1 to 9): an app with still screens and no video (Netflix's profile picker, Plex's library, the browser on a
    static `data:` page). `app.stillSec` counts up without falling back (round 16 fell every 4 to 5 s); after 60 s `level` falls to 0.75 and `offset`
    stays within 3; a key or the pointer puts back in 0.3 s; a video keeps `level` 1.0 for 5 minutes; a changed page puts back in 300 ms; ticks stay near 10 a second.
19. Box dim (`boxdim-result.md` 1 to 9): LG's browser on the static two-box page, hands off, `boxDim` with `boxDimAfterMin` 1 and ramp 20 s: `stillSec` grows,
    `engaged` true after 60 s, `lowest` falls to 0.8, the boxes look like the same UI less bright with no rectangle or band (photograph it) [N];
    capture 0.80 x the shot inside a bright box; a key un-engages within 300 ms; a video hole stays black and un-dimmed; `warmUsMax`/`sampleUsMax` under 2000.
    Round 16 never engaged: these are the first real readings after the fix.
20. Transitions (`transitions-result.md` 1 to 13; after `LT transitions on`): a warm launch grows from its tile in 220 ms, `lateGaps` 0, `maxGapMs` 20 or
    less; Home shrinks into the tile in 180 ms; interrupt, app to app, LG Settings, HDMI (no black frame, `veilCaps` 0), Netflix then Home dissolve;
    `motion.reduced`: one 120 ms fade; RSS flat over 100 launches. The doc's step 9 (a partial-alpha blend over the video plane) can end the design: record and stop there.
    Revert: `LT transitions off`.
21. Keyboard Backspace (`keyboard-result.md`, typing check 4): own keyboard in LG's Search, `kb UP`, RIGHT x6, OK deletes one character; FAIL if Search ignores
    the key event. Revert: `LT keyboard lg`.
22. Overlay cards (`overlays-result.md` 7, 8, 10): 7 the camera pop-up over Netflix (card from y 134, arrows reach the app, OK expands, Back dismisses, a door
    card within 20 s is not a second card); 8 real Home Assistant records with the plugin linked (a persistent notification is a card, a doorbell one card;
    OK on Clear clears it in HA) [N: Nitin's plugin link]; 10 cost: gate over the film with and without a card, 5 minutes of Netflix with a card every 30 s,
    shell client Rss flat. Revert: `LT theme set '{"overlays":{"enabled":false}}'`.
23. Screenshots and recording (`capture-result.md` 6 to 12, after steps 1 to 5 passed in round 14): needs the encoder in `/var/lib/lumatv/bin/lumatv-venc`
    **[N]**: `venc` wedged the kernel once, so Nitin's OK and the TV left alone. Recording of Home 30 s: a whole h264 1280x720 file; the dot is the rule
    (restart the shell: the recording ends within 4 s); limits; `kill -9` of the child ends it with a reason and Netflix still plays. Revert: `capture.enabled`
    false, helper restart, `pidof lumatv-venc` empty.
24. Cameras (`ha-cameras-result.md` 7, 9, 11 and the live path 12 to 13, with Nitin's steps 1 to 4 done) [N: real doorbell or door walk for 4]: helper CPU/RSS
    idle, during a pop-up and a grid of six; busy (a PIN prompt up: no card until it goes, then the card if still in date 12 s); cooldown (two events in
    30 s: one card; a doorbell twice: two); off (nothing on 6054). Live: Reolink first, key-to-playing 2 to 5 s, `dropped` 0, `lost` 0, `delayMs` under 5.
    Revert: `cameras.enabled` false, helper restart.

## 6. Saver rules and the saver [-]

25. Saver on Home (`system-layer-result.md` 1 to 5; the QML saver while the Rust one is not built): clock at level 0.4 and dimmer, over 20 minutes the dim
    reaches 0.4 of that; wake on a key, the pointer and Back. A camera event while it is up: no video over it (default `camera.popup: hide`), the event is a
    line in `lines` of the `saver` topic. A doorbell while it is up does not wake the screen by itself under LumaTV's saver (the next key does); under LG's
    (`Screen Saver`) it wakes. `curl 127.0.0.1:18731/v1/saver/rules` lists the shipped classes and one row per installed plugin; one `POST` changes a row
    and `saver-rules.json` appears in the data directory. `ctl status` has `oledcare.saver` true while it is up. Revert: `POST` the row back and delete
    nothing else (the file is the app's own state: leave it).

## 7. The hardware planes (HwPlanes; its section "TV-round checklist" in `hw-planes-result.md` is the master list) [N for the display ones]

26. `R deploy` ships `hwplanes.sh`; the driver is `python3 compositor/tools/tv_hwplanes.py STEP` (`read`, `alpha`, `pointer`, `afbc`, `layout on|off|alpha-on|alpha-off`,
    `shot NAME`, `feeder CLIP SIZE CODEC FPS SECS`, `vwin`, `sizes`, `stop`, `ctl WORDS`, `fetch`, `compare A B`). Read-only first (`read`, `afbc` A1/A2 list what the
    plane and the GPU support, `vwin`): any time, [-]. P0 (the OSD's constant alpha against the pages' format; the video-hole black), P1/P2 (pointer on the cursor plane against
    the GPU, 60 moves each), A3 (live layout switch `layout on|off|alpha-on|alpha-off`), V2 to V7 (video windows: the feeder in a video window, the
    sub window beside the wallpaper's session, sizes 114x64 to full screen) and C1 (feeder with cameras) change the display for seconds: [N] eyes on the
    panel, each step puts its switch back (`tv_hwplanes.py stop`: kill the feeder, release every claim, `vwin off`, `pointer gpu`). An interrupted run is ended by
    `python3 compositor/tools/tv_kms.py restore` and `ctl afbc off`. Every switch is off by default; nothing here is left on.
27. Scan-out (b) and (c) (`scanout-research.md` TV check list; a display change, needs a free set and Nitin's go) [N]: `R kms b c` (about 30 s a case, LG's
    compositor stops and the panel is black); `panel-b.png` (does the OSD decode `…151`: a right picture, scrambled or striped, or black), `panel-c.png` and
    `panel-c2.png` (direct and composed show the same picture, the number rows at the top and bottom agree: a mismatch is tearing), `c-scanout-direct.json`
    `"mode":"direct"`, no `import`/`addfb`/`test-commit`/`acquire` counts. (c): read `OSD 1 : 0` in `/proc/lg/fbdev/viewctrl` first; the `echo 1 1` write
    only with Nitin's go, `echo 1 0` after. Revert: `python3 compositor/tools/tv_kms.py restore`.
28. Scan-out (d), after (a) and (b): `tv_gate.py --only frames --refresh 120 --scanout direct` against round 16's `frames-120.json`: direct frames in the
    slides, `late` and `burst_over20` below round 16's, no `import`/`test-commit`/`acquire` count; `refused` names the rest (`layers`, `modifier`,
    `no-descriptor`, `video`, `cursor`, `dimmed`, `translucent`). Round 17's interrupted run read 0 direct of 22,946 (a system layer over Home; the OLED
    care's orbit moved Home off the origin: run it with `oledcare.enabled` false and the receiver's mute mark in mind). [N] for the tearing look. Revert: `ctl scanout off`.
29. OSD0_1, the linear case (`display-chip-direct.md` open question 8; `tools/osd01-probe.sh`; round 17 under LG's compositor showed no layer): only under
    LumaTV's compositor with linear pages, `revert.sh` armed as the dead-man, Nitin at the set [N]. Raw register tests (items 5 and 8 of the doc's list) only
    after the rest and only with his go.

## 8. Lag over playback [N for Prime]

30. Netflix: `python3 docs/research/perf/gate.py --surfaces quick-netflix,picker-netflix,lgqs-netflix --tag r18-lag` (LumaTV's panels against LG's
    quick settings over the 4K Dolby Vision film): end of the burst, frames over 20 ms, p90 render. Round 16's bars: every panel open in 146 to 159 ms over Netflix and HDMI, p90 under 3 ms, 0 late. A slow reading that is Netflix's own drawing (an earlier picker run read 944 ms) is the app's, so it is INFO.
31. Prime Video: `gate.py` has no `-prime` surface (the Netflix ones deep-link a title, which needs an ASIN and a sign-in). Method until one exists
    [N, Nitin starts a title]: Prime playing, `docs/research/perf/capture.sh` `full` trace, `kb.py` markers for gear (`@quick-prime-open MENU`) and Input,
    `keypath.py TRACE --table`. Record key to first frame and compare with step 30. A surface with a Prime deep link is the host work that makes this [-].
    Round 17's `lag/` directory was never produced: nothing earlier to compare.

## 9. Privacy: boot timeline and the consents [G for the reboot; N for the confirmations]

32. Boot timeline (`privacy-menu-result.md`, "TV check: boot timeline"): preconditions in step 4; `reboot` [G], then with nothing pressed:
    `declutter.sh status`: `early.thisBoot` true, `early.last.ran` true, `startSec` about 13 to 14, `stubbedSec` under 16.9, `uptimeSec` under 17.5,
    `running` at most the push client, `faultmanager`, `crashd`, `remotelogger`; `declutter/log` has the `early:` line with three clock values and `early: settled`;
    `systemctl show contentminer.service -p LoadState,ActiveEnterTimestampMonotonic` reads `masked` and 0 (same for `rdxd`, `nudge`, `uploadd`,
    `user-context-manager`, `iot-client`); `/var/log/messages` of this boot has no `pbs`/`acr2`/`admanager`/`adoverlay-service`/`sportsalarm`/`contentminer`/`uploadd`/
    `nudge`/`iot-client` line but the stub's exit (launches end within about 70 ms); the stub is mounted over all seven. A name in `running` means LG started
    it before the hook. Repeat once after `snapshot-boot-manager --remove` (`/sys/power/mode` `making`, not `resume`) [G]; it settles whether kdump's script runs
    again on a resumed boot. Skip file: `touch /var/lib/lumatv/declutter/skip`, reboot, `early.last.ran` false with the skip reason, then remove it [G]. Revert: remove the skip file.
33. The consents (`privacy-menu-result.md`, last list 1 to 6): menu and destinations, owner-only writes refused, cancel leaves every setting unchanged; each declutter
    group on and off; hosts table before and after with the update block present; a mapped LG advertising consent both ways; **Amazoff** on (a Prime-only CA, one
    jail redirect), a title with ads, seeking and a Sye path, ordinary and DRM playback, then off (CA and key gone, goproxy not reinstated). **Never** confirm
    "Delete my personal information". All of it is [N]: the owner's single-use confirmation is the control, an AI never supplies it. Revert: Amazoff off, then the groups as found.

## 10. Panel refresh: the standby rule [-], 15 minutes

34. Standby and the panel refresh (`refresh-guard-result.md` 3 to 6): `python3 compositor/tools/tv_gate.py --only standby` (the standby step, then the guarded wake); after a minute `curl 127.0.0.1:18731/v1/state/panel`: `standby.since` just now, `dueAtStart`
    as LG's numbers say, `countAtStart` = `offrsCount`. With a run due, `python3 compositor/tools/refresh_guard.py status` says "a wake would wait N more min",
    and `tv_gate.wake()` prints "waiting until 15 minutes of standby have passed" (`--force-wake` is the owner's override; the round does not use it).
    The measure that matters: a due TV in standby 15 minutes untouched: `offrsCount` +1 and `lastRunHours` at the panel hour (it also says whether the count moves at
    the start or the end of a run and whether panel hours tick during one); a wake at minute 3 leaves the count. Then, helper guard: `LT theme set
    '{"oledcare":{"refreshGuard":true}}'`, an assistant with Open apps at Always, standby with a run due: `launch_app` answers `refresh_in_progress`,
    `panel.guard.deferrals` 1; after 15 minutes it goes through. Switch `refreshGuard` on for good only after this; put the assistant's level back.
    Round 17 ended with the TV on, so no refresh ran: the round's last step is this one.

## 11. End-state rules (all of them, written to `end-state.json` and compared with `start-state.json`)

- LG's compositor live and `lumatv compositor status` `live` false (the gate's clean-up ends the session); no `lumatv-compositor` process; the Home stub with 0 faults;
  persistent go-live untouched (Nitin's word; it stays paused until steps 7 and 8 pass).
- Only Home running; `lumatv inputs status` running `[]`; Home in front; picture mode `filmMaker`; Always Ready as found.
- `theme.json`, `layout.json`, `helper.json`, `Tokens.js` byte for byte as found (md5), except the shipped `oledcare` timings if the start state held test ones
  (say which); `keyboard lg`, `transitions` as found, `wallpaper gpu`, `overlays`/`cameras`/`capture` as found (off), `refreshGuard` as found.
- Every Rust kind off again (`ctl shell rust -<kind>` or the file empty), `ctl afbc off`, `pagealpha` off, `vwin` off, `pointer gpu`, `scanout off`, `callbacks clients early`.
- The receiver unmuted and at its start level and input (read through Home Assistant; `TV Audio`).
- The scratch directories under `/tmp` removed; `/tmp/lumatv` may stay (tmpfs). Nothing deleted from `/var/lib` and no telemetry, token or sign-in touched.
- The TV left in Active Standby so that LG's panel refresh can run (step 34), not on at Home.

## Not on this list, and why

- The Marquee app: parked until the Rust screens are complete.
- Persistent go-live (`--enable-after`, a reboot on LumaTV's compositor): Nitin's word after steps 7 and 8; the supervisor's own escape (hold Home 650 ms) is the way back.
- Scrypted bridge install and pairing, the Home Assistant token and plugin enable, the Bluetooth proxy link: Nitin's steps (`scrypted-cameras-design.md`, `ha-plugin-result.md`).
- Any change of network, firewall, DNS, keys, accounts or backups: human-only, never part of a round.
