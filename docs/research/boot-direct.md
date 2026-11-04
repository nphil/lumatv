# Boot direct: LumaTV from the first frame, LG's Flutter Home a fallback only

Built behind a flag that is off by default: `lumatv boot-direct on|off|status`, which writes `/var/lib/lumatv/boot-direct` (the line `on`, and
a `tool PATH` line naming `tools/lumatv`). With the flag off the boot, the compositor and the keys are what ran on 2026-11-03; the tests that
pin that are unchanged. **Nothing here has run on the TV.** Host tests: `tests/test_boot_direct.py` (the tool), the rows of
`tests/test_native_startup.py` (the boot worker's order), `boot_direct.rs` and `tests/boot_direct.rs` in the compositor.

**The owner's rule:** LG's Flutter Home never starts, not even briefly. It is a fallback in two cases only: LumaTV's Home is gone (it crashed,
or the UI died), or Home is held for 5 seconds.

## What it does

- The Home stub (the web app of 2026-10-25: LG's Home id, bound over LG's directory) goes in as the very first act of the boot worker's
  `compositor-boot` (`bd_early_boot`, from the dispatch, before the storage guard, the data-directory migration, the firmware check, the
  mount and the restart). Every launcher of Home then opens the stub.
- It stays through the compositor's restart. The stub's card is Home to SAM, bootd, the wake and the saver; LumaTV's native Home draws over
  it, and its UI starts with the compositor instead of waiting for SAM's Home to map.
- Home held 5 s opens LG's real Home. Home there, or an app taking the front, puts LumaTV's Home back. Back is LG's own there.
- Every way out of LumaTV's compositor drops the stub. A boot that stops half way, or a stub that does not start, is a fault; two in a row
  switch the flag off.

## (a) Binding before LG's surface-manager starts, and the windows that stay

Not possible for a root mod. The set resumes from a hibernation snapshot; `/var` is writable after `mount-readwrite` at 10.8 to 13.1 s, and the
earliest hook that can run there, `kdump.service`, starts the worker at 12 to 14 s. surface-manager is up at 2.6 s, SAM at 5.3 s. Putting
LumaTV in the snapshot or the read-only image is in HANDOFF's "Tried and dead". `boot-bind-first` (96563425) and `boot-no-screen-on`
(d38e3161) are main's 0ec00c7f and a3ad63b9 apart from README and repo.json: nothing is left to merge.

The 2026-11-03 21:41 boot (Unraid syslog, uptime in seconds): worker ~13; `compositor-boot` done 22.7; `lumatv-sm` 25.7; compositor ready ~31;
SAM's `last_input_handler` launches Home 32.86; native Home 33.54. The 9 s of `compositor-boot` (storage guards, firmware check,
`native_start`) and the 3.6 s shader link are unmeasured per step; a reorder (activate right after the mount) waits for timings.

**Windows that stay open.** Neither can be closed without the TV, and the first not at all:
1. *Before the hook (0 to ~13 s).* A Flutter Home resumed with the snapshot, or one that bootd opens with its first app at ~13.9 s on a
   power-on that opens Home (`homestub-result.md`: untested; on a `reboot` bootd opens the last input, never Home, HANDOFF boot section).
2. *From the hook to the stub.* The early step binds the stub about 2 s after the worker starts: the 2026-10-25 boots had it in at 15 s with
   the worker at ~13 s (HANDOFF, "Boot"); the same steps, so the same estimate, unmeasured here. Up to 20 s if SAM does not answer yet (the
   bind first). The first thing it records is when it began and which Flutter Home processes it found (`/tmp/.lumatv-boot-direct-early`:
   `start`, `flutter_before`, `stub`).

In the five syslog boots (31 Oct to 3 Nov) Home was first launched at 32.8 s (twice), 33.8 s and, on the 20:38 boot, by the preload manager
at 60.0 s; one boot shows no launch: never before the hook. `check-boot-direct.py` FAILS on any Flutter Home process when the stub went in,
any `NL_HOME_SHOWN` anywhere in the boot, and any SAM launch of Home before the stub. A boot that shows one has an open window, and the
owner decides; the check also says from which second LG's log covers the boot, so that a launch before it is not read as none.

## (b) Keeping Flutter Home from launching

The stub: `payload/homestub`, a blank transparent web page with LG's Home id and category, bound over `/usr/palm/applications/com.webos.app.home`
with SAM made to read the directory again (`blockSystemApp`, `releaseBlockSystemApp`, as the 2026-10-25 stub does). It is the only option that
covers every launcher seen in the logs: SAM's `last_input_handler`, bootd (`bootingHome`, at wakes), the preload manager (Home is
`preloadMode: criu`; SAM logged its launch of Home at 60 s on the 20:38 boot), the Home key (`launchDefaultApp`), `luna-send` and apiadapter.

| Option | Why not |
|---|---|
| `setDefaultApp` / `defaultApps.home` | only the Home key; power-on and `last_input_handler` still open `com.webos.app.home` |
| `blockSystemApp` left on | persists in `blockedSystemAppList`, and breaks LG's Home as the fallback |
| `sam-conf-tv.json` `BootTimeApps` | read at ~5 s, before any hook |
| close Home after it launches | the launch, its CPU and RAM and its shelf calls still happen; bootd and the preload manager launch it again |
| a native (non-web) stub | SAM's native lifecycle (`registerApp`, roles) is unproven |
| report Home in front with no Home app | SAM, bootd and the Home key would still start Flutter Home |

Cost of Flutter Home under LumaTV's Home, measured 2026-10-25: RSS 72 MB hidden, 80.5 MB in front (PSS 35.6, 45.6); the stub 32.4 MB (PSS
15.3). It makes network shelf calls (`NL_SHELF_*`) and logs `NL_HOME_SHOWN` at each launch. No receipt of 2026-11-03 measures its CPU or RAM.
The Glasshouse project (rorygallagher2024/glasshouse) uses the same bind; it reports that on webOS 11.2 the preload manager started Flutter
Home again behind its stub and that `preloadmanager/setPreloadPolicy` for the id turned that off. This set runs 10.2.1, where the 2026-10-25
stub never saw it; if it shows up the check fails on it and the policy call is the fix (not built).

Placement, all behind the flag:
- `bd_early_boot`, first in the dispatch for the boot worker's `compositor-boot` (not a hand-run one, not on a live compositor): it takes the
  mutation lock for at most 10 s (no stub, and a log line, if it is not free or the stock UI is held), writes `boot-direct.pending`, records
  what it found and binds the stub (`bd_early_stub`). A detached watch (`boot-direct watch`) gives the stub up, and opens LG's Home, if the boot
  has not finished after 3 min, or 15 min while the worker is alive, unless LumaTV's compositor is live by then; it covers every exit that
  nothing else handles (a killed worker, a lock that `compositor-boot` itself cannot take).
- Every other exit before LumaTV's compositor is started takes the stub back (`bd_early_gone`, `bd_early_undo`): the dispatch's storage
  guard, the lock and the migration, then in `compositor-boot` the storage guard, a refused or held mount and a failed `native_start`.
- The activation adopts the stub (`bd_early_in_place`), and after the restart `bd_home_up` waits for WebAppMgr, launches Home and looks for it
  in SAM's list, three tries, and gives LG's Home back when it never runs. `compositor on` by hand puts the stub in before its restart
  (`bd_manual_stub`).

## (c) How LumaTV stands for Home

The stub's card is `com.webos.app.home`, mapped and transparent. SAM, bootd, the wake and the saver see Home in front exactly as with Flutter
Home; the 2026-11-02 saver fix (`saver-own-result.md`) relies on that and holds. The compositor draws the native Home where that card is in
front (`scene.rs`, `shell_home_drawn`) and its foreground list comes from real windows only. The one difference is the UI client's start: it
waits for SAM's Home to map (`START_ALONE`, 15 s at most), and a stub's launch waits for WebAppMgr, so with the flag on it starts at once.

## (d) Home held 5 s: LG's Home, and back

`boot_direct.rs`, all of it inert while the flag is off.

- Home held 5 s runs `lumatv home lg`: the stub is dropped, SAM reads Flutter, Home is launched. The release of the hold is not a Home tap.
  Before 5 s nothing fires: a Home let go earlier is a Home tap at its release, to LumaTV's Home, as ever (tests: 1 s, 2.5 s, 4.75 s).
- **LG's own 2.5 s hold (Screen Zoom).** It is `handleSystemKeys` in surface-manager's QML/JS (`KeyEntryPoints.json`), which the Rust compositor
  replaces (`shell-architecture.md`, option C); the compositor has no handler for it (its only `zoom` is the video window's) and no hold on
  Home but this one (`keys.rs`: 1 s for the gear, Input, digits and Back; 2 s for Mute). Nothing fires at 2.5 s, and the test pins that.
- **5 s and the supervisor's 10 s.** The hold acts once, at 5 s. The supervisor's escape (`lumatv-sm`, `HOLD_HOME_SECS` 10) reads the remotes
  by itself and takes no word from the compositor, so a key still down at 10 s still takes the set to LG's compositor (`stock-ui-held`,
  `recover-home.sh --stock`, which drops the stub), whatever the 5 s ask has done. Chosen so that the way out never depends on code that can
  fail. A hold released between 5 s and 10 s leaves LG's Home alone, with LumaTV running; one held on past 10 s leaves LumaTV's compositor
  until `lumatv compositor on`.
- While LG's Home is the front card (`com.webos.app.home` whose client is `flutter-client -i com.webos.app.home`, which the stub is not),
  the native Home is not drawn (`set_home_rust(false)`, the path a dead UI took); the UI's other layers stay over it. LG's Home never sees
  the Home key: its press and release are taken.
- Home on LG's Home (also when LG's Home asked for the key itself), or any other app in front, runs `lumatv home luma` and the native Home is
  drawn again at once, over LG's card, which the tool replaces by the stub within seconds. **Back and Exit are not taken**: inside LG's Home
  they are LG's own (a test: the same effects with the flag on as off, and a Back that LG's Home asked for is delivered to it). With an app in
  front the stub is not launched over it: the next Home key opens it.
- LG's Home that is not in front after 15 s is given up; the native Home was never taken away.
- The tool runs as a child of the compositor, one at a time, the latest ask after the one that runs, stopped at 120 s, with the supervisor's
  `NOTIFY_SOCKET` removed. It takes the mutation lock, waiting 10 s.

## (e) When LumaTV fails

- **The UI dies.** The client exits, stops answering for 75 s (`POLL_TIMEOUT`), cannot start or is not ready in 40 s (`READY_TIMEOUT`):
  `gave_up`, the main loop returns `shell_rust_failure` and the compositor exits with an error. The supervisor's `fallback()` follows at once
  (one failure counted, `stock-ui-held` written), hands the screen to LG's compositor and runs `recover-home.sh --stock`, whose
  `stock-ui-restore` drops the stub by its identity (unmount, SAM read again, `killApp`) and launches LG's Home. The same for a compositor
  that crashes or hangs, and for Home held 10 s. Tests: `shell_rust_failure` after a client exit and after silence (`shell_rust.rs`); the
  stub's drop in `tests/test_stock_ui_restore.py`; the supervisor's own side is its existing path. A hung UI costs its 75 s of a black or frozen
  Home first; Home held 5 s opens LG's Home sooner.
- **LumaTV comes back.** After a fallback the set stays on LG's compositor and LG's Home until the owner runs `lumatv compositor on` (the
  hold `stock-ui-held` also keeps the next boot's `compositor-boot` from starting it). `compositor on` puts the stub in before its restart:
  `homestub_up` ends LG's Flutter Home and the restart closes every client, so none lingers (`bd_manual_stub`: tests of its order and of
  every case where it does nothing). A boot does the same in its first step.
- **A compositor that runs with no UI** (the Rust shell is not configured) with the stub in front for 5 s asks for LG's Home: the stub alone is
  a black screen. Home on it is LG's own while there is no UI to go back to. No UI can appear later in a running compositor: the client
  giving up ends it.
- **A stub that does not start** after the restart (three tries) gives LG's Home back and counts a fault.
- **LumaTV not starting at a boot** (storage low, firmware not on the verified list, a refused mount, a failed `native_start`, a failed or
  interrupted activation): the stub goes in before those checks, so each of them takes it back and opens LG's Home. `compositor off` ends as
  before (`comp_switch off`).
- **A boot that stops** (a power cut, a killed worker) leaves nothing but `boot-direct.pending`: the bind mount and SAM's reading are gone at the
  next boot, which skips the stub and counts a fault. Two faults in a row switch the flag off. A finished boot clears both.
- Deleting the flag file by hand gives the old arrangement at once (native Home over a stub Home) until `boot-direct off` or the next boot.

## (f) Risks, rollout, rollback

Risks, all untested on the TV:
1. The windows of (a): a Flutter Home launched before the stub. The check says so if it happened.
2. SAM not answering at 12 to 14 s: the early step waits up to 20 s for it after binding the stub; if it never answers the boot goes without
   the stub and the check fails.
3. The stub's launch after the restart races WebAppMgr's (`bd_home_up`, three tries; timing unmeasured).
4. A preload-manager relaunch of Flutter Home behind the stub (webOS 11.2 report above).
5. The real remote's Home key: whether it repeats while held (the hold is timed from the first press and tolerates a 2 s gap between repeats,
   so it works either way), and whether the arrows and OK reach LG's Home while it is shown (the UI is told Home is not drawn, as with an
   app in front, which is the proven case).
6. Standby, wake and the saver with the stub as Home: the same card in the compositor's eyes, but not run.
7. `home lg` and `home luma` while the boot worker or another mutation holds the lock: after 10 s the request is dropped and the 15 s wait ends it.
8. LG's compositor runs with the stub bound for about 10 s before the restart (longer if the wake preparation times out): a Home opened by
   hand in that time is blank.

Rollout, each stage stopping on a surprise:
1. Install with the flag off; `check-boot.py` still PASSes (the boot is unchanged).
2. `boot-direct on` on the live compositor: the stub goes in, no reboot; the 5 s hold, Home, an app from LG's Home, `boot-direct off`.
3. `boot-direct on`, reboot, `check-boot-direct.py` before touching Home; then a cold power-on the same way.
4. The crash drill, then a day of use with the flag on, then the saver, a standby and a wake.
5. Only then a CHANGELOG entry, and the flag's default in the installer.

Rollback, one step: `lumatv boot-direct off` (the stub dropped, the flag removed, LG's Home opened if the stub was in front). The faults do
the same by themselves. If the set cannot be reached: Home held 10 s is the supervisor's stock escape, which drops the stub.

## TV test plan

Before anything: the code is on `main`, pushed to origin and forge; the installed hashes go in the handoff. `tools/autostart.sh` is unchanged.

Install, flag off, one lease session:
1. `sh compositor/tools/build-tv.sh`, then `sh compositor/tools/tv-first-light.sh deploy`.
2. `tv-first-light.sh cutover disable`, `install`, `probe`, `enable` (the sequence of `cutover-result.md`): `install` writes
   `/var/lib/lumatv/compositor/boot-direct-api`. The display restarts twice.
3. `tools/lumatv` as a copy-only install like `step3_install.py` of 2026-11-03 (`.new`, `sh -n`, sha256, atomic `mv`, hashes recorded).
4. `lumatv boot-direct status` reads `flag false, api true`; `ls $APP/payload/homestub` shows `appinfo.json index.html icon.png`;
   `check-boot.py --now` PASSes.

Live, no reboot:
5. `lumatv boot-direct on`: Home stays LumaTV's; status `stub true, sam web, flutter false`; `ctl shell` `bootDirect.on true`.
6. Owner: press Home briefly several times (LumaTV's Home each time, no LG Home); hold Home about 3 s and let go (the same). Hold Home 5 s and
   let go at about 7 s: LG's Home appears, LumaTV's is gone. Back inside LG's Home does what Back does there. Home: LumaTV's Home within about
   1 s; status `stub true` about 3 s later. Again, and open an app from LG's Home, then Home. `lumatv boot-direct off`: the stub is gone.
   Optional, once: hold Home past 10 s: the supervisor takes the set to LG's compositor (LumaTV is off until `lumatv compositor on`).

Reboot (the boot is clean only until the first Home hold):
7. `echo 240 > /var/lib/lumatv/bootcap` (frames and `state.txt` of the boot, `tools/bootcap.sh`), `lumatv boot-direct on`, `reboot`.
8. After 3 min, **before touching Home**: `python3 /data/home/tmp/lumatv-boot-direct-20261103/check-boot-direct.py` (also `--json`). It exits 3
   if LG's Home was asked for during the boot, and says so.
9. Repeat 7 and 8 with a cold power-on (off at the remote, then on; or the plug) for the case where bootd may open Home itself. Then step 6.

Crash drill (optional, one counted failure):
10. `pkill -f lumatv-ui` on the TV: the compositor ends, the supervisor falls back, LG's Home appears with the stub gone
    (`boot-direct status`: `stub false, flutter true`). `lumatv compositor on`: LumaTV's Home with `stub true, flutter false`.

Capture: the check's output and JSON, the bootcap frames, `/tmp/.lumatv-boot-direct-early`, the boot's lines from `/tmp/lumatv-logs/current.log`,
the Unraid syslog from the boot, the installed hashes. `check-boot-direct.py` adds to `check-boot.py`: the early step's record (the stub in,
nothing of Flutter running before it); no `NL_HOME_SHOWN`, SAM launch of Home before the stub, Flutter Home client or Flutter process at any
point of the boot; the stub mounted and SAM reading `web`; no fault, strike or mark left; Home in front, drawn by the native UI, LG's Home not
in front. It prints the native Home's time against 33.5 s and from which second LG's log covers the boot.

## Not built

The worker reorder, the shader disk cache, a stub before the hook (not possible), the preload policy call, a native stub, per-step timing inside
`native_start`; `tv-cutover.sh enable` does not place the stub (run `lumatv boot-direct on` after it); the old QML-era `homestub_*` code stays,
since `stock_ui_restore` and the stub's placement use it.
