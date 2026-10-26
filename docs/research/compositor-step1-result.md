# Step 1 under LumaTV's compositor: the remote's key routing and the keyboard

Build-order item 1 of "Surfaces under LumaTV's compositor" (`compositor-c2-result.md`): the key filters LG's compositor runs
in its own QML (gear, Input, the app keys, Quick Access digits, Mute held, Exit; Volume and Mute are done) and IME routing
(`text_model` to `input_method`, maliit's own keyboard window), without drawing anything of LumaTV's.

## Resume here

State (2026-10-26): built, committed and host-tested (safety timer aa3a2eb, key routing 21b6013, keyboard path 83faf05, gate checks
d2afb2b, overlay-aware checks 840a0c0). First TV run (Main, 15:33): `live` PASS; `keys`: Input, Netflix button, digit tap and hold,
Exit OK, gear FAIL; `ime` FAIL. Both failures were in the checks, not the routing: LG's Settings and Search are overlay windows, which
SAM never names as the foreground app, and the checks waited for SAM to name them ("First TV run" below); they now look for a mapped
window of the app in `ctl windows`. Home as LG's `RF_HOME` 773 is now Home in the compositor and in the supervisor's hold-Home escape
(05f572e; Main's decision, "Key codes"), and the `escape`, `homeheld` and `homekey` checks hold 773. Next: re-run
`sh compositor/tools/tv-first-light.sh gate --only homeheld,keys,ime,homekey,escape` (deploy and `cutover install` first: the bundle changed).
The TV is not touched by this work.

## What was built

- `keys.rs`: the remote's key filters as a state machine with time as an argument. A gear tap, Input, the app buttons and
  Exit with the keyboard up are taken whole; a tap is decided at the release, a hold at the tick (gear, Input, digits and Back
  1 s, Mute 2 s); three gear taps within 3 s are the AV reset. Every decision is a unit test.
- `input.rs`: feeds it the evdev events and the tick, loads `<data-dir>/compositor/keys.conf` (extra key codes) and the
  theme's `remote.appKeys` (Settings > Remote; re-read when the file changes), and journals the first press of a code from 768
  up that the table cannot name. Back or Exit closes the foreground app, never Home, and the keyboard first when one is up.
- `system_ui.rs`: `State::system_request` is the one place a key asks the system layer for something, and the hook the
  UI channel takes over. LG's Quick Settings, input picker and Quick Access slots are QML inside LG's compositor, so until
  LumaTV draws its own, a request is answered with the nearest app and a journal line saying so: gear tap launches
  `com.palm.app.settings`, Input tap and Input held `com.webos.app.homeconnect` (the Home Hub, LG's own Input-hold target),
  a digit held `com.webos.app.magicnum` (0 held opens its editor), the app buttons the app Settings > Remote chose (nothing
  chosen: LG's own app; `""`: nothing; the app already in front: nothing; LG's own app in front: the app gets the key), gear held
  toggles Settings, three taps `com.webos.app.tvhotkey` (`mute-hidden-action`), Mute held `com.webos.app.accessibility-overlay`.
- `server/ime.rs` (mirrors luna-surfacemanager's `WaylandTextModel`, `WaylandInputMethodContext`, `WaylandInputPanel`): a
  `text_model` activates only on the keyboard-focused surface (another is declined and told `leave`); the input method gets a
  context and the app `enter`; requests and the input method's commit, preedit, delete and keysym answers are forwarded both
  ways. With no input method bound, the activation asks the system for one (`initctl emit ime-activate`, once per 3 s) and goes on
  when maliit binds and names its display. The input method's `grab_keyboard` is a `wl_keyboard` the keys go to while its
  keyboard is on the screen; off the screen, the remote steers the app again at once. The release of a key the app was given
  the press of goes to the app, so the OK that opened the keyboard leaves nothing held. The activation ends on `deactivate`, a
  focus change, the input method or the app going, or Exit.
- `scene.rs`, `window.rs`: a window of kind Keyboard (maliit's `input_panel` surface) never takes the focus and sits full
  width on the bottom edge (or where the app asked with `set_input_panel_rect`), above the app. `refresh_panel` tells the app
  the panel's state and rect.
- `ctl keys`, `ctl ime`, `ctl note TEXT` on the control socket: the table, app-button choices, unnamed codes and counters;
  the bound input methods, the active field, the panel rect, `grab` and `grabbing`, `asked`, the counters (`commits`,
  `preedits`, `keysyms`, `grabbedKeys`); a marker line in the journal.
- The safety timer and the gate (below, and "The TV test").

Host tests: unit tests in `keys.rs`, `tests/keys.rs`, `tests/ime.rs` (a client plays the field, another the input method),
`tests/test_cutover.py` (`SafetyTimer`, `GateKeysAndIme` against a small model of the journal, the routing and SAM, `GateDriver`).

## Key codes

| Key | Code | Source |
|---|---|---|
| gear | 139 (`KEY_MENU`) | read on this set: a tap opened LG's Quick Settings (`shell-host-result.md`) |
| Input | 241 (`Key_webOS_InputSource`) | read on this set (`round8-result.md`) |
| Home | 773 (`RF_HOME`), 172, 102 | 773: LG's table, PlainHome (`home-launcher-replacements.md`), and `perf/kb.py` and round 8's `r8key.py` inject it as Home, which LG's compositor takes; 172 and 102: the routing's earlier codes; the physical Magic Remote's own code is not recorded |
| Back, Exit | 412 or 158, 174 | the existing routing |
| Mute, digits | 113, 2..11 (1 to 9, then 0) | the existing routing |
| Netflix | 1037 (`NETFLIX`, 0x40D) | LG's network-input-service key table (gist Simon34545/fc5c91e0456789dd7a56a947c1148939), not read on this set |
| Prime Video | 1038 (`AMAZON`, 0x40E) | the same table, not read on this set |
| Disney+, LG Channels | none | **unconfirmed**: no source found |

Unconfirmed: Netflix and Prime Video are LG's table's values, not yet seen from this remote; Disney+ and LG Channels have no code
at all, so those two buttons reach the app until one is taught. Teaching a code needs no build: press the button during a
session and read `ctl journal` (or `compositor.log`) for `input: key N has no name in the key table`; add `NAME N` to
`/var/lib/lumatv/compositor/keys.conf` (names: `gear`, `input`, `netflix`, `amazon`, `disneyplus`, `lgchannels`) and start the
next session. `ctl keys` lists the table. Home is not in it: 773, 172 and 102 are all Home in `route()` (`input.rs`, from any device)
and in the supervisor's reader (`lumatv-sm.c`, which still leaves the SSAP key devices out: the escape is a person's, at the TV),
so none of them shows in the unnamed-key line.

## The TV test (Main)

Two kinds of session: the set's own safety timer ends each (12 minutes for the first, 6 for the ones after a check that ends the
live session), the checks' waits add up to under 5 minutes in the first, and the run ends on LG's compositor with the flag removed
and the receiver unmuted. From `/data/home/lumaglass`, LG's compositor live, nothing of ours running (`cutover status`):

    sh compositor/tools/tv-first-light.sh deploy
    sh compositor/tools/tv-first-light.sh cutover install
    sh compositor/tools/tv-first-light.sh gate --only homeheld,keys,ime,homekey,escape

`deploy` builds and copies the bundle, `install` records the firmware and the bundle the session starts from, the gate
(`tv_gate.py --only`) wakes the set, mutes the receiver, runs `homeheld` (not live: a virtual remote holds Home as 773 and the
supervisor's `--home-held` must see it through the key-state bitmap, then stop seeing it), `enable --session --max-minutes 12`, `live`,
`keys`, `ime`, `evidence`;
`homekey` ends that session, so a fresh one follows (`--max-minutes 6`, `live after-homekey`), `escape` ends the second, and
`disable` ends the run. Skip a check with `--skip NAME`; `keys` and `ime` are in a full gate after `hdmi`. If the driver or the
workstation dies, the set ends the session by itself when its timer runs out. Result lines: `GATE homeheld:`, `GATE keys:`, `GATE ime:`,
`GATE homekey:`, `GATE escape:` (also in `docs/research/compositor/gate-<date>.txt`); files in `/tmp/lumatv-tv-out/out/gate/`:
`keys-*.txt` (the journal after each key), `keys.json`, `ime-*.json`, `ime.png` (the panel with the keyboard up),
`homeheld-result.txt`, `homekey-result.txt`, `escape-result.txt`, `compositor-final.log`.

Pass criteria. Home is in front at the start and at the end, and every app a key opened is closed again. An app counts as up when
`ctl windows` has a mapped window of it: SAM's foreground names only the front card, and LG's Settings, Search and Quick Access
(`com.webos.app.magicnum`) are overlay windows it never names.

`GATE keys: PASS` needs all of, each read from the compositor's journal after a `ctl note` and from the window list:
gear tap 139: `input: gear tap: Quick Settings ... launching com.palm.app.settings`, a Settings window (about 8 s to map: 25 s are
allowed); Input 241: `input: Input tap: ... launching com.webos.app.homeconnect`, a Home Hub window; Netflix 1037: `input: app
button: netflix, launching netflix`, a Netflix window; digit 5 (code 6) tapped: no `5 held` line, held 1.4 s: `input: 5 held: Quick
Access slot 5 ... launching com.webos.app.magicnum`; Exit 174 over Settings: `input: Back or Exit: closing com.palm.app.settings`,
no Settings window left. The detail line names each step `ok` or `NOT: step (why)`, then the key table and the unnamed codes seen.
The gear's three taps and Mute held are host tests only (the first opens a service menu, the second could unmute the receiver).

`GATE ime: PASS` needs all of: LG's Search app (`com.webos.app.voice`), launched for its on-screen keyboard
(`{"activateType":"mrcu","launchMode":"vkb"}`, listed as "Virtual keyboard Search" in gist Ircama/1f29c2d2def75da6604756a3c91a8ab4),
with a mapped window within 15 s; the keyboard up within 28 s (if it does not come up by itself, OK (28) on the field is sent at 10 s
and again at 18 s); `ctl ime` showing an input method bound with a display, the field's `context` true, and a `panel` whose y plus h
is 1080 (bottom edge); OK (28) on the keyboard, then the arrows and OK if that key types nothing, making `commits` plus `preedits`
plus `keysyms` rise by at least 1 (the keys reached the field through the grab); Back (412), or else Exit (174), taking the panel off
the screen. The detail line gives `maliit-server.service` and the `MaliitServer` process before and after the keyboard came up, how
many times the system was asked for an input method, how many OK presses it took, the panel rect, what typed and what closed it.
`ime.png` shows the keyboard over the app; a check that fails early still leaves `ime-result.txt`, `ime-journal.txt` and
`ime-windows.json`.

`GATE homeheld: PASS` (the supervisor's `--home-held`, the one check of its key-state bitmap on the real kernel): while a virtual
remote holds Home as 773 the answer is exit 0, after the release exit 1, and the injector exits 0. `GATE homekey: PASS` (the
supervisor's own reader): a virtual remote made after the compositor started holds Home as 773 for 1.5 s; the
session ends as an escape (`hold Home 650 ms (supervisor)` in the supervisor's log, `escaped` recorded, failures unchanged, SAM
untouched, LG's Home on the panel). `GATE escape: PASS` (the compositor's own route): `ctl remote 773` held 1 s; `fallback: hold Home`,
`escaped` recorded, LG's Home back, SAM untouched. Host tests: the compositor's, 773, 172 and 102 each as a tap (launches Home, never
reaches the app) and a hold (asks for the fallback, the release launches nothing again), and 773 never an unnamed key; the
supervisor's C tests, the three codes in its hold tracker and in the key-state bitmap, whose buffer reaches code 773 (checked at
compile time); the virtual remote's default key is 773.

A failure names its reason in the line. Likeliest first: the Search app brings up no keyboard either way (the check then says the
window never came on the screen, and `ime-journal.txt` shows whether a text model reached the compositor: `ime:` lines); maliit
not binding (`asked` above 0 and no method: look at `systemctl status maliit-server`); the keyboard up but nothing typed
(`ime-typed.json`: `grab` false means maliit never asked for the keys, `grabbedKeys` 0 with `grab` true means they never reached
it); a button code the table does not know (the `keys` step that did nothing, and the unnamed-key line above). The Netflix step
follows Settings > Remote: a button the owner pointed at another app is expected to open that app, one set to nothing to open nothing.
A `homekey` that fails at its injector (`homekey-inject.txt` says `uinput: ...`, injector exit 1) means this kernel does not take key
code 773 on a virtual device.

## First TV run (Main, 2026-10-26 15:33)

`live` PASS. `keys`: Input (the Home Hub, a card, mapped 0.5 s after the key), the Netflix button (1.3 s), a digit tapped (nothing)
and held (Quick Access, `com.webos.app.magicnum`, an overlay, mapped 0.6 s after the hold fired) and Exit (`closing
com.palm.app.settings`, Home back) passed. The gear's journal line was there and Settings mapped as an overlay 8.1 s after the tap,
but SAM kept naming Home (the compositor's push listed `['com.webos.app.home', 'com.palm.app.settings']`), so the check that waited
for SAM to name Settings failed. `ime` failed the same way (Search is an overlay), yet the compositor log has the whole path:
Search launched with `{"activateType":"mrcu","launchMode":"vkb"}` mapped 1.5 s after the launch and activated a text model 0.5 s later
with no key pressed; the compositor asked for an input method (`initctl emit ime-activate`) and `/usr/sbin/MaliitServer` (pid 4071)
connected 185 ms later (`maliit-server.service` was `inactive` before; its state after was not recorded, the detail line now says);
it bound `input_method`, made three surfaces (two of them the keyboard's by `input_panel`), grabbed the keyboard, and one mapped as
Keyboard, 1920x438 at (0, 642), y plus h 1080. The compositor offers `text_model_factory` 1, `input_method` 2 and `input_panel` 1 (LG's
versions) and the journal's bind line names the version the input method took. Typing on the keyboard and closing it were not
reached in that run.

## Safety timer

`cutover.sh enable --session [--max-minutes N]` (default 15, whole minutes 1 to 1440; a trial gets its seconds plus 5
minutes) starts a detached job on the set that runs `cutover.sh disable` after N minutes unless the session has ended by then;
`disable` and `revert` cancel it, a new session replaces it, a persistent enable has none. A session whose job did not write
its record within 3 s is not mounted (exit 8). The gate's first session is 12 minutes, each later one 6 (the last gate: 4
minutes, then 33-35 s each). `cutover.sh status` shows the minutes left; `safety-timer.log` in the state directory says what
the job did. Details: `cutover-result.md`, "Safety timer". Host tests: `tests/test_cutover.py`, `SafetyTimer` (the job runs
for real at one second to the minute).
