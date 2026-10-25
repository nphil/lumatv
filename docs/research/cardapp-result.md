# Card app: a 64-bit LumaTV app as a registered card window

Todo: "64-bit app as a registered card window (launch, Back, Home, switcher)". A 64-bit app drawn with LumaShow's toolkit (`rustui/`) on the 64-bit Mali blob (`gfx64/`) has to run as a real webOS app under LG's compositor today and, unchanged, under LumaTV's own: installed with the ipk, started by SAM, shown as a card (not an overlay), Back and Home behaving as in LG's apps, listed in the app switcher, named by SAM's foreground info.

## Resume here

State (2026-10-25 17:00 EDT): the first TV run (16:15-16:21, "TV results") found the app ending at once under SAM's jail (uid 5227, root `/var/palm/jail/<appId>`), whose `/dev` has `mali0` and `dma_buf_unified` but no `dri/card0`, `dma_buf_te` or `dma_heap`, so `gfx64/launch.sh` found no buffer allocator. Fixed offline, NOT VERIFIED ON THE TV: the heap shim has a third source, `/dev/dma_buf_unified`, and `launch.sh` accepts it as an allocator. On the set's chip (O22A3, kernel GPU release b46) LG registers that device as a second misc node of the Arm test exporter, so the request is the `dma_buf_te` one (`_IOR('E', 0x01, {u64 pages})`, returns the dma-buf fd); the older LG exporter's `{u32 pages; u32 map_cached}` has the same number and bytes. Source, ABI and the host checks: `gfx64-result.md`, "Under SAM's jail". The Mali device's own allocation was not used: the blob's strings name only the dma-heap paths for window buffers (`/dev/dma_heap/system-uncached`, `%s`, `protected`), so the shim's heap answer is the one hook.

Next:

1. Rebuild (`sh gfx64/fetch-blob.sh && sh gfx64/build.sh && sh rustui/tools/build-card.sh && sh tools/build-ipk.sh`) and run the same TV step list again (`tv-card.py all --approved`, Nitin's go for install and remove). New in the list: `launch: no buffer allocation failed` and the info line `launch: buffer source` (expect `dma_buf_unified`); `facts` now lists `/dev/dma_buf_unified`.
2. If the app still ends at once, `card.log` says where. Not yet seen from inside the jail, so each is open: `/dev/mali0` or `/dev/dma_buf_unified` not opening for uid 5227 (`heapshim: alloc ... FAILED`, or a blob error naming the device), the Wayland socket (`/tmp/xdg/wayland-0`; does the jail's `/tmp` hold LG's?), the blob reading something else outside the jail's `/dev` or `/sys`.
3. Every check after `launch: the 64-bit path was used` is unexercised. Fill the rest of "TV results" and move what held from "Inferred".

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
| The window (generalized from the spike's: app id, type, `KEYS_BACK`, state, events; also the spike's Home) | `rustui/app/src/window.rs`, adapter in `output.rs` |
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

## Host test

Run (2026-10-25, Mesa llvmpipe, the compositor as the working tree had it after `221b75b`, `168f69a`, `8f69c05`):

- `cd rustui && cargo test --features window -p lumatv-rustui --lib --test card_host --test embedded -- --test-threads=1` (`-p`: the vendored `lumatv-render` has no test modules and does not compile as a test target): 14 unit tests, 6 host tests (the first five three passes in a row, 24.4-24.6 s each), `embedded` 1; `damage` (2, 208 s) and the rest of the suite also pass. `cargo fmt --check` and `cargo clippy --all-targets` clean.
- `python3 -m unittest tests.test_card_launcher tests.test_cardapp tests.test_card_tv tests.test_tool_rules`: 13 + 10 + 7 + 3.
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

`docs/research/cardapp/host/{status,keys}.png` are the compositor's pictures of the two pages. The launcher's tests run it against `fakels2.c` (the real glib loop, SAM played from a script) and `fakeapp.sh`: registration payload and order of cancel and unregister, the exact messages the app gets for registered, relaunch and close, a refused name, a refused call, a preload started with `--hidden`, parameters passed as one argument, a close ignored by the app becoming SIGTERM then SIGKILL (137), an app that answers SIGTERM, SIGTERM to the launcher, the app's exit status passed on, the app leaving when the launcher is killed, the picture directory cleared, an exec failure (127).

What none of this says: that the set's hub gives the launcher the bus name, what SAM does with it, what LG's compositor sends, what the Mali blob does with the card's window.

## TV step list

NOT VERIFIED ON THE TV. `python3 rustui/tools/tv-card.py all --approved` does steps 1 to 3 (`facts`, `install --approved`, `scenario`; `remove --approved` is step 4); its checks are named as below.

Needs: power state `Active`, Home in front, nothing else of ours running, Nitin's go for the install. Build first: `sh gfx64/fetch-blob.sh && sh gfx64/build.sh && sh rustui/tools/build-card.sh && sh tools/build-ipk.sh`.

1. `facts` (read-only). Look at: who `id` says (the app runs as someone else, see step 3's users line), `/dev/mali0`, `/dev/dri/card0`, `/dev/dma_buf_te` and `/dev/dma_buf_unified` modes, `/media/developer` mount flags (a `noexec` would stop everything), free space, which compositor is live.
2. `install`. Checks: `install`, `the launcher is executable`, `SAM lists the app as native with lifecycle 2`, `a launch point exists`.
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
| `keys: OK reaches the app…` / `Back goes up…` | The info line `codes the compositor sent` shows what LG sends for OK and Back; add them to `card::key_of` if they are not 28/352/353 and 412/158/1/14 |
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
- `install`: PASS through `appInstallService/dev/install` (rc 0, `{"installed":true,"version":"0.1.0"}`, the launcher executable). The two SAM checks that follow at once FAILED because SAM had not rescanned yet: a `listApps` a minute later lists `org.nphil.lumatv.card` as `native`, `nativeLifeCycleInterfaceVersion` 2, `main` `bin/lumatv-card`, `handlesRelaunch` true, `visible` true, `removable` true, `trustLevel` default, `privilegedJail` false. The check should wait for SAM's rescan (open in the script).
- `scenario` (launched four times by the script's steps, `card.log`): SAM starts the launcher as uid 5227 gid 5000 with `reason undefined`; the launcher registers (`registered org.nphil.lumatv.card`, `SAM: registered`): a dev install does give the launcher a bus name and SAM accepts `registerApp` (was "Inferred"). The 64-bit app starts and ends with 69 within 20 ms: `gfx64: 64-bit path unavailable: no buffer allocator (/dev/dri/card0, /dev/dma_buf_te); no fallback given`. The app's root is LG's jail, `/var/palm/jail/org.nphil.lumatv.card` (`jail_native_devmode.conf`): its `/dev` holds `console dma_buf_unified dvb fb0-3 gfx hidraw0 hma input lg log logfunnel mali0 null pts pvrsrvkm shm snd urandom vdec video video20 video60 vo` and no `dri`, `dma_buf_te` or `dma_heap`. In that file `/dev/dma_buf_te` is inside `if $mstar == true` and `/dev/dri/card0` inside `if $rtk == true` / `if $nvt == true`, none of which this set is (`LG1213`, `chip=O22A3`). PlxNative's jail (`/var/palm/jail/com.beb.plxnative/dev`) has the same list: LG's 32-bit driver allocates through `dma_buf_unified`/`mali0`. Every later check (keys, Home, switcher, relaunch, close, exit) is unexercised: the FAIL lines of `summary.txt` for them say the app was not running, the PASS lines (Back ends the app, Home in front, nothing left) hold trivially.
- The script's first scenario run aborted on the foreground read: SAM's `getForegroundAppInfo` with `{}` answers a flat object (`appId`, `processId`, `windowId`); the list with `windowType` needs `{"extraInfo":true}` (fixed in `e981d5e`).
- `remove`: PASS (rc 0, the directory gone, SAM no longer lists it). The jail skeleton `/var/palm/jail/org.nphil.lumatv.card` stays, as LG's jailer leaves it for every removed app (the other removed apps' are there too); `rm -rf` it if wanted.

Settled by the run: the `/media/developer` mount is executable; SAM runs a dev-installed version 2 native app and keeps the launcher's registration; the hub gives the launcher a role for its own app id (`LSHUB_NO_ROLE_FILE` did not appear). Not settled: everything from the first buffer on.

## Inferred, not observed

- Observed on the TV (2026-10-25): `appInstallService/dev/install` gives the launcher a hub role for its own app id (it registered; `LSHUB_NO_ROLE_FILE` did not appear). The role files other workers write by hand (`payload/capture/role.json`) are a separate question.
- That SAM does not require `registerApp` of a version 2 app to keep it running, what it does to one that never registers, and that `handlesRelaunch: true` is right for an app that handles SAM's `relaunch` itself (Netflix and the browser, which do, have it; YouTube, PlxNative and Moonlight have `false`).
- That SAM does not compare the window's `processId` (the app) with the pid it started or the registration's (the launcher).
- Observed: SAM starts the app as uid 5227 gid 5000 in LG's jail (`jail_native_devmode.conf`), with `/dev/mali0` (660 root:video) and `/dev/dma_buf_unified` but no `/dev/dri` or `/dev/dma_buf_te`; `/bin/sh`, `awk`, `ls`, `head` and `uname` were found (the launcher ran to the app's exit). Whether `/dev/mali0` and `/dev/dma_buf_unified` open for that uid is still unseen (the app ended before the GPU).
- That the set's kernel registers `dma_buf_unified` as LG's `b46` source does (the chip's `GPU_RELEASE-o22 = b46/`, `Kbuild`), taking the Arm test exporter's allocation request; if it is the older LG exporter, the same request applies (`gfx64-result.md`). That the jail's buffers behave as `dma_buf_te`'s do (56 fps class, importable by the 64-bit blob) is not measured.
- That LG's compositor sends `state_changed(minimized)` to a covered card and sends no frame callbacks to it.
- What LG's compositor sends for OK and the arrows (the card reads the usual Linux codes and shows what arrived).
- That a card with `set_state(fullscreen)` at creation is shown by LG's compositor as it is by LumaTV's (Qt apps send it on every `showFullScreen`).

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
