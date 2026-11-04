# Boot direct: LumaTV from the first frame, LG's Flutter Home out of the way

Built behind a flag that is off by default: `lumatv boot-direct on|off|status`, which writes `/var/lib/lumatv/boot-direct` (the line `on`, and
a `tool PATH` line naming `tools/lumatv`). With the flag off the boot, the compositor and the keys are what ran on 2026-11-03; the tests
that pin that are unchanged. **Nothing here has run on the TV.** Host tests: `tests/test_boot_direct.py` (the tool, 35), the new rows of
`tests/test_native_startup.py` (activation order, flag on and off), `boot_direct.rs` (20 and one in `shell_rust.rs`) and `tests/boot_direct.rs` (8).

## What it does

- LumaTV's compositor still takes over at the boot hook (~13 s) and the restart (~25.7 s); it cannot be bound before LG's surface-manager.
- Flutter Home is replaced by the web stub of 2026-10-25 (LG's Home id, bound over LG's directory) at the activation, before the restart.
  Every launcher of Home opens the stub, so Flutter Home does not run.
- The stub's card is Home to SAM, bootd, the wake and the saver. LumaTV's native Home draws over it and starts with the compositor.
- A Home hold of 1 s opens LG's real Home. Home or Back there, or an app taking the front, puts LumaTV's Home back.
- Every way out of LumaTV's compositor already drops the stub. A boot that stops half way, or a stub that does not start, is a fault; two in a
  row switch the flag off.

## (a) Binding before LG's surface-manager starts

Not possible for a root mod. The set resumes from a hibernation snapshot; `/var` is writable after `mount-readwrite` at 10.8 to 13.1 s, and the
earliest hook that can run there, `kdump.service`, starts the worker at ~13 s. surface-manager is up at 2.6 s, SAM at 5.3 s, bootd launches the
first app at ~13.9 s. Putting LumaTV in the snapshot or the read-only image is in HANDOFF's "Tried and dead".

The owner's reboot of 2026-11-03 21:41 (Unraid syslog, uptime in seconds):

| Event | s |
|---|---|
| boot worker starts | ~13 |
| `compositor-boot` done (mount, `native_start`) | 22.7 |
| `lumatv-sm` starts, compositor started | 25.7, 26.5 |
| engine built (48 programs linked, none from a cache) | 30.75 |
| compositor ready | ~31 |
| SAM's `last_input_handler` launches Home; Flutter Home connects | 32.86, 32.91 |
| UI client started (40 ms after Home maps) | 33.08 |
| native Home | 33.54 |

`boot-bind-first` (96563425, "bind before the compositor's first start") and `boot-no-screen-on` (d38e3161) are main's 0ec00c7f and a3ad63b9
apart from README and repo.json: the worker already mounts at 22.7 s and restarts at 25.7 s. Nothing is left to merge.

What boot direct changes on this path: Flutter Home does not start, and the UI client starts at the compositor's ready (~31 s) instead of
when Home maps, so the native Home is expected ~2 s earlier (an estimate, to be measured). What it leaves: `compositor-boot` takes 9 s
(storage guards, firmware and bundle check, `native_start`: helper, capture, vplane, preload, frame app, GFN, feeder) and the shader link 3.6 s
(an in-memory cache). Per-step times are not measured. Candidates, none built: activate right after the mount with `native_start` in
parallel, a disk cache for the programs, the stub at the hook (13 s) so that bootd's first launch gets it. They need the timings first.

## (b) Keeping Flutter Home from launching

The stub: `payload/homestub`, a blank transparent web page with LG's Home id and category, bound over `/usr/palm/applications/com.webos.app.home`
with SAM made to read the directory again (`blockSystemApp`, `releaseBlockSystemApp`). It is the only option that covers every launcher seen
in the set's logs: SAM's `last_input_handler`, bootd (`bootingHome`), the preload manager (Home is `preloadMode: criu`; SAM logged its launch of
Home at 60 s on the 20:38 boot), the Home key (`launchDefaultApp`), `luna-send` and apiadapter. They all open the id.

| Option | Why not |
|---|---|
| `setDefaultApp` / `defaultApps.home` | only the Home key; power-on and `last_input_handler` still open `com.webos.app.home` |
| `blockSystemApp` left on | persists in `blockedSystemAppList`, and breaks LG's Home as the fallback |
| `sam-conf-tv.json` `BootTimeApps` | read at ~5 s, before any hook |
| close Home after it launches | the launch, its CPU and RAM and its shelf calls still happen; bootd and the preload manager launch it again |
| a native (non-web) stub | SAM's native lifecycle (`registerApp`, roles) is unproven |
| report Home in front with no Home app | SAM, bootd and the Home key would still start Flutter Home; SAM's reaction to a foreground app that is not running is unknown |

Cost of Flutter Home under LumaTV's Home, measured 2026-10-25: RSS 72 MB hidden, 80.5 MB in front (PSS 35.6, 45.6); the stub 32.4 MB (PSS
15.3). It makes network shelf calls (`NL_SHELF_*`) and logs `NL_HOME_SHOWN` at each launch. No receipt of 2026-11-03 measures its CPU or RAM. The
Glasshouse project (rorygallagher2024/glasshouse) uses the same bind under LG's id; it reports that on webOS 11.2 the preload manager started
Flutter Home again behind its stub and that `preloadmanager/setPreloadPolicy` for the id turned that off. This set runs 10.2.1, where the
2026-10-25 stub never saw it; if it shows up, the policy call is the fix (not built).

Placement, in `verb_compositor_boot_activate` and only there, behind the flag: `bd_stub_up` (the stub in, a mark `boot-direct.pending` written),
the restart, then `bd_home_up`. WebAppMgr restarts with the compositor, so SAM's own launch of the stub can be lost; `bd_home_up` waits for
WebAppMgr, launches Home and looks for it in SAM's list, three tries, and gives LG's Home back when it never runs.

## (c) How LumaTV stands for Home

The stub's card is `com.webos.app.home`, mapped and transparent. SAM, bootd, the wake and the saver see Home in front exactly as with Flutter
Home; the 2026-11-02 saver fix (`saver-own-result.md`) relies on that and holds. The compositor draws the native Home where that card is in
front (`scene.rs`, `shell_home_drawn`) and its foreground list comes from real windows only. The one difference is the UI client's start: it
waits for SAM's Home to map (`START_ALONE`, 15 s at most), and a stub's launch waits for WebAppMgr, so with the flag on it starts at once.

## (d) Long-press Home: LG's Home, and back

`boot_direct.rs`, all of it inert while the flag is off.

- Home held 1 s (the Router's hold, `keys::HOLD`) runs `lumatv home lg`: the stub is dropped, SAM reads Flutter, Home is launched. The release
  of the hold is not a Home tap.
- When LG's Home is the front card (`com.webos.app.home` whose client is `flutter-client -i com.webos.app.home`, which the stub is not),
  the native Home is not drawn (`set_home_rust(false)`, the path a dead UI already took). The UI's other layers (volume, toasts, the switcher)
  stay over LG's Home.
- Home or Back there, or any other app in front, runs `lumatv home luma` and the native Home is drawn again at once, over LG's card, which the
  tool replaces by the stub within seconds. With an app in front the stub is not launched over it: the next Home key opens it.
- LG's Home that is not in front after 15 s is given up; the native Home was never taken away.
- The tool runs as a child of the compositor, one at a time, the latest ask after the one that runs, stopped at 120 s, with the `NOTIFY_SOCKET`
  of the supervisor removed. It takes the same lock as every mutation, waiting 10 s.
- The supervisor's own escape (Home held 10 s) is its own evdev reader and is not touched.

## (e) When LumaTV fails

- The stub goes in only inside `compositor-boot activate`, after the storage and firmware checks and the mount. A failed restart, or a
  compositor that is not ready, drops it (`bd_abort`) before LG's compositor is restored.
- The supervisor's fallback (a crash, failures at 2, Home held 10 s) runs `recover-home.sh --stock` and `stock-ui-restore`, which already drops
  the stub by its identity (unmount, SAM read again, `killApp`). `compositor off` ends the same way (`comp_switch off`).
- Nothing persists but the flag file: the bind mount and SAM's reading are gone at the next boot.
- A boot that stopped between the stub going in and Home launching leaves `boot-direct.pending`: the next boot skips the stub and counts a
  fault. A stub that never starts counts one. Two in a row switch the flag off. A finished boot clears both.
- The UI client given up (`gave_up`, which it never leaves) with the stub in front for 5 s asks for LG's Home: the stub alone is a black screen.
  Home and Back are then LG's own, so LG's Home cannot be left for a Home that is not there.
- Deleting the flag file by hand gives the old arrangement at once (native Home over a stub Home) until `boot-direct off` or the next boot.

## (f) Risks, rollout, rollback

Risks, all untested on the TV:

1. The stub's launch after the restart races WebAppMgr's (`bd_home_up` is the answer; its timing is unmeasured).
2. A preload-manager relaunch of Flutter Home behind the stub (webOS 11.2 report above).
3. The real remote's Home key: whether it repeats while held (the hold is timed from the first press, so it works either way), what the
   Router does with a held Back on LG's Home, and that the arrows and OK reach LG's Home while it is shown (the UI is told Home is not
   drawn, as it is with an app in front, which is the proven case).
4. Standby, wake and the saver with the stub as Home: the same card as before in the compositor's eyes, but not run.
5. `home lg` and `home luma` while the boot worker or another mutation holds the lock: after 10 s the request is dropped; the 15 s wait ends it.
6. A black screen between the stub going in and the compositor's restart (about 3 s, in a boot nobody watches).

Rollout, each stage stopping on a surprise:

1. Install with the flag off; `check-boot.py` still PASSes (the boot is unchanged).
2. `boot-direct on` on the live compositor: the stub goes in, no reboot; Home hold, Back, an app from LG's Home, `boot-direct off`.
3. `boot-direct on`, reboot, `check-boot-direct.py`.
4. A day of use with the flag on, then the saver, a standby and a wake.
5. Only then a CHANGELOG entry, and the flag's default in the installer.

Rollback, one step: `lumatv boot-direct off` (the stub dropped, the flag removed, LG's Home opened if the stub was in front). The strikes do the
same by themselves. If the set cannot be reached: Home held 10 s is the supervisor's stock escape, which drops the stub.

## TV test plan

Before anything: the code is on `main`, pushed to origin and forge; the installed hashes go in the handoff.

Install, flag off, one lease session:
1. `sh compositor/tools/build-tv.sh`, then `sh compositor/tools/tv-first-light.sh deploy`.
2. `tv-first-light.sh cutover disable`, `install`, `probe`, `enable` (the sequence of `cutover-result.md`): `install` writes
   `/var/lib/lumatv/compositor/boot-direct-api`. The display restarts twice.
3. `tools/lumatv` as a copy-only install like `step3_install.py` of 2026-11-03 (`.new`, `sh -n`, sha256, atomic `mv`, hashes recorded).
4. `lumatv boot-direct status` reads `flag false, api true`; `ls $APP/payload/homestub` shows `appinfo.json index.html icon.png`;
   `check-boot.py --now` PASSes.

Live, no reboot:
5. `lumatv boot-direct on`: Home stays LumaTV's; status `stub true, sam web, flutter false`; `ctl shell` `bootDirect.on true`.
6. Hold Home 1 s: LG's Home appears, LumaTV's is gone. Back: LumaTV's Home within ~1 s; status `stub true` ~3 s later. Again with Home.
7. Open an app from LG's Home, then Home: LumaTV's Home. `lumatv boot-direct off`: the stub is gone.

Reboot:
8. `echo 240 > /var/lib/lumatv/bootcap` (frames and `state.txt` of the boot, `tools/bootcap.sh`), `lumatv boot-direct on`, power-cycle.
9. After 3 min: `python3 /data/home/tmp/lumatv-boot-direct-20261103/check-boot-direct.py`; keep its output and `--json`.

Capture: the check's output and JSON, the bootcap frames, the boot's lines from `/tmp/lumatv-logs/current.log`, the Unraid syslog from the boot, the
installed hashes. `check-boot-direct.py` adds to `check-boot.py`: boot direct on and read by the compositor; the stub mounted and SAM reading
`web`; Home running with no Flutter process; no `NL_HOME_SHOWN` after the stub went up and no Flutter Home among the compositor's clients; no
fault, strike or mark left; Home in front, drawn by the native UI, LG's Home not in front. It prints the native Home's time against 33.5 s.

## Not built

The worker reorder, the shader disk cache, the stub at the hook, the preload policy call, a native stub, per-step timing inside
`native_start`; the old QML-era `homestub_*` code stays, since `stock_ui_restore` and the stub's placement use it.
