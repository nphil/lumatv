# Boot direct: LumaTV from the first frame, LG's Flutter Home a fallback only

Built behind a flag that is off by default: `lumatv boot-direct on|off|status`, which writes `/var/lib/lumatv/boot-direct` (the line `on`). With the
flag off the boot, the compositor and the keys are what ran on 2026-11-03; the tests that pin that are unchanged. **Nothing here has run on the
TV.** Host tests: `tests/test_boot_direct.py` (the tool, and the escape's part of it), the rows of `tests/test_native_startup.py` (the boot
worker's order), `tests/test_stock_ui_restore.py`, `tests/test_launcher_app.py`, `boot_direct.rs` and `tests/boot_direct.rs` in the compositor and
the supervisor's own tests (`compositor/sm/build.sh` runs them).

**The owner's rule:** LG's Flutter Home never starts, not even briefly. It is a fallback in two cases only: LumaTV's Home is gone (it crashed, or
the UI died), or Home is held for 10 seconds, the supervisor's escape. There is no hold of 5 s and no `home lg|luma`: the compositor has no way
to LG's Home of its own, and the escape is the only one, from outside it.

## What it does

- The Home stub (the web app of 2026-10-25: LG's Home id, bound over LG's directory) goes in as the very first act of the boot worker's
  `compositor-boot` (`bd_early_boot`, from the dispatch, before the storage guard, the data-directory migration, the firmware check, the
  mount and the restart). Every launcher of Home then opens the stub.
- It stays through the compositor's restart. The stub's card is Home to SAM, bootd, the wake and the saver; LumaTV's native Home draws over
  it, and its UI starts with the compositor instead of waiting for SAM's Home to map.
- Home held 10 s is the one way to LG's Home (d): the supervisor reads the remotes by itself and ends the compositor, the UI and everything else
  LumaTV started, drops the stub, restores the stock UI and launches LG's Home. It is no fault: the flag stays on. LumaTV is back by a reboot or
  by the LumaTV app in LG's launcher (`compositor on`).
- Every other way out of LumaTV's compositor drops the stub as well. A boot that stops half way, or a stub that does not start, is a fault; two
  in a row switch the flag off. An escape is not one of them.

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

## (d) The escape: Home held 10 s, and the ways back

The compositor has no hold on Home and no way to LG's Home: Home held for any time is one Home tap at its release, flag on or off
(`tests/boot_direct.rs`: 12 s, with a tool beside the flag file that is never run), and Back and Exit are LumaTV's own. The owner's decision
of 2026-11-04 removed the hold of 5 s, `lumatv home lg|luma` and the phases that tracked whose Home was on the screen. LG's own 2.5 s hold (Screen
Zoom) lived in surface-manager's QML, which the Rust compositor replaces, and has no handler either.

**The one way out** is the supervisor's: `lumatv-sm` reads the remotes' evdev nodes itself (`HOLD_HOME_SECS` 10) and takes no word from the
compositor, so it works with the compositor frozen or blind to its keys. `escape()` in `compositor/sm/lumatv-sm.c`, in this order:
1. writes `stock-ui-held` (the recovery needs it), `last-fallback` and `escaped`, and the tmpfs mark `/tmp/.lumatv-escaped`; no failure is
   counted, and the unclean-run marker is forgotten;
2. ends the compositor (SIGTERM, SIGKILL after 1 s) and the UI;
3. stops, then kills, every other process that runs from LumaTV's directories (`cleanup_everything`): a program under the data directory
   `/var/lib/lumatv` (the helper and what it started, the colour reader, the player, the feeder, the bundle's loader), a program under
   `/media/developer/apps/usr/palm/applications/org.nphil.lumatv*` (the card and GeForce NOW apps), or a script of those given to an interpreter
   among its first arguments (the tool, the boot worker, a service's own supervising script, which would start again what was only killed).
   All are stopped before any is killed, with a second look for what was forked meanwhile; identity is by pidfd. LG's programs, the recovery
   script and other apps match nothing. What was ended goes to `reason.txt` of the evidence bundle (`ended:`): the helper holds the log's
   collector, and it is among the dead;
4. restores the vsync divider, saves the bundle (`kind: escape`), starts `recover-home.sh --stock` on its own, drops its mount and execs LG's
   compositor.

The recovery then runs `lumatv stock-ui-restore`: the stub is unmounted, SAM reads Flutter again, the stub's page is killed, the unfinished
boot mark `boot-direct.pending` is removed (the strikes are left as they are), then WebAppMgr is restarted and LG's Home launched. Pressed twice
it does the same: the second hold finds no supervisor to read it, and every step above is a no-op on a second run (tests: the sweep and the whole
escape in `lumatv-sm.c`; `test_an_escape_leaves_no_stub_no_unfinished_boot_and_the_flag_on` runs the restore twice).

**Not a fault, flag on.** The escape counts no failure (`failures` is as it was), leaves `boot-direct` as it is, and leaves `boot-direct.strikes`
as it is: it is neither a bad boot nor a good one. `bd_strike` counts nothing while `stock-ui-held` is there, so a boot cut short by the escape
(the activation's `bd_fail`, the watch's give-up) cannot cost a strike, and the restore has cleared the mark the next boot would have counted.

**The way back, two ways:**
- *A reboot, cold or warm.* The hold and `escaped` last for the rest of the boot they were made in: nothing starts LumaTV again by itself, the
  worker's second pass included. The boot worker's first act (`escape_lapse`, before the early step) drops both when the tmpfs mark is gone, which
  a reboot does (the mark is in `/tmp`, as the worker's own marks are), so the boot comes up on LumaTV with the stub first and no Flutter Home. A
  hold with no `escaped` is a fault's and stays until `compositor on`.
- *The LumaTV app in LG's launcher* (`index.html`, `launch.js`, `app.js`; app id unchanged). Opened while LumaTV is not the compositor it runs
  `compositor on` at once, with no key: the stub goes in before the restart (`bd_manual_stub`), which ends LG's Flutter Home; the restart
  closes the app's own page, and LumaTV's Home is what comes up. `compositor on` consumes the hold and clears `escaped`. A recovery that is
  still running ("a recovery of LG's Home is running", about a minute after an escape) is waited for, 3 s at a time up to 120 s; any other
  refusal is shown with its reason, with "Try again" and "Open LG's Home" (the launch of `com.webos.app.home`), and Back leaves for it. The tool
  has put LG's compositor and Home back itself wherever it got as far as the restart. Tests: `tests/test_launcher_app.py` (node, the page and
  the decision against stand-ins).

Marker `boot-direct-api` is 2: the bundle's compositor is the one without the 5 s hold. The tool refuses `boot-direct on` and the early step on
marker 1, whose compositor would ask a verb that is gone.

## (e) When LumaTV fails

- **The UI dies.** The client exits, stops answering for 75 s (`POLL_TIMEOUT`), cannot start or is not ready in 40 s (`READY_TIMEOUT`):
  `gave_up`, the main loop returns `shell_rust_failure` and the compositor exits with an error. The supervisor's `fallback()` follows at once
  (one failure counted, `stock-ui-held` written), hands the screen to LG's compositor and runs `recover-home.sh --stock`, whose
  `stock-ui-restore` drops the stub by its identity (unmount, SAM read again, `killApp`) and launches LG's Home. The same for a compositor
  that crashes or hangs, and for Home held 10 s. Tests: `shell_rust_failure` after a client exit and after silence (`shell_rust.rs`); the
  stub's drop in `tests/test_stock_ui_restore.py`; the supervisor's own side is its existing path. A hung UI costs its 75 s of a black or frozen
  Home first; the escape is quicker.
- **LumaTV comes back.** After a fault's fallback the set stays on LG's compositor and LG's Home until the owner runs `lumatv compositor on` or
  opens the LumaTV app (the hold `stock-ui-held` also keeps the next boot's `compositor-boot` from starting it; an escape's hold lapses at the
  next boot instead, (d)). `compositor on` puts the stub in before its restart: `homestub_up` ends LG's Flutter Home and the restart closes
  every client, so none lingers (`bd_manual_stub`: tests of its order and of every case where it does nothing). A boot does the same in its
  first step.
- **A compositor that runs with no UI** (the Rust shell is not configured) with the stub in front is a black screen, and the escape is the way
  out. No UI can appear later in a running compositor: the client giving up ends it.
- **A stub that does not start** after the restart (three tries) gives LG's Home back and counts a fault.
- **LumaTV not starting at a boot** (storage low, firmware not on the verified list, a refused mount, a failed `native_start`, a failed or
  interrupted activation): the stub goes in before those checks, so each of them takes it back and opens LG's Home. `compositor off` ends as
  before (`comp_switch off`).
- **A boot that stops** (a power cut, a killed worker) leaves nothing but `boot-direct.pending`: the bind mount and SAM's reading are gone at the
  next boot, which skips the stub and counts a fault. Two faults in a row switch the flag off. A finished boot clears both. An escape in the
  middle of a boot is not a stop: the restore clears the mark, and nothing is counted while the hold is there.
- Deleting the flag file by hand gives the old arrangement at once (native Home over a stub Home) until `boot-direct off` or the next boot.

## (f) Risks, rollout, rollback

Risks, all untested on the TV:
1. The windows of (a): a Flutter Home launched before the stub. The check says so if it happened.
2. SAM not answering at 12 to 14 s: the early step waits up to 20 s for it after binding the stub; if it never answers the boot goes without
   the stub and the check fails.
3. The stub's launch after the restart races WebAppMgr's (`bd_home_up`, three tries; timing unmeasured).
4. A preload-manager relaunch of Flutter Home behind the stub (webOS 11.2 report above).
5. The real remote's Home key at 10 s: the supervisor's reader starts the hold at the first press or repeat and ends it at the release, so it
   works whether the key repeats or not; the gate's `homekey` did it on a virtual remote, the real one has not run.
6. Standby, wake and the saver with the stub as Home: the same card in the compositor's eyes, but not run.
7. The sweep by directory ends what the owner started by hand from LumaTV's directories too, and a tool or the boot worker in the middle of a
   step: a SIGKILL, like a power cut, which those steps are built to survive. The helper holds the log's collector, so nothing is logged after
   the escape but the bundle's `ended:` line.
8. LG's compositor runs with the stub bound for about 10 s before the restart (longer if the wake preparation times out): a Home opened by
   hand in that time is blank.

Rollout, each stage stopping on a surprise:
1. Install with the flag off; `check-boot.py` still PASSes (the boot is unchanged).
2. `boot-direct on` on the live compositor: the stub goes in, no reboot; Home held 10 s (LG's Home, nothing of LumaTV's left), the LumaTV app
   back, `boot-direct off`.
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
   `/var/lib/lumatv/compositor/boot-direct-api` (marker 2). The display restarts twice.
3. `tools/lumatv` and the launcher app's files (`index.html`, `launch.js`, `app.js`, `style.css`) as a copy-only install like `step3_install.py`
   of 2026-11-03 (`.new`, `sh -n`, sha256, atomic `mv`, hashes recorded).
4. `lumatv boot-direct status` reads `flag false, api true`; `ls $APP/payload/homestub` shows `appinfo.json index.html icon.png`;
   `check-boot.py --now` PASSes.

The plan (the owner's, 2026-11-04):
5. Enable the flag: `lumatv boot-direct on`. Home stays LumaTV's; status `stub true, sam web, flutter false`; `ctl shell` `bootDirect.on true`.
   Home pressed briefly, or held for several seconds and let go, is LumaTV's Home each time.
6. Hold Home 10 s on the real remote. LG's Home appears and **no LumaTV process is left**:
   `ls -l /proc/[0-9]*/exe 2>/dev/null | grep -e /var/lib/lumatv/ -e org.nphil.lumatv` prints nothing (the compositor, the UI, the helper, the
   colour reader, the player and the feeder are gone), `boot-direct status` reads `stub false, flag true, pending false` with the strikes as
   they were, `stock-ui-held` and `escaped` are there, `failures` is what it was, and the newest crash bundle (`/var/lib/lumatv/crash`) says
   `kind: escape` and lists what it `ended:`. Holding Home 10 s again on LG's Home changes nothing.
7. Open the LumaTV app from LG's launcher: back in LumaTV with no key pressed (a recovery that is still running is waited for). `boot-direct
   status` reads `stub true, sam web, flutter false`, there is no Flutter Home process, `stock-ui-held` and `escaped` are gone.
8. Reboot: LumaTV, with no Flutter Home. First `echo 240 > /var/lib/lumatv/bootcap` (frames and `state.txt` of the boot, `tools/bootcap.sh`), and
   hold Home 10 s once more before the `reboot`, so that the boot is the one after an escape: its log has "the escape of an earlier boot has
   lapsed". After 3 min, **before touching Home**: `python3 /data/home/tmp/lumatv-boot-direct-20261103/check-boot-direct.py` (also `--json`). It
   exits 3 if LG's Home was asked for during the boot, and says so.
9. Cold power-cycle (off at the remote, then on; or the plug): the same as 8 without the escape, for the case where bootd may open Home itself.

Crash drill (optional, one counted failure):
10. `pkill -f lumatv-ui` on the TV: the compositor ends, the supervisor falls back, LG's Home appears with the stub gone
    (`boot-direct status`: `stub false, flutter true`) and the hold stays across a reboot. `lumatv compositor on`: LumaTV's Home with
    `stub true, flutter false`.

Capture: the check's output and JSON, the bootcap frames, `/tmp/.lumatv-boot-direct-early`, the boot's lines from `/tmp/lumatv-logs/current.log`,
the Unraid syslog from the boot, the `/proc/*/exe` listing before and after step 6, the crash bundle, the installed hashes. `check-boot-direct.py`
adds to `check-boot.py`: the early step's record (the stub in, nothing of Flutter running before it); no `NL_HOME_SHOWN`, SAM launch of Home
before the stub, Flutter Home client or Flutter process at any point of the boot; the stub mounted and SAM reading `web`; no fault, strike or
mark left; Home in front, drawn by the native UI, LG's Home not in front. It prints the native Home's time against 33.5 s and from which second
LG's log covers the boot.

## Not built

The worker reorder, the shader disk cache, a stub before the hook (not possible), the preload policy call, a native stub, per-step timing inside
`native_start`; `tv-cutover.sh enable` does not place the stub (run `lumatv boot-direct on` after it); the old QML-era `homestub_*` code stays,
since `stock_ui_restore` and the stub's placement use it.
