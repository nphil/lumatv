# Card app: a 64-bit LumaTV app as a registered card window

Todo: "64-bit app as a registered card window (launch, Back, Home, switcher)". A 64-bit app drawn with LumaShow's toolkit (`rustui/`) on the 64-bit Mali blob (`gfx64/`) must run as a real webOS app under LG's compositor today and later, unchanged, under LumaTV's own: installed with the ipk, launched by SAM, shown as a card (not an overlay), Back and Home behaving as in LG's apps, listed in the app switcher, named by SAM's foreground info.

## Resume here

State (2026-10-25, offline): plan written, nothing built yet. Nothing here has run on the TV (another worker holds it); everything under "Inferred" is untested.

Order of work: (1) the card app's window and state machine in `rustui/app`, (2) the lifecycle channel and the 32-bit launcher, (3) the host test under LumaTV's compositor, (4) the launcher against a fake luna hub, (5) packaging in `tools/build-ipk.sh` and the install helper, (6) the TV script.

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
| LG's fullscreen container minimizes the card a new card covers, and raises a covered card that asks for `set_state(fullscreen)` again; a client that is relaunched from minimized does exactly that (WAM does) | `docs/research/compositor-c2-result.md` ("Second gate", last paragraph), `compositor/crates/lumatv-compositor/src/server/webos_shell.rs` (`settle_cards`, `SetState`) |
| LumaTV's compositor already models this: `KEYS_BACK` true sends Back to the client (`input.rs` `route`), otherwise it sends `close` and `closeByAppId {reason: windowPolicy}`; Home is `launchDefaultApp`; every `set_state` is answered at once; a covered card gets `state_changed(minimized)` and is fullscreen again when the card over it goes; foreground pushes name the CARD window's `appId` and client pid; `listRecents` lists mapped cards | `compositor/crates/lumatv-compositor/src/{input,scene,luna_methods}.rs`, `server/webos_shell.rs` |
| A bare executable under `/tmp` cannot register on the bus (`LSHUB_NO_ROLE_FILE`); a role comes with `appInstallService/dev/install` | `docs/research/ambient-result.md` ("Probe slot") |
| The 64-bit client path: `gfx64/launch.sh --bundle B --shim S -- CLIENT ARGS`, env `LUMATV_GFX_LIB`; Wayland at `XDG_RUNTIME_DIR=/tmp/xdg`, `WAYLAND_DISPLAY=wayland-0` | `gfx64/launch.sh`, `rustui/tools/tv-run.sh` |

## Design

Three programs, one app directory (`/media/developer/apps/usr/palm/applications/<appId>.card`).

- **`bin/lumatv-card`, 32-bit C, SAM's `main`.** The luna-service2 library is 32-bit and ls-hubd gives bus names by executable path, so the registration has to be made by an executable the install knows, and it cannot be the 64-bit app (the same constraint that made `lumatv-sm`). It parses the launch JSON, starts the 64-bit app through `gfx64/launch.sh`, registers as `<appId>`, calls `registerApp {"appId":…, "subscribe":true}`, and relays SAM's `relaunch` and `close` to the app over a socket pair. It leaves with the app, after cancelling the subscription. If the hub refuses the name the app still runs, without SAM's events (logged).
- **`bin64/lumatv-card-app`, Rust, the app (`rustui/app`).** One Wayland connection, a `wl_shell` toplevel plus `wl_webos_shell_surface` with type CARD, `appId`, `title` and `KEYS_BACK` true, the toolkit's engine in its own process as in the spike. A pure state machine (`card.rs`) decides, so the rules are unit-tested: Back goes up a page, and at the root closes the app (the process leaves; SAM and the compositor see a card go, as after `close`); Home is never seen (the compositor's); `state_changed(minimized)` stops drawing, `fullscreen` draws again; SAM's `relaunch` while minimized asks for `set_state(fullscreen)`; the compositor's `close` and SAM's `close` both end the process; a launch with `reason: preload` or `snapshot` makes no window until the first `relaunch`.
- **Package.** Its own ipk inside the main one (`payload/card/<appId>.card.ipk`), installed through `appInstallService/dev/install` like the frame app, with its own copy of the 64-bit runtime (the `gfx64` bundle, 37 MB, 26 MB compressed). A shared runtime is the better shape once a second 64-bit app exists.

Why exit instead of calling `closeByAppId` at the root: the call needs a permission group nothing here names, and SAM learns of the exit from the process either way. Why a socket pair and not luna in the app: no 64-bit luna.

## Host test

LumaTV's compositor on the host (the harness of `compositor/crates/lumatv-compositor/tests/harness`, software renderer) with the real 64-bit app binary as its client on Mesa's software GL, the launcher's end of the channel played by the test. To show: maps as CARD with the right `appId`, title and `KEYS_BACK`; fullscreen; SAM's `getForegroundAppInfo` names it and `listRecents` lists it; Back (KEY_PREVIOUS) goes up a page, and at the root the app leaves and the card below returns; Home (the compositor's `launchDefaultApp`, then a Home card mapping) minimizes it and it stops committing; `relaunch` brings it back to fullscreen; `close` ends it. The launcher is tested against a fake luna hub (`stub/`).

## TV step list

(filled in when the build is done)

## Inferred, not observed

- That the install gives a dev-installed native app's executable a hub role for its own app id (PlxNative registers that way; nothing here has installed one of ours).
- That SAM does not require `registerApp` of a v2 app to keep it running, and what it does to an app that never registers.
- The environment and user SAM starts a dev-installed native app with (the step list prints them), and whether `/dev/dri/card0` and `/dev/mali0` are open to it.
- That LG's compositor sends no frame callbacks to a minimized card (the app does not swap while minimized, and ends the process on SIGTERM even from inside a swap).
