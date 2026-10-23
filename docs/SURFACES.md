# LG surface catalogue

Every screen, popup, overlay, banner, menu and notification the LG C4
(OLED65C4PUA, webOS 10.2.1) can show, with LumaTV's planned answer for each.
Companion to `PLAN.md`, whose Surfaces table names about twenty of them, and
to `mocks/`. Source: the firmware snapshot `/data/home/lgtv/uiinv` and the
set itself, read-only.

## Resume here

State on 2026-10-23 19:30: all ten research slices have landed and their raw
output is saved in `docs/research/surfaces/*.json` (rows with evidence, plus
findings) with the service message strings in `service-strings.txt`. The
tables below are being rewritten from them; until that is finished the group
tables hold only the first rows and the file is partial. The TV was only read
(Active Standby throughout, no tweak applied, nothing to revert; one stray
`/tmp/lstxt.txt` was written at 18:59 and removed again).

Done:

- Read `PLAN.md`, `mocks/settings-map.md` and the mock gallery. Mocked so far:
  Settings (Picture, Sound, Remote and pointer, AI access), AI request
  prompt, confirm dialog, toast, input picker, search and keyboard.
- Universe of apps: 149 dirs in `uiinv/apps`, plus 10 ids that only
  `listApps.json` has (`amazon`, `netflix`, `com.apple.appletv`,
  `com.crunchyroll.webapp`, `com.geforcenow.play`, `com.lgtvod.us.app`,
  `com.lighting.huesync`, `com.webos.app.adapp`, `com.webos.app.discovery`,
  `com.webos.app.membership`), plus `com.webos.app.voiceagent` (on the set,
  not in the snapshot) = 160 ids. The live `listApps` (2026-10-23) adds only
  `com.miniclip.eightballpool` (stub). `/usr/palm/applications` holds 134
  dirs; `/media/cryptofs` also holds Twitch, DAZN, Highbrow, Tennis TV and
  Karaoke stubs that `listApps` does not return.
- Rows for the third-party and homebrew apps (group 15) are written.
- Live settings read from the set (read-only `getSystemSettings`) that decide
  what is seen: `general.lifeOnScreenMode` = `alwaysReady` (the Gallery+ /
  Line View screen shows when the set is "off"), `general.screenSaverEnabled`
  = on, `noSignalScreenSaver` = on, `screenSaverAd` = off, `homePromotion` =
  off, `personalRecommend` = off, `aiNudge` and `aiSettingsNudge` = off,
  `adCookie` = on, `remotePointer` = on, `homeAutoLaunch` = on,
  `powerOnScreen` = `todays`, `lastInputApp` = `com.webos.app.hdmi2`;
  `option.livePlus` = off, `option.hbbTV` = off, `option.country` = USA,
  `option.audioGuidance` = off.

Next:

1. Rewrite the group tables from the slice rows (one row per surface; every
   app id and every `WebOSCompositor/views` file named in a row or in
   "No UI"), keeping the plan's current answers where it has one.
2. Coverage check: all 160 app ids and all view files appear in the text.
3. Fill Totals, "Facts the replacements must keep", "Missing from PLAN.md"
   and "Needs Nitin"; compare against `mocks/` (24 screens by 19:15, list in
   `mocks/BATCH2.md`) for the not-yet-mocked ranking.

Scratch (under `/tmp`, may be wiped): `tvsrc/apps` (small text files of
`/usr/palm/applications`), `tvsrc/inputcommon/qml`, `tvsrc/l10n` (service
strings), `/tmp/service_strings.txt`, `/tmp/surfaces/ids_{A,B,C,D}.txt`.
Rebuild: paramiko to `root@192.168.1.242`, stream `find … | tar -T - -cf -`
(BusyBox tar has no `--null`; never redirect to a file on the TV). Service
message strings are the JSON keys of
`/mnt/lg/wee/ui_l10n/usr/share/localization/<service>/resources/<lang>/cppstrings.json`
(`/usr/share/localization/*/resources` are symlinks to it).

## Reading the tables

Columns: Surface (what it shows) · LG id · kind · size · Seen · Plan ·
Answer with its reason · Hook · Ph.

- **Seen**, an estimate from this set's use (Filmmaker mode, Denon AVR over
  eARC on HDMI 2, Live TV rare, Magic Remote pointer sometimes, Netflix,
  YouTube, Prime Video, Plex and HDMI inputs): `daily` most days, `sometimes`
  about weekly, `rare` a few times a year or less, `never` cannot appear on
  this set or only in factory, hotel or store modes.
- **Plan**: `Yes` named in `PLAN.md` with an answer; `Part` named without a
  hook, phase or full scope; `Out` listed there as not owned; `No` absent.
- **Answer**: Replace (LumaTV draws it), Restyle (LG's logic, LumaTV's look),
  Off (disabled or stubbed), Keep (LG's own, reason given). `Keep (dormant)`
  means it cannot appear on this set and needs no work.
- **Hook**: names from the Hooks table of `PLAN.md` (System UI redirect, View
  swap, Shadow module, Overlay mount, Key filter and launch redirect), plus
  `LG setting` (switched off through settingsservice or configd), `Service
  off` (masked through Glasshouse) and `Theme` (shared web-app theme); other
  mechanisms are named in the row.
- **Ph**: phase 1-9 of `PLAN.md`; `*` the plan names no phase and the
  placement is proposed here; `—` no work.
- Sizes are the snapshot's `size_kb` (or `appsize` from `listApps`); QML
  modules give file counts.

## 1. Home & launching

| Surface | LG id · kind · size | Seen | Plan | Answer | Hook | Ph |
|---|---|---|---|---|---|---|
| **Home**: launcher bar, hero banners, promotions, recommendations | `com.webos.app.home` · flutter · 45.8 MB | daily | Yes | **Replace**: LumaHome already draws it; LG's Home stays resident behind as the fallback | Launch redirect (Home stub) | 1 |
| **Apps tile**: LG Content Store, all apps | `com.webos.app.discovery` · web · n/a (not in snapshot dirs) | sometimes | Yes | **Replace**: own all-apps grid with search, hide and favourites | Key filter and launch redirect | 7 |
| **Launch splash**: image shown while an app starts | `WebOSCompositor/views/StarfishImageSplash.qml` · compositor-qml · 333 lines | daily | Yes | **Replace**: rebuilt in the resident shadow module | Shadow module | 5 |
| **Spinners**: loading spinner and infinite spinner | `StarfishSpinnerView.qml` (×2 instances) · compositor-qml · 153 lines | daily | Yes | **Replace**: rebuilt, one shared look | Shadow module | 5 |
| **Long-press ring**: progress ring while a key is held | `StarfishLongPressSpinnerView.qml` · compositor-qml · 154 lines | sometimes | Yes | **Replace**: rebuilt with the spinners | Shadow module | 5 |
| **Recents and multi-view frames**: app cards, window frames | `controllers/fullscreen/StarfishRecents*.qml`, `views/decoration/*` · compositor-qml | sometimes | Yes | **Restyle**: LG's logic stays, closing goes through `closeByAppId` | Shadow module | * |
| **Context menu**: hold-OK menu on a Home tile | `com.webos.app.contextmenu` · system-qml · 36 files / 14 qml | sometimes | Yes | **Replace**: rebuilt resident, Hearth-style Move/Hide/Uninstall | System UI redirect | 5 |

## 2. Inputs & devices

| Surface | LG id · kind · size | Seen | Plan | Answer | Hook | Ph |
|---|---|---|---|---|---|---|
| **Input picker**: HDMI and Live TV tiles with device glyphs, CEC names | `com.webos.app.quickinputpicker` · system-qml · 89 files / 51 qml | sometimes | Yes (mock 08) | **Replace**: rebuilt resident, own inputs only | System UI redirect | 4 |
| **On-screen remote**: number pad, colour keys, trick-play | `com.webos.app.screenremote` · system-qml · 133 files / 57 qml | rare | Yes | **Replace**: rebuilt resident | System UI redirect | 5 |

## 3. Volume & sound

| Surface | LG id · kind · size | Seen | Plan | Answer | Hook | Ph |
|---|---|---|---|---|---|---|
| **Volume and mute OSD**: bar and mute icon | `views/StarfishVolumeView.qml`, `views/volume/StarfishVolume.qml`, `StarfishVolumeMute.qml` · compositor-qml · 240 + 484 + 130 lines | daily | Yes | **Replace**: shows the Denon's own level (receiver pushes `MV`/`MU`), not webOS's drifting counter | Shadow module | 4 |

## 4. Notifications & prompts

| Surface | LG id · kind · size | Seen | Plan | Answer | Hook | Ph |
|---|---|---|---|---|---|---|
| **Toasts, alerts, PIN prompt, input popups** | `SystemUI/Notification` · system-qml · 38 qml (+ `views/StarfishNotificationView.qml`) | daily | Yes (mocks 05-07) | **Replace**: rebuilt to the item contract; every service message arrives through `com.webos.notification` | View swap | 2 |
| **Notification Centre**: list of past notifications | `com.webos.app.notificationcenter` · web · 10.1 MB | rare | Yes | **Replace**: own drawer | Key filter and launch redirect | 7 |

## 5. Settings & wizards

| Surface | LG id · kind · size | Seen | Plan | Answer | Hook | Ph |
|---|---|---|---|---|---|---|
| **Quick panel**: picture mode, sound out, tiles | `com.webos.app.quicksettings` · system-qml · 799 files / 165 qml | sometimes | Yes (built, resident) | **Replace**: rebuilt resident; LG's rebuilds per open | System UI redirect | 3 |
| **Settings**: Picture, Sound, General, Support | `com.palm.app.settings` · web · 25.6 MB (+93 MB while open) | sometimes | Yes (mocks 01-04) | **Replace**: own Settings; LG's app only for its deep screens, themed | Launch redirect + Theme | 3 |
| **Settings sub-apps**: network, family, LG account and other Enact apps | Enact web apps · web | rare | Yes | **Restyle**: one shared theme | Overlay mount | 6 |

## 6. Live TV

| Surface | LG id · kind · size | Seen | Plan | Answer | Hook | Ph |
|---|---|---|---|---|---|---|
| **Live TV banner, info, no-signal** | `com.webos.app.inputcommon` · native QML · 10.5 MB / 273 qml | rare (no-signal sometimes) | Yes | **Restyle**: banner only; no guide (Nitin, 2026-10-23) | Overlay mount | 6 |

## 7. Search, voice & keyboard

| Surface | LG id · kind · size | Seen | Plan | Answer | Hook | Ph |
|---|---|---|---|---|---|---|
| **On-screen keyboard** (maliit) | `views/StarfishKeyboardView.qml` hosts maliit · compositor-qml · 85 qml | sometimes | Yes (mock 09) | **Restyle**: deterministic-row keyboard look | Overlay mount | 6 |
| **Search and voice** | `com.webos.app.voice` · native QML · 14.8 MB | sometimes | Yes (mock 09) | **Restyle** or replace | Overlay mount | 6 |

## 8. Power, boot & screensaver

| Surface | LG id · kind · size | Seen | Plan | Answer | Hook | Ph |
|---|---|---|---|---|---|---|
| **Boot and power-off logo** | `com.webos.app.bootlogo` · system-qml · 142 files / 5 qml | daily | Yes | **Replace**: own animation; `lgLogoDisplay` offered as a setting | System UI redirect | 5 |
| **Screensaver** | `com.webos.app.screensaver` · flutter · 13.4 MB | sometimes | Yes | **Replace**: drawn in the compositor on the `Screen Saver` power state, keeps OLED pixel shift | Research (compositor) | 8 |

## 9. Game

| Surface | LG id · kind · size | Seen | Plan | Answer | Hook | Ph |
|---|---|---|---|---|---|---|

## 10. Casting & sharing

| Surface | LG id · kind · size | Seen | Plan | Answer | Hook | Ph |
|---|---|---|---|---|---|---|

## 11. Ads, promos & tracking

| Surface | LG id · kind · size | Seen | Plan | Answer | Hook | Ph |
|---|---|---|---|---|---|---|
| **Nudges, sports alarm, store promotions** | `Nudge`, `SportsAlarm`, `StoreStreamer` · system-qml · 297 / 245 / 49 files | sometimes | Yes | **Off**: empty view; `nudge.service` off stops them at the source | System UI redirect + Service off | 7 |
| **Ad and content-recognition overlays** | `acr*`, `ad*` apps · web / native | never (ad hosts blocked) | Yes | **Off**: through LG's own settings | LG setting | 7 |

## 12. Accounts & first run

| Surface | LG id · kind · size | Seen | Plan | Answer | Hook | Ph |
|---|---|---|---|---|---|---|

## 13. Service & factory

| Surface | LG id · kind · size | Seen | Plan | Answer | Hook | Ph |
|---|---|---|---|---|---|---|

## 14. Accessibility

| Surface | LG id · kind · size | Seen | Plan | Answer | Hook | Ph |
|---|---|---|---|---|---|---|

## 15. Third-party & protected

| Surface | LG id · kind · size | Seen | Plan | Answer | Hook | Ph |
|---|---|---|---|---|---|---|
| **Netflix** | `netflix` · native · 87.6 MB | daily | Out | **Keep**: third-party UI and protected video; only its launch splash and spinner are LumaTV's | — | — |
| **YouTube** | `youtube.leanback.v4` · native (Cobalt) · 20.3 MB | daily | Out | **Keep**: third-party UI; a homebrew web "YouTube AdFree" with the same id sits in `/media/developer`, `listApps` registers the native one | — | — |
| **Prime Video** | `amazon` · native · 23.2 MB | sometimes | Out | **Keep**: third-party UI and protected video | — | — |
| **Apple TV** | `com.apple.appletv` · native · 51.3 MB | rare | Out | **Keep**: third-party UI and protected video | — | — |
| **Crunchyroll** | `com.crunchyroll.webapp` · web · 746 KB | rare | Out | **Keep**: third-party UI | — | — |
| **GeForce NOW** | `com.geforcenow.play` · web · 789 KB | rare | Out | **Keep**: third-party UI | — | — |
| **PlxNative**: unofficial native Plex client | `com.beb.plxnative` · native · 36.3 MB | sometimes | Out | **Keep**: homebrew UI, its own renderer | — | — |
| **Moonlight**: game-stream client | `com.limelight.webos` · native · 4.3 MB | rare | Out | **Keep**: homebrew UI | — | — |
| **Punktfunk**: low-latency desktop and game streaming client | `io.unom.punktfunk.client-webos` · native · 25.5 MB | rare | Out | **Keep**: homebrew UI | — | — |
| **Hue Sync**: Signify's sync app | `com.lighting.huesync` · web · 15.7 MB | rare | Out | **Keep**: third-party UI; LumaTV's Ambient light replaces its use | — | — |
| **LG Movies & TV**: LG's rental and purchase store | `com.lgtvod.us.app` · web · 574 KB | rare | No | **Off**: store promotion; left out of LumaTV's launcher | Launcher filter | 7 |
| **Viewster**: stub of a defunct VOD service | `com.lge.app.viewster` · web · 11 KB | never | No | **Keep (dormant)**: 11 KB stub, no `listApps` entry | — | — |
| **8 Ball Pool**: store stub, added after the snapshot | `com.miniclip.eightballpool` · stub · live `listApps` only | never | No | **Keep (dormant)**: third-party stub | — | — |
| **AmazOff**: Prime Video ad patcher (nginx on the set) | `com.amazoff.patcher` · web · 8.1 MB | rare | No | **Keep**: homebrew; its background nginx runs without UI | — | — |
| **Homebrew Channel** | `org.webosbrew.hbchannel` · web · 2.5 MB | rare | No | **Keep**: homebrew installer; needed for root access | — | — |
| **Homebrew Channel Updater** | `org.webosbrew.safeupdate` · web · 664 KB | never | No | **Keep**: homebrew | — | — |
| **LG App Update Blocker** | `org.webosbrew.appupdateblocker` · web · 128 KB | never | No | **Keep**: homebrew | — | — |
| **LG Input Hook**: front for the input-hook service | `org.webosbrew.inputhook` · web · 52 KB | never | Part (admin server "not taken") | **Keep**: its service runs without UI | — | — |
| **64 to 32-bit Bridge**: EGL/GLES/Wayland bridge | `org.webosbrew.bridge-64to32` · native · 37.5 MB | never | No | **Keep**: support library; hide from the launcher | Launcher filter | 1 |
| **HyperHDR loader** | `org.webosbrew.hyperhdr.loader` · web · 4.4 MB | rare | Part (reference for Ambient light) | **Keep**: homebrew | — | — |
| **LumaTV app** (still `LumaGlass`): own app | `org.nphil.lumaglass` · web · 5.0 MB | rare | Yes | **Keep**: ours; renamed `org.nphil.lumatv` in phase 1 | — | 1 |
| **LumaTV frame**: overlay pane for mini apps | `org.nphil.lumaglass.frame` · web overlay · 340 KB | sometimes | Yes | **Keep**: ours | — | 1 |
| **Magic Remote pointer image** | native (`pointerResourceStyle: LG`) | sometimes | Out | **Keep**: native cursor, swap unverified; every surface has a hover state | — | — |
| **Video planes** (apps, inputs, protected content) | hardware planes placed by the compositor | daily | Out | **Keep**: protected video stays on the planes LG places | — | — |

## No UI

_(filled when the slices land)_

## Missing from PLAN.md

_(filled when the slices land)_

## Needs Nitin

_(filled when the slices land)_
