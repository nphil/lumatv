# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

The idle timers and the saver count every device the set has, not only remotes and keyboards. A mouse, a game controller (its buttons, a stick or trigger moved a quarter of its range, the hat) and a phone remote's touchpad or scroll reset the countdown and wake a saver that is up, as a key does; the compositor reads them for that and routes nothing from them, and a controller's motion sensor, whose resting noise would never let the TV idle, is left alone. A notch of the Magic Remote's wheel counts under `remote.pointerShow` `movement` too (with the cursor hidden it counted for nothing). These devices do not keep the loop out of its slow tick.

LG's screensaver can no longer hold the keys for good. A key that finds it up asks tvpower to end it; a call with no answer in 2 s is made again, a refused one (tvpower said "power is busy" in the first checks) is asked again every second without another key, the saver's app is asked to close after 3 s, and after 6 s its window is hidden and the keys reach the app under it. A key that arrives while tvpower says `Screen Saver` and the window is not mapped yet is the saver's too. The OLED care no longer dims, shifts or samples the app under LG's saver.

The compositor makes its subscriptions to LG again when they fail. The idle timers (`nop/getTimerInfo`, which carry the saver's 180 s, the no-signal power-off and every other LG idle timer now that the compositor runs them) and tvpower's four registrations were made once at the start: a service not up yet, or restarted, left them dead and silent. A refused one is made again after 1, 2, 4 ... 30 s, one with no answer after 10 s, and the timers' every 10 minutes. `ctl status` has a `power` object with tvpower's state, the timers and when each is due, the subscriptions, and the saver's wake counters.

The stock recovery puts the panel's OLED pixel brightness back from `paneldim.json` when a hung compositor was killed under a screensaver dim, without waiting for the helper.

A Settings switch shows its value from its first frame. A row made when it scrolled into reach, when a page or category opened, or when its words changed built its thumb at the off end and slid it to the stored value, so an on switch seemed to flick off and back on. A switch now glides only when its value changes; a slider's thumb likewise only moves when its place or lift changes, not at every frame.

Home's dock reads like text: Right on a row's last app goes to the first app of the next row, and Left on a row's first app goes back to the last of the row above. The dock scrolls to the row on its usual spring and the focus stays inside the card. The two ends of the dock hold. A tile's name pill grows with the text size and goes to the side of the tile where the card has room for it (at 200 % text over the neighbouring row's edge), so it is never cut by the card.

Home held for 10 seconds, the supervisor's escape to LG's Home, ends everything LumaTV started and not only the compositor and its UI: the helper and what it started, the colour reader, the video-plane player, the camera feeder, the card and GeForce NOW apps, and the scripts that start and look after them (the tool, the boot worker, a service's own supervisor). All of it is stopped before any of it is killed, so that nothing starts again what was killed; LG's programs, the recovery and other apps are not touched. The escape is no fault, so it counts no failure, and its stock UI hold now lasts for the rest of that boot and no longer: the next boot starts LumaTV again, where it used to stay on LG's compositor until `lumatv compositor on`. The LumaTV app in LG's launcher starts LumaTV when it is opened while LumaTV is not the compositor, with no key pressed. It waits for a recovery of LG's Home that is still running, and a start that fails is shown with its reason, "Try again" and "Open LG's Home". Not run on the TV yet.

The Smart home card's lamps, fans and thermostats take what lumashow-ha 0.3.1 sends. A lamp's colour wheel marker stands at the hue the lamp says (`hs_color`), a lamp showing a white has no colour on the ring, the warmth track runs between the lamp's own limits (the owner's run from 1200 K to 6535 K, and the helper asks for a white inside them), and the effect chips are all of its effects, up to 128 (a floor lamp lists 111; a list was dropped whole above 16). A thermostat has a sheet of its own: the room now with what the thermostat is doing and the humidity it reads, the set point (or the two of a range) with a step down and a step up, and the modes as chips; it asks the helper for `temperature`, `range` and `hvac_mode`, which become `climate.set_temperature` and `climate.set_hvac_mode`. A short OK on a lamp, fan, switch or scene that Home Assistant lets the TV do nothing to (no control allowed for its kind in the LumaTV entry) sends nothing and says why for a few seconds, where it opened a sheet with nothing on it. The colour sheet's calls are paced inside the integration's 10 calls in 10 seconds: a quarter of a second apart for the first three of a turn, then one in 1.5 s with the latest value.

A Home at rest draws a few frames a second, from the clock's hand, instead of one at every refresh. Two faults did it with the motion off. A closed channel guide reported a fetch due at 0 for good (its page is set when its surface is built, before the model has a clock), so the shell's loop turned at full speed and each turn committed Home's poses again, a frame of the compositor's each and about half a core of the UI. And after Home's last change the engine's floor was never brought up to date, because a page that owes nothing counted as Home drawn straight into it, which asked for a frame at every refresh that drew nothing. On the host with Nitin's cards, the wallpaper and the weather sky still: 120 frames a second before, 4 after, the UI at 0.6 % of a core instead of 52. The weather card's sky still draws at every refresh while it moves (theme `weather.animated`, on by default, and not under reduced motion).

The compositor asks the panel back to 120 Hz after a 25 or 50 fps video left it at 100. With the refresh policy automatic, Home in front and nothing feeding the main window (no video window, no claim holding one, `connect_src=NONE`) for 1.5 s while the flips measure the re-timed period, it writes `1 60` to `/proc/lg/de/fr_rate` once; a video that comes back ends the episode unwritten. The Ring's full view left the panel at 99.9 Hz until a 30 or 60 fps video played. Whether the firmware takes the request with no source is open: round 22 measured five variants ignored.

The channel guide: LG's TV Guide rebuilt in LumaTV's look over the live picture, from LG's own guide service (`iepg`). Every way into LG's guide opens it instead, and LG's is never drawn while the shell runs: the Guide key goes to the shell while Live TV is in front, so LG's guide is not even launched, and List, Live TV's menu entry and the banner's Guide button, which launch LG's guide through SAM, are answered by the compositor, which leaves LG's window out of every frame from its first commit, asks the shell for its own guide when SAM reports the launch, and has SAM close LG's app. LG's guide is stock while the shell is not running. The Scheduler is an app of its own and stays LG's; this guide has no scheduler or recordings page yet. Rows are channels (the antenna's at once, the internet's on the Internet chip) and the grid is time; the arrows move between programmes and channels and the view scrolls only as far as the focus needs, on springs the compositor runs, so the app draws nothing of its own in the frames of a scroll. CH+ and CH- page, the Magic Remote's wheel moves a row, OK tunes a programme that is on the air and opens a sheet for any other, Info opens the sheet, and Back closes it, then the guide. Only the programmes of the rows and hours on the screen are asked for, a few channels a call, and a channel without listings says so. `docs/research/channel-guide.md` has the design and the TV check list.

The channel guide's layout follows rules instead of cases, and reads clean at 100, 150 and 200 % text in both looks. A title is said in the part of its cell the screen shows: whole, else its first letters and an ellipsis, else nothing, so no title is cut by the grid's edge or runs under the next cell, and a cell of a few minutes is a divider and not a fragment. The titles of the first column, the words of the times over them and the pinned title begin in one place, and the title "Guide" and the filters stand on one baseline. The line of now no longer runs through the rows (it crossed the title of nearly every first-column row): the ruler has a band under its words for the marker of now, and the programme on the air has a rule of progress that ends at now. The lit plate keeps off the grid's edges and the next cell, a channel with no logo says its call sign on a tile with an edge, and the detail sheet is as tall as what it says over a page that its scrim hides (it kept room for five lines of description and showed the page's words cut beside it). The layout lint reads the cell of each title, the lines of the grid and the plate, and the surface tests hold the guide to it over looks, text sizes, clocks and places, with edge channels the make-believe set now has (a ticker of 5 to 30 minute programmes, a long name, no logo, no listings). `tests/render_rust_guide.py` makes 36 pictures.

The frame gate (`tv_gate.py`) turns camera pop-ups off before its surfaces and puts them back after, with the `X-Luma-Client` header the helper's settings route requires (the documented step was refused, so a run had pop-ups on), and ends with `gate.sh refresh auto`, also after a failure: `refresh 60` and `120` leave the compositor's refresh policy manual, which kept Marquee at 60.

The compositor opens input devices that appear after it started, and opens a device made anew. lginput2 makes its virtual devices (`Smart Remote RCU Input`, where Home Assistant's and the phone apps' keys arrive, and `LGE Network Input`) anew at every wake from standby, and the compositor had read only what it found at its start: from the first wake on those keys went nowhere, the idle timer ran out and the screensaver came up. `/dev/input` is watched (inotify, with a look every two seconds as the back-stop); a new `event*` node with keys opens, on its own seat, and a device made anew goes back to the slot and the seat of the one that went. Game controllers (no keys below `BTN_MISC`) are left to the games, whose sticks and gyro would move the pointer. The perf gate asks the compositor whether it has the key device open (`kb.py --device`, `/proc/PID/fd`) and stops with a message when it has not: the 2026-11-04 120 Hz gate, whose keys never arrived, timed the saver's 10 Hz sweep (69 to 819 ms to the first frame) with nothing animating.

The compositor's look at `/dev/input` no longer holds its loop for 115 to 178 ms every two seconds (the compositor's own drops in the 120 Hz gate). Every look opened each `event*` node that was not open: the 18 nodes LG's init makes with no device behind them (`event14` to `event31`) each ran `modprobe`, and the nodes of the game controller and the touchpad, which have no keys, were closed again, each close waiting for an RCU grace period. A node is looked at when it is new or has changed (its link in `/sys/class/input`, its inode and change time); a node with no device behind it is not opened.

The frame gate no longer reads a trace whose ring overran as a display that was off. CPU 0 takes every device interrupt; with a USB game-controller dongle polled at 1 kHz, the network and video decode it filled its 8 MB ring in 8 to 12 s, so a capture began without its first 0.1 to 4.8 s and the OSD interrupt seemed to return where CPU 0's events begin (first frames of 474 to 1323 ms on 2026-11-04; round 11's Dolby Vision finding was the same). A window that begins before every CPU's events are in the trace, known from the dump's header, is not read for its first frame, burst or OSD interrupt and fails `trace_complete`. The ring is 16 MB a CPU, and `cpu_idle`, which nothing read and which was a quarter of CPU 0's events, is no longer traced. LumaTV's own GPU jobs are not counted as an app's drawing, which had every key capture run three times (1387 s for 873). The gate measures the panel (`DE2VSYNC`) before it starts and stops when it is not at the asked refresh, which the TV had re-timed to 100 Hz for a camera pop-up.

The shell's repeating requests (`[helper] GET /v1/cameras/N/frame` for each still, `[channel] POST /v1/ui` for the state report) are logged to the RAM log three times a minute per route, with a count of the rest on the first line after the minute. They were a quarter of its 1 MiB. The host harness prints every one as before.

A camera's label on the Smart home card sits on a plate of the theme's canvas colour, as strong as the contrast lint needs over any picture (dark text on bright foliage in the light look was hard to read). The camera full view's dimmed backdrop uses the grid's tile of the camera until its own still arrives.

The Downstairs door (a Ring doorbell) plays in the full view and has a picture in its grid tile and behind the video. Scrypted gives that camera's WebRTC stream to one client on its own machine, so the address the bridge handed out was refused from the TV. The bridge (0.2.1, `lumashow-scrypted`) is that client, as Scrypted's HomeKit plugin is, and serves the stream on its own RTSP address; its pictures are Scrypted's `takePicture` snapshots, a new one a minute because each wakes the battery doorbell. The doorbell starts a session in 0.6 to 3 s, in about 10 s, or not within 20 s, and the TV waits about 7.5 s: a slow start shows the note, and "Try again" within 20 s takes the start over.

Live TV's channel banner is LumaTV's own, in the Lucent theme, and one slim strip centred above the subtitles' band, where the toasts rest: the station's logo (its call sign's letters where it has none), channel number and name, and what is on with its time and how much is left, for five seconds at a launch and at each channel change. INFO, which Live TV no longer opens its own card for, grows it for eight seconds with what the programme is about, what is next and its marks. At 100 % text it is 800 by 114 where the first was 1457 by 154. Its glass is the theme's sheet, no more than 1.5 % see-through, so the print of an ad behind it does not read through its words (every word reads at 4.5 to 1 over a white ad with black print), and "No information" is said quietly, with no time and no line. LG's banner is left undrawn on Live TV's picture while this stands in (`POST /v1/mask`, `mask.rs`), from a CH+ or CH- press, from each channel change and at a launch, for as long as LG's lasts: the bar, and the grey "Guide" button LG hangs under it at a launch, a view of its own outside the bar's rectangle. It is given back when the UI is not running, the owner's banners (`banners.own`) are off or the tuner cannot be followed. The banner LG shows for OK stays LG's, Guide button included.

A camera the full view cannot start now says why, over the camera's stills, instead of leaving a black "Starting" screen: a start Scrypted refuses (a camera it reports offline, the plant room on 2026-11-04) is a note with "Try again", and is asked for again by itself after 3, 6 and 12 s and then every 30 s; a stream that drops after it played is asked for again the same way. A camera that never played (the Ring doorbell, which wakes on each start) is never asked again by itself, and the refusal of a start while another camera plays stops that session. The helper's log carries the reason a live session failed and how long a session with no picture lasted, the view follows a stream that changes size under the reader, and an unreachable stream reads "Scrypted's stream for this camera could not be reached".

The bars beside a picture that is not 16:9 (a portrait doorbell, a 4:3 camera) are filled with a blurred, dimmed copy of the camera's picture: in the full view beside the video window, in a still, and in a grid tile, which then shows the whole picture instead of its middle. The copy is made small (48 pixels on its longest side, blurred, scaled up by the compositor) from the picture the helper decodes at one eighth, asked for every 2 s while the video plays; nothing is decoded twice and the video plane is not touched. A 16:9 camera has none. `theme.cameras.fillBars` turns it off.

After Back on an app that is closed with its window (a web app under WAM such as Homebrew Channel, a native app such as Punktfunk that exits) the remote's arrow and OK keys work on Home at once, not after a turn of the wheel. The compositor now publishes what is in front when a window is destroyed, as it does when one is unmapped or minimized; Home was never told it was in front again, so it did not take the keys, and the app manager's foreground list kept the closed app.

The compositor serves LG's own keymap (`evdev:lgwebostv:us`, reduced to the remote's keys and the plain ones) instead of a hand-written one that named CH+, CH-, Back, Info and the colour keys and gave them no symbol: LG's Qt apps (Live TV, the HDMI inputs) took each of them for Qt key 0 and ignored it, so CH+ and CH- never changed the channel. `compositor/tools/gen-keymap.py` regenerates `server/keymap.xkb` from the set's xkb tables.

The Home weather card is planned for the size it has, at either text size: the reading, the next thing, the hours and the week take the room there is, side by side on a wide short card, and the sky behind them ends in no edge and is drawn in the theme's palette. Thunderstorms and snow now and then, one to three minutes apart, take over the whole card for a few seconds (lightning that lights its glass, snow that drifts across it); the sky holds where it is while something covers Home and comes back from there, and under reduced motion it does not move. A clear night, a partly cloudy or overcast sky and a windy one are drawn as one rectangle over the sky's box, not one for each star, cloud and streak: about 30 layers a frame where they were 75 to 160.

Home's weather (the card, the weather view and the screensaver) can come from a Home Assistant weather entity instead of the built-in source: Settings > Home & look > Home has a Weather source row and an entity picker, and the ZIP row shows for the built-in source. A failed read keeps the last reading, shown as stale; it never falls back to the built-in source. The radar centres on the place Home Assistant has for home.

The boot worker activates LumaTV's compositor even when the wake policy's preparation fails, with no landing app, instead of leaving the set on LG's compositor with the mount in place; a power-loss cold boot stopped at that preparation's timed-out `systemctl daemon-reload`. A standby activation that fails because the set was switched on meanwhile is tried once more as an active boot.

The Smart home card shows the shared Home Assistant things and the TV's cameras as raised glass items in small, wide and large sizes that scroll by whole rows, with the next row or item peeking behind a fade. OK enters the card, the arrows move between items, the focused item lifts as raised glass in both looks, and Back leaves; OK on a light or switch toggles it in the same frame and puts it back with a short reason when Home Assistant refuses; OK on a camera opens the full-screen viewer; OK on a sensor opens its history (a line chart for numbers, bands for on/off sensors, a list for text); a held OK on a colour or colour-temperature light opens a colour sheet with a hue ring, saturation, brightness, warmth, presets and effects; Menu on an item sets its size and place, kept per entity by the helper, and Home's layout editor has an Items button that opens the same menu. The Magic Remote's wheel scrolls the card under the pointer: the compositor now sends `wheelup` and `wheeldown`. The helper serves sensor history from Home Assistant's recorder (`GET /v1/onhome/history`) and sends colour and effect to lights.

The helper's weather prefetch thread checks for a stop every 50 ms instead of every 5 s, so a stop no longer leaves it to the process exit.

The native compositor executes LG idle timers, keeps screensavers above idle apps with the required application-lifecycle notifications, and wakes them through LG’s existing power service without forwarding the wake key into the app. LG playback policy remains authoritative.

Receiver settings use individually qualified controls and confirm changes from receiver readback, without changing main-volume behavior.
Receiver discovery declares its XML encoding so compatible Denon receivers return their reported capabilities.

Home Cameras opens the native grid and full-screen viewer. Home Assistant and Scrypted credential stores enforce owner-only directories and exclusive private temporary files before atomic replacement.

The bottom volume popup keeps its full shadow and antialiasing outside reported subtitle bounds through cue changes.

A reconnecting shell is no longer replayed one-shot topics (`homekey`, `show`, `luna`, `imekey`, `menu.open`, `confirm.ask`), which could open the switcher or a dialog unprompted.

A failed scan-out page setup frees its dumb buffer, framebuffer and GL objects, and a refused warm target is tried once instead of on every direct-scanout attempt.

Screen recording reads frames through pixel pack buffers a frame behind instead of stalling the loop on the GPU, with the alpha made opaque on the GPU.

Picture-mode labels capitalize Personalized without changing the native mode identifier.

The UI no longer rebuilds its state report for pointer motion that changes nothing, and re-reads the theme and layout files only when their size or time changed.

Home no longer re-serialises its layout and theme on every redraw to see whether the chrome changed, and the Quick panel lays its rows out again only when they change.
Partial LG picture replies retain the current controls; confirmed input, mode or range changes still invalidate stale values.
Picture-mode strips keep focused choices visible and align focus and pointer bounds with their rendered cells. OLED pixel brightness uses its own native settings context. Glass-strength changes update existing Settings controls without rebuilding unrelated switches on every key repeat.
Card focus outlines retain the rendered button geometry while their highlight fades, without lifting or enlarging the outline independently.
The status-bar gear opens Quick Settings. Home & look contains the native Wallpaper editor, preserving live preview and save-on-close without the redundant All settings link.
Glass type replaces the Home-specific label. Non-Rose theme wallpapers use curated dark/light shadow, midtone and highlight colors; Rose Pine retains the original wallpaper.
Home shows at most 60 dock tiles; a longer launch list no longer makes the dock card disappear.
A live camera feeder that stops reading ends its session and frees the video path instead of blocking it, and a session that fails to start no longer leaves later streams attached to it.
A button an app was told is down is released when the pointer hides, so a held press no longer sticks.
The UI's key socket finishes a partly written line before sending the next, so the UI never reads two lines spliced together.
An idle screen with no video claim or window does no per-frame work for the video-window topic.

The native compositor uses the Rust UI for all shell kinds, with stock LG Home
as the failure path. Obsolete project QML frontends and restyles are removed;
ownership-checked restoration removes previous overlays.
Boot activates the mounted native compositor; confirmed standby skips foreground-dependent preparation, while active wake intent and stock recovery remain guarded.

### Added
- Compact single-toast display replaces the previous toast and keeps transient LG messages out of notification history and unread counts.
- Volume displays original encoded-audio glyphs. PCM with Dolby Surround or DTS Neural:X processing also shows its processing brand, with an explicit PCM carrier label.
- Receiver volume preserves half-unit readings in the popup and Sound settings instead of rounding, with room for the decimal at both text sizes.
- Automatic compositor pacing selects 120 for foreground Marquee and restores the startup base elsewhere, with a manual override.

- Experimental native wake policy restores the previous foreground app on normal wake and power-on, retaining native explicit-destination priorities and bounded recovery in the existing boot worker. Rollback of an originally absent policy path sets the explicit stock path, with different setting metadata.
- Native wake trials handle bootd log truncation and Factory-setting precedence, retain failed configuration readback, and refuse unsafe defaults or changed setting ownership. Selector and fallback output retain the final app-ID character through LG’s native shell reader. Startup serializes policy reload, existing destination landing and final acceptance with fresh worker/activation tokens; native explicit choices and newer foregrounds remain protected. Early-start records accept LG’s space-padded timestamps without weakening freshness checks. First-launch inputcommon wrappers require corroborated native input/action records; an unproven newer action never revives older intent.
- Optional UDP syslog forwarding keeps up to 1 MiB of ordinary logs in RAM, with no
  flash fallback. Settings > System > About > Logging edits the destination.
- LumaTV's compositor is live on the set (persistent), with LG's compositor as the automatic
  fallback (hold Home about 1 s). The full device gate passed every blocking check: PlxNative's
  close 15 of 15, x1, the shell, standby and the fallback tests; the frame lines show only
  isolated 33 ms intervals and a few slow first frames.
- The Magic Remote pointer appears only when the wheel is turned; Settings > Remote & pointer >
  Show pointer switches back to LG's show-on-movement. (ae3c348 cc57205)
- Hardware planes behind switches: live page-layout switch (`ctl afbc`, `ctl pagealpha`),
  video-window claims for the Cameras live view, a picture-in-picture surface API, the
  OLED-care orbit as the primary plane's position (`--scanout-offset`) and `ctl pageprobe`.
- The GeForce NOW app ships inside the main package; `lumatv apply` updates it by build id.
- Screensaver rules per notification source: show over the saver, use its notification area,
  wake or hide. Sources include cameras, doorbells, timers, Home Assistant, ntfy and plugins.
- Offline display-register decoding and noise-aware comparisons with `tools/regdecode.py`.
- Compositor (not live yet): the frame loop and the Mali driver's submission threads run
  SCHED_FIFO; a buffer the driver is still warming is stood in for by the last frame; the idle
  tick follows the panel's refresh grid and the latch moves later by at most 1 ms per frame;
  the X1 probe pre-allocates its swapchain and stamps its gaps; a start-up trace and late-flip
  lines explain any gap the gate still sees.
- Compositor (not live yet): the Input key opens LumaTV's input picker on its press under
  LumaTV's shell (first frame 31 ms instead of 81); a 1 s hold still opens the Hub.
- Compositor (not live yet): the shell client no longer inherits the frame loop's real-time
  priority and starts off the loop thread; the system layer stays mapped and is drawn only
  while live, so a toast costs one frame; one late frame no longer shifts the pacer twice; the
  GL context asks for the GPU's realtime priority.
- Compositor (not live yet): the on-screen keyboard is placed by its window's input region, so
  it always opens at the bottom; before, maliit's full-screen window sometimes put it at the top.
- `cutover.sh enable` fails and clears its flag if LumaTV's compositor falls back during the
  enable; `lumatv revert` restarts LG's compositor when LumaTV's was live.
- GeForce NOW: the controller reader threads run at real-time priority off CPU 0 while a
  stream is in front (only the app's own reader threads, granted by the helper and undone when
  the stream ends); the stats line shows press latency, and `lumatv-gfn-latency` measures it.
- Settings › System › Support › Update gains "Block LG system updates" (the Homebrew Channel's
  own switch, with the live hosts state shown); while it is on, Check for Updates does nothing;
  `lumatv status` and `lumatv updateblock status` report the block.
- Input banners: the no-signal screen of an HDMI input follows mock 18 (plug glyph, input name,
  title, a line, and Inputs and Home buttons), and the Guide chip under the channel banner is a
  glass pill.
- `lumatv store on|off|status|probe`: LG's Content Store in the Lucent look (Manrope, a glass
  plate with a ring and ink on the item LG focuses, found by what the page computes),
  transitions capped at 180 ms; off by default; `off` and `revert` restore LG's store exactly.
- Live TV comes to the front under LumaTV's compositor: a window attached to a surface group
  (Live Dmost) is a member of its owner, not a card that takes the front, the keys and SAM's
  foreground.
- Settings: tabs sit on a tray, single tabs draw no bar, Delete/Reset/Forget use the danger
  style, and App Permissions shows each app's icon and a proper empty state.
- `venc/`: the TV's hardware H.264 encoder driven from LumaTV's own process (video tap into
  the encoder, zero copy), in small steps under a watchdog, with a TCP stream server; LG's V4L2
  M2M encoder node is never used (its mmap writes kernel memory).
- Built behind switches, off until each TV check passes:
  - App switcher: a second Home press (or Back from the top of Home) shows the running apps as
    live pictures with Quit and Quit all; the first Home press still acts at once
    (`remote.appSwitcher`).
  - Smooth transitions: apps grow out of their tile and go back into it, one app fades over
    another, and video apps arrive on a veil with no black frame (`motion.transitions`).
  - Overlay cards over any app (doorbell, timers, alarms, Home Assistant alerts, scores) in the
    top right, never in the bottom quarter, OK acts and Back dismisses (`overlays.enabled`).
  - OLED care: LumaTV's own screens drift one pixel every 30 s around a 3 px ring and an idle
    Home eases to 85% after 5 minutes (`oledcare.enabled`).
  - OLED care for apps, each its own switch in System › OLED care: a still app eases down and
    drifts a few pixels (`appDim`, `appShift`), and the bright parts of an app's picture that stay
    still for minutes dim to 0.8 with feathered edges (`boxDim`). A key, a real change or video
    in the hole puts the picture back at once; video is never dimmed or moved.
  - Panel refresh guard: test rounds wait until 15 min of standby have passed before waking a TV
    whose OLED refresh is due (`--force-wake` overrides); the helper's `panel` topic reports LG's
    refresh runs, `oledcare.refreshGuard` refuses an assistant's wake during a run, and System ›
    OLED care shows the last run.
  - Screenshots and screen recordings of what LumaTV composes, with a recording dot; video
    stays black (`capture.enabled`).
  - A 120 Hz refresh test mode for Home and menus (`--refresh 120`, `ctl refresh`).
- The volume bar rises to 184 px while an app is in front, clear of subtitles; on Home it stays
  at 64 px.
- LumaTV's own keyboard shows when a field activates, for apps that never ask for the panel.
- Quick panel docks at the right edge as in mock 10 (648 wide), with steppers beside their
  names and no LG Quick Settings button; Settings tabs and segments mark the chosen option in the
  accent; the Notification Centre head and times hold at 200% text.
- LG's Search app (`lumatv voice on`, off by default): field, keyword chips, genre cards, typed
  suggestions, mic hint and top row follow mock 09.
- Built behind switches, off until the TV check: recent apps on Back, Quick Access toasts,
  quick-panel choices and tile editing, the LumaTV startup cover, LG's launch splash and spinner
  in LumaTV's look, and an Always Ready off switch. Notification cards are opaque and name the
  HDMI port.
- Cameras through Home Assistant's LumaShow integration: event pop-ups (person, vehicle,
  animal, doorbell) in the top right over whatever plays, with a preview, OK to a full view and
  a grid of the shared cameras; the TV pairs as one more LumaShow device and holds no token. Off
  by default (`cameras.enabled`) until the TV check passes.
- Home's wallpaper can play as a looping HEVC clip on the TV's video plane under LumaTV's
  compositor (`lumatv wallpaper video`, default `gpu`): the GPU wallpaper stops drawing, Home's
  glass stays above it, and the GPU wallpaper returns at once on any player failure. The
  compositor clears video holes in LG's order.
- LumaTV's own on-screen keyboard, off by default (`lumatv keyboard luma`, back with
  `lumatv keyboard lg`): the shell client draws it and the compositor binds it to the field, so
  maliit no longer has to start; layouts follow the field (letters, e-mail, url, symbols, number
  pad), the D-pad wraps and repeats, the pointer works, and LG's keyboard stays the fallback.
- Compositor: malloc makes no mmap or trim on the frame path and the loop's memory is pinned;
  the gate reports the loop's wake-up latency and the kernel waits behind late frames and can run
  under synthetic background load; a crash bundle is shown in Settings › Support after a
  fallback; the idle tick runs 10 times a second instead of 60.
- `npuprobe/`: times a TFLite model on the CPU and through LG's NPU delegate; TV probe script.
- Cameras: the full view plays the camera live on the video plane, relayed by Home Assistant
  from its own go2rtc with sub-second latency (HLS as the fallback); no camera password reaches
  the TV (`cameras.live`, off by default).
- Settings lists: section heads grow with the text and read in the light theme; danger labels
  (Delete Device) and disabled-row text pass contrast at 100% and 200%. Segmented controls,
  status words, the AI access lock row and long page headers pass too: all 98 Settings scenes
  pass the layout audit in dark and light at both text sizes.
- Input banners: the pill, the INFO cards, the prompts and the no-signal key-help line keep
  the Lucent glass and ink when LG's restored input process holds its own style sheet: every
  patched file reads its values from LumaStyle.
- Settings: Bluetooth Speakers gains Sound Balance for two matching connected speakers and a
  Position row on each connected speaker's page (Left/Right or Front/Rear, as LG offers;
  asks before Left and Right swap). Every LG setting now has a LumaTV page or row.
- Light sync over Zigbee2MQTT: nothing is sent while Zigbee2MQTT updates a device over the
  air (and for 30 s after); a running stream ends and the lights go back as they were, then
  it resumes by itself. Late answers from the Zigbee network halve the rate (20, 10, 5 fps),
  then pause it for 60 s.
- Quick panel and input picker draw their content from one cached texture, so opening and
  closing over Home costs fewer draws per frame (101 to 72 batches).
- Compositor (not live yet): LumaTV's compositor routes the remote's gear, Input, app
  buttons, Quick Access digits, Mute held and Exit, and brings up the on-screen keyboard for
  text fields; a test session ends by itself after 15 minutes and LG's comes back. The Magic
  Remote's Home (evdev 773) is Home there too, and holding it now reaches the supervisor's
  escape to LG's compositor (its key-state buffer stopped short of code 773).
- Volume bar: the receiver's level shows as a whole number (no decimals or unit), and its
  audio format as a one-colour logo (Dolby Atmos, Dolby Audio, DTS:X, DTS) in the bar's ink;
  PCM, stereo and unknown formats show nothing.
- Frame rate: after 25 or 50 fps video the panel goes back to 120 Hz when Home is in front
  and no video source is connected (LG's kernel otherwise stays at 100 Hz until the next
  video), so Home never runs at 50 fps; `frameRate.returnOnHome` false turns it off.
- Volume and mute: a slim bar at the bottom centre is the only volume display, over Home,
  every app and every input; LG's popup is patched out of the shadow module (`lumatv revert`
  brings it back). It answers on the key before the receiver does, shows the Denon's level
  or the TV's own, the receiver's audio format (Dolby, DTS, PCM and the surround mode), and
  a slashed speaker while muted.
- Home Assistant on the TV: LumaShow's plugin runs through the helper as a user of its
  own, switched on and linked only by the owner's own commands (`plugin-consent`,
  `plugin-enable`, `ha-link`). Its cards fill the At-home lane, its tiles the quick panel,
  its notices the drawer, and its page is Settings > Home & look > Home Assistant. Locks,
  alarms and cameras are never touched from the TV; the placeholder chips are gone.
- Settings: minimal text on every page. A row shows its name and value; explanations sit
  behind one info mark, opened by holding OK. The rail shows one short fact per section,
  Remote & pointer has tabs, the Settings bar shows the link that is really up, and held
  Left/Right speeds up long sliders.
- Settings: native pages for nearly all of LG's Settings, on LG's own keys and value lists:
  Picture (every tab, White Balance, Fine Tune, Video Range, HDMI Signalling), Sound (Sound
  Out, Dolby Atmos, Equalizer, Bluetooth speakers, USB devices), Accessibility, Time & power,
  Wi-Fi and IP settings, the HDMI rows, Home Settings and screen saver rows, Support and
  privacy, Safety, Channels, Always Ready and OLED care. Live Plus asks before it turns on.
  What remains of LG's menus opens from a "More in LG settings" row. As in LG's Settings,
  the NEXTGEN TV rows show only on a TV with an ATSC 3.0 tuner.
- The gear tap and the Input key open LumaTV's quick panel (tall, docked at the left where
  LG's sat, sliding out of the edge) and input picker, on Home and over apps; LG's own
  stay one button away and take over whenever LumaTV cannot show its own. Both slide in
  110 ms and keep their rows still while sliding. The page dims whole instead of fading,
  the sheet slides without a fade, and each inputs refresh drops the previous subscription
  (eight were left open before, and the picker grew in memory at every opening). Closed
  panels stay in the scene off screen for 4 s and the gear's press joins the quick panel
  early, so opening and closing no longer rebuild Home's render list.
- Home widgets: Settings > Home & look > Home widgets (show, hide, size, order, reset), an
  on-TV layout editor (hold OK on a card), and Clock, Weather, News and At-home cards that
  use the room they get; `lumatv layout` writes the owner's layout file.
- Saver: art mode can draw its print on the TV's idle video decoder instead of the GPU
  (`screensaver.video`, off by default while the panel stays black in LG's Screen Saver
  state; on the set it cut Mali from 21.9 % to 1.9 %). The player is `lumatv-vplane`
  (`lumatv vplane install|start|stop|remove|status`). The shuffle order is kept across
  restarts, and the twelve-hour clock shows 12-hour time.
- Saver: while Home is in front LumaTV answers LG's idle question with no, so LG's Flutter
  saver never starts and LumaTV's clock or art saver is what the TV shows; a key, the
  pointer, standby or leaving Home ends it (`screensaver.own` off gives LG's saver back).
- Restyles of LG's own processes in the Lucent look: the input banners, Live TV's channel
  banner and INFO card, the no-signal screen, the banner position editor and the set-top
  box and input lock prompts (`lumatv inputcommon on|off`, which also sets LG's CRIU dump
  of the input app aside so it starts cold), and the on-screen keyboard's open and close
  motion (`lumatv keyboard on|off`). Both patches fail loudly when LG changes the code.
  LG's Search app can take the same look (`lumatv voice on|off`, off by default); it is a
  resident process SAM keeps alive through a close, so the switch ends it and the next open
  shows the chosen look. The on-screen keyboard's keys (caps, function keys, focus,
  candidates, number pad) take it too (`lumatv keys on|off`, off by default), which
  restarts LG's keyboard service and rolls back on any load error.
- GeForce NOW app (`org.nphil.lumatv.gfn`, its own ipk): OpenNOW's engine and core on the
  TV with the stream on the video plane through LG's NDL DirectMedia (4K120 HEVC Main10
  HDR10, low-delay game mode), controllers read from evdev with grab and rumble, and a
  Lucent UI (sign-in by device code, library with box art, launch, in-stream overlay, a
  Settings screen for tier, HDR, bitrate and stick dead zone). When NDL refuses Opus the
  feeder decodes the sound itself, and it registers with SAM so relaunch and close reach it.
- Settings and every custom screen: words are never cut, crammed, moved by focus or
  unreadable. The settings service's partial answers are read (OLED pixel brightness and
  Energy saving are live again; a key the TV lacks hides its row), focus lifts the plate
  only, cards and pills centre their text, lists scroll by whole rows, and tab bars scroll
  when labels do not fit. A layout and contrast audit at 100 % and 200 % text guards it.
- Helper: Bluetooth proxy for Home Assistant. The TV answers as an ESPHome Bluetooth
  proxy (passive scanning, raw advertisements, encrypted API, mDNS) fed by LG's bluetooth2
  `le/startScan`; off until the owner runs `lumatv-helper ble-link`, scans in windows of
  5-30 s with an equal or longer rest, and never while the panel is dark, Bluetooth audio
  is up or the adapter is off.
- Declutter group `autoinstall`: LG's silent app installs stopped (`option.appUpdateMode`
  manual, `support.automaticUpdate` off), the factory stubs LG's updater turns into full
  apps removed, and LG's not-installed country apps kept on the deleted list. Store
  installs and updates stay.
- Voice search: the mic key listens, Home Assistant's Assist pipeline turns the speech
  into text, and the text fills Search. Off until the owner links Home Assistant, chooses a
  microphone and switches it on; the helper's `voice-link` saves the address and token, and
  nothing the words say is run.
- LumaTV's own alert, input pop-up and PIN prompt over LG's notification view;
  toasts keep an app's own icon unless it is LG's generic file.
- Quick panel plugin tiles and an All settings button; the input picker draws
  its tiles and device icons from the TV's input list.
- Ambient light settings page, art-mode and logo options, and the screensaver:
  a drifting, dimming clock or a crossfade of six public-domain paintings, with
  no timers running while hidden.
- The notification drawer (bell on Home), plugin cards on Home, and weather,
  news and clock as first-party plugins on the plugin broker.
- The app library from the Apps tile (search, hide, rename, move, uninstall),
  search, the Live TV banner, and openers for the context menu, confirmation and
  on-screen remote.
- `tools/tokens.py web`: Lucent tokens as CSS for LumaTV's web apps.
- The UI engine draws into the compositor's scan-out pages (`--features ui`).
- The helper's `GET /v1/device` and device facts in every plugin welcome.
- The light-sync probe slot (`ambient/capture/tv_probe_slot.py`): LG's
  `vt_devmode` flag exists only while a capture runs.
- The card app (`org.nphil.lumatv.card`): a 64-bit app in its own card window,
  drawn with the Rust toolkit. SAM starts a 32-bit launcher that registers the
  app and passes SAM's relaunch and close on. It ships as an ipk inside the
  main one with its own 64-bit runtime (31 MB larger package). Under SAM's
  jail, whose `/dev` has only `dma_buf_unified` to allocate from, the heap shim
  takes the window buffers from that device.
- Hue Entertainment streaming as a second ambient output: `lumatv-helper
  hue-link` saves the bridge's keys privately with its certificate pinned, and
  the stream stops when the screen goes off.
- LumaTV's own banners over video (`LumaOverlay.qml`, switch `banners.own`,
  off by default), holding the gear key opens the notification drawer
  (`remote.gearHold`), number keys open dock tiles (`remote.numberKeys`), and
  the remote's app keys open the app chosen per key in Settings.
- Settings: Glass depth (Flat, Normal, Deep), a sleep timer on System and in
  the quick panel, and panel hours on OLED care.
- LG's input banners in the Lucent look (`lumatv inputcommon on`, off by
  default) and the on-screen keyboard's open and close in the theme's layer
  motion, by anchored patches that refuse when LG's code changes.
- Cutover safety for LumaTV's compositor: a firmware check against a list of
  verified builds, a watchdog that falls back to LG's compositor on a frozen or
  empty panel or an unsettled run, the hold-Home escape from the first moment of
  boot, and a gate that allows no gap over 20 ms.
- Declutter group `autoinstall` (profile `ads`): LG's silent app installs
  stopped at their two switches (`option.appUpdateMode` manual,
  `support.automaticUpdate` off), the factory stubs that LG's app updater turns
  into full apps in standby removed (DAZN came that way on 2026-10-25), and
  every not-installed id of LG's country list on SAM's deleted list so the
  wake-time sync adds nothing. The store's own installs and updates stay. Host
  tests run the tool against a made-up set (`DECLUTTER_ROOT`), and switching
  one group off re-applies the list edits of the others still on.
- Screensaver: the clock is dimmer, a camera pop-up no longer covers it, and Settings can choose per source (video, doorbell, timers, Home Assistant, ntfy, notices, plugins) whether it shows over the saver, inside it, wakes the TV or stays hidden. (RustSystem 9d7a15d..b34e5d6)
- Privacy: LG's ad, recognition and upload pieces are stubbed and masked first thing in the boot hook (about 13 to 14 s in), before pbs, the network and any LG server call; the menu says per item whether this boot's pass really beat LG's start. (b62e7b5 d4cfb30 8e89bc2)
- Settings is drawn by the Rust shell (opt in with --shell-rust settings): Privacy, Profiles, Home & look with the new During the screensaver page, System, Support and Picture; the remaining pages followed. (RustSettings 561eeb7 62848c0)
- Home moves to the Rust shell as the `home` kind (opt-in: `ctl shell rust +home`): wallpaper, status bar with the profile chip, cards, dock, library, search, layout editor, Who's watching?; the QML Home stays the fallback. (RustHome d2de289..dfda106)
- Tools: tools/regdecode.py names and diffs /proc/lg/fbdev/regdump captures against LG's OSD register headers, offline; docs/research/display-chip-direct.md maps the display chip and ranks hardware-plane wins. (1a41d27 f772573)
- Rust Home: notification drawer and unread badge, camera stills, and a scrolling library rail at larger text sizes. (af5e478 5332da4 adc25d8 c88ff3a)
- Gate: x1 keeps the raw strict result beside gap attribution; a missing or incomplete trace is inconclusive, never a pass. (ce83f61 2731f63)
- Rust Settings: Network, Join Network and IP Settings ported, with host renders and tests. (05bc9d4)
- Cameras app in the Rust shell: grid of live stills, full view with the live stream on the video plane (its video-window claim wired to the hardware-planes allocator), scrub strip with thumbnails and event marks, events/clips sheet and per-camera pop-up options page, behind `--shell-rust cameras` with the QML viewer as fallback. (5c2d800..b8a3d5b)
- The GeForce NOW app ships inside the main package and `lumatv apply` updates an installed copy by build id (`lumatv gfn install|remove|status`); the TV bundle builds with the Rust shell by default and installs it beside the compositor; the gate can repeat the app-close check (`--apps-repeat N`). (bd5a0c2 b65ef9a)
- Hardware planes: pages can switch between XR24, AR24 and AFBC while running (`ctl afbc`, `ctl pagealpha`), the shell claims a spare video window for the Cameras live view, and a picture-in-picture surface API is in; all off by default, with a TV-round checklist. (7853a04 02aeeae 3f9db0e 804a178)
- Settings (Rust shell): AI access (read and reduce only), Home Assistant and Cameras pages, which completes the port of every Settings page. (65ca46b 15ff0ce 4dc3556 0ed8d51)
- Cameras full view reads the compositor's video-window grant: granted-but-not-drawing shows stills instead of a black hole; a refused window leaves the stream on the video plane with a glyph beside the camera name. (7a09728)
- Gate: `tv_gate.py --rust-kinds` drives each Rust shell kind on the set (on, shown, first frame, frames, GL errors, off switch) and reports unbuilt kinds SKIPPED. (87364b4)
- Compositor: a direct frame may carry the OLED care's orbit as the primary plane's position (--scanout-offset, off by default) and `ctl pageprobe WxH SECONDS` shows a test picture of another size on the plane; display-layer research settles OSD0_1 as the 4K right-half reader and adds tools/layerdeep-tv.sh for the set-side checks. (76160d6 93f6e74 5c9dc80 a0284cd)
- Rust panels: the quick panel, input picker, notification drawer, context menu, confirm dialog, app switcher chrome (over the compositor's live pictures) and an on-screen keyboard for Settings' typed text are drawn by the Rust UI, per kind with `ctl shell rust +quick +picker +drawer +menu +confirm +switcher +entry`. (793549e..fca1e8f)
- System layer (Rust shell): capture mark, camera pop-up (stills) and overlay cards as kinds `capture`, `popup` and `cards`, a placed mute mark that drifts, and the pop-up layout editor (kind `layout`); the QML client leaves out what the Rust shell draws. (671bbb5 8f7c0cd 876346f 4e3a99c 958fa8d eee468e)
- Rust panels: the keyboard of apps' text fields (letters, e-mail, URL, symbols, number pads, enter keys, voice) is drawn by the Rust UI (`ctl shell rust +keyboard`); the Magic Remote pointer now reaches the Rust client; the quick panel's edit list keeps whole tile names at large text, the PIN pad shows its key glyphs, and a long pick card scrolls. (..1d1b8a8)
- The Rust UI draws the AI access prompt and the input banners (pill, channel bar, no-signal screen) as kinds `ai` and `banner`; the toast column follows its anchor and low-anchored cards stack upward. (0cddf37 a09ddfd 6478b3d 4f283ff fac28e1)
- The Rust Home draws the launch screen, Quick Access, the on-screen remote, the voice bar and the mini frame, asks the video-plane player for the moving wallpaper, opens Settings pages from Search, and opens the wallpaper popover from the gear. (03a160c bf68332 d8badbd 6489eb8)
- Notification drawer (Rust): the quick panel's width and place, notifications grouped by source with repeats folded into a count (OK opens a group, each group has its own Clear), and the panel grows out of the bell and back into it. (532a0bb)
- Display lab (Settings > Picture > Expert): a guided A/B check of the display chip's sharpener, hardware dimming and the cut-out window, driven with the remote; writes go only through the helper's hard list with read-back, a journal, a 6 s dead-man and restore on every way out. The wheel press (key 1199) is OK; the 1198 keep-alive is ignored. (3ff3b31 37fc2bb cf9f383 8380f3d)
- Display lab: Medium sharpness and a Medium/Strong comparison added; the dimming steps dropped (the firmware owns that register); a look the TV does not take says "Couldn't apply" instead of showing two identical pictures. (c306da0)
- Rust UI: grid-fitted text (FreeType-style autohint via skrifa, grey antialiasing) behind `text.gridFit`, off by default, with a Display lab "Text" step to compare on the TV; LumaShow's `ui` crate is vendored in rustui/lumashow-ui with a default-off glyph-source hook.
- Gear: holding the gear opens LumaTV's Settings (Remote & pointer › Hold gear: Settings, Notifications or Nothing); three taps no longer open LG's hidden support menu. Its items moved into Settings: About this TV, Auto power sync, No signal image, LG logo, AV reset (confirm), Auto Tuning, and Privacy › Allow LG Remote Service (off; launches blocked unless the owner allows them). (9f84f989, 27b9a68a, 8305a761, d9dc613a)
- Settings › Remote & pointer › Buttons: every remote button (app buttons, colours, channel, List, More) can open an app or input, go Home, open the quick panel, Settings or notifications, mute, turn the picture off, do nothing, or act as another button; LG Input Hook's list is imported once, so the app can be removed. (8c69fd98)
- Input picker: each tile shows what is on that input: a live picture for the input in front and the last picture with its age for the others (small JPEGs kept on the TV, nothing black or protected kept); Settings › Inputs & devices › Show input previews (on) turns it off and deletes them. (9358d70c, 2501ef63, a7ee35a5)
- Compositor log: a client that destroyed a surface is followed by one "close trace" line (its commits, frame callbacks, buffer releases and destroys, in ms from its first destroy). (7c7cb446)
- Weather view: a Radar page that plays NEXRAD frames for the last 2 h and HRRR forecast frames for the next 6 h (marked FORECAST) over a base map, and a Globe page with NASA rain and cloud. (7ae0391c, 4b2c1272)
- App icons: LumaTV, Card, Frame, GeForce NOW and Marquee each get their own colour and glyph instead of the shared dark tile; Frame gains an extraLargeIcon. (1ced7b41)
- Smart home on Home: the lights, switches, scenes, thermostats, covers and sensors Home Assistant shares with the TV appear as live tiles on a Smart home card, with OK to switch or run them and a sheet for brightness, colour temperature, speed, set point and position; locks, alarms, cameras and doors are never operated from the TV. (5afc7e41)
- Smart home locks: OK locks from anywhere; unlocking is only the person at the remote (a confirm, then a token only the shell holds); every other route refuses unlock/open, and every attempt is journalled. (c7683837, 9222ef6d)
- OLED care: the screensaver and an idle Home dim through the panel's own OLED pixel brightness (Settings › System › OLED care › Dim with the panel, on), journaled and restored on every way out, never during video or HDR; GPU dim remains the fallback. The compositor now exits cleanly on SIGTERM/SIGINT. (1355351a, a58d66f2, 425a8e71)
- Cameras: a Scrypted event pops up a card with the live camera in a hardware video window (on Home and over idle apps; stills at 8 a second over a playing video), one live camera at a time. Over a playing app the pop-up uses the second hardware video window (SUB0): the sub pipeline opens under the front app's id, because LG's AV connector forces any pipeline owned by com.webos.app.home onto MAIN. Pop-ups ask the bridge for the camera's low stream (<=1280x720@30, else <=1920x1088@20) and cameras without one fall back to the CPU texture; with Home in front the pop-up uses the CPU texture, so a 25 fps camera no longer re-times the panel to 100 Hz. `lumatv feeder install` sets up the feeder; test events via POST /v1/cameras/event. (b79dd141, df56b311, f5227463, 0f207d4c, 7cf6befc, bc641553, 58ea12ae, ca633c8d, 4307d7d3, ea61f3df, TV round 25 b992cb2f)
- Cameras: the full view is an app of its own: OK from a card or tile brings it forward with the camera's native stream on the main window, and Back returns to the app; over a playing app it opens on SUB0 and pauses the app's video (resumed on Back only if it was playing), the app keeping MAIN; previews pick their quality per path (live up to 1080p30 on Home, stills over a playing app). (92aad301, 0a26df49, dc03f982, 8b025405)
- Camera pictures play over any app: where no video window can carry one (an app's video, a 4K120 game, no window free) the pop-up and picture-in-picture are decoded by the feeder on the CPU and drawn by the compositor; the foreground list of LG's Multi View is an experiment switch (`ctl vwin fg`). (cbdcf424…32f699f4)
- Home: the weather card has a moving sky, says what is next and shows the coming hours; OK opens a weather view with Now (reading, hours, ten days), This week, a full-screen radar with floating controls and Details tiles with NWS alerts, fetched ahead in the background. (cd09bd9a, 219ac6e4, 740048bf)

### Changed

- Device gate: with Rust kinds on, the shell and keys checks read the Rust client's panels, keys and
  saver (`tv-rustjson.py --merge-rust`), and the Home check starts its D-pad walk on the status bar.
- Display chip: the mixer's hole 0 opens the video plane through the UI (hole register `0xCC48029C` value 3; `x` in
  1920-wide UI units, `y`, `w` and `h` in panel pixels), measured over a playing video with
  `tools/holetest-tv.sh`; the Display lab's window registers opened a quarter-size window at the right edge, and the
  notes carry the values for the centred one.
- Privacy: tracking, ad and uploader programs are stubbed and masked first in the boot hook;
  each menu row distinguishes this boot's result from an older pass.
- Volume: the mute mark stays in the top corners (hopping between four places against
  burn-in) instead of the lower middle, clear of subtitles and the status bar. The
  receiver's "Dolby Atmos - DD+" reads as "Dolby Atmos", and "Dolby Digital+" no longer
  reads as "Dolby Digital". Over Home and its saver the mark hops along the status bar's
  row instead, clear of the clock and weather cards. A mute that was on before a
  compositor restart shows its mark at once.
- `lumatv preload on` keeps only Netflix, the Home stub and the HDMI input apps
  on LG's preload list: LG loads every enabled entry whenever memory allows, so
  Live TV, LG Channels, the voice app, Miracast, YouTube, the browser and Prime
  Video (30-70 MB each) are switched off. The HDMI inputs stay ready so a switch
  to the PC is instant. The live policy is accepted only while the screen is on;
  in standby the helper's next screen-on applies it.
- Settings places initial focus when a page is entered, after service values
  have arrived, rather than selecting an unavailable page's last link during
  construction. The remote page describes the 650 ms Home escape.
- Receiver readings show only while the helper is up, the topic is fresh and
  its stamp is under six seconds old, through one gate for the volume display,
  the quick-panel tile and the receiver page. Polls that change nothing move
  the stamp without raising an event, so they do not wake Home's long poll.
- Settings rows write LG's own keys and values, rows LG marks inactive are
  dimmed, and deep links open the LG screen they name.
- The compositor takes direct scan-out while LumaTV's UI shows nothing over a
  full-screen window, the UI app's socket wakes the event loop, the frame latch
  learns the GPU's finish from the in-fence, and a covered card is minimized and
  returns on fullscreen.
- The helper takes the receiver's volume, mute, input and power over one held
  telnet connection, so changes reach the volume display as the receiver makes
  them; the HTTP status read is a fallback while telnet is down.
- The colour reader polls 10 times a second with an integer NEON reduce
  (output bit-identical, about 1.9 M instructions a poll down to 15 k).
- Compositor: foreground pushes leave the frame path, a frame is held while a
  driver buffer's first GPU job runs, texture frees are spread over flips, a
  system layer can sit on the overlay plane above a directly scanned-out
  window, the pacer follows the panel's measured refresh, each input device has
  its own `wl_seat`, and a keyboard made after focus is entered at once.
- The 64-bit card app binds every `wl_seat`, so the remote's keys reach it under
  LG's compositor, which sends a key on the pressing device's seat only.
- Boot: a set that boots into standby no longer gives LG's Home back, the
  preload policy waits for the screen, the compositor restart comes right after
  boot-bind so Home is up sooner, a set configured to start on its last input
  keeps it, and the logo windows keep a stamped record in `logo-events`.
- The compositor starts with `QT_QPA_UPDATE_IDLE_TIME=0`, removing Qt's fixed
  5 ms wait before each update; dropped animation frames fell from 8-9 % to
  3.4-4 % on a D-pad sweep and from 46-67 % to 2.5-4 % opening Quick Settings.
- Home's wallpaper motion and clock hands step on one shared tick (default
  10 a second) instead of separate timers, and the second hand steps without
  its overshoot animation, so idle Home no longer presents an uneven stream of
  frames with a periodic hitch.
- The project is renamed LumaTV. Every name (display name, app id, data dir,
  QML module directory, tool and boot-hook names) comes from `brand.json`;
  the build fills the `{{displayName}}`, `{{shortId}}`, `{{appId}}`, `{{dataDir}}`
  and `{{qmlModule}}` placeholders in the staged copy and names the ipk, the
  frame app and the installed files from it. The app id is now
  `org.nphil.lumatv`, the data dir `/var/lib/lumatv`, the tool `tools/lumatv`
  and the boot hook `50-lumatv`.
- The first verb run by the new tool on a set that has the LumaGlass layout
  moves `/var/lib/lumaglass` to `/var/lib/lumatv`, points the compositor env
  file, the early-boot hook, the init.d link, the configd key filter and the
  Quick Settings entry at it, and removes the apps `org.nphil.lumaglass` and
  `org.nphil.lumaglass.frame`. Revert and off work on a set that has only the
  old layout.
- `repo.json` lists no package until the first LumaTV release regenerates it.
  Its `brand.json` `repo` entry (`nphil/lumatv`) names the GitHub repository
  that release URLs point at.
- LumaHome is built when the compositor starts, not at the first visit to
  Home, so its heartbeat and a load error are visible at once. The Home
  surface's opacity is set to 0 or 1 outright: Qt 5.12's `Binding` left LG's
  Home at 0 when LumaHome stepped aside.
- Revert, the boot fallbacks and a failed apply remove the Home stub before
  the compositor restarts. `apply` leaves Home's assets bound under it.
- After a compositor restart with the Home stub mounted, the Home launch waits
  for the new WebAppMgr: at boot the old one closed 0.5 s after the launch,
  taking the stub with it, and the panel stayed black. A black panel on an
  Active set at boot is no longer taken for a boot into standby, so the launch
  is repeated. The guard gives LG's Home back for a missing module only after
  3 s, since a process read between fork and exec shows no module.
- `theme.json` is the one token source: colour roles for both materials in
  Lucent's names, type roles, radii, spacing, focus, state washes, motion and
  the font, in the shipped `payload/theme/theme.json`. Home merges the user's
  file over it key by key (the shipped files are staged as `theme.default.json`
  and `layout.default.json`, replacing the literal defaults in the QML), and
  `lumatv apply` cuts the user's files down to what differs from the shipped
  copy, so what was never changed follows an update. `radius` and `tileRadius`
  became `radii.card` and `radii.tile`; a material's `scrim` list became
  `wallScrim`, `scrim` now being the modal scrim colour.
- `tools/tokens.py` replaces `palette.py` and resolves the same tokens for the
  consumers that are not the compositor's QML: the web CSS (`--lu-*` as in
  Lucent's tokens.css, plus the Sandstone variables, generated per app), the
  settings-sheet palette Quick Settings is patched with (byte-identical to
  before) and `theme/Tokens.js`, which QML in other processes imports by file
  URL. `lumatv.css` carries no colour of its own.
- Manrope is the one typeface: four static faces (400, 500, 600, 700) cut from
  the pinned variable font by `generators/gen-fonts.py`, rendering identically
  to the faces they replace, loaded from `theme.font.files`. The web theme and
  Quick Settings use it (Quick Settings' tile titles are drawn at 0.9 of LG's
  size, Manrope being wider); Inter is gone from the package (1.7 MB).
- Home's key filter fails open and pairs presses with releases. A key is taken
  only while Home is in front, LumaHome's heartbeat is under 3 s old and its
  handler takes it; an error anywhere in the filter or the handler passes the
  key to LG untouched and logs one line per 5 s. A release goes where its press
  went, so a held OK that opens an app leaves no stray release for it. Unit
  tests: `bun test tests/`.
- OK acts on release. Held 700 ms (`motion.holdMs`) it picks a dock tile up;
  anywhere else a long press now acts like a short one. A held OK and a
  picked-up tile are cleared when Home is hidden or shown.
- Focus rest: the highlight follows each key press at once, while lift, scale,
  shadow depth and the name pill start only after the focus has rested 150 ms
  (`motion.restMs`) and drop at once when it leaves; the dim of unfocused cards
  and the dock's accent glow follow the highlight. A D-pad sweep draws one
  frame per key press: 150-157 frames and 1195-1271 ms of GPU in a 5.2 s
  sweep, against 220-224 frames and about 1770 ms.
- Gate: kwait names the loop thread and, when the flips are on time, the client's reaction behind x1 gaps; x1 A/B switches --x1-rt and --x1-callbacks; round 17's x1 failures read as the client's own wake-ups, not a compositor regression. (X1Regress 1d93bf0 ff44c89)
- Compositor: the Rust UI is found and started through the private loader as the compositor itself is, a kind switched on at runtime connects, the UI's helper requests carry the host port and client header, the QML volume bar steps aside when the Rust `osd` kind is on, and the Rust Home is no longer treated as covered when no app is in front. (ead2a6e 28f6d8b)
- Compositor: the Rust UI's shader programs are linked once on a thread of their own and cached, so enabling a Rust kind at run time no longer stalls the loop (the supervisor had fallen back on two missed beats); `ctl status` reports the longest loop pass (`stats.passUsMax`). (023cca6 b39557a db122f7)
- Round 18 (750bd99): Rust shell kinds run on the set (six start defects fixed during the round); not go-live: PlxNative still crashes on close in 12 of 15 passes.
- Rust shell: a focused row's plate keeps its rounded ends in Settings, the quick panel, the drawer and the Home library (list viewports clip to the window plus the focus lift's reach); Home's library clip no longer scrolls with the cells and shows two whole rows at 200 % text; the host lint fails a focused plate that any ancestor clip cuts. (c65415b)
- Gate: shell readings skip the Rust client's own `wants` (the quick-app check had read it since the Rust shell switch); the callbacks line counts commit-sent callbacks; `tv_gate.py --frames-clients`. (34118b6 514b059)
- LG's system-update block is laid by the early boot step, about 14 s in and before the network, whenever Block LG system updates is on, so a cold or instant boot no longer raises LG's "update now?" prompt; the Push promotions switch also stops LG's IoT client (`iot-client`) from being launched by the service hub. (a9af833 7cc1f01)
- Rust UI sharpness: dock icons are sampled without mipmaps, text layers sit on whole pixels (a lint catches any that don't), and a focused tile keeps its icon at the lifted size instead of enlarging a small copy. (99bf49e 82de04f 7bfb50f)
- Compositor: lginput2's Input Manager handshake (LG's /IM_MQ_WEBOS queue) is answered, so a Magic Remote that reconnects after sleeping works at once instead of after 4 s, its first press is no longer lost, and lginput2 leaks no descriptors; key 1199 is the remote's cursor-off, not a press; Settings > Remote & pointer > Show pointer: Wheel only or Like LG. (0486007 f7b4610 4163c65)
- Rust Home: top-row names sit above their tiles and every name pill stays inside the dock, including under a row's end tile; app launch and return use the tile transition with no black frame; Uninstall asks first; a held OK opens the tile's options without launching; Settings > Home & look > Card glass: Clear (the classic translucent look, default) or Frosted. (6955074..bb0aa19)
- Compositor: pages are AR24 with per-pixel alpha by default, so video shows through LumaTV's layer (with XR24 every video hole was opaque black); a toast or panel over video redraws only its own area and the compositor goes back to zero frames when it closes; the gate checks that video is visible (`tv_gate.py --video-apps`). (476c041f c5c4e22b 56527c6c)
- Home: the profile chip's avatar is a disc concentric in the pill's left cap (it was a rounded square that stuck out past the curve); the chip is sized to the name's measured width and the avatar letter is centred in the ink that reads on its colour. (4b3425c8)
- Home: the status bar's right-end buttons sit concentric with the pill's right cap. (96acfdcd)
- Home: covered by an app, the Rust Home stops its clock and news updates, so idle video composes no frames for it.
- Overlays over a playing video: Home no longer draws behind the app (its clock hand and news slide cost a composed frame a second, and its layers were redrawn under every overlay frame); overlay frames now cost 1.8–5.7 ms (worst 8 ms, was 26 ms) with 0 dropped animation frames. `ctl refresh` counts dropped animation frames (`motion`) and the video check judges overlays by them and by the frames the shell itself draws. (f2a0f166, e07c561e, e57ff71f, 320e1c9e)
- Settings: "Allow LG Remote Service" is a status row (Blocked/Allowed) whose allow step asks once, styled as danger. (4af74546)
- Toasts rest at the bottom centre, 54 px above the bottom edge, above the volume bar while it is up, and rise in from the foot; the input popup keeps the top right. (f95ba57c)
- Input picker: the focus plate is concentric with its tile, a reopened picker lights one tile, and the tiles are one row of equal height. (572ff6da, ca05a22e)
- Input picker: the sound line follows the audio service and names the receiver from HDMI-CEC ("Sound: Denon AVR-S760H (HDMI ARC)"). (ca004d2d)
- Toasts: with an app in front they rest just above the subtitle band (16 px above y 905). (d95ff0b2)
- Volume bar: bottom padding halved (32 px over Home, 92 px over an app). (0e886f5b)
- The helper runs under a supervisor that restarts it after any crash or signal (only `lumatv helper stop` and `revert` end it); `apply` always leaves it running and waits out a recovery of LG's Home instead of rolling back; the shell draws status glyphs and weather pictures from built-in copies, keeps the last profile on the chip and shows "Reconnecting…" on the weather card while the helper is away. (75fe94d0, c05150c9)
- OLED care: an idle Rust Home dims through the panel (the care now sees the Rust shell's Home), and panel-dim calls name the picture mode and input only. (ca3ef23e)
- Home: app icons are resampled once, with Lanczos, to the exact pixels they are drawn in, and the lifted tile sits on whole pixels, so the dock's icons are as sharp as LG's Home; Rust text is grid-fitted by default (`text.gridFit`), same face and weights. (0189b390)
- Gate: a set that was persistent ends persistent. (eb5c3a84)
- Settings: a row takes its focus plate only while focused, so long pages (Inputs & devices) draw every row on a full Home; `ctl shell show toast` raises a LumaTV toast for tests. (6b4add12, 2aebe853)
- Camera pop-up and notification cards sit in the top-right corner, 32 px from the edges (was 64 and 134); the pop-up can be moved in Settings › Home & look › Layout of pop-ups, and its live video follows it. (1e715ee6)
- Video windows: a window that is not full screen is asked for in the UI's own pixels, which videooutputd scales to the panel (it was asked for in panel pixels, twice as large and off-screen for a corner window); a video that stays put after two placements is left alone. Fixes the camera pop-up box, the full view's re-place storm and the art saver's video size. (21a560e3)
- Frames: Home's wallpaper stands still and its weather rests under Settings and panels, and Settings skips its fade at 100 Hz or more (wallpaper steps redrew the whole screen three times each, 13-16 ms of GPU a frame at 120 Hz); the frame gate reads the compositor's own thread and keeps each cycle's ring. (8642b179, 9e100ea4, 85ac5a6d)
- Idle Home no longer redraws most of the screen a few times a second (the engine ignores a restyle or pose that says what is already so); Home's wallpaper and weather rest while anything is over Home and for 2 s after a key; the frame gate judges each window by the refresh the panel ran at. (b421f486)
- Settings at 120 Hz: sheets hide Home's cards, dock and status bar once faded in, and a fast rail walk builds only the page it rests on (a Settings frame drew 150-190 layers; the refresh fits about 80); the frame gate counts drops from the compositor's own ring. (807b87e8, 5a4d3f65, 89507fcb)
- Engine keeps Home as one picture under panels and copies what a page still owes from the last drawn page, so a full-screen change is one frame of drawing (host, most layers per frame at 120 Hz: quick 112 -> 36, picker 98 -> 11, settings-scroll 175 -> 68); toast burst limit 300 ms. Off switches LUMATV_FLOOR=0, LUMATV_PAGE_COPY=0. (f14f87dd, 4b95e031, a988b80a)

### Fixed

- The recovery gate waits for the actual recovery lock, requires every requested result and checks the supervisor's 10-second Home hold as the stock LG escape.

- Picture-mode arrows and selection use LG's current context, native mode choices and confirmed readback; stale per-mode controls remain disabled during the switch.
- The screensaver can display over idle apps after LG approves its existing timeout, retaining native playback protections; wake input is kept out of the controls underneath.
- Silk lighting controls remain effective with wallpaper motion off, and repaint without requiring text layers.
- Interrupted maintenance blocks boot activation and configuration migration while preserving status reads and human emergency stop commands.
- Rust Home applies every wallpaper control live, including independent warp, lighting and colour grading.
  The graded original also supplies clear glass; held Home updates without restarting.
- Home search and Rust text entries use the custom Lucent keyboard, with address and number layouts,
  Unicode cursor editing, intact long values, fully masked passwords and TV-readable key contrast.
- The Rust compositor requests standby and wake from LG’s power service on a fresh Power press. Held repeats do not reverse an in-progress power change.

- /var no longer fills from the shell's own logs: the compositor gives the Rust and QML shell clients a pipe it copies into a log
  capped at 1 MiB (one `.old` generation), cuts lines at 4 KiB and shrinks a larger file an earlier build left to its tail; its own
  `compositor.log` is emptied past 8 MiB; a failing surface apply is logged once, not every frame. `shell-rust.log` had reached
  180 MiB of the 966 MiB partition, whose LG cleanup empties unprotected files at 98 %.
- `lumatv apply` refuses, before changing anything, unless the copies it really writes leave a tenth of /var (64 MiB at least) and
  of its inodes free; an apply that writes nothing new is not held to it. `sm.log` and `capture.log` are bounded while the
  supervisor and the colour reader run (256 KiB), not only when they start.
- Rust shell: Home gets no key, OK or pointer sample while any kind above it owns input (Settings, the
  apps' keyboard, the panels, the Cameras viewer, the layout editor, the alerts); before, D-pad and OK
  under an open Settings also moved Home's dock and launched its app. One order of owners
  (`Shell::system_wants`) decides it, and an OK release whose press Home never saw is no OK. (5e4fafd 3f8efad)
- Compositor: the Home key is told to the Rust shell whenever its Home is drawn, so a second press opens the
  app switcher (nothing is focused under the Rust Home, which the offer used to need). The switcher's card
  for YouTube is its icon and name like the other video-plane apps, not a black rectangle, and the mute
  mark keeps to the left corners while the quick panel, the picker or the drawer is up.
- Compositor: an app's frame callbacks go out with the commit that carried them (`ctl callbacks
  clients commit|early|landing`, default `commit`), so LG SDL's present never waits at window
  teardown. PlxNative closes cleanly 15 of 15 on the set (12 of 15 crashed with the callbacks at
  the draw); the focus plate of a focused row keeps its rounded ends in Rust Settings and the
  quick panel.
- Screensaver: a dimmer clock and no automatic camera video popup over an active saver.
- Gate: new PlxNative crash reports fail app-close checks even when SAM reports a clean exit;
  X1 traces distinguish display timing from the test client's scheduling delay.
- Compositor: YouTube and PlxNative run under LumaTV's compositor. Every seat now offers touch
  with the keyboard and pointer and sends no `wl_seat.name`, as LG's compositor does: Cobalt
  (YouTube) never initialises its touch proxy and destroyed it when a seat had no touch (SIGSEGV
  32 ms after it connected), and LG's libwayland-client calls a listener entry a client left
  empty instead of aborting. The `wl_starfish_pointer` and `wl_starfish_output` globals are
  always offered: LG's SDL (PlxNative, Apple TV, Prime Video) sends the cursor standby timer to
  the pointer global at its bind, when its window goes fullscreen and at every pointer enter,
  and PlxNative died in `SetMRCUTimer` without it (Apple TV and Prime Video set the timer hint
  to -1, which skips the call). `wl_webos_seat.info` names each seat by its input device as LG
  does, `wl_output` sends no `scale` (LG never does), and the gate launches YouTube and PlxNative
  (`gate.sh apps`: foreground within 5 s, alive at 10 s, closed cleanly).
- Compositor: PlxNative closes without crashing. A destroyed surface's frame callbacks (pending,
  committed, in flight) are destroyed with it and never answered, as LG's compositor does:
  answering them made PlxNative's libwayland-client send requests on freed proxies at close
  (SIGSEGV in 6 of 8 closes after YouTube). The cursor standby timer is kept per client. A
  `set_state` asking for the state a window already has gets no answer, as under LG's
  compositor: LG's browser, which NVIDIA's GeForce NOW opens, asked for fullscreen again 0.3 s
  after each answer and took the front back from Home.
- Compositor: an input device that goes away or a luna bridge closed at the other end is taken
  out of the loop instead of holding it at 100% of a core; the ring counts passes and what woke
  them, `ctl status` reports what clients left behind (`held`), and a bridge line split across
  reads is read whole.
- Gate: `gate.sh apps` also launches Home over the browser GeForce NOW opens, which must keep the
  front, and the Netflix check's fresh start reads SAM's list as `luna-send` prints it, so a
  running Netflix is closed first as intended.
- Shell at 120 Hz: the quick panel, input picker and the dock's row scroll step once per rendered
  frame (`kit/FrameMotion`). The basic render loop has no animation driver, so every QML
  animation stepped on Qt's 16 ms timer and slides ran at 60 steps a second at 120 Hz.
- Transitions: a late frame is a gap over 20 ms between two moving frames (a veil waiting for
  its video no longer counts), a veil whose card never gets a hole ends at its 1.5 s cap, layers
  below a video hole are no longer composited, buffer frees wait while a card moves, and each
  late frame is journaled with the loop's timings (`ctl transitions` `lateLog`).
- Video wallpaper: the placement is no longer refused by videooutputd (a plain exported window
  carries its original input, as LG's crop does); videooutputd's answers are journaled and
  `tools/vplane_wall.py probe --video` reads LG's VIDEO capture. The plane shows the clip about
  7% darker than the GPU wallpaper in Filmmaker mode (the plane's picture processing).
- Frame gate: surfaces for Settings, the Apps library, Home's rows, the app switcher, smooth
  transitions and both keyboards; `tv_gate.py` runs them all by default and the quick set with
  `--frames-quick`. The Settings, Apps library and launch surfaces run on the owner's Home
  layout; the apps check reads SAM's list and exit status through the log rotation; other apps'
  QML errors no longer fail a surface. Gate runs no longer write an explicit wallpaper-motion
  false back as true.
- Saver vote: a refused tvpower subscription is cancelled instead of leaking (the compositor held
  26 to 141 of them), the shell's luna calls and cancels reach the compositor in order, and the
  gate reports the vote when the saver does not start.
- OLED care: an app is still when its picture is. Apps that redraw the same frame every few
  seconds (LG's browser, PlxNative) now engage box dim and app dim/shift; the box dim's sample
  no longer stalls the loop.
- Panel refresh guard: `getPanelUsageTime` is asked with `{"subscribe":false}`, the only form
  the TV accepts.
- Volume bar: a held key moves the shown level at a steady, gently rising pace and the helper
  sets the Denon to that level, so the display and receiver agree during and after a hold.
- GeForce NOW app: Play no longer fails with "different account context" (the launch carries the
  account's scope), the header shows the live membership tier, and Settings has a Sign out row.
- Compositor: direct scan-out of a sole full-screen client buffer behind `--scanout direct` /
  `ctl scanout` (off by default), falling back to composition in the same frame.
- Compositor: the shell may call the Homebrew Channel's `getConfiguration` and `setConfiguration`
  (and nothing else of that service, which also runs commands as root), so Settings > System >
  Support > Update can read and change "Block LG system updates" instead of showing "Homebrew
  Channel: not allowed by the compositor" (not compiled or installed yet).
- Perf gate: a `voice-focus` surface walks focus in LG's Search app; `gpu-ui-impact.py` reads
  the set's one-line Mali counters; the Round 19 slot sends no key when Live TV does not come
  to the front (OK on Home's focused tile had opened GeForce NOW).
- Compositor: landed flips are handled first in each loop pass, so a frame callback no longer
  waits behind a draw; a window made or named before its first buffer draws no frame; the
  first commit with the mode is non-blocking; a refresh skipped with nothing due is no longer
  counted as missed; the watchdog's fallback reason comes from the child's exit status.
- Video-plane saver: the player's window is double-buffered and repaints a buffer only after
  the compositor releases it, so no half-painted frame (a veil without its caption) can show.
- Home: a read of `theme.json` that landed while Home was writing it reset the theme on
  screen and dropped earlier choices from the next save; such a read is now ignored.
- Settings: Just Scan is read and written per input, as LG's Settings does, and App
  Permissions asks LG's permission service the way LG does.
- GeForce NOW app: started by SAM with an empty environment, it now finds the compositor,
  its core and engine, and logs to `/tmp/lumatv-gfn/feeder.log`; the core is told the
  engine's path and the settings one at a time, so the streamer is found; its data goes to
  a directory the TV's jail lets it write.
- Netflix failed with tvq-pb-101 after a compositor restart taken while an HDMI input was in
  front: the input's audio decoder port stayed open. `lumatv` now ends the input apps through
  SAM before it restarts the compositor, and `lumatv inputs release` frees a port left open.
- Volume bar: it measures its words as drawn, so the audio format is no longer cut off.
- Video-plane saver: the player's role carries every name LG's media library registers, and
  the supervisor hands it the compositor's runtime directory.
- Hardware JPEG: PmLog is loaded before LG's photo library; the helper's picture route
  decodes on the block (4K to 1080p in 33 ms).
- Luna calls from the compositor QML work on the set: `LS` lives in `WebOSCompositorBase`, not
  `WebOSServices`, and eleven files that imported only the latter failed with "LS is not defined"
  (the Settings launch, the notification log, the app list). The offline host now exports `LS`
  from `WebOSCompositorBase` only, and a test fails when a file uses `LS` without the import.
- `tests/qmlhost/compile_all.py` compiles every QML file of the surface with the set's import
  layout; `tools/build-ipk.sh` stops on any syntax, type or import error, so a surface that
  cannot build never reaches the TV.
- A restart from standby no longer hands LG's Home back. The Home stub guard treats a missing
  heartbeat with the panel off as not yet and checks again from the first screen-on; a stopped
  beat on a set that is on is still a fault. The boot apply no longer waits 140 s on the preload
  manager (it holds `setPreloadPolicy` while the screen is off; the helper sends the policy at
  the first screen-on) nor 30 s for pixels on a panel that stays off.
- The ipk now ships the tools `systemui on` and `webglass on` call:
  `tokens.py`, `systemui.py` and `systemui_patch.py`.
- The "auto" light or dark glass choice read a canvas that stayed empty on
  this firmware, so it always chose dark. It now reads the wallpaper.
- A plugin could be cut off when the helper stopped without its Shutdown message: the connection
  ended on the stopping flag before Shutdown was queued, and was closed before the queue was
  written. The outbox byte count could also wrap when the writer took a frame before it was counted.
- Camera pop-ups: a replacing pop-up card no longer drops its new window (cameras.live carries the card's purpose); pop-ups are on again. (ea61f3df, TV round 25 b992cb2f)
- Compositor: apps get their frame callbacks when their frame is drawn, as under LG's compositor; PlxNative no longer crashes when closed while animating. (AppCompat 112d9cc)
- Rust UI: opens a new helper connection after each answer (the helper closes them), so AI access, the screensaver rules, the receiver and the quick panel no longer read "not answering"; dock icons skip empty icon fields; "At home" shows the Home Assistant state from the right topic; profile names clear their avatars; the Settings bar shows only the way out. (9b6f72e 87f212c d2427c3 6bff4a6)
- Tests: load-sensitive compositor tests wait on the journal or the server thread instead of fixed windows; FrameMotion.qml imports its own module (Lu was undefined on the set); stale expectations follow the current panel, saver and motion values. (..ff4d977)
- Compositor: an app's frame callbacks go out with the commit that carried them (`ctl callbacks clients commit|early|landing`, default commit), so LG SDL's present never waits at window teardown; PlxNative closes cleanly 15 of 15 on the set (12 of 15 crashed with callbacks at the draw). (f2e4b5d)
- Display lab: the window step uses the measured hole (centred over the video, one setting instead of two). (bb7a491b)
- Settings: every switch says what ON means in plain words and flips in place with a toast (Privacy: Block …, Allow …, Keep …); Prime Video ads is a status with one plain question and the helper retires the old AmazOff app itself; "Held settings" is now "LG privacy settings kept off" and explains itself. (a8086db7, 40055f68, 30a8c7a9)
- Quick panel (QML): the focused row's plate is no longer cut by the list's edge. (30bdedca)
- Camera video no longer leaves the panel at 100 Hz (main-window decoder runs as a direct video; the frame-rate watcher returns with free run and also runs when a camera stream ends); Back from the Cameras full view raises the app instead of relaunching it, so YouTube resumes. (888dc431)

### Added

- The component kit, `payload/compositor/lumatv/kit/`: the token layer `Lu`
  (a QML singleton fed by LumaHome), `LuText`, `LuRow`, `LuToggle`,
  `LuSlider`, `LuButton`, `LuSegmented`, `LuChip`, `LuHalo` (the dock's accent
  glow, now shared) and `FocusLift`. Every component follows the focus rule
  and Nitin's cue: no line under the focused item. `theme set
  '{"kitGallery":true}'` shows them all on one sheet (Back closes it).
- Tests: `bun test tests/` (key filter, saved-first cache, tint, kit) and
  `python3 -m unittest discover -s tests -p 'test_*.py'` (tokens, theme file).
- Home's weather and news cards draw what the native helper serves (`kit/Helper.qml`:
  one state read, then a long poll, so the cards fetch nothing themselves) and, until
  the helper has answered, the document it saved last time in
  `/var/lib/lumatv/cache/weather.json` and `news.json`. A reading is never drawn after
  6 hours (or when stamped over 5 minutes in the future) and an older reading never
  replaces a newer one. The ZIP, units and news edition are the helper's settings
  (`base.weather`, `base.news`) instead of keys in `theme.json`.
- `lumatv helper start|stop|status`: the native helper is staged under
  `/var/lib/lumatv/bin` (an update cannot overwrite a running executable) and started
  by every apply and boot, stopped by revert. The ipk carries `helper/dist/lumatv-helper`
  and the build refuses to run without it.
- `lumatv preload on|off|status` keeps only Netflix and Prime Video warm: YouTube, LG
  Channels and the browser are dropped from the preload list in configd and in the running
  preload manager, LG's list is saved once and restored by `off` and `revert`, and the
  helper sends the policy again at every switch-on, which webOS 10 needs. Measured on the
  set: no change in free memory or Home-return gaps, as it closes nothing that is running
  (it changes what a cold start preloads), so it ships off.
- `lumatv logo on|off|status` replaces LG's power-on and power-off logos with LumaTV's:
  windows for `BootLogo` and `MasterBrand` that load one of three animations each
  (`theme.json` `logo.on`, `logo.off`, default 1; stills in `docs/mocks/logo`). The
  power-off window answers `tvpowerd` once, whatever happens, so a missing or broken logo
  never holds a power-off.
- The startup warm-up draws every text style, image and shader component the surfaces use
  once, hidden, after Home has presented, so a first open compiles and rasterises nothing;
  `theme.json` `warmup` switches it off.
- `docs/research/perf/gate.py` is the per-surface frame gate (p90 render under 3 ms over all
  the surface's animated frames, no frame over 20 ms inside a running animation, first frame
  within 50 ms, open by 180 ms, RSS flat over the cycles, no QML error in the log) and
  `startup.py` and `preload_ab.py` are the A/B drivers; the gate holds LG's compositor to its
  measured floor of 17 % late frames and LumaTV's own to none.
- `tests/qmlhost` loads a compositor surface on the workstation (PyQt5 on Xvfb) with stand-ins
  for LG's modules and a fake helper: load errors, screenshots, scripted window life cycles.
- `lumatv compositor on|off|status` and the boot worker's `compositor-boot` put LumaTV's own
  compositor over LG's `surface-manager` when the bundle is installed and enabled, check that
  LG's binary is the one the bundle was installed against, and fall back to LG's compositor
  by revert, by `off` or by the supervisor. While it is live, the Home stub's guard does not
  read LumaHome's heartbeat, apply does not restart the compositor, and nothing restarts it
  while a Home recovery holds `/tmp/lumatv-recover.lock`.
- `lumatv notifications on|off|status` points the compositor's notification view
  (toasts, alerts, input popups, the PIN prompt) at a copy of LG's own tree that is
  made and patched on the set: Manrope, the toast in the theme's colours, and the
  live shadow layer of each toast, alert and button turned off. LG's data layer and
  item contract are untouched, so a notification of any kind arrives as before; a
  stage or a compositor that refuses the file leaves LG's view in place, and revert
  puts LG's view back.
- `lumatv surfaces` lists every switch (Home, Quick Settings, the settings theme,
  notifications, logos, helper, preload, compositor) with whether it is on, read
  from the flag or state its own verb uses, and the way back to LG's version.
- The gate has entries for LG's own toast and alert (`gate.py --surfaces
  toast,alert`), opened by a luna call, as the baseline the rebuilt ones must beat.
- The notification view's toast is LumaTV's own (`payload/notification/Toast.qml`, laid over LG's
  tree when `notifications on` stages it): a glass pill at the top right under the status bar with
  an icon tile, a title over a dimmer subtitle, and an accent link, where LG draws a grey bar at the
  bottom. The alert is LG's, centred over the palette's scrim and in the palette's colours, its buttons
  a lighter glass with a light edge when focused; LG's red focus colour is the accent.
- LumaTV's Settings (`Settings.qml`, `settings/`) is a layer of Home, opened from the gear's popover
  ("All settings"), with Home's widgets and bar stepping aside while it is up. A rail of sections, each
  with a line on its current value, and a page: Home & look (dynamic tint, appearance, clock, wallpaper
  motion: the theme's own knobs, written to the user's `theme.json`) and AI access (listener, pairing,
  assistants, what each may do, recent activity, lock-out). The last rail row opens LG's Settings app.
- The AI prompt (`AiPrompt.qml`) draws the helper's `ai.prompt` over Home and keeps the remote's
  direction, OK and Back keys from everything behind it. It has no button and calls nothing: the helper
  reads the answer from the physical remote's input device itself. The AI page only ever asks; whatever
  gives an assistant more waits for that answer, and giving less (off, pause, remove, a lower level,
  lock-out) is done at once.
- `kit/Helper.qml` follows the `ai` and `ai.prompt` topics, sends DELETE with the same headers as POST
  and returns the document of 202 and 409 answers.
- The offline host's fake helper models the `/v1/ai` routes (a prompt opens for what gives more and is
  answered only through the test's own door), and the host parks the pointer in the corner, so a
  hover cannot move a test's focus.
- `lumatv revert` and `apply` no longer restart SAM or `homelaunchpoints` when they put back the icon
  edits of an older version: the files are restored and the launcher reads them at its next start.
  SAM is never restarted; a test fails if any shell tool in `tools/` restarts, stops or kills it.
- Wallpaper tint: LumaHome samples the wallpaper at 32x18, cuts each channel
  into 8 levels, skips near-black pixels and takes the most common bin; a
  newer wallpaper cancels a sample still running. The result is kept in
  `theme/tint.json`. A dock tile with no plate colour glows in the tint's
  accent (`accent: "wallpaper"` does so for every tile and for the web menus
  that `tokens.py` themes), and `tint.mix` moves the focused glass toward it.
- `lumatv homestub on|off|confirm|status` retires LG's Flutter Home behind a
  web stub bound over its directory (`payload/homestub`), so LumaHome is the
  only Home. The stub is one WebAppMgr renderer at 32 MB resident against the
  Flutter process's 72-80 MB. SAM is made to read it with `blockSystemApp` and
  `releaseBlockSystemApp`, never restarted, and the block list is
  byte-identical afterwards; LG's process is stopped; Home's assets bind stays
  under the stub for the way back. `status` reports it.
- `homestub on` starts a 120 s trial that reverts itself unless `confirm`
  or OK on a toast in LumaHome arrives. Only a confirmed switch is kept in
  `config.json` and mounted at boot, by the boot worker's first act, ahead of
  anything that could start LG's Home; a boot after one that never finished
  skips it.
- A guard process (`homestub-watch`, 0.05 % of a core) gives LG's Home back
  about 6 s after LumaHome's once-a-second heartbeat stops, at once when the
  compositor restarts without the LumaTV module or LumaHome fails to load, and
  when Home is held for 650 ms anywhere, until the next apply or reboot. Two
  faults in a row switch the stub off. The 650 ms hold also cancels LG's own
  2.5 s Home hold (Screen Zoom). The heartbeat, mode, escape and confirm files
  go through LG's synchronous `TextFile` type: `XMLHttpRequest` PUTs twice
  reached the file 5 s late with the timer on time.
- `lumatv systemui on|off|status` runs Quick Settings from a copy of LG's
  QML tree under `/var/lib/lumatv/systemui`, by pointing the panel's entry
  in the configd object `com.webos.surfacemanager.systemUIManager` at it.
  Nothing is written to `/usr`, and `off` restores the stock path in one
  configd write. The copy is rebuilt whenever the stock tree's fingerprint
  moves, so a firmware update cannot leave old QML running against new
  libraries.
- `payload/systemui/QuickSettings/Component/CommonBoxShadow.qml` replaces LG's
  drop shadow, which rendered every shadowed item into an offscreen texture and
  blurred it in two passes on each repaint, with kernels the call sites size up
  to 51 samples. The replacement evaluates the shadow analytically from the
  signed distance to a rounded box: one pass, no texture read, and no repaint
  when the item it sits behind animates.
- The settings menus wear the LumaTV material. Quick Settings is a floating
  sheet, inset from the screen edges and rounded on every corner, with glass
  tiles and header buttons; focus lifts a surface rather than inverting it to a
  light fill. `tools/systemui_patch.py` applies this to the staged copy, and
  fails loudly on a missing anchor so a firmware update cannot half-patch it.
- `lumatv webglass on|off|status` gives the same treatment to the two web
  settings surfaces, Settings and Notification Centre. WebAppMgr injects
  `webOSUserScripts/userScript.js` from an app's own directory, so each app
  gets an overlay mount whose upper layer holds just that script, the
  stylesheet and the font - no copy of a 23MB bundle, and unmounting restores
  the stock app exactly.
- `tools/tokens.py` derives the menu palette from `theme.json`, so the menus are
  tinted by the wallpaper and theme in use. A contrast guard darkens or
  lightens the derived sheet until its body text clears 7:1 against it, which
  is what keeps a pale theme from producing an unreadable menu.
- Quick Settings stays built between opens, the way the volume OSD does.
  Stock rebuilds the whole panel from 165 QML files on every press: 420ms from
  the key to the open transition, which then waits another 180ms and runs
  stretched under the build, fully open at 860ms. A close the panel makes
  itself (back, the settings key again) now stands it down to the view
  container's `background` state instead of exiting, which keeps the
  lifecycle at launch, so the next press relaunches the same instance: open
  transition 10ms after the launch, fully open at 170ms. A close from outside
  (Home, power, another app) still exits as stock, because the manager
  refuses to relaunch an app whose last event was a close; the first press
  after one is a normal cold open. Compositor RSS is flat across 40 relaunches.
- `/var/lib/lumatv/env.extra` passes extra variables to the compositor,
  one `KEY=VALUE` per line, for tuning and measurement. Only
  `QT_QPA_UPDATE_IDLE_TIME`, `QML_FORCE_DISK_CACHE`, `QSG_*` and `QV4_*` keys
  with plain values are copied into its env file; anything else is ignored.
  `revert` and the boot fail-safe delete the file, so an experiment does not
  outlive a compositor that failed to start with it.

### Changed

- LumaHome is built once and kept. It was destroyed whenever an app came to
  the front and rebuilt on every return to Home, which froze the panel for one
  frame of 187-320 ms and then 3-6 more of 43-116 ms; a return now shows at
  most two frames of 57-87 ms. The kept copy holds about 52 MB of compositor
  memory while an app is in front.
- While Home is hidden its clock, theme check, news and weather stop; showing
  it again ticks at once and refreshes feeds older than their interval.
- `apply` relaunches Home when the restart leaves only a background (preload)
  instance of it, which had left the panel black after a wake.
- The key filter is written to configd only when the chain would change.
  Each write makes the compositor reload every filter and rerun LG's init(),
  which in Game mode opens the Game Dashboard.
- The frame probe reports pacing rather than a rate: worst interval and missed
  60Hz deadlines per second, plus one line per gap over 20 ms. A single dropped
  frame is invisible in a per-second average.
- The compositor keeps its compiled QML between starts: `QML_FORCE_DISK_CACHE=1`
  in its env file overrides LG's `QML_DISABLE_DISK_CACHE=1`, which cut
  restart-to-Home from 5.8 s to 4.6-5.2 s. The cache
  (`/home/root/.cache/surface-manager/qmlcache`) is cleared whenever the
  staged compositor set changes, and at revert.
- `apply` restarts the compositor when its env file changes, not only when
  its QML does; the file is read only at start.

## [0.4.0] - 2026-09-15

### Changed

- The Home screen is drawn by the compositor. `payload/compositor/lumaglass`
  (LumaHome.qml, status bar, widget cards, dock) renders the wallpaper,
  glass chrome, clock, Home Assistant, news and weather widgets and the app
  dock on top of the stock Home, which stays resident underneath as an
  invisible placeholder. Cards, tiles and the status bar are single shader
  quads that sample one cached blur of the wallpaper, so nothing runs per
  frame while idle and focus and layer changes animate at 60 fps.
- Navigation has two layers: the dock (default) and the widgets. Up from
  the top dock row or Back moves to the cards; Up from a top card reaches
  the status bar; Down from a bottom card returns to the dock. Back from
  the widget layer is left to Home. The Magic Remote pointer focuses by
  hover.
- Remote keys reach the layer through `payload/keyfilter/lumaglass.js`,
  registered by the tool in configd's `com.webos.surfacemanager.keyFilters`
  (picked up live, no restart). A shadowed `controllers/StarfishKeyFilter
  .qml` is never loaded on this firmware: module types resolve to
  `/usr/lib/qt5/qml` whatever `QML2_IMPORT_PATH` says, only relative-file
  references from an already-shadowed view follow the shadow.
- Theme and layout live in `/var/lib/lumaglass/theme/{theme.json,
  layout.json}` and reload live within 2 s of a change. Layout is a column
  grid with explicit row heights (the reference pins 222/238/392 px);
  every element, the dock included, is a widget entry with col/row/span/
  rows. Widget types resolve to a built-in, a QML file in the theme
  directory, or an installed pack under `/var/lib/lumaglass/widgets/
  <packId>/pack.json`. Theme files the user has edited are never
  overwritten by an update; untouched ones follow the payload.
- Tile plates are derived in memory from each icon's edges (full-canvas
  artwork gets a plate of its own edge colour drawn edge to edge, floating
  artwork sits inset on a plate in its dominant hue); `theme.tiles.plates`
  and `theme.tiles.icons` override per app id. The per-app appinfo/icon
  rewrite and its `sam` restarts are gone; `apply` restores any files it
  had touched.
- Dock tiles can be rearranged: hold OK (or press-and-hold with the pointer)
  on a tile to pick it up, Left/Right/Up/Down swap it with the neighbour
  (which slides across), OK drops it and Back puts it back. The order is
  written through `homelaunchpoints/moveLaunchPoint`, the list stock Home
  reads, so it persists and both homes agree. LG's Apps tile is unmovable.
- The profile button shows the LG account (nickname, initial and avatar
  colour from `accountmanager/getLoginUserData`) and opens a popover with
  the account and a "Manage LG account" row that launches the LG Account
  app. Popovers take Up/Down/OK, and Back closes them.
- The wallpaper is alive: the still image is drawn through a slow domain
  warp (two sine fields, a few pixels of drift, the warp also lighting the
  waves for depth), evaluated per vertex on a 48x27 mesh so the fragment is
  one texture read, ticked at 30 fps by a timer rather than the animation
  clock. `theme.wallpaperMotion` sets amplitude, speed, depth, fps or turns
  it off; `motion.reduced` disables it. The glass blur stays baked from the
  still image so nothing above changes.
- Wallpaper motion is adjustable in two places: a Wallpaper panel in the
  companion app (`lumaglass theme get|set` behind it) and a popover under
  the gear on Home, where Left/Right change amount, speed, depth, wave size,
  detail and frame rate with the widgets and dock faded out so the wallpaper
  is seen bare; Back saves to theme.json. Other popovers dim the page behind
  them by 30% (22% on the light material).
- Wallpaper look toggles, all free per frame: vignette and specular
  (per-vertex, with the wave lighting), bloom (a screen blend of the
  already-baked blur, fetched per vertex), saturation and contrast (baked
  once into a graded copy of the wallpaper that the live pass, the still
  path and the glass blur all read). Measured 58-60 fps with everything on.
- Wallpaper grain is 0.4% (was 1%): with the TPDF dither that still breaks
  banding on the dark gradients without reading as texture.
- Mini apps. OK on a widget card hands it the keys (the edge brightens);
  in the news card Left/Right step through stories (the slideshow holds for
  30 s after a manual step) and OK opens the article: a glass frame grows
  out of the card to a centred 1520x860 window, shows a spinner, and the
  article renders inside it. Back shrinks the frame back into the card.
  Web content is rendered by a second, overlay-type web app
  (`org.nphil.lumaglass.frame`, transparent, composited above Home) so
  Chromium's work never touches the compositor's frame budget; it embeds
  the page in an iframe where the site allows framing and otherwise builds
  a reader view (headline, hero image, body) from the fetched HTML, since
  news sites send `x-frame-options: deny`. Up/Down scroll. The frame app
  ships as an ipk inside the payload and `apply` installs it through
  appInstallService when its version changes. Any HTML5 app URL can be
  opened the same way.
- The news card cross-fades between stories (hero images mixed in the
  card's own shader, text dipping out and in) instead of cutting.
- Frame budget: a sustained animation with the full layout and the live
  wallpaper measures 60 fps. Card and tile quads shade only the shadow term
  outside their rect, use tighter margins and mediump colour math, the
  scrim is folded into the wallpaper pass, and the covered stock Home
  surface is skipped at opacity 0.
- The clock has an analog style (face drawn once, hands as rotation
  transforms, ticking second hand, weekday/date/week strip beside it) next
  to the digital one; `theme.clock.style` selects it.
- Tile focus is depth only (lift, scale, deeper shadow, rim, sheen); `theme.focus.tileGlow` and `theme.focus.ambient` bring the accent halo and dock wash back.
- Side safe margin is 64 px (was 96); the dock centres its tile grid and
  shows the bottom fade only when it scrolls.

### Fixed

- Back closes the notifications popover and, from the dock, moves to the
  widgets. The key filter is registered first in configd's list: the stock
  `handleSystemKeys` answers Back with NotAccepted while Home is up, which
  ends the chain before a later entry.
- `apply` no longer restarts the compositor on every run: the staged Home
  QML tree is hashed by paths relative to the tree, so `luma.next` and
  `luma` compare equal when their contents do.
- `apply` restarts the compositor when the QML, wallpaper or key-filter
  script changed. It only restarted a compositor that lacked the shadow
  module, so a set with the mod already live kept running the previous
  QML until the next boot.

## [0.3.6] - 2026-09-14

### Changed

- The cause given in 0.3.5 for the `webos-sddp` crash is corrected. It does
  not crash "on the first wake on this firmware"; it segfaults in
  `SDDPSetDevice` whenever it reads the interface list and finds a
  point-to-point interface with the multicast flag - the Tailscale `tun` on
  the reference set. With `tailscaled` stopped it starts and stays up; with
  the tunnel present it dies within 3s of starting. It only looked like a
  wake crash because at boot it starts before Tailscale's hook creates the
  tunnel and re-reads interfaces on the first wake. A stock set has no such
  interface. The toggle's behaviour is unchanged; README, the handoff and
  the code comments now say why.

## [0.3.5] - 2026-09-14

### Fixed

- The set now stays in active standby after power-off, so a later power-on
  is instant with the mod already on screen. It never did before: every
  power-off was a hidden reboot, because the crash report the compositor
  restart provokes (inputcommon, `QFontDatabase::removeAllApplicationFonts`)
  was never actually removed. `disarm_restart_crashes` read the names back
  from `ls`, which word-split them on spaces and printed the 0x02 bytes rdxd
  uses in place of `/` as `?`; `rm -f` on those names was a no-op that still
  counted as a success, so "disarmed 9" meant nothing. faultmanager then
  answered the suspend request with `rebootToSuspend`, and tvpowerd rebooted
  the set ~6s after it had entered active standby. The sweep now uses
  pathname expansion, which returns the raw bytes, and runs a second time
  once Home is back on the panel, since the crash lands ~3s after the
  restart and can miss the first pass.

  Measured: three power-off / wake cycles of 150s each with no reboot,
  uptime carrying across, and the same compositor still running.

### Added

- An "Instant on" toggle in the app, `standby warm|cold` in the CLI. Warm
  turns Quick Start+ on and `enableSDDP` off, remembering the previous SDDP
  value; cold turns Quick Start+ off and restores it. The tradeoff is the
  user's: warm means an instant power-on with the mod already up, at the cost
  of higher standby draw and no Control4 discovery; cold is the lowest
  standby draw with a ~35s boot at every power-on. `enableSDDP` matters
  because `webos-sddp` crashes on the first wake from active standby on this
  firmware, and its crash report arms the same reboot at the next power-off.
  `status` reports the live mode as `standby`: `warm`, `cold`, or `system`
  (Quick Start+ on with SDDP still enabled - the stock state, which is not
  instant-on in practice).

## [0.3.4] - 2026-09-14

### Changed

- The mod now reaches the screen at boot+33s instead of boot+55-60s, and the
  boot no longer shows stock Home before switching. Two changes get there.

  The apply used to run from Homebrew Channel's `init.d` run-parts at ~33s,
  by which time Home has long since painted. It now triggers at ~12-14s from
  a PATH shim in front of `grep` for `kdump.service`, which starts at 14.2s,
  reads an `EnvironmentFile` under `/var`, and on a set without `crashkernel`
  on the kernel command line is a no-op that exits 0. The shim always execs
  the real grep, so it cannot stop that unit, and run-parts stays as the net
  in case the early apply fails.

  The restart path also slept 14s before asking for Home and 8s after.
  Measured, asking for Home immediately still put pixels on the panel at
  +12s, so both sleeps are gone. Readiness is now taken from the panel: a
  captured frame is 835 bytes while it is still blank and over 79000 once
  Home is composited, whereas the Home process, `NL_HOME_SHOWN` and
  `getForegroundAppInfo` all go true about 8s before anything is drawn.

  Restarting the compositor that early also provokes none of the stock-app
  segfaults a later restart does, so there are no crash reports to disarm.

### Fixed

- The cause of the compositor's first start ignoring the override, withdrawn
  as unexplained in 0.3.3, is that `/var` does not show the writable ext4
  subtree until `mount-readwrite.service` relocates it at 12.96s.
  `surface-manager-daemon` starts at 2.63s, so `/var/systemd/system/env` is
  not a path that exists yet and the optional read is a silent no-op - the
  file's atime is untouched across a boot, and LG's own env files for
  `ls-hubd`, `configd`, `memchute`, `tvpowerd`, `bootd` and `sam` are equally
  inert that early. One compositor restart per boot is therefore structural,
  and 13s is the floor for anything this mod can do.

  Baking the mod into the hibernation image the set resumes from
  (`snapshot resume=/dev/mmcblk0p54`) was tried and does not work: the image
  is captured at ~15s, before the 13s floor plus a compositor restart can
  complete, so it always captures the stock compositor.

- The late hook no longer applies a second time on top of the early one. It
  stands down while an early worker is running or has succeeded, and only
  takes over if that worker died with the boot still incomplete.

## [0.3.3] - 2026-09-14

### Fixed

- A boot could leave the modded Home layout with the stock compositor: no
  wallpaper, no clock, weather or news. `apply` only restarted the compositor
  when the panel already reported Active, and on a boot where the panel had not
  finished waking it skipped the restart and nothing retried, so the set ran the
  whole session on stock QML. The restart now happens whenever the running
  compositor lacks the shadow module, whatever the panel is doing; a restart
  while the panel is dark is invisible, never restarting is not

### Changed

- The claim that the compositor picks the override up on its first start is
  withdrawn. Measured across consecutive boots it takes the global
  `DefaultEnvironment` from `/etc/systemd/system.conf.d/30-webos-global.conf`
  instead of the unit's `EnvironmentFile`, although the same file applies to
  every later restart, and `var.mount` completes at 1.946s while
  `surface-manager-daemon` does not start until 2.591s - so the file was
  readable. Cause not established, and nothing depends on it any more. One
  restart per boot remains the norm; what keeps power-off immediate is clearing
  the crash reports that restart produces, not avoiding the restart

## [0.3.2] - 2026-09-14

### Fixed

- A power cut while the shadow compositor module was being written left the TV
  with a black screen on the next boot. `/var` is ext4 `data=ordered`, so the
  directory entries were committed and the file contents were not: the module
  had all 318 files present holding 212KB instead of 2030741 bytes. The
  compositor started on it, resolved nothing and presented nothing, and because
  it was running and healthy no liveness check noticed. `build_shadow` now
  syncs the contents to disk before the rename publishes them
- Shadow integrity is checked against a fingerprint of the whole tree, not just
  the one file we inject. The truncated files were never looked at, so the
  damaged module was reported as current
- `boot-bind` restarts the compositor after repairing a module the compositor
  had already loaded. Rebuilding on disk does nothing for a process that read
  the bad copy at startup

## [0.3.1] - 2026-09-14

### Fixed

- The firmware-staleness guard never fired. `stock_module_fp` used
  `find -printf`, which busybox 1.35 does not implement, and the failure was
  silent: the pipeline hashed empty input, so every fingerprint compared equal
  to every other and a replaced compositor module would have gone unnoticed.
  It now fingerprints the recursive listing

## [0.3.0] - 2026-09-14

### Changed

- The compositor no longer gets its QML from a bind mount over `/usr`. A full
  shadow copy of the `WebOSCompositor` module lives in `/var/lib/lumaglass/qml`
  with the modded file swapped in, and `QML2_IMPORT_PATH` points at it through
  `/var/systemd/system/env/surface-manager.env`, an optional environment file
  the compositor's own unit already reads. The compositor therefore starts on
  the modded QML at boot+2s. Only Home's assets are still bound

### Fixed

- Cold boot no longer shows stock Home for ~25s before the mod appears, and no
  longer restarts the compositor. A restart was previously unavoidable: the
  compositor reads QML once at startup, it starts at boot+2s, and Homebrew
  Channel's hooks cannot run before boot+20s
- Power-off is immediate again. A compositor restart makes LG's igallery
  preview segfault in `QFontDatabase::removeAllApplicationFonts` - reproducible
  on stock firmware with no mod present - and faultmanager then reports the
  crash with recovery `rebootToSuspend`, so tvpowerd answered the next power
  press with a full reboot into standby instead of switching off. Not
  restarting the compositor avoids the crash; when an interactive apply does
  have to restart it, the reports that restart produces are removed

### Added

- `boot-bind` rebuilds the shadow module when the stock one changes, so a
  firmware update cannot leave stale QML bound to new libraries, and drops the
  environment override entirely if the rebuild fails or if it finds no
  compositor running - a persistent override could otherwise repeat a failure
  on every boot
- `status` reports `live`, whether the running compositor actually has the mod,
  separately from `applied`

## [0.2.4] - 2026-09-14

### Fixed

- The boot hook no longer forces the panel on. `apply` calls `turnOnScreen`
  before relaunching Home because a compositor restart can leave the panel
  blanked; on the boot path that turned a set back on after a crash-recovery
  reboot-to-suspend. The hook now marks itself as the boot path and the panel
  is left to whatever the system decided

## [0.2.3] - 2026-09-14

### Fixed

- 0.2.2 could leave the compositor on stock QML under the modded layout: a
  black Home with only the sidebar and dock. It judged whether the compositor
  predated the binds by `/proc/<pid>`'s mtime, which on this kernel is only the
  time the `/proc` entry was last instantiated, not the process start. The
  decision now uses the kernel's own record (`/proc/<pid>/stat` field 22), in
  seconds since boot, and the bind time is recorded the same way and scoped to
  the boot id, so it cannot be confused by an unsynchronised wall clock or a
  record left by a previous boot
- The shader-failure check is anchored on a log line count taken before the
  restart again, never a timestamp: the log spans boots and its early lines
  carry an unsynchronised clock

### Changed

- 0.2.2 claimed cold boot no longer restarts the compositor. That was wrong,
  and the measurement behind it was the same bogus timestamp. The compositor
  starts at boot+2 s and Homebrew Channel's hooks run at boot+20 s, and `/etc`
  is a read-only overlay with no way to order a unit ahead of it, so a cold
  boot always takes exactly one restart. `boot-bind` stays: it makes that
  restart happen as early as the hook allows, and any later compositor start
  picks the modded QML up on its own

## [0.2.2] - 2026-09-14

### Changed

- Cold boot no longer restarts the compositor. The compositor starts about two
  seconds after Homebrew Channel's hooks, and a full apply spent ~5 s staging
  before its binds landed, so it read stock QML and had to be restarted: a
  fifteen-second black screen on every boot. The boot hook now binds the
  previous boot's verified set first, in well under a second, so the compositor
  loads the modded QML on its own first start
- `apply` is idempotent. A staged set identical to what is already live is left
  mounted rather than drained and re-bound, and the compositor is judged against
  the time that content actually went live. A no-op apply now takes about a
  second and makes no restart
- `fps` goes through `apply`. The inline version bound both QML paths, which
  are one inode, and stacked two mounts

### Added

- `boot-bind` and `boot-ok` verbs for the boot hook. `boot-bind` only mounts a
  set that a full apply has verified on this firmware, and skips itself once
  after a boot that never reported ok

## [0.2.1] - 2026-09-14

### Fixed

- Boot hook no longer holds Homebrew Channel's failsafe window open. `startup.sh`
  arms a failsafe flag, runs `run-parts`, then clears the flag ten seconds later;
  applying inline took around 45 seconds, so a set powered off in that window came
  back in failsafe mode with every root customization disabled. The hook now
  returns immediately and applies in a detached child
- `apply` only restarts the compositor when it has to. If the compositor started
  after the binds went live it already loaded the modded QML, and if the panel is
  not active the binds are left staged for its next start. Both cases previously
  cost a needless restart, one of them on the boot path
- `apply` and `revert` take a lock. A detached boot apply can now overlap a user
  launching the app, and two concurrent runs could leave the wrong number of
  mounts live

## [0.2.0] - 2026-09-14

### Added

- News card in the bottom-left corner: photo, headline and as much of the summary
  as fits, rotating every 10 seconds. Top world and US stories come from keyless
  public RSS (BBC News, falling back to The New York Times), so the widget ships
  with no API key and no quota

## [0.1.0] - 2026-09-14

### Added

- Initial release: LumaGlass homebrew app for rooted LG webOS 10 TVs
- Liquid glass compositor theme with Rose Pine color palette
- Abstract wallpaper displayed through transparent Home screen
- Decluttered home layout with refactored UI strings
- `apply` command: Install mod via bind mounts, restart services
- `revert` command: Remove mod, restore original assets, restart compositor
- `persist` command: Enable/disable state persistence across reboots
- `fps` command: Toggle compositor FPS debug overlay
- `ntfy` command: Configure ntfy.sh integration for notifications
- `status` command: Display current mod state and configuration
- `log` command: View debug logs from recent operations
- Web UI app built with Homebrew Channel integration
- Self-updating via GitHub Releases
- Custom Homebrew Channel repository support
- Non-destructive install: all changes are mount-based, easily reverted
- Tested on LG webOS 10.2.1 (32-bit ARM userspace, Rockhopper chassis)

### Technical

- Build tooling: POSIX shell scripts, no webOS SDK required
- Deployment: ar-based IPK packaging, deterministic builds
- CI/CD: GitHub Actions + Gitea mirror workflows
- Compositor QML: Custom materials and shaders for glass effect
- Home app modifications: XML layout, image overlays, localization strings
- Persistent state: `/var/lib/lumaglass/` with auto-backup of original assets
