# native-tv study

Source: https://github.com/ihtisham1211/native-tv (MIT, read at its default branch, not tested on a TV by LumaTV). Paths below are in that repository unless prefixed `lumaglass:`. "Not verified" means read in source or docs only.

## What it is

A rooted-LG toolkit: a C++14 daemon on the TV (CivetWeb HTTP dashboard, MQTT/Home Assistant, privacy and debloat controls, backups) plus a native TV client (SDL2 + GLES2, `native-tv/src/main.cpp`) drawn on the GPU with no browser. It replaces LG's Home with the client. There is no video playback anywhere in it (grep for decoder/NDL/starfish finds only firmware-version reads).

## How it runs native code on webOS

- **Launch model.** The client is a normal dev-installed native app (`native-tv/assets/dashboard-app/appinfo.json`: `type: native`, `main: nativetv-tv`, id `com.tvweb.dashboard`). SAM starts it under the jailer with one JSON argument; `platform/client_launch.cpp` reads `screen` or the jailer's `nid` from it. Libraries sit beside the binary (`$ORIGIN`); SDL2, GLES and FreeType are the TV's own (`tools/runtime_payload.py`, `SYSTEM_LIBRARIES`).
- **Lifecycle registration.** `main.cpp:330-342` sets `SDL_HINT_WEBOS_REGISTER_APP` before `SDL_Init`, but only when SAM started the process (not for `--time`/`--duration-ms` runs). LG's SDL then calls `luna://com.webos.applicationManager/registerApp` (or `registerNativeApp`), after which SAM sends the app `relaunch`/`close`/`pause` events. The open-source SDL does the same: webosbrew SDL-webOS `src/core/webos/SDL_webos_init.c:66-92,178-181` (reads `APPID` and `nativeLifeCycleInterfaceVersion`, then registers, then registers a screen-saver request and a tvpower subscription).
- **Rendering and input.** SDL window on the TV's Wayland/GLES2 stack (`platform/gl_window.cpp`); swap interval 1, loop priority -10 (`main.cpp:432`, `:329`). Keys arrive as SDL key events; Back/Exit need `SDL_WEBOS_ACCESS_POLICY_KEYS_BACK/EXIT` (`main.cpp:330-331`) and scancodes 482/505 map to Back. Focus loss/hidden stops drawing and pauses the clock (`main.cpp:509-525`).
- **Home takeover.** LG's Home directory is bind-mounted with a *web* stub (`assets/home-app/index.html`, `appCategory: ["home"]`). The stub shows black and asks SAM to launch the native client with `screen: home` on load, on `visibilitychange` and on `webOSRelaunch`. A daemon watcher (`src/daemon/home_orchestrator.cpp`, `home_crash_breaker.cpp`) hands LG's Home back after three client exits in two minutes.

## Ranked ideas for LumaTV

**1. Register the Home stub with SAM so a launch reaches it. Evidence strong, fix effort medium-high.**
- Matches the open bug exactly. `docs/development/tv-research.md:1647-1675` (B3, webOS 10.3.2): without registration, a launch of a native app that is running behind another card changes nothing (SAM logs `NL_APP_LAUNCH_BEGIN`, surface-manager logs no window change; three launches 15 s apart did the same); with registration the window comes to the front 20-70 ms later. `lumaglass:docs/HANDOFF.md:683,700` records the same symptom for the native stub.
- Registration is the only way SAM can send a running native app `relaunch` (`docs/development/implementation.md:1629-1637`). OSE SAM source agrees in outline: `PolicyManager::relaunch` sends the event only when `isRegistered()`; otherwise it closes (SIGTERM) and starts a fresh process (webosose/sam `src/manager/PolicyManager.cpp`). Retail SAM was seen doing nothing instead, so the OSE path is not proof for the TV.
- Fit: `lumaglass:docs/research/boot-direct.md` section (g) calls the stub unregistered because it has no hub role and is a static aarch64 binary with no LS2. Facts that narrow this: a dev-installed app gets a role for its own id (`cardapp-result.md:173,194`), `lumatv-card` already registers from a 32-bit C helper (`cardapp-result.md:93`), and SAM accepted `registerApp` from a process it started (`cardapp-result.md:169`). The stub is bind-mounted over LG's directory, so it has no install-time role.
- Possible role route (not verified for this id): native-tv gives its own process a hub name by writing `services.d`, `roles.d`, `api-permissions.d` and `manifests.d` files under `/var/luna-service2-dev`, then `ls-control scan-volatile-dirs` (`native-tv/assets/early-service/install.sh:17-47`; `tv-research.md:1236-1262`). Whether a role for `com.webos.app.home` can be granted this way, and what name SAM matches (OSE matches `APPID-N` from the `LS2_NAME` env, `src/bus/client/NativeContainer.cpp:202`), needs a read-only probe on the TV.
- Hazard to design around: a registered client that SAM did not start leaves its registration behind and the next launch starts nothing (`tv-research.md:1660-1666`). Register only when SAM started the process, as `main.cpp` does.

**2. Fallback for 1: launch again after the window change. Effort low, already half-built.**
- native-tv found a launch from the covering card reaches SAM a few ms before that card is shown, while the app is still in front, and is ignored; in 13 of 14 Home presses the client returned only when the server launched it 2.5 s later, and a launch made after the window change worked every time (`tv-research.md:1667-1675`). Its server now re-launches 0.4 s after the stub appears (`implementation.md:1622-1626`).
- Fit: LumaTV's compositor already hears SAM's `launch`/`relaunch` events (`boot-direct.md`, section (g)). The failing case is different (no window change at all over an HDMI input), so this covers only the timing race, not the bug. Cheap to try as a second launch 0.4 s later in the probe.

**3. Faster, cleaner packaging. Effort low-medium, optional.**
- `assets/dashboard-app/install-app.sh:55-236` builds the `.ipk` on the TV from shell tools (`ar` header written with `printf`, `tar` and `gzip` as separate steps because webOS 9 BusyBox `tar` lacks `-z`), installs with `appInstallService/dev/install` and a local `ipkUrl`, and polls for `"state":"installed"`. A refresh compares md5 of each file and skips the reinstall while the app is running.
- `tools/runtime_payload.py` stages ARM libraries recursively, excludes the TV's own system libraries by list, records SHA-256 and licence notices, and replaces binary inodes so running processes keep reading the old file. Release builds use `@webos-tools/cli@3.2.6` `ares-package --no-minify` and a pinned webosbrew native-toolchain (`.github/actions/package-ipk/action.yml`).
- Fit: useful as a checklist for `tools/lumatv` and the payload scripts (inode replacement, running-app skip, pinned packager). Nothing here is faster per se; no measurements are given in the repository. Not verified against LumaTV's current scripts.

**4. Video decoder and resource release. Nothing direct; one pointer.**
- native-tv has no decoder code. SAM's close path is the useful part: a registered app is sent `close` and must exit within the transition timeout, an unregistered one gets SIGTERM then a kill (OSE `NativeContainer::close`/`kill`; retail timings not verified). Registration therefore also gives a clean place to release resources.
- Adjacent project, not part of this study and only skimmed: Native-Webos/webos-native-app-kit (GPL-3.0, so reference only). Its notes claim NDL stays acquired until `NDL_DirectMediaQuit` (leaks across launches), that a handler on fatal signals should release it, that unloading NDL blanks the video plane (stop feeding instead of unloading to pause), and that SDL-webOS turns `relaunch` into `SDL_RaiseWindow` and `close` into `SDL_QUIT` (`docs/research/NATIVE-MEDIA.md:47-74`, `APPLICATION-LIFECYCLE.md:27-38`). Worth reading before touching decoder release; unverified here.

**5. Small items.**
- SDL also registers a screen-saver request and a `tvpower` power-state subscription at registration (`SDL_webos_init.c:75-90`); a registered stub could use the same calls for the saver and wake handling.
- Hold registration back from bounded or hand-run test processes (see the hazard in 1).

## Not applicable

The Home takeover by bind mount, `profile.blockedAppList`/`blockSystemApp` nudges, the empty LG layout and the configd key filter (`tv-research.md:50-270`, `:460-480`) duplicate what LumaTV's compositor already does at the key level. The Node-free daemon, MQTT and dashboard have no bearing on the stub bug.
