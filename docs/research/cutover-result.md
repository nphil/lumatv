# Cutover: LumaTV's compositor as the live one across boots

Watchdog, firmware check, hold-Home escape, the boot-time enable path and the frame gate of C4 (PLAN: "a watchdog and
a boot-time firmware check switch back to it on any failure, and the hold-Home escape forces it. Gate: the X1 test with
zero frames over 20 ms in 60 s"). Offline work, 2026-10-25; nothing here has run on the set. Facts marked
[INFERENCE] were not observed.

## Resume here

State (2026-10-25, commits `ba6e1ab` and `fe0003f`, and this note's): everything below is built and host-tested, and none
of it is verified on the TV. The flag that makes LumaTV's compositor start at boot (`enabled`) was never set by this work
and nothing here sets it except the user's commands (`lumatv compositor on`, `cutover.sh enable`) and the gate's own
`--enable-after` after a full pass. `compositor/firmware/verified.list` has no entry, so a persistent enable is refused
on the set's firmware (webOS 10.2.1-5202) until the gate has passed on it.

Checks that ran here: `sh compositor/sm/build.sh` (arm and host builds, the supervisor's C tests, no warnings);
`python3 -m unittest tests.test_compositor tests.test_cutover tests.test_lumatv_inputcommon tests.test_surfaces_registry
tests.test_tool_rules tests.test_keyboard_patch tests.test_inputcommon_stage` all OK (51, 72, 17, 5, 25, 11 and 10 tests;
the shell library also under `set -u`, and `RealSupervisor` runs it against the supervisor's own host build). Mutations of
the library, the cutover script, the gate rule, the gate driver and the slot rule were each caught by at least one test.
Not run to the end: `unittest discover` of the whole suite (`tests/test_settings.py` alone runs over four minutes, and
`tests.test_voice_search` imports `test_settings` as a top-level module, so it only loads under `discover -s tests`).

Seventh gate (2026-10-25 19:29-19:38, `4ee268a`, `docs/research/compositor/slot-2026-10-25-1929/`): steps 1 and 2 passed on the set
(`install`: `firmware: unverified: verified.list has no entry`, `supervisor-api` 2; `firmware` PASS with 10 files hashed in 0 s from the
cache, none unreadable, every bad list refused; `homeheld` PASS: `/dev/uinput` and python3 are there, `--home-held` 0 while the virtual
remote held Home and 1 after). Step 3's session died 3 s in (the compositor's InvalidId panic, `compositor-c2-result.md` "Seventh gate
on the set"), the supervisor fell back by exec as designed (`fallback: compositor closed the bridge socket`, LG's Home on the panel 4 s
later, SAM unchanged), and `live`, `homekey`, `wd-hang`, `wd-crash` and `escape` therefore ran against LG's compositor: unproven. The
trial before it lived 122.9 s with `watchdog 1 power-seen 1` (the `getPowerState` reply has the `state` key: the empty-panel rule is on).

Next: the compositor's dying-client guard, then step 3 again.

## What decides whether LumaTV's compositor runs

| Layer | Decides | Where |
|---|---|---|
| Flag | `enabled`: empty or `persistent` holds across boots; `session <boot id>` only in the boot it was made in | `comp_flag`, `decide()` |
| Firmware | a persistent enable needs this build on `verified.list`: SHA-256 of LG's binary and 8 libraries, and the supervisor's fingerprint | `comp_fw_check`, `decide()` |
| Bundle | every file the size install recorded, `bin/` also its SHA-256; the four files the supervisor cannot start without | `comp_bundle_ok`, `bundle_missing()` |
| Supervisor | `lumatv-sm --would-run` (the same `decide()` as its start): escape recorded, fingerprint, failures; a binary that cannot run is `broken` | `comp_sm_says` |
| Boot | the previous boot completed (`.boot-pending`), LG's binary unchanged, Home not held | `comp_boot_decide` |

The early hook (`lumatv compositor-boot`, before `apply`'s restart) mounts only when every layer says yes; it answers one
word for the log (`mounted`, `already`, `off`, `failsafe`, `firmware`, `bundle`, `escaped`, `failures`, `session`, `home`,
`broken`, `error`) and logs the reason. The supervisor decides again at its start, so a mount that should not be there
costs one restart and falls back. `lumatv compositor check` (and `cutover.sh boot-check`) prints that decision as JSON
with the firmware hashed for real and nothing changed, which is how the boot path is read on the set without a reboot.

Session or persistent: `cutover.sh trial` and the gate's `enable --session` write `session <boot id>`, valid in this boot
only and on any firmware; the hook and the supervisor ignore (and the hook deletes) one left from an earlier boot, so a
test cut short can never become a boot-time enable. `enable` and `lumatv compositor on` write `persistent`.

State files in `/var/lib/lumatv/compositor`, new ones in bold: `enabled`, `escaped`, `failures`, `fingerprint`,
`lg-binary`, `last-fallback`, `sm.log`, `compositor.log`, **`verified.list`** (copy of the repo's), **`manifest`** (SHA-256
of LG's files as installed: LG's binary is hidden once mounted), **`bundle.manifest`**, **`supervisor-api`** (2: the
binary answers `--would-run` and `--home-held`; an older one would take them for a start and exec LG's compositor from the
hook, so it is never asked), **`running`** (below), **`fw-verified`** / **`fw-unverified`** (the last firmware verdict
and the sizes and times it was made on; a boot that finds them unchanged reads ten `stat` results instead of hashing 60 MB).

## Watchdog

Falls back to LG's compositor by exec in place (the unit's main process becomes LG's binary; nothing is restarted, SAM
included) and brings WebAppMgr and Home back (`recover-home.sh`, which restarts `webapp-mgr`, and `booster` and
`maliit-server` when they run; never `sam.service`). Counted as a failure (two in a row keep LG's):

- the compositor exits or dies (existing), misses two heartbeats of 500 ms (existing), does not connect in 20 s (now on
  the monotonic clock: the wall clock is unset and jumps while a boot syncs time);
- **frozen output**: frames drawn (`stats.drawn`) and no page flip (`flips` in the summary) for 6 s and 20 frames; an
  idle screen draws nothing and a landing flip is progress, so neither counts;
- **empty panel**: no window mapped (`ctl windows`) for 45 s while the set is on (tvpower's state `Active`, from a
  `power2/getPowerState` subscription of the supervisor's own); a boot into standby is no black panel. Both rules read the
  compositor's control socket every 2 s with a 150 ms bound and are skipped while the heartbeat is late;
- **a run that never settled**: `running` is written when a start is decided and removed after 120 s healthy, on SIGTERM
  and on a deliberate fallback; one found at the next start (power cut on a black screen, kill, hang of the supervisor
  itself) counts as a failure, in the hook's question (`--would-run`) and at the real start. Ten minutes healthy still
  clears the failure count.

Not failures: the hold-Home escape and a Home held at start (recorded as `escaped`: LG's until `lumatv compositor on`).

## Hold Home at boot

The compositor's escape (650 ms, `input.rs`) needs the compositor running and reading the remote. The supervisor now
reads the evdev nodes itself, from its first moment, and rescans every 6 s (the compositor opens the nodes once, so a
remote that pairs later is invisible to it): Home (`KEY_HOME` 102, `KEY_HOMEPAGE` 172) down for 650 ms is the same
escape. At start, Home already down for 700 ms is the escape before anything of LumaTV's runs (`escaped` =
`hold Home at start`); the early hook asks the same question (`--home-held`) before it mounts (`hold Home at boot`). The
network key devices (`Smart Remote RCU Input`, `LGE Network Input`: the SSAP keys of automations and apps) are left out of
the supervisor's reader: it is a person's escape, at the TV.

## Firmware check

The list holds, per build, the release line, the supervisor's fingerprint and `file <sha256> <size> <path>` for LG's
compositor binary, `/etc/starfish-release` and the libraries the compositor is built against (`libmali.so.0.46.0`,
`libWebOSCoreCompositor`, `libwayland-webos-server`, `libwayland-server`, `libluna-service2`, and the client libraries
whose protocol tables `protocol/lg-tables.txt` re-implements: `libwayland-webos-client`, `libgm-wayland`,
`liblsm-connector`). A file that cannot be read, a missing one, an entry that covers other files, another release line or
one changed digit is `unverified`. A firmware update therefore starts on LG's compositor, by name and by hash, until the
gate has passed on it and `tv_gate.py --enable-after` (or `cutover.sh firmware record`) has put the new entry in the list,
which is then committed, deployed and installed. `lumatv compositor status` shows `firmware` as `verified`,
`unverified` or `unknown` (changed since the last check) from the cache, never hashing.

## The frame gate

`tv-gate.sh x1`, tested without the set (`tests/test_cutover.py`, `FrameRule`):

- no gap over 20 ms between the client's swaps, the first second included (no warm-up is excused; Nitin's call, open
  since the third gate); a gap of exactly 20 ms is in the 17.5-20 bucket and passes;
- the run is the whole 60 s (`x1 SECS` under 60 cannot pass) and holds 59.5 fps or more;
- the pacer's `late` did not move;
- the compositor's own log between the client's start and its end holds no loop pass, client frame gap or page-flip
  interval over 20 ms (`loop: N ms`, `frame gap N ms`, `page-flip stamp N ms`), and the log could be read.

On the fifth gate's result line the rule gives `FAIL 2 gap(s) over 20 ms in the client's swaps (worst 22.5 ms); pacer late
1 -> 2`. The sixth gate's x1 (worst 21.0 ms, two gaps, `late` 0 -> 0) fails the same way. `tv_next_slot.py` applies the
same limit to the trial (`trial_problems`): no fallback of the run, no protocol error, nothing over 20 ms in the log, the
watchdog reading its inputs.

Defect found and fixed in the gate's verdict: `tv_gate.py` counted every `GATE ...` line that was not a PASS, and
`GATE standby: requested` (the power-off call going out) is one, so no gate could pass whatever the checks said. A verdict
is now a line that says PASS or FAIL.

## What the gate runs now

`firmware` and `homeheld` (LG's compositor live), then a session of `enable --session` with `live`, `volume`, `x1`,
`netflix`, `hdmi`, `evidence`, `standby`/`woke`; then `homekey`, `wd-hang`, `wd-crash`, each ending the session and each
followed by a fresh one (`live after-<check>`); then `escape` (the compositor's own routing), then `disable`.

- `live` also needs the supervisor's log to say the watchdog reads both inputs and tvpower's state;
- `firmware`: the entry made from the set's own files (10 hashed, none unreadable, seconds taken) must verify against a
  scratch list; a flipped digit, another release line and an empty list must each be refused; the set's own list's verdict
  and the supervisor's answer are reported. Nothing the set holds changes;
- `homeheld`: `tv-uinput-key.py` (a virtual keyboard through `/dev/uinput`, python3) holds Home for 3 s; `lumatv-sm
  --home-held` answers 0 in the middle and 1 after;
- `homekey`: a virtual remote made after the compositor started (it never opens it) presses Home 9 s later; only the
  supervisor's rescan can see it. Expected: unmounted, `hold Home 650 ms (supervisor)`, `escaped`, failures unchanged;
- `wd-hang` (SIGSTOP) and `wd-crash` (SIGKILL) of the compositor: back on LG's within 15 s and 10 s, `failures` +1, no
  `escaped`, the compositor gone, Home on the panel;
- every fallback check (`escape`, `homekey`, `wd-*`) compares SAM's pid and start time before and after: unchanged.

## TV steps (not verified on the TV)

Each a slot of its own, receiver muted by the scripts, the set idle; LG's compositor is live at the start and at the end.
`python3 compositor/tools/tv_next_slot.py` runs steps 1 to 3 in order.

1. `sh compositor/tools/tv-first-light.sh deploy`, then `cutover install`. Expect `firmware: unverified: verified.list has
   no entry`, `supervisor-api` 2, a `bundle.manifest` line for every file of `bin/` and `lib/`. `cutover status`:
   `flag: off; firmware: unverified`.
2. `gate.sh firmware` and `gate.sh homeheld` (the slot does both). Expect firmware PASS: 10 files hashed (seconds taken
   is the figure the boot path avoids by the cache), none unreadable, the supervisor `no off: not enabled (exit 3)`.
   A missing library name here (the check lists the paths) means the guarded list needs that path corrected: the paths
   are from the census, not read on this set. homeheld needs `/dev/uinput` and python3; neither is confirmed.
3. `cutover trial 120` and the gate. Read, in order: the trial verdict line in `summary.txt`; `GATE live` (`watchdog 1
   power-seen 1`: if `power-seen` is 0 the reply of `power2/getPowerState` has no `state` key the supervisor reads, and the
   empty-panel rule is off); `x1` (the rule above); `homekey`; `wd-hang`; `wd-crash`; `escape`. Anything after `escape`
   is LG's compositor again.
   With Nitin at the set, in a session of its own (`cutover.sh enable --session`, then `disable`; no script can make this
   check): tap Home ten times on the physical remote and read `sm.log` (no `fallback:`), then hold it for a second
   (`hold Home 650 ms`, the supervisor's or the compositor's line, whichever reads the key first). The supervisor's Home
   reader takes a tap as a press and a release; a remote driver that never sent the release would make every tap an
   escape, and this is where that shows. Do it before step 4.
4. After a pass of every check (x1 included): `tv_gate.py --enable-after` lists the build, installs, enables for good and
   runs `live`. Commit `compositor/firmware/verified.list` and the gate evidence.
5. Boot path without a reboot: `lumatv compositor check` should say `would-mount`; `lumatv compositor status`
   `firmware: verified`.
6. A restart of the set (Nitin's OK: the approval of 2026-10-25 covered the logo check): LumaTV's compositor should be live
   by itself, `sm.log` showing `start:`, `settled: two minutes healthy` two minutes later, `compositor-boot: mounted` in
   the tool's log. Then a restart with Home held from the power-on key until LG's UI appears: `compositor-boot: home`
   (`escaped` = `hold Home at boot`) or, if the key came down after the hook, `hold Home at start`; LG's compositor
   either way, and `lumatv compositor on` to come back.
7. Two power cuts within two minutes of two boots: the third boot is LG's (`failures` 2, `compositor-boot: failures`).
   Do this only with Nitin at the TV.

## Open

- The `power2/getPowerState` reply is read for a `state` key (`Active` ends in the set being on); its shape is from the
  compositor's own use of it, not captured. `GATE live` says whether it was read.
- The empty-panel rule counts mapped windows, and under LumaTV's compositor today Home is the stub's transparent window:
  a black Home is not an empty panel. When LumaHome is drawn by the compositor, "nothing visible" can be added to it.
- The supervisor's two control queries every 2 s are two loop passes on the compositor's side; if x1 gaps ever recur
  every 2 s, the `loop:` lines (pass `control-client`) will say.
- `compositor/sm/` is not under any owner's list; the supervisor is the watchdog, so it was changed here.

## Also wired in `tools/lumatv` (LgRestyles' leftovers, at Main's request)

- `lumatv inputcommon on|off|status` runs `tools/inputcommon.sh` under the mutation lock and prints its JSON (`status` with
  `"on"` in front). The switch is the flag `.inputcommon-on`, off by default and changed only when the tool says ok.
- The registry (`lumatv surfaces`) has the row `inputcommon` (way back: `inputcommon off`). `apply` mounts when the flag is
  on and the overlay is not in place (the mount does not survive a reboot), beside the other overlays, and leaves one that
  is: `on` drops inputcommon's process snapshot, which at every launch of the app would cost the next input switch a cold
  start. After an update of the app, `inputcommon on` stages the new patch. `revert` turns it off. Nothing in it restarts a
  service.
- `build_shadow` runs `tools/keyboard_patch.py` on the shadow module's copy of `views/StarfishKeyboardView.qml` with the
  shipped theme's motion; a view LG changed (an anchor missing, the theme without usable motion) is left as LG's and the
  refusal is logged. It is not behind a switch (the shadow is LumaTV's own copy of the module), matters only while LG's
  compositor is live, and takes the theme's motion as of the build: a changed `layerMs` shows after the next rebuild.
- Tests: `tests/test_lumatv_inputcommon.py` (verb, wiring, the shadow build on LG's own view), the registry's.

CHANGELOG line: "LumaTV's compositor starts at boot only on a firmware build the gate passed on; a supervisor that falls
back on a frozen output, an empty panel, a run that never settled and a held Home key."

On the set (2026-10-25 19:43): the ipk does not carry `tools/inputcommon.sh` or `tools/inputcommon_patch.py` (`tools/build-ipk.sh`
lists neither), so `lumatv inputcommon on|off|status` answers `{"ok":false,"error":"tools/inputcommon.sh did not answer"}` on a set
installed from it. With the two committed files put in the app's `tools/` by hand the verbs work (`on`: mounted, `snapshotsDropped 1`;
`off`: unmounted; `status` right); results in `lg-restyles-result.md`. The builder needs both files (and `tools/keyboard_patch.py`,
which `build_shadow` runs and the ipk does not ship either) in its list.
