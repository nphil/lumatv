# Card app: a 64-bit LumaTV app as a registered card window

Todo: "64-bit app as a registered card window (launch, Back, Home, switcher)". A 64-bit app drawn with LumaShow's toolkit (`rustui/`) on the 64-bit Mali blob (`gfx64/`) has to run as a real webOS app under LG's compositor today and, unchanged, under LumaTV's own: installed with the ipk, started by SAM, shown as a card (not an overlay), Back and Home behaving as in LG's apps, listed in the app switcher, named by SAM's foreground info.

## Resume here

State (2026-10-25 evening, offline): the second TV run (17:41-17:45, "TV results") at `f3ed5a6` ran the 64-bit app under SAM's jail: launched as a fullscreen card in 2.4 s, Mali-G52 through the 64-bit blob, buffers from `/dev/dma_buf_unified` (4-20 ms each), the page on the panel (`tv-20261025-1741/status.png`), registered with SAM, SAM's foreground `processId` the app's, left running, minimized and idle under Home, in the switcher, back by relaunch in the same process, ended by SAM's `close`. What failed: no key reached the app under LG's compositor (OK, Right, Down, Back: no `input Key` line, although `getFocusedWindowInfo` named the card as the focused fullscreen window), so the keys, Back and Back-held checks failed. The app was removed again; the set is as found.

The cause, read from LG's compositor's source (`luna-surfacemanager`, "What the set does"): LG's compositor has one `wl_seat` per input device and sends a key on the seat of the device that pressed it; `window.rs` bound the first `wl_seat` global only, the default seat, which no remote's key goes out on. Fixed: every seat is bound, the ones that appear while the app runs too, with a keyboard each (host-tested with a seat added after the app started and a key on it alone). The TV script now also waits for SAM's rescan after the install and for the capture's file before fetching it, and reports the seats from the log. Built for the set at 18:18 from `fa2f91f`: `rustui/build/card/` (the app 2.09 MB) and `build/card-ipk/org.nphil.lumatv.card.ipk` (29.4 MB, the new app inside), so `tv-card.py install` can go without a rebuild while the gfx64 bundle stays.

Next:

1. Rerun `python3 rustui/tools/tv-card.py all --approved` (Nitin's go) and then `remove --approved`: the TV step list below, "keys" rows. The seat lines of `card.log` (`seats: N bound, N with a keyboard`, `wl_seat NAME bound at version V (K seats)`, `keyboard focus on seat NAME`) are the evidence either way.
2. If keys still do not arrive with every seat bound: `WAYLAND_DEBUG=1` for one run (the bundle's libwayland-client honours it; `gfx64/launch.sh` would export it), which shows whether `wl_keyboard.enter` and `key` come at all, and on which seat.

For Main:

- `tools/lumatv` does not install the app yet: the hook is under "Hook request for tools/lumatv" (owner of that file). Whether `apply` installs it by default is a decision: the package grows by 31.6 MB (4.0 to 35.6 MB), 26 MB of it the 64-bit runtime, and the launcher shows a "LumaTV Card" tile.
- PLAN.md's "Native helper" section still says only the overlay window type is proven; it is host-proven for the card, TV-unproven.
- Principle 0: installing and removing an app on the set is on the list of things an AI does not do on its own; `tv-card.py install` and `remove` need Nitin's explicit go (the `--approved` flag is a reminder, not a lock), or Nitin runs them.

## What exists

| What | Where |
|---|---|
| The app: rules (Back, state changes, SAM's relaunch and close), 8 unit tests | `rustui/app/src/card.rs` |
| Its page: LumaHome's Lucent glass on the shipped wallpaper held still, a status page and a keys page, two buttons; geometry tests (nothing overlaps), 3 unit tests | `rustui/app/src/card_ui.rs` |
| The loop (Wayland window, engine, channel); one binary `lumatv-card-app` | `rustui/app/src/bin/card.rs` |
| The window (generalized from the spike's: app id, type, `KEYS_BACK`, state, events; also the spike's Home); every `wl_seat`, present or later, with its keyboard | `rustui/app/src/window.rs`, adapter in `output.rs` |
| The channel between launcher and app, JSON lines on descriptor 3, 3 unit tests | `rustui/app/src/lifecycle.rs` |
| The 32-bit launcher SAM starts (`main`), 15.7 KB; the fake hub and fake app of its test | `rustui/app/stub/{lumatv-card.c,fakels2.c,fakeapp.sh,build.sh}` |
| The package: appinfo, splash (`gen-splash.py` makes it from the main one), icons from `assets/` | `card/` |
| Build of the set's programs into `rustui/build/card/` | `rustui/tools/build-card.sh` |
| The ipk: its own package inside the main one (`payload/card/<appId>.card.ipk`) with its own copy of the 64-bit runtime | `tools/build-ipk.sh` |
| Install, update and remove on the set (`appInstallService/dev/install`, like the frame app) | `tools/cardapp.sh` |
| The TV script and its on-set probe | `rustui/tools/tv-card.py`, `tv-card-probe.sh` |
| Host tests | `rustui/app/tests/card_host.rs`, `tests/test_card_launcher.py`, `tests/test_cardapp.py`, `tests/test_card_tv.py` |

App id `<appId>.card` (`org.nphil.lumatv.card`), title "LumaTV Card", version 0.1.0. On the set: `bin/lumatv-card` (32-bit, SAM's `main`), `bin64/lumatv-card-app` (2.0 MB, aarch64, glibc 2.36), `gfx64/{launch.sh,heapshim.so,bundle/lib/*}` (the runtime, 37 MB), `assets/wallpaper/wall_1080.png`.

## Behaviour

| Key or event | The card does |
|---|---|
| Back (the remote's KEY_PREVIOUS 412; KEY_BACK 158, Escape and Backspace too) | On the keys page: back to the status page. On the status page: ends the app, which the compositor shows as the card going and the one below returning |
| OK / Left, Right | Presses the focused button (Keys, Close app) / moves focus; OK on the keys page goes back |
| Home | Never seen by the card: the compositor sends `launchDefaultApp`, Home comes to the front and the card gets `state_changed(minimized)` and stops drawing and swapping |
| Back held (Exit) | Not taken (`KEYS_EXIT` unset): the system closes the app, SAM's `close` reaches it |
| SAM `relaunch` | Counts it; a card that is covered or hidden asks for fullscreen (`set_state(3)`), which raises it and tells SAM it is in front |
| SAM `close`, `wl_webos_shell_surface.close`, the launcher going | The app ends |
| Launch with `reason: preload` or `parameters.snapshot` | The window is made (no `set_state`, no buffer), nothing is drawn until the first relaunch |
| SIGTERM | `_exit(0)` from the handler, even inside a swap that waits for a frame callback |
| A swap that has waited 8 s for its frame callback | A watchdog thread ends the process (log: "a swap has waited"), which SAM and the compositor take as the card closing; a launch then starts it afresh |

Window: `_WEBOS_WINDOW_TYPE_CARD`, `appId`, `title`, `_WEBOS_ACCESS_POLICY_KEYS_BACK` true (as Netflix and HDMI 2 send it), key mask default, no surface group (a grouped window is not in the switcher), 1920x1080.

## What the set does (observed)

Each row names where it was read.

| Fact | Source |
|---|---|
| SAM starts a native app as `<main> '<json>'`: one argument, `{"@system_native_app":true,"reason":"preload"\|"undefined","interfaceVersion":2,"event":"launch","interfaceMethod":"registerApp","appId":…,"parameters":{…}}`; a preloaded app gets `reason: preload` or `parameters.snapshot: true` and is not to show | `docs/research/compositor/probe-2026-10-25.txt` (processes of Netflix, YouTube, Amazon, inputcommon), `compositor/slot-2026-10-25-1219/gate/clients.txt` |
| A registered native app calls `luna://com.webos.applicationManager/registerApp` from a bus name equal to its app id; payload `{"subscribe":true}` (browser, Netflix, YouTube), `{"appId":…,"subscribe":true}` (Live TV, HDMI 2), `{"id":…}` (PlxNative). SAM answers `{"event":"registered"}` and then pushes on the same call: `relaunch` (`reason` undefined or boot, `appId`, `parameters`) and `close` (`reason` undefined, memoryReclaim, powerOff). The app exits after `close` and cancels its subscription about 150 ms later | `docs/research/arch/x4/all.jsonl`, `pass2.jsonl.gz` (registerApp rows and SAM's pushes) |
| Dev-installed native apps are normal here: `com.beb.plxnative` (native, `nativeLifeCycleInterfaceVersion` 2, registers as `com.beb.plxnative`), `com.limelight.webos` and `io.unom.punktfunk.client-webos` (native, no lifecycle version, no registration), `org.webosbrew.bridge-64to32` (`main` is a 64-bit executable, `external_test_64`). Installed under `/media/developer/apps/usr/palm/applications/<id>`, `privilegedJail` false, `trustLevel` default, `visible` true | `/data/home/lgtv/uiinv/listApps.json`, `apps/<id>/appinfo.json`; PlxNative's registration in `x4/pass2.jsonl.gz` |
| Launch sequence for a native app: SAM tells memorymanager `setRequiredMemory`, pushes `splash` (`splashBackground`, `showSpinner`) and `launch` to the compositor, then the app starts | `x4/pass2.jsonl.gz`, PlxNative at 30376.14 |
| A native card client sets `appId`, `title`, `_WEBOS_ACCESS_POLICY_KEYS_BACK: true`, `_WEBOS_ACCESS_POLICY_KEYS_EXIT: true` (Netflix also `_WEBOS_ACCESS_POLICY_FORCESTRETCH: false`); the window type is CARD by default (HDMI 2 sends it); key mask stays the default 0x0FFFFFF8 | `compositor/slot-2026-10-25-1219/gate/netflix-windows.json`, `hdmi-windows.json` |
| Back: a window with `KEYS_BACK` gets the key; any other window gets LG's exit dialog or a `closeByAppId`. Back held 1 s becomes Exit, which closes the app unless the window has `KEYS_EXIT`. Home: on release `launchDefaultApp {category: home, params: {launchType: homeKey, activateType: home}}`, SAM then sends Home `relaunch`, and the covered card is minimized | `docs/research/surfaces/KeyEntryPoints.json` (rows `handleSystemKeys:Key_webOS_Back short`, `…Exit…`, `…Key_Super_L short (Home)`), `x4/all.jsonl` (launchDefaultApp at 27853.596 and its `relaunch`) |
| The remote's Back reaches evdev as KEY_PREVIOUS (412), not KEY_BACK (158) | `docs/HANDOFF.md` ("Key codes") |
| A card is in the app switcher when its window is CARD, has an `appId`, is not window class hidden, is not in a surface group, **has been fullscreen** and is not fullscreen now; the app also has to be in `listLaunchPoints`, or the recents controller closes it | `/data/home/lgtv/uiinv/qml/qml/WebOSCompositor/models/StarfishRecentsWindowModel.qml` (`accepts`), `controllers/fullscreen/StarfishRecents.qml` (`__setLastfullscreenApps`, `makeUpRecents`) |
| LG's fullscreen container minimizes the card a new card covers, and raises a covered card that asks for `set_state(fullscreen)` again (a card that was never mapped too, third gate); WAM does that on a relaunch | `docs/research/compositor-c2-result.md` ("Second gate", "Third gate"), `compositor/crates/lumatv-compositor/src/server/webos_shell.rs` (`settle_cards`, `SetState`) |
| LumaTV's compositor models this: `KEYS_BACK` true sends Back to the client (`input.rs` `route`), otherwise it sends `close` and `closeByAppId {reason: windowPolicy}`; Home is `launchDefaultApp`; every `set_state` is answered at once; foreground pushes name the CARD window's `appId` and client pid; `listRecents` lists mapped cards | `compositor/crates/lumatv-compositor/src/{input,scene,luna_methods}.rs` |
| A bare executable under `/tmp` cannot register on the bus (`LSHUB_NO_ROLE_FILE`); a role comes with `appInstallService/dev/install` | `docs/research/ambient-result.md` ("Probe slot") |
| The 64-bit client path: `gfx64/launch.sh --bundle B --shim S -- CLIENT ARGS`, env `LUMATV_GFX_LIB`; Wayland at `XDG_RUNTIME_DIR=/tmp/xdg`, `WAYLAND_DISPLAY=wayland-0` | `gfx64/launch.sh`, `rustui/tools/tv-run.sh` |
| `wl_webos_shell` is version 1 on the set; the generated client panics if asked to bind `1..=2`, which the spike's window did since LG's tables were folded in | `compositor/protocol/webos/webos-shell.xml`; fixed in `window.rs` |
| LG's compositor is `luna-surfacemanager` (webOS OSE, `WebOSCoreCompositor` on QtWayland 5.12) built with its multi-input support: a `wl_seat` (`WebOSInputDevice`) per input device, made when the device's first event arrives (`queryInputDevice`: the pre-made spare takes the device id and a new spare global is created), plus the default seat for device id 0; a key is sent on the seat of the device that pressed it (`WebOSSurfaceItem::processKeyEvent` → `seatFor(event)` → that seat's `wl_keyboard.key`, Linux code = xkb code − 8) and on no other; keyboard focus is set on every seat when the fullscreen item takes focus (`takeWlKeyboardFocus`); a keyboard bound after focus gets `enter` at once (`checkFocusResource`, in Qt 5.12 as well). The device id rides in the event's modifier bits, which the set's key filter reads (`preProcess(key, pressed, autoRepeat, deviceId)`, `compositor.getDeviceType(deviceId)`: RCU, MRCU) | `github.com/webosose/luna-surfacemanager` `modules/weboscompositor/{webossurfaceitem,weboscorecompositor,webosinputdevice,weboskeyboard}.cpp`, `webosose/qtwayland` `qwaylandkeyboard.cpp`, upstream `qtwayland` 5.12 `qwaylandkeyboard.cpp`; `/data/home/lgtv/uiinv/qml/qml/WebOSCompositor/controllers/StarfishKeyFilter.qml` (`preProcess`, `setMRCUCursorVisibility`), `KeyEntryPoints.json` ("injected without device id"); the seat count on the set is not read (`card.log` will say) |
| `kb.py` writes into the `Smart Remote RCU Input` evdev device, which the compositor takes as a remote (a device id of its own, so a seat of its own); WAM received its keys as `wl_keyboard.key` with the Linux code (`WaylandKeyboard::OnKey key:106` for Right) | `docs/research/perf/kb.py`, `docs/research/surfaces-p1-result.md` (fault injection) |
| `wayland-client`'s `GlobalList::bind` binds the first global of an interface; with several `wl_seat`s the rest need `wl_registry.bind` by name, and `Dispatch<WlRegistry>` for the ones announced later | `~/.cargo/registry/src/*/wayland-client-0.31.15/src/globals.rs` (`bind`: `.next()`) |

Keys, the card against LG's own clients (what was compared for the 17:41 failure, and why only the seats changed):

- `wl_seat`: LG's is version 4 with capabilities pointer, keyboard, touch (`QWaylandSeat::DefaultCapabilities`; the fork adds no bit), bound at 4 by the card, as before; the unknown-bit fallback is a precaution. The difference is the count: LG's own clients bind every seat (Qt's client does per `wl_seat` global; Chromium's ozone and `libwayland-input-helper` too [INFERENCE for those two]), the card bound one.
- Keymap: LG serves an xkb keymap from `QT_WAYLAND_XKB_RULE_NAMES=evdev:lgwebostv:us` (`uiinv/sm_env.txt`); `wl_keyboard.key` carries the Linux code (xkb − 8), which is what the card reads; the keymap is for clients that want symbols.
- `wl_webos_input_manager` is cursor visibility and `wl_webos_seat` (gyroscope, accelerometer); `wl_webos_xinput` is a client passing a keysym to the fullscreen surface (the virtual keyboard's path); `text_model` is the IME. None carries the remote's keys; the card binds none.
- `_WEBOS_ACCESS_POLICY_KEYS_BACK` (set) and `KEYS_EXIT` (unset, as the design wants) decide only what the key filter does with Back and Exit; OK and the arrows are not policy keys. The key mask (`set_key_mask`) is read only for surface-group members (`acceptsKeyEvent` in `processKeyEvent`'s group branch); a plain card is not one.
- Focus: there is no request for it. The fullscreen view's container gives its item focus; `focusInEvent` sets Wayland keyboard focus on every seat; `getFocusedWindowInfo` naming the card 17:41 is that focus.

## Design

Two programs, one app directory (`/media/developer/apps/usr/palm/applications/<appId>.card`).

- **`bin/lumatv-card`, 32-bit C, SAM's `main`.** The luna-service2 library is 32-bit and ls-hubd gives a bus name to the executable it has a role for, so the registration has to be made by a 32-bit executable the install knows: the same constraint that made `lumatv-sm`. It parses the launch JSON, starts the app through `gfx64/launch.sh` on a socket pair (the app's descriptor 3), registers as `<appId>`, calls `registerApp {"appId":…, "subscribe":true}` and hands SAM's `relaunch` and `close` on. A close the app has not answered in 4 s becomes SIGTERM to its process group and, 2 s later, SIGKILL. It ends with the app: cancels the subscription, gives the name back, exits with the app's status, and clears the app's picture files in `/tmp` (memory). A refused name or call leaves the app running, told `{"kind":"sam","registered":false}`.
- **`bin64/lumatv-card-app`, Rust.** One Wayland connection, the toolkit's engine in its own process as in the spike, a pure state machine (`card.rs`) for the rules. It makes no luna call.
- **Package.** Its own ipk inside the main one, installed through `appInstallService/dev/install` like the frame app, with its own copy of the 64-bit runtime. A shared runtime is the better shape once a second 64-bit app exists (the compositor's bundle under `/var/lib/lumatv/compositor/lib` is the same files).

Decisions and why:

- Back at the first page exits the process instead of calling `closeByAppId`: the call needs a permission group nothing here names, and SAM and the compositor learn of the exit from the process either way (LG's own `close` ends in the app exiting).
- Not swapping while covered: a swap waits for a frame callback; LumaTV's compositor pays callbacks to every surface, LG's may not for a card it hides [INFERENCE]. The SIGTERM handler and an 8 s watchdog on the swap end the process from inside one for the same reason: a swap issued just before the compositor hides the card could otherwise wait for ever, deaf to SAM's `close` and `relaunch`. `eglSwapInterval(0)` was not used instead: nothing here has run the blob at interval 0, and it would still wait for a buffer to be released.
- Window state follows the compositor's word (`state_changed`), except that a relaunch is taken as the user asking for the app and the card draws at once; `set_state(fullscreen)` is the request.
- No role file is written. The install is all this does to the hub.
- Two processes mean the window's pid (the app) is not the pid SAM started and the registration comes from (the launcher). SAM's foreground info forwards the compositor's `processId` in every capture seen, but whether SAM cross-checks it against the pid it started is not known (see "Inferred"). The other shape (the 64-bit app as `main` and a 32-bit registrar beside it) moves the mismatch to the registration instead.
- Every `wl_seat` is bound, with a keyboard each, and seats announced later are bound as they come: LG's compositor sends a key on the seat of the device that pressed it (table above), so the first seat alone, the default one, hears no remote. A seat that goes away releases its keyboard. A capability word with a bit the generated enum does not know still counts its keyboard bit.

## Host test

Run (2026-10-25, Mesa llvmpipe, the compositor as the working tree had it after `221b75b`, `168f69a`, `8f69c05`):

- `cd rustui && cargo test --features window -p lumatv-rustui --lib --test card_host --test embedded -- --test-threads=1` (`-p`: the vendored `lumatv-render` has no test modules and does not compile as a test target): 14 unit tests, 6 host tests (the first five three passes in a row, 24.4-24.6 s each), `embedded` 1; `damage` (2, 208 s) and the rest of the suite also pass. `cargo fmt --check` and `cargo clippy --all-targets` clean.
- Seats (evening, after the fix): `LIBGL_ALWAYS_SOFTWARE=1 cargo test --features window -p lumatv-rustui --test card_host -- --test-threads=1`: 7 host tests pass (the new `seats` one and `back` in 7.9 s together, a warm shader cache); `cargo fmt --check` and `cargo clippy --features window --all-targets` clean.
- `python3 -m unittest tests.test_card_launcher tests.test_cardapp tests.test_card_tv tests.test_tool_rules`: 13 + 10 + 7 + 3; `test_card_tv` is 9 since the evening (the seats check, its failure).
- `sh rustui/tools/build-card.sh` (23 s): aarch64 app and arm launcher; `sh tools/build-ipk.sh`: the main ipk with `payload/card/org.nphil.lumatv.card.ipk` (29.4 MB) inside.
- A headless Weston 10 (the `xdg_shell` path of `window.rs`, which the Home spike also uses): the card app presents its first frame, and the spike's `--mode window --run idle:3,dpad:3` still runs and reports.
- A smoke run of the real `lumatv-compositor` binary on a real socket with the real app binary (`ctl windows`: Card, Fullscreen, `KEYS_BACK` true, the app's own pid; `ctl remote 412`: the app ends).

`card_host.rs` runs the compositor in process with its own harness (`compositor/crates/lumatv-compositor/tests/harness`, software renderer) and the app binary as a child on a socket pair (`WAYLAND_SOCKET`), with the test playing the launcher (the app's channel) and SAM (luna calls into the compositor). Shown:

| Test | Shown |
|---|---|
| maps as a card | type Card, `appId`, title, `KEYS_BACK` true, no `KEYS_EXIT`, fullscreen, 1920x1080; SAM's `getForegroundAppInfo` names it (`_WEBOS_WINDOW_TYPE_CARD`, its pid is the socket pair's creator's here); `listRecents` has it; the compositor's picture is the wallpaper upright with the glass card over it; no GL error; no compositor error |
| Back | OK opens the keys page, Back returns (the app stays), Back at the first page ends it with exit 0 and "Back at the first page"; the compositor sent no `close` and no `closeByAppId` (the app took the key); nothing is in front afterwards |
| Home and relaunch | the Home key reaches nobody (`launchDefaultApp {category: home, params.launchType: homeKey}` is sent); Home maps over the card, which is told `minimized` and then commits nothing in 3 s; SAM's foreground is Home; the switcher still lists the card; `relaunch` makes the app ask for fullscreen, the card is in front, Home minimized, the card commits again; SAM's `close` ends the app with exit 0 and Home is fullscreen again |
| preload | window made, unmapped, state default, no foreground, no `ready`; the first relaunch maps it fullscreen and in front |
| close | `closeByAppId` (compositor `close`) ends the app; a hung-up launcher channel ends it |
| stall | the compositor stops presenting (no frame callbacks), the card is given four key presses, its swap waits, and 8 s later it ends with exit 0 and the log line (9.9 s) |
| seats | with the card up and one keyboard bound, the test adds a second `wl_seat` global (`dh.create_global`, as LG's compositor does for a new device); the app binds it and asks for its keyboard within 5 s; OK sent on that keyboard alone opens the keys page, Back on the first seat's keyboard goes up; the global removed: the app releases the keyboard (one left) and OK on the first still opens the page; the log has `seats: 1 bound, 1 with a keyboard`, `bound at version 4 (2 seats)`, `went away (1 seats)` |

`docs/research/cardapp/host/{status,keys}.png` are the compositor's pictures of the two pages. The launcher's tests run it against `fakels2.c` (the real glib loop, SAM played from a script) and `fakeapp.sh`: registration payload and order of cancel and unregister, the exact messages the app gets for registered, relaunch and close, a refused name, a refused call, a preload started with `--hidden`, parameters passed as one argument, a close ignored by the app becoming SIGTERM then SIGKILL (137), an app that answers SIGTERM, SIGTERM to the launcher, the app's exit status passed on, the app leaving when the launcher is killed, the picture directory cleared, an exec failure (127).

What none of this says: that the set's hub gives the launcher the bus name, what SAM does with it, what LG's compositor sends, what the Mali blob does with the card's window.

## TV step list

NOT VERIFIED ON THE TV. `python3 rustui/tools/tv-card.py all --approved` does steps 1 to 3 (`facts`, `install --approved`, `scenario`; `remove --approved` is step 4); its checks are named as below.

Needs: power state `Active`, Home in front, nothing else of ours running, Nitin's go for the install. Build first: `sh gfx64/fetch-blob.sh && sh gfx64/build.sh && sh rustui/tools/build-card.sh && sh tools/build-ipk.sh`.

1. `facts` (read-only). Look at: who `id` says (the app runs as someone else, see step 3's users line), `/dev/mali0`, `/dev/dri/card0`, `/dev/dma_buf_te` and `/dev/dma_buf_unified` modes, `/media/developer` mount flags (a `noexec` would stop everything), free space, which compositor is live.
2. `install`. Checks: `install`, `the launcher is executable`, `SAM lists the app as native with lifecycle 2`, `a launch point exists` (the two wait up to 120 s for SAM's rescan; the info line says how long it took).
3. `scenario`. Each check, what it means, what to do if it fails:

| Check | If it fails |
|---|---|
| `launch: the card is in front` | Read `card.log`. No `launch:` line: SAM never started the launcher (the exec bit, `noexec`, `main`, a jail): `journalctl -u sam` / `/var/log/messages`. A `launch:` line and no app: the `gfx64:` lines (`unavailable`, `no /dev/mali0`, a loader error) or `card[...]: failed:`. App up but not in front: LG's compositor ignored `set_state(fullscreen)` for a card: see the foreground JSON and the `input State(...)` lines |
| `launch: the launcher and the 64-bit app run` / users line | The users line says who the app is (SAM's jail: uid 5227). The jail's `/dev` has no `dri/card0` or `dma_buf_te`, so the buffers come from `/dev/dma_buf_unified` (ordinary pages, the `dma_buf_te` 56 fps class) |
| `launch: the window is a card` | The foreground JSON names the window type |
| `launch: the launcher registered with SAM` | `LSRegister org.nphil.lumatv.card: …` in `card.log`: the hub gave the launcher no bus name, the open question of the role. The card still shows and Back works; SAM's `relaunch` and `close` do not reach it, so the checks `relaunch`, `close` and `exit` fail with it. Do not write a role file on the set without Nitin's OK (a hub role is a permission grant); say what the hub logged (`LSHUB_NO_ROLE_FILE`) and ask. `SAM refused registerApp` is the call being denied: the app role has no outbound to SAM |
| `launch: SAM lists it as running` / `leaves it running for 20 s` | SAM does not count the app as launched or killed it: this is the registration or the pid question. Compare `processId in SAM's foreground info` with the pids on the line above |
| `launch: the 64-bit path was used` | A `gfx64:` line says why not; there is no 32-bit fallback, so the app does not run on the other path. `no buffer allocator` again: the jail's `/dev` lost `dma_buf_unified` or the new package did not install |
| `launch: no buffer allocation failed` / info `launch: buffer source` | `heapshim: alloc … FAILED (errno N)` in `card.log`: 12 is ENOMEM from the exporter (memory is short: `CmaFree` and `MemAvailable` in `facts`), 13 or 2 is the open (the jail's permission or path). The source line should read `dma_buf_unified in …` for every buffer |
| `keys: the app bound the compositor's seats, with a keyboard each` | No `seats:` line: the app did not get past `window::open` (see `launch`). `N bound, M with a keyboard` with M < N: a seat without the keyboard capability bit; the `wl_seat NAME bound` lines say which seats there were. The info line `seats the compositor advertised` is the count of LG's seats (expected: the default one, one per remote or keyboard that has sent a key since boot, one spare); `keyboard focus came on seats` lists the seats that sent `wl_keyboard.enter` (expected: all of them once the card is in front) |
| `keys: OK reaches the app…` / `Back goes up…` | With the seats bound and focus on them: the info line `codes the compositor sent` shows what LG sends for OK and Back; add them to `card::key_of` if they are not 28/352/353 and 412/158/1/14. With seats bound and no `input Key` line at all: the info line `seats that appeared while the card ran` says whether the key's device got a new seat (bound late, after the key); if `keyboard focus` names no seat, the fullscreen item never took focus at the Wayland level (compare `getFocusedWindowInfo`), and a `WAYLAND_DEBUG=1` run ("Resume here" 2) is the next step |
| `home: Home is in front` | Home's launch path; not the card's |
| `home: the card was told it is minimized` | LG's compositor does not send `state_changed(minimized)` to a covered card [INFERENCE that it does]. The card would then keep swapping: look for a stall (`home: the covered card is idle` fails with busy ticks, or the app stops answering) and report; the fallback is to treat `launchDefaultApp` or `getForegroundAppInfo` as the signal in the launcher |
| `home: the covered card is idle` | Ticks over 5 in 5 s: the card draws while covered |
| `home: the app switcher lists the card` | `recents.json`: a launch point missing (step 2), or the window is not a card the compositor counts (class hidden, group) |
| `relaunch: …` | `SAM: relaunch` missing: SAM did not push the event (`handlesRelaunch` in appinfo: `true` is read as "the app handles it" [INFERENCE]; try `false`). Same pid failing: SAM closed and started the app again |
| `back: …` | The app should end, Home return, the launcher log `done: status 0`, the picture directory vanish |
| `close: SAM's close reaches the app` / `exit: Back held closes the app` | `SAM: close` missing means SAM killed the process without the event, or the close never came |

4. `remove`. Leaves the set as found.
5. Afterwards: write the TV results below, with the evidence directory.

TV results (2026-10-25 16:15-16:21, `docs/research/cardapp/tv-20261025-{1615,1617,1621}/`, code `e981d5e`; the set on Home, LG's compositor, power `Active`):

- `facts`: over SSH the shell is root; the set is aarch64; `/dev/dma_buf_te` 666, `/dev/dri/card0` 600 root, `/dev/mali0` 666; `/media/developer` has no mount entry of its own (it is under `/var`, ext4 `rw,nosuid` on `/dev/mmcblk0p58`; SAM executed the launcher from it, so no `noexec`); MemAvailable 878 MB, CmaFree 105 MB. Dev-installed native apps on the set: amazon, com.apple.appletv, com.beb.plxnative, com.limelight.webos, io.unom.punktfunk.client-webos, netflix, org.webosbrew.bridge-64to32, youtube.leanback.v4.
- `install`: PASS through `appInstallService/dev/install` (rc 0, `{"installed":true,"version":"0.1.0"}`, the launcher executable). The two SAM checks that follow at once FAILED because SAM had not rescanned yet: a `listApps` a minute later lists `org.nphil.lumatv.card` as `native`, `nativeLifeCycleInterfaceVersion` 2, `main` `bin/lumatv-card`, `handlesRelaunch` true, `visible` true, `removable` true, `trustLevel` default, `privilegedJail` false. The checks wait for SAM's rescan since the evening (up to 120 s).
- `scenario` (launched four times by the script's steps, `card.log`): SAM starts the launcher as uid 5227 gid 5000 with `reason undefined`; the launcher registers (`registered org.nphil.lumatv.card`, `SAM: registered`): a dev install does give the launcher a bus name and SAM accepts `registerApp` (was "Inferred"). The 64-bit app starts and ends with 69 within 20 ms: `gfx64: 64-bit path unavailable: no buffer allocator (/dev/dri/card0, /dev/dma_buf_te); no fallback given`. The app's root is LG's jail, `/var/palm/jail/org.nphil.lumatv.card` (`jail_native_devmode.conf`): its `/dev` holds `console dma_buf_unified dvb fb0-3 gfx hidraw0 hma input lg log logfunnel mali0 null pts pvrsrvkm shm snd urandom vdec video video20 video60 vo` and no `dri`, `dma_buf_te` or `dma_heap`. In that file `/dev/dma_buf_te` is inside `if $mstar == true` and `/dev/dri/card0` inside `if $rtk == true` / `if $nvt == true`, none of which this set is (`LG1213`, `chip=O22A3`). PlxNative's jail (`/var/palm/jail/com.beb.plxnative/dev`) has the same list: LG's 32-bit driver allocates through `dma_buf_unified`/`mali0`. Every later check (keys, Home, switcher, relaunch, close, exit) is unexercised: the FAIL lines of `summary.txt` for them say the app was not running, the PASS lines (Back ends the app, Home in front, nothing left) hold trivially.
- The script's first scenario run aborted on the foreground read: SAM's `getForegroundAppInfo` with `{}` answers a flat object (`appId`, `processId`, `windowId`); the list with `windowType` needs `{"extraInfo":true}` (fixed in `e981d5e`).
- `remove`: PASS (rc 0, the directory gone, SAM no longer lists it). The jail skeleton `/var/palm/jail/org.nphil.lumatv.card` stays, as LG's jailer leaves it for every removed app (the other removed apps' are there too); `rm -rf` it if wanted.

Settled by the run: the `/media/developer` mount is executable; SAM runs a dev-installed version 2 native app and keeps the launcher's registration; the hub gives the launcher a role for its own app id (`LSHUB_NO_ROLE_FILE` did not appear). Not settled: everything from the first buffer on.

TV results (2026-10-25 17:41-17:45, `docs/research/cardapp/tv-20261025-{1741,1745}/`, code `f3ed5a6` from a clean worktree; the set woken from Active Standby, Home in front, LG's compositor, receiver muted through Home Assistant for the run and unmuted after, 0.355 both ways):

- `facts`: as on the first run; MemAvailable 959 MB, CmaFree 166 MB.
- `install`: PASS (rc 0, `{"installed":true,"version":"0.1.0","shipped":"0.1.0","running":false}`, the launcher `-rwxr-xr-x`). The two SAM checks right after FAILED again (`listApps` without the app, no launch point): the rescan race; the launch 2.4 s later worked and `recentsAppList` listed the app.
- `scenario` (`card.log`, two launches by the script's steps plus one by hand): SAM started the launcher as uid 5227 gid 5000, `reason undefined`; `registered org.nphil.lumatv.card`, `SAM: registered`; `gfx64: 64-bit path: .../gfx64/bundle/lib/libmali-bifrost-g52-g24p0-wayland-gbm.so`; the app: `GL: Mali-G52 | OpenGL ES 3.2 v1.g24p0`, shaders compiled and linked in 1363 ms, the page built in 133 ms, `heapshim: alloc 8986624 bytes -> fd 19 via dma_buf_unified in 4.0 ms` (then 4.4 and 19.9 ms for the second and third buffers), `first frame presented` 1.7 s after the start, `set_state(3)` answered with `State(3)`; the card in front 2.4 s after the launch, SAM's foreground info `_WEBOS_WINDOW_TYPE_CARD`, `processId` 7550 (the app; the launcher 7531), `running` lists it, RSS 136 MB, still in front 20 s later; the page on the panel: `status.png` (the glass card with "Window fullscreen card", "Graphics Mali-G52, 64-bit driver", "SAM registered", "Last key none yet", the mute badge of the receiver at the right). Home (773 through `kb.py`): Home in front, the card got `State(1)` (`minimized`), kept running, 0 clock ticks in 5 s, listed in `recentsAppList`. Relaunch: `SAM: relaunch (undefined)`, the card asked for state 3, got `State(3)`, same pid, in front again. SAM's `close`: `SAM: close (undefined)`, `exit: SAM closed the app`, the launcher and app gone. Keys: FAIL. OK, then Back (`kb.py`, 50 ms presses) logged nothing in the app (no `input Key` line, no page change), so `keys`, `back` (the app stayed, Home not in front, no `done` line, SAM still listing it) and `exit` (Back held) all FAIL. By hand afterwards, with the card in front: `getFocusedWindowInfo` `{"appId":"org.nphil.lumatv.card","type":"fullscreen"}`, `getForegroundWindowInfo` the card as `_WEBOS_WINDOW_TYPE_CARD` (and LG's volume window from the earlier gate), then OK, Right, Down, Back through `kb.py`: still no `input` line of any key; `closeByAppId` ended it and Home came back. The five screenshots were on the set (`/tmp/cardtv`, 400 KB each) but the script's `get` ran before the capture had written them; fetched by hand into `tv-20261025-1741/`.
- `remove` (`tv-20261025-1745/`): PASS (rc 0, the directory gone, SAM's `listApps` without it, no card process).

Settled by the run: `/dev/mali0` and `/dev/dma_buf_unified` open for uid 5227 in the jail and the 64-bit blob draws from `dma_buf_unified` buffers; SAM does not compare the window's `processId` with the launcher's; LG's compositor shows a card that sent `set_state(fullscreen)` at creation, sends `state_changed(minimized)` when Home covers it and no frame callback (0 ticks in 5 s), and delivers `relaunch` and `close` through the launcher's registration. Not settled: keys (above, "Resume here" 1).

## Inferred, not observed

- Observed on the TV (2026-10-25): `appInstallService/dev/install` gives the launcher a hub role for its own app id (it registered; `LSHUB_NO_ROLE_FILE` did not appear). The role files other workers write by hand (`payload/capture/role.json`) are a separate question.
- That SAM does not require `registerApp` of a version 2 app to keep it running, what it does to one that never registers, and that `handlesRelaunch: true` is right for an app that handles SAM's `relaunch` itself (Netflix and the browser, which do, have it; YouTube, PlxNative and Moonlight have `false`). Observed 17:41: with `true` SAM pushes `relaunch` to the registered launcher and the app comes back in the same process.
- Observed 17:41: SAM does not compare the window's `processId` (the app, 7550) with the pid it started (the launcher, 7531); its foreground info names the app's.
- Observed 17:41: SAM starts the app as uid 5227 gid 5000 in LG's jail (`jail_native_devmode.conf`); `/dev/mali0` and `/dev/dma_buf_unified` open for that uid (`GL: Mali-G52`, three 9 MB buffers `via dma_buf_unified` in 4-20 ms each).
- That the set's kernel registers `dma_buf_unified` as LG's `b46` source does (the chip's `GPU_RELEASE-o22 = b46/`, `Kbuild`), taking the Arm test exporter's allocation request: the request worked 17:41 (which of the two exporters answered is not read). That the jail's buffers behave as `dma_buf_te`'s do (56 fps class) is not measured; the 64-bit blob imported and drew them.
- Observed 17:41: LG's compositor sends `state_changed(minimized)` to a covered card and no frame callbacks to it (0 clock ticks in 5 s).
- What LG's compositor sends for OK and the arrows: nothing reached the app 17:41; the codes are still unknown. The seat-per-device model (the "What the set does" row) is read from LG's open source and the set's QML, not seen on the set: how many seats it has, and that the remote's keys come on one that is not the first global, is what the next run's `card.log` says.
- Observed 17:41: a card with `set_state(fullscreen)` at creation is shown by LG's compositor (in front 2.4 s after the launch, `State(3)` answered).

## Hook request for tools/lumatv

`tools/lumatv` is another owner's; nothing in it has changed. To make `apply` and `revert` handle the app (`tools/cardapp.sh` is already in the package):

```sh
# beside the sourcing of tools/compositor.sh (line 321):
if [ -f "$APPDIR/tools/cardapp.sh" ]; then . "$APPDIR/tools/cardapp.sh"; fi
# in verb_apply after install_frame_app, if the app is to be installed by default:
card_install || log_line "apply: the card app did not install"
# in do_revert_internal beside the app's other removals:
card_remove
```

`tests/test_tool_rules.py` (SAM is never restarted, stopped or killed) passes with the script; `card_close` asks SAM's `closeByAppId` for the card app only.

## Later

- Under LumaTV's compositor on the set: the same `tv-card.py scenario`, plus `lumatv-compositor ctl windows` and `ctl journal` for the window's own account. The luna bridge answers SAM's foreground subscription from the card window's `appId` (`luna_methods.rs`), as the host test does by hand.
- A shared 64-bit runtime for apps (`/var/lib/lumatv/gfx64`), staged once by the tool, takes 26 MB off every app after the first.
- Pointer (Magic Remote) input in the card, a TV-measured frame gate for the page (its idle frame is one text update a second).
