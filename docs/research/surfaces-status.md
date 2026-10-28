# Surface status: LG screens and menus replaced, restyled or switched off

State on 2026-10-27, from the repository only (code, CHANGELOG, HANDOFF, PLAN, `docs/research/*-result.md`, TV captures and logs); rows changed by the TV check of 2026-10-28 (`48ee380` on the set, LumaTV's compositor live) say so. Scope: the 111 rows of `docs/SURFACES.md` whose plan is Replace (62), Restyle (25) or Off (24); the 67 Keep rows are out of scope. Row numbers are SURFACES.md line numbers.

## Answer

No. 41 of 111 are live on the TV: 35 on 2026-10-27 and six more after the TV check of 2026-10-28 (Quick Access, quick-panel popups and editing, the input banners, the no-signal screen, Search, the standard toast). Of the 28 daily and sometimes surfaces in scope, 18 are live (Home, volume, mute, power-off logo, standard toast, quick panel, its tiles and its popups, input picker, Apps library, Settings, keyboard keys, saver, Quick Access, input banners, no-signal screen, Search); the other 10 are not, and the LG screens a viewer still meets are listed below.

## Status legend

- **LIVE**: built, on by default on the TV and seen working there. For Off rows: the off state is in place and was checked by the declutter or Home-stub runs (no picture exists of an absence).
- **BUILT-OFF**: built and deployed to the set, but switched off by default (`lumatv surfaces`: `notifications`, `cover`, `splash`, `store`, `standbyscreen` and the own keyboard are off), or needing a state the set is not in.
- **BUILT-UNTESTED**: code exists, the surface was never seen on the TV.
- **NOT BUILT**: no code, or the plan was dropped.
- **BLOCKED**: built but cannot run without something outside the repo (here: Nitin's Home Assistant link).
- **Look** judges a built surface against the approved mock (`docs/mocks`) and Lucent's TV profile (Manrope, glass and ink tokens from `payload/theme/theme.json`, focus lift, 4.5:1 text, 200 % text). **MATCHES** and **GAPS** rest on a capture viewed for this note; **UNSEEN** means no TV capture or kept render exists. Off, not-built and dropped rows have no look.

## Counts

| Status | Daily | Sometimes | Rare | Never | Total |
|---|---|---|---|---|---|
| LIVE | 4 | 14 | 5 | 18 | 41 |
| BUILT-OFF | 2 | 6 | 22 | 3 | 33 |
| BUILT-UNTESTED | 0 | 1 | 24 | 2 | 27 |
| NOT BUILT | 0 | 0 | 9 | 0 | 9 |
| BLOCKED | 0 | 1 | 0 | 0 | 1 |
| **Total** | **6** | **22** | **60** | **23** | **111** |

Look, built Replace and Restyle rows (76): MATCHES 8, GAPS 12, UNSEEN 56 on 2026-10-27; the 10-28 TV check moved 79, 88, 122, 145, 146 and 182 to MATCHES and left 89 at GAPS (not recounted).

## Still LG, met in normal use (daily and sometimes, not LIVE)

| How often | What a viewer meets | Why |
|---|---|---|
| daily | LG's grey launch splash with the dots on app launches | `lumatv splash on` (69) patches LG's compositor, which is not drawn while LumaTV's is live, so it stays off; Home launches draw LumaTV's own card (seen on the TV 10-28); what a launch by a remote key shows before the app's first frame was not looked at |
| daily | the other message screens (alerts, volume and format prompts, the rest of the messages) | not raised under LumaTV's compositor, where the shell's `Notify` draws them; `lumatv notifications on` changes only LG's own notification view, which is not drawn while LumaTV's compositor is live (125, 96, 98 and the rows that inherit them) |
| daily | LG's Always Ready art and clock in standby | `lumatv standbyscreen off` is built, not applied: the setting is still on (202) |
| sometimes | LG's input banner at boot, and LG's photo on the No Signal screen | `inputcommon` is on (88, 89): the pill and the INFO card are LumaTV's on the TV; the no-signal screen keeps LG's photo while `general.noSignalScreenSaver` is on |
| sometimes | the "device connected, switch?" popup | not raised on the set (95, 128): HDMI 3 reported nothing connected during the 10-28 check |
| sometimes | a Search that never finishes | `lean` stops `com.webos.service.nlpmanager`, so a search ends in a loading ring (182, restyled and live) |
| sometimes | LG's launch spinner, startup cover and boot animation | spinner: LG's view is not drawn under LumaTV's compositor (70); cover: `cover` shows only in a cold boot, which the 10-28 check did not make (198); the boot animation was never seen (196) |
| sometimes | Quick Access: LG's own handling of a digit where LumaHome is not in front | LumaHome's toast and question run on Home only (79, live) |
| sometimes | no app switcher at all | built, behind `remote.recents` (72) |
| sometimes | voice bar | needs the Home Assistant link (183) |

Also LG-side but rarer: the Home Hub tile in the apps grid (86), LG Channels on its remote key (238), the LG sub-apps (wizards, scheduler, user guide) until the web theme is run on the set.

## Look gaps, most visible first

| Surface | What differs | File |
|---|---|---|
| Input INFO card (88) | matches on the TV (2026-10-28: dark glass, hairline, light-wash chips); it needs a live source, and the channel INFO card and the Guide chip (164, 165) are unreachable because Live TV does not come to the front under LumaTV's compositor | `tools/inputcommon_patch.py` |
| No-signal screen (89) | live: the sheet over LG's photo is restyled, but mock 18's block (glyph, input name, Inputs and Home) never shows while `general.noSignalScreenSaver` is on | `tools/inputcommon_patch.py` (Screensaver.qml), `payload/inputcommon/` |
| Search app and keyboard (181, 182) | live and matching mock 09 on the TV (2026-10-28): field, chips, genre cards, typed suggestions, mic hint, top row, option sheet; open: the Magic Remote's hover, a search that finishes (`lean` stops the NLP service), a loading ring that is faint on the sheet; the keyboard's panel is LumaKeyboard's | `tools/voice_patch.py`, `payload/voice/`, `tools/maliit_patch.py` |
| Volume bar (110) | bottom-centre bar lands on Netflix subtitle rows; bar stays at Nitin's bottom-centre choice, a clear spot needs his pick | `panels/VolumeOsd.qml` |
| Quick panel (145) | matches mock 10 on the TV (2026-10-28): docked right, steppers beside their words, no "LG" button; Ambient tile still needs the link; the Receiver tile reads "Not answering" at times | `panels/QuickPanel.qml` |
| Apps library (68) | rail wrap and stale clock fixed offline (34bfc16), rendered clean at 100 and 200 % text, not re-seen on the set | `panels/AppsGrid.qml` |
| Settings (149, 150) | selected tab and segment marks now an accent plate with a wider mark (16abb15, offline); disabled rows measure 6:1 offline | `kit/LuSegmented.qml`, `kit/LuRow.qml` |
| Notification drawer (133) | footer text fix 0a2d782 renders clean; head, times and body fixed at 200 % text and contrast (16abb15) | `panels/Drawer.qml` |
| LG Settings app (151) | themed, but LG's two-pane layout and focus pill | `payload/webapps/lumatv.css` |

## Daily (6)

| Row | Surface | Plan | Status | Evidence | Look |
|---|---|---|---|---|---|
| 66 | Home | Replace | LIVE | homestub-result; round9/tv-2026-10-26/10-home-after-reboot.png | MATCHES (glass, Manrope, wallpaper tint, dock); the At-home lane shows a placeholder until the Home Assistant link is made |
| 69 | Launch splash | Replace | BUILT-OFF | Home launches draw `panels/Splash.qml`, rebuilt to mock 15 (180 px picture, name, ring, dimmed wallpaper), seen on the TV 2026-10-28 from Home tiles and number keys (Netflix, the store: `restyle-fixes/unbuilt-screens/tv/quick-access-*.jpg`); an app with no splash picture of its own gets the same card in LG's compositor through `lumatv splash` (`tools/splash_patch.py`, shadow module), which stays off: while LumaTV's compositor is live LG's launch views are not drawn, so the patch cannot be seen or checked on the set; apps with their own art (Netflix, YouTube, Prime, Plex) keep it, as the plan says | MATCHES mock 15 on the TV for Home launches (tile, name, ring, "Opening", dimmed wallpaper); the patched LG splash is UNSEEN (offline: `restyle-fixes/unbuilt-screens/after/15-launch-splash.png`, `splash-card.png`) |
| 110 | Volume OSD | Replace | LIVE | round8-result (a) PASS; round9/r11 `mutefix-*` captures | GAPS: bottom-centre bar lands on Netflix subtitle rows (`mutefix-format-unmuted-vol0.png`; Main owns, round9-result) - `panels/VolumeOsd.qml`; otherwise matches mock 11 |
| 122 | Toast: standard | Replace | LIVE | 2026-10-28 on the TV under LumaTV's compositor, with `notifications` off: `createToast {"sourceId":"com.webos.app.home","message":"This function is not available."}` drew the shell's pill top right (info glyph, the line in Manrope, still up 2.2 s later: `restyle-fixes/unbuilt-screens/tv/toast-standard-under-lumatv-compositor.jpg`); the gate's toast first frame was 54 to 104 ms from the delivery in round 10 (limit 50, `compositor-gate9-result`) | MATCHES the toast family of mock 07 (dark sheet, top right, glyph, Manrope); the three-line variant and the toast over Netflix not raised on the TV |
| 197 | Power-off logo | Replace | LIVE | `logo-events` 10-25/26: `power-off logo 1 launch`, `answered tvpowerd` (round9/tv-2026-10-26/reboot) | UNSEEN (panel is dark; nothing can capture it) |
| 202 | Always Ready and Gallery+ | Replace | BUILT-OFF | `lumatv standbyscreen off` sets `general.lifeOnScreenMode` to off and keeps the old mode for `on` (Settings > Home & look > Standby has the same switch); not applied: `lifeOnScreenMode` still `alwaysReady` (saver-own-result 10-26) and the switch reads on in the 10-27 walk (`home-look-standby.png`) | n/a |

## Sometimes (22)

| Row | Surface | Plan | Status | Evidence | Look |
|---|---|---|---|---|---|
| 68 | Apps tile (LG Content Store) | Replace | LIVE | round8-result (e) PASS; `round8/tv-2026-10-25/45-apps-grid.jpg` | GAPS: rail rows wrapped into the next row and the bar clock was stale on the set; fixed offline (34bfc16), not re-seen - `panels/AppsGrid.qml` |
| 70 | Launch spinner and infinite spinner | Replace | BUILT-OFF | the same `splash` switch swaps LG's grey disc for the dimmed screen, a ring and "Loading" in a dark pill after a second (`payload/splash/spinner.qml`); Home's own spinner (`Splash.qml`) is the same look and was seen on the TV 2026-10-28; `luna://com.webos.surfacemanager/showSpinner` answers "unknown method" under LumaTV's compositor, so LG's spinner view cannot be raised there and the switch stays off | MATCHES mock 15 offline (`restyle-fixes/unbuilt-screens/after/15-spinner-word.png`, `splash-spinner.png`); the patched LG spinner is UNSEEN on the TV |
| 72 | Recents (app switcher) | Replace | BUILT-OFF | `panels/Recents.qml` in `Layers`, opened by Back from the top of Home when `remote.recents` is on (Settings > Remote & pointer, off until the TV check); LG's `recent/listRecents`, the app closed (`closeByAppId`) before `removeRecent` on a held OK; offline host tests pass | MATCHES mock 14 offline (`restyle-fixes/unbuilt-screens/after/14-recents-focus.png`) except: no "Playing" mark (nothing reads what plays) and a time shows only where LG's snapshot file name carries one; UNSEEN on the TV |
| 79 | Quick Access (number keys) | Replace | LIVE | digits open dock tiles as before (`remote.numberKeys`, on); the toast, the owner's own choice per number and the question for a number with no dock tile are `remote.quickAccess.on`, switched on after the TV check of 2026-10-28: 2 opened Netflix under LumaTV's launch card with "2 · Netflix / Opening." top right for about 2 s, and a choice of the store for 5 opened it with "5 · LG Content Store / Opening." (`restyle-fixes/unbuilt-screens/tv/quick-access-*.jpg`) | MATCHES mock 31 on the TV; two decisions kept over the mock: a digit acts on press (mock 03, shipped) so the question reads "Pressing 4", and the toast has no "Hold 0" line; the question cannot appear on this set while the dock holds ten or more tiles (it holds 23), so it is seen offline only |
| 85 | Input picker | Replace | LIVE | round8-result (d): Input key (evdev 241) opens it; gates r12-r14 over Home, HDMI and Netflix (round9-result) | MATCHES in `round8/tv-2026-10-25/41-key241.jpg` (glass sheet, Manrope, icons, eARC mark); later layout changes (round 15 cached texture) not re-captured |
| 88 | HDMI input banner | Restyle | LIVE | `inputcommon` on since 2026-10-28 03:45; the TV check of that day: dark glass pill 1.4-6.4 s on HDMI 1 and 2, INFO card dark glass on HDMI 2 (`restyle-fixes/banners/tv/`, lg-restyles-result); stock grey pill still shown at boot (`reboot/boot-002-up16.33.png`) | MATCHES: pill and INFO card (34 px corners, hairline, "HDMI 2" dim over "Apple TV", light-wash chips, about 550 px wide: `hdmi2-info-card-0341.jpg`); the card needs a live source (black input: the INFO key draws nothing) - `tools/inputcommon_patch.py` (DetailInfoBanner.qml) |
| 89 | No-signal screen | Restyle | LIVE | `inputcommon` on; seen on HDMI 1 (nothing connected) 2026-10-28 from 7.9 s | GAPS: `general.noSignalScreenSaver` is on, so LG's photo mode runs and keeps LG's block: LG's photo with "No Signal" in the restyled sheet (dark glass, Manrope, dim key-help line: `restyle-fixes/banners/tv/hdmi1-nosignal-photo.jpg`); mock 18's block (glyph, input name, Inputs and Home) never shows while that setting is on, which is the owner's choice and was not changed - `tools/inputcommon_patch.py` (Screensaver.qml) |
| 95 | HDMI or CEC device connected | Replace | BUILT-OFF | `AdvancedAlertDelegate.qml` in the notification view (`lumatv notifications`, off): now says the port ("On HDMI 3. Switch to it?", "Switch to HDMI 3") and its sheet is opaque | MATCHES mock 18 offline (`restyle-fixes/unbuilt-screens/after/18-device-connected-port.png`) except "turned on" and "The Denon follows", which LG's data cannot say; UNSEEN on the TV |
| 111 | Mute bubble | Replace | LIVE | round8-result (a) mute; round9 `mutefix3` (mark returns after restart); r11 `saver-own-step2.png` | MATCHES (slashed-speaker disc, hops every 10 s, clear of subtitles after 00dbca3) |
| 125 | Alert: standard and modal | Replace | BUILT-OFF | registry: `notifications` off; alert drew on the panel 10-25 (surfaces-p2-result); the card is opaque and a modal question's backdrop hides the page, as LumaHome's own Confirm does (Lucent v2 revision of 10-25; mock 06 dims the page and is older, the rule wins) | MATCHES mock 06 offline (`restyle-fixes/unbuilt-screens/after/06-alert.png`, `06-alert-standard.png`) apart from that backdrop and the trash glyph on the danger button; UNSEEN on the TV |
| 128 | Input alert | Replace | BUILT-OFF | `AdvancedAlertDelegate.qml` (mock 18) through `notifications`, see 95; never seen on the set | MATCHES mock 18 offline (`restyle-fixes/unbuilt-screens/after/18-device-connected-port.png`); UNSEEN on the TV |
| 145 | Quick panel | Replace | LIVE | round8-result (d): gear tap opens it; `40-gear-tap-home.jpg`; on the TV 2026-10-28 (`restyle-fixes/unbuilt-screens/tv/quick-panel-*.jpg`) | MATCHES mock 10 on the TV: docked right, steppers beside their words, no "LG" button, a Receiver tile that reads "Not answering" at times; no Ambient tile until the plugin is linked - `panels/QuickPanel.qml` |
| 146 | Quick panel popups and edit mode | Replace | LIVE | `QuickPopup.qml`, `QuickEdit.qml`, `QuickMore.qml` in `QuickPanel.qml`: OK on a stepper opens a value picker (Sleep timer shows when the TV would turn off), a slider card for OLED pixel brightness, a held OK edits (move, remove, Add a tile); the arrangement is `quickPanel.tiles` in theme.json; `remote.quickPanelEdit` switched on after the TV check of 2026-10-28: the hint line, the Sleep timer card (10 min read 9:58 back from `getRemainTime`, then Off), the Picture mode card (Back closed only the card), a held OK, moving and dropping a tile, adding three off tiles (one of them OLED pixel brightness), its slider card, and the order in theme.json (`restyle-fixes/unbuilt-screens/tv/`); the owner's arrangement was put back | MATCHES mock 29 on the TV except: the move marks point up and down (the mock's chevrons point left and right on a vertical list), "Add a tile" has a solid outline, not a dashed one, and in 2 of 3 presses one OK on "Add a tile" added the first off tile at once instead of leaving the list to choose from (the third press, with a capture loop running, showed the list: cause not found) |
| 147 | Quick panel tile set | Replace | LIVE | `40-gear-tap-home.jpg`: Picture mode, Receiver, Sleep timer, Wi-Fi, AI access, All settings | MATCHES |
| 149 | Settings | Replace | LIVE | round8-result (b); `settings-ux/tv-walk-2026-10-27` (27 captures) | GAPS: selected tab and segment marks are faint (`picture-mode.png`: Mode tab, Off segment); disabled rows (Aspect ratio, Just Scan) read dim, TV contrast not measured (offline audit passes) - `kit/LuSegmented.qml`, `kit/LuRow.qml` |
| 150 | Settings sections without a mock | Replace | LIVE | tv-walk-2026-10-27 (Inputs, Network, Ambient light, Home & look, Remote, AI access, System) | MATCHES; Ambient light reads Off and Home Assistant "Not switched on" until Nitin links them |
| 181 | On-screen keyboard | Restyle | LIVE | `keys` on; round9-result 10-26 PASS (`lg-restyles/tv/keys-final-on.png`); `keyboard` motion patch off; under the restyled Search app on the TV 2026-10-28 | GAPS: key caps are Lucent (dark, lavender Search); the Search app around them is LumaTV's look since `voice` went on (182); the keyboard is its own panel under the app, not inside one card as in mock 09 - `tools/maliit_patch.py` |
| 182 | Search and voice results | Restyle | LIVE | `voice` switched on after the TV check of 2026-10-28 (`restyle-fixes/search/result.md`, `restyle-fixes/search/tv/`): field, chips, genre cards with the shader's rounded corners, hint, top row, option sheet and typed suggestions drawn; frame gate first frame 15.3 ms, no late frame; `voice off` gives LG's look back in a new process | MATCHES mock 09 on the TV; not run: the Magic Remote's hover, and "No results found." (`com.webos.service.nlpmanager` is stopped by `lean`, so a search ends in a faint loading ring that Back does not leave) - `tools/voice_patch.py` |
| 183 | Voice listening bar | Restyle | BLOCKED | `panels/VoiceBar.qml` built; needs Nitin: Home Assistant token and the "Search by voice" switch (voice-result) | UNSEEN |
| 196 | Boot logo | Replace | BUILT-UNTESTED | BootLogo built, `logos` on; the 10-26 cold boot capture starts black at 14.6 s with no logo (`reboot/boot-000-up14.60.png`) | UNSEEN |
| 198 | Startup cover | Restyle | BUILT-OFF | `lumatv cover on` swaps in a copy of LG's CoverUI module in the shadow with `MainView.qml` patched (`tools/cover_patch.py`, `payload/cover/`): the first app's tile, name, ring and the brand mark; LG's sequencing, stolen-device screen and fade untouched; off: the cover shows only in a cold boot, and the runbook leaves the reboot to Nitin (2026-10-28 TV check did not reboot) | MATCHES mock 26 offline (`restyle-fixes/unbuilt-screens/after/cover-patched.png`, LG's own in `before/cover-stock.png`) except the tile is a plug glyph, not the HDMI port; UNSEEN on the TV |
| 200 | Screensaver | Replace | LIVE | saver-own-result 10-26 PASS (`r11/saver-own-step2.png`); `screensaver.own` on | MATCHES for the clock (Manrope, dim ink, burn-in mover); art and video modes not seen on the panel; LG's Flutter clock still draws in LG's Screen Saver state (`round8/54-saver-whose.jpg`) |

## Rare (60)

| Row | Surface | Plan | Status | Evidence | Look |
|---|---|---|---|---|---|
| 73 | Multi View window frames | Restyle | NOT BUILT | dropped: PLAN Surfaces table keeps LG's Multi View (Nitin 2026-10-23) | n/a |
| 74 | Multi View picker | Restyle | BUILT-UNTESTED | in `payload/webapps/apps.txt`; its comment: web theme "not run on the set yet" | UNSEEN |
| 77 | Pointer right-click menu | Replace | NOT BUILT | shell-slice-result: pointer right-button menu not done; no code | n/a |
| 78 | "More Actions" panel (on-screen remote) | Replace | BUILT-UNTESTED | `panels/ScreenRemote.qml`, Home only (the ScreenRemote key); over apps not wired (shell-host-result) | UNSEEN |
| 86 | Home Hub | Off | NOT BUILT | no `keyaction.inputHub` repoint in `tools/`; Home Hub tile is in the apps grid (`45-apps-grid.jpg`) | n/a |
| 90 | Banner position editor | Restyle | BUILT-UNTESTED | `inputcommon` on since 2026-10-28; rendered Lucent in round 9; not opened in the 10-28 TV check | Rendered again offline in both materials, no overlap at 100 and 200 % text (`restyle-fixes/banners/after/banner-edit.png`); not seen on the set |
| 92 | Set-top and lock states on an input | Restyle | BUILT-UNTESTED | inputcommon prompts patched; offline `lg-restyles/35-lg-prompt.jpg` only | Rendered again offline (`restyle-fixes/banners/after/alert.png`); not raised on the set |
| 96 | HDMI format prompts | Replace | BUILT-OFF | alert/toast renderer: `notifications` off; the patched alert and toast drew on the panel 10-25 (surfaces-p2-result), this message never raised | UNSEEN |
| 97 | Universal Control (IR blaster, set-top box) | Restyle | BUILT-UNTESTED | in `payload/webapps/apps.txt`; its comment: web theme "not run on the set yet" | UNSEEN |
| 98 | USB and storage prompts | Replace | BUILT-OFF | alert/toast renderer: `notifications` off; the patched alert and toast drew on the panel 10-25 (surfaces-p2-result), this message never raised | UNSEEN |
| 101 | Bluetooth keyboard, mouse and gamepad pairing | Replace | BUILT-OFF | alert/toast renderer: `notifications` off; the patched alert and toast drew on the panel 10-25 (surfaces-p2-result), this message never raised | UNSEEN |
| 102 | Magic Remote status | Replace | BUILT-OFF | alert/toast renderer: `notifications` off; the patched alert and toast drew on the panel 10-25 (surfaces-p2-result), this message never raised; battery warning never filtered | UNSEEN |
| 104 | Wi-Fi Direct, cable and Bluetooth turn-on toasts | Replace | BUILT-OFF | alert/toast renderer: `notifications` off; the patched alert and toast drew on the panel 10-25 (surfaces-p2-result), this message never raised | UNSEEN |
| 112 | ARC, soundbar and wireless-speaker suggestions | Replace | BUILT-OFF | alert/toast renderer: `notifications` off; the patched alert and toast drew on the panel 10-25 (surfaces-p2-result), this message never raised | UNSEEN |
| 113 | Bluetooth headphone or speaker connection | Replace | BUILT-OFF | alert/toast renderer: `notifications` off; the patched alert and toast drew on the panel 10-25 (surfaces-p2-result), this message never raised | UNSEEN |
| 115 | Personalized Sound Wizard and AI Acoustic Tuning | Restyle | BUILT-UNTESTED | in `payload/webapps/apps.txt`; its comment: web theme "not run on the set yet" | UNSEEN |
| 123 | Toast: advanced (icon and link) | Replace | BUILT-OFF | `notifications` off; advanced toast built (`Toast.qml`) | UNSEEN |
| 126 | Alert: full-screen warning | Replace | BUILT-UNTESTED | `Alert.qml` full-screen variant (`type: warning`); never seen | UNSEEN |
| 127 | Alert: icon, checkbox, countdown, busy, progress | Replace | BUILT-UNTESTED | `Alert.qml` check box, progress, spinner, countdown kept from LG; never seen | UNSEEN |
| 129 | PIN prompt | Replace | BUILT-UNTESTED | `PincodePrompt.qml`; needs Nitin to set a lock to test (HANDOFF) | UNSEEN |
| 131 | Second-screen pairing prompt | Replace | BUILT-UNTESTED | same alert path; answered only by the remote; never seen | UNSEEN |
| 132 | Exit-app alert | Replace | BUILT-OFF | alert/toast renderer: `notifications` off; the patched alert and toast drew on the panel 10-25 (surfaces-p2-result), this message never raised | UNSEEN |
| 133 | Notification Centre | Replace | LIVE | round7 slot7 `gear-hold-home.png` (drawer on the set); round8 plugin drawer | GAPS: footer text overflowed its card in the 10-25 capture; fixed 0a2d782 offline, not re-run - `panels/Drawer.qml` |
| 134 | App permission prompts | Replace | BUILT-OFF | alert/toast renderer: `notifications` off; the patched alert and toast drew on the panel 10-25 (surfaces-p2-result), this message never raised | UNSEEN |
| 135 | Software update prompt | Replace | BUILT-OFF | alert/toast renderer: `notifications` off; the patched alert and toast drew on the panel 10-25 (surfaces-p2-result), this message never raised | UNSEEN |
| 136 | App launch prompts | Replace | BUILT-OFF | alert/toast renderer: `notifications` off; the patched alert and toast drew on the panel 10-25 (surfaces-p2-result), this message never raised | UNSEEN |
| 137 | App, AI package and storage messages | Replace | BUILT-OFF | alert/toast renderer: `notifications` off; the patched alert and toast drew on the panel 10-25 (surfaces-p2-result), this message never raised | UNSEEN |
| 138 | Low-memory app restart | Replace | BUILT-OFF | alert/toast renderer: `notifications` off; the patched alert and toast drew on the panel 10-25 (surfaces-p2-result), this message never raised | UNSEEN |
| 151 | Settings deep screens and sub-apps | Restyle | LIVE | `webglass` on; LG Settings app themed on the set (`round8/62-lg-settings-open.jpg`, `63-`) | GAPS: dark glass and Manrope, but LG's two-pane layout and its focus pill remain |
| 152 | Setting-change confirmations and suggestions | Replace | BUILT-OFF | alert/toast renderer: `notifications` off; the patched alert and toast drew on the panel 10-25 (surfaces-p2-result), this message never raised | UNSEEN |
| 153 | Personalized Picture Wizard | Restyle | BUILT-UNTESTED | in `payload/webapps/apps.txt`; its comment: web theme "not run on the set yet" | UNSEEN |
| 154 | Support tools | Restyle | BUILT-UNTESTED | in `payload/webapps/apps.txt`; its comment: web theme "not run on the set yet" | UNSEEN |
| 155 | Help and guides | Restyle | BUILT-UNTESTED | in `payload/webapps/apps.txt`; its comment: web theme "not run on the set yet" | UNSEEN |
| 156 | Usage Care | Restyle | BUILT-UNTESTED | in `payload/webapps/apps.txt`; its comment: web theme "not run on the set yet" | UNSEEN |
| 157 | Weather location | Off | LIVE | nothing to switch: Home weather has its own location (Atlanta, 10-26 Home capture) | n/a |
| 164 | Channel banner | Restyle | BUILT-UNTESTED | `inputcommon` on; round 9 and 18 PASS under LG's compositor (`banners-r18/channel-ok.png`); 2026-10-28 under LumaTV's compositor Live TV never takes the front (SAM keeps Home foreground while the app's card is listed as mapped), so no banner can be raised | The "Guide" chip below was flat LG grey: now a sheet pill with a ring on focus at the banner's inset (`restyle-fixes/banners/after/channel-guide.png`), checked with LG's stock sheet in memory; not seen on the TV - `tools/inputcommon_patch.py` |
| 165 | Channel detail banner | Restyle | BUILT-UNTESTED | `inputcommon` on; channel INFO seen round 9, `banners-r18/channel-info.png`; not reachable on 2026-10-28 (Live TV, see 164) | Shares the INFO card with 88 (dark glass on the TV 10-28 on an input, re-checked offline); not seen in Live TV |
| 168 | TV Scheduler | Restyle | BUILT-UNTESTED | in `payload/webapps/apps.txt`; its comment: web theme "not run on the set yet" | UNSEEN |
| 169 | Channel tuning and Channel Manager | Restyle | BUILT-UNTESTED | in `payload/webapps/apps.txt`; its comment: web theme "not run on the set yet" | UNSEEN |
| 174 | Live TV prompts | Replace | BUILT-UNTESTED | inputcommon prompts patched; never raised on the set | Rendered offline with the input prompts (`restyle-fixes/banners/after/alert.png`); not raised on the set |
| 184 | Microphone indicator | Replace | BUILT-UNTESTED | mic indicator in `VoiceBar.qml`; never seen | UNSEEN |
| 185 | AI Concierge | Off | BUILT-UNTESTED | no check that LG's concierge entry cannot launch (no LumaHome entry; `voice` group masks voice services) | n/a |
| 187 | TV help chatbot | Off | BUILT-UNTESTED | no check that LG's help chatbot cannot launch | n/a |
| 190 | Voice notices | Replace | BUILT-OFF | alert/toast renderer: `notifications` off; the patched alert and toast drew on the panel 10-25 (surfaces-p2-result), this message never raised | UNSEEN |
| 199 | Boot popup | Replace | NOT BUILT | no boot-popup patch in `payload/` | n/a |
| 204 | Auto power-off countdown | Replace | BUILT-UNTESTED | `Alert.qml` countdown variant; never seen | UNSEEN |
| 205 | Sleep timer and timer power-off | Replace | LIVE | Sleep timer tile in the quick panel (`40-gear-tap-home.jpg`); the end-of-timer alert is the `notifications` path (off) | MATCHES (tile) |
| 206 | OLED Pixel Cleaning prompts | Replace | BUILT-OFF | alert/toast renderer: `notifications` off; the patched alert and toast drew on the panel 10-25 (surfaces-p2-result), this message never raised | UNSEEN |
| 207 | Thermal and fan shutdown | Replace | BUILT-OFF | alert/toast renderer: `notifications` off; the patched alert and toast drew on the panel 10-25 (surfaces-p2-result), this message never raised | UNSEEN |
| 213 | Game Dashboard | Replace | NOT BUILT | no game card in `QuickPanel.qml`; PLAN item only | n/a |
| 214 | Game Optimizer | Restyle | BUILT-UNTESTED | in `payload/webapps/apps.txt`; its comment: web theme "not run on the set yet" | UNSEEN |
| 225 | DLNA remote control | Replace | BUILT-OFF | alert/toast renderer: `notifications` off; the patched alert and toast drew on the panel 10-25 (surfaces-p2-result), this message never raised | UNSEEN |
| 227 | Mobile pairing PIN popup | Replace | NOT BUILT | no mobile-pairing popup code in `payload/` | n/a |
| 238 | LG Channels | Off | BUILT-OFF | App-key setting built (`PageAppKey`, Settings › Remote & pointer); default keeps LG's target, so the key still opens LG Channels | n/a |
| 256 | Network and terms overlay | Restyle | NOT BUILT | not in `apps.txt`; native Wi-Fi and IP pages cover joining a network | n/a |
| 258 | Terms, privacy and country-change prompts | Replace | BUILT-OFF | alert/toast renderer: `notifications` off; the patched alert and toast drew on the panel 10-25 (surfaces-p2-result), this message never raised | UNSEEN |
| 259 | Server notice | Replace | BUILT-OFF | alert/toast renderer: `notifications` off; the patched alert and toast drew on the panel 10-25 (surfaces-p2-result), this message never raised | UNSEEN |
| 272 | Hidden key combinations | Off | NOT BUILT | no swallow of the accident-prone combinations in `lumatv.js` | n/a |
| 283 | Accessibility | Restyle | LIVE | native Accessibility pages seen on the set (round8 `31-system-accessibility.jpg`, `32-access-*`) | MATCHES (Settings look) |
| 286 | Learn Remote Control | Restyle | NOT BUILT | not in `apps.txt` | n/a |

## Never (cannot appear on this set) (23)

| Row | Surface | Plan | Status | Evidence | Look |
|---|---|---|---|---|---|
| 71 | Long-press progress ring | Replace | BUILT-UNTESTED | hold ring in `Splash.qml` (Home dock hold); LG's ring never shows here | UNSEEN |
| 124 | Toast: light | Replace | BUILT-UNTESTED | light toast not separately built or seen | UNSEEN |
| 171 | Live Pick and recording indicators | Off | LIVE | already off on the set (catalogue 2026-10-23: `livepick` absent, `dvrReady` false) | n/a |
| 175 | My Channels wedge | Off | LIVE | LG Home gone with the Home stub (homestub-result) | n/a |
| 201 | Video screensaver container | Off | LIVE | already off (`lgchannels.isVideoScreenSaverEnabled` false, catalogue) | n/a |
| 215 | Games hub | Off | LIVE | LG Home gone with the Home stub | n/a |
| 234 | Home hero banner, sponsored shelves and usage logging | Off | LIVE | LG Home gone with the Home stub (homestub-result) | n/a |
| 235 | Q-cards | Off | LIVE | LG Home gone with the Home stub (homestub-result) | n/a |
| 236 | New & Hot and Music card | Off | LIVE | LG Home gone with the Home stub (homestub-result) | n/a |
| 237 | Premium | Off | LIVE | LG Home gone with the Home stub (homestub-result) | n/a |
| 239 | LG Movies & TV | Off | LIVE | `apps` group removed LG Movies & TV (declutter-result) | n/a |
| 240 | Sports (My Team) and sports alarm | Off | LIVE | `push` and `ads` groups: nudges and sports services masked (declutter-result) | n/a |
| 241 | Nudges | Off | LIVE | `push` and `ads` groups: nudges and sports services masked (declutter-result) | n/a |
| 242 | Live Plus (content recognition) overlays | Off | LIVE | `ads` and `settings` groups (declutter-result 10-25 verified) | n/a |
| 243 | Ad and shopping overlays | Off | LIVE | `ads` and `settings` groups (declutter-result 10-25 verified) | n/a |
| 244 | Ad banner card and ad manager | Off | LIVE | `ads` and `settings` groups (declutter-result 10-25 verified) | n/a |
| 245 | Store promotion strip | Off | LIVE | `ads` and `settings` groups (declutter-result 10-25 verified) | n/a |
| 246 | Home Office and Remote PC | Off | LIVE | no LumaHome tile; the apps grid hides LG system apps (`kit/Apps.qml`) | n/a |
| 247 | LG Buddy and Copilot link | Off | LIVE | no LumaHome tile; the apps grid hides LG system apps (`kit/Apps.qml`) | n/a |
| 248 | Viewing-data messages | Off | LIVE | `ads` and `settings` groups (declutter-result 10-25 verified) | n/a |
| 273 | LG Remote Service session | Replace | BUILT-OFF | alert/toast renderer: `notifications` off; the patched alert and toast drew on the panel 10-25 (surfaces-p2-result), this message never raised | UNSEEN |
| 274 | System recovery messages | Replace | BUILT-OFF | alert/toast renderer: `notifications` off; the patched alert and toast drew on the panel 10-25 (surfaces-p2-result), this message never raised | UNSEEN |
| 289 | AI Caption | Replace | BUILT-OFF | alert/toast renderer: `notifications` off; the patched alert and toast drew on the panel 10-25 (surfaces-p2-result), this message never raised | UNSEEN |

## Built and off: switch on, and what the next TV check looks at (daily and sometimes)

Every surface here is built to its mock and was run offline only (host renders in `restyle-fixes/unbuilt-screens/`, host tests `tests/test_unbuilt_screens.py`, `tests/test_cover_splash_patch.py`). Each is off until the TV check; the way back is the same switch. A toggle is in Settings > Remote & pointer (Buttons tab), or `lumatv theme set '{...}'` on the TV.

| Rows | Switch on | Switch off | What the TV check looks at |
|---|---|---|---|
| 69, 70 | Home launches need nothing. For LG's own splash and spinner: `lumatv splash on` (rebuilds the shadow module and restarts LG's compositor once: the picture flashes) | `lumatv splash off` | Launch Netflix from a Home tile: LumaTV's launch screen from the OK release to the app (180 px picture, name, ring, "Opening", wallpaper dimmed). With `splash` on: open an app with no art (Homebrew Channel, HyperHDR) by a tile and by a remote key: the card, not the grey with eight dots; Netflix keeps its own art. `lumatv splash status` says `patched: true` and `lumatv log` has no "the splash patch was refused". Spinner: `luna-send -n 1 luna://com.webos.surfacemanager/showSpinner '{"show":true}'` dims the screen, shows a ring and, a second later, "Loading"; `{"show":false}` ends it; keys stay blocked while it shows |
| 72 | Settings > Remote & pointer > Recent apps on Back, or `{"remote":{"recents":true}}` | the same toggle | Open Netflix and Plex, go Home. Back (dock to cards), Back again: the switcher over dimmed Home, the newest app first. Right moves, OK switches (launch screen, then the app), a held OK (about 0.7 s) closes that app (`luna-send -n 1 luna://com.webos.applicationManager/running '{}'` no longer lists it) and its frame goes, Back returns to Home. With no app open Back is LG's. First frame within 50 ms of the key, no frame over 20 ms |
| 79 | Settings > Remote & pointer > Quick Access for the number keys, or `{"remote":{"quickAccess":{"on":true,"slots":{}}}}` | the same toggle | On Home press 2: Netflix opens and a toast "2 · Netflix / Opening." sits top right over the launch screen for about 2 s. Settings > Remote & pointer > Number key 5 > an app; press 5: that app, not the fifth dock tile; "The dock's tile" puts it back. The question "Nothing on N yet. Assign it?" appears only for a number with no dock tile, which this set's 23-tile dock never has: checked offline only |
| 95, 128 | `lumatv notifications on` | `lumatv notifications off` | Power a device on HDMI 3 (the PS5): a sheet top right, "<name> connected / On HDMI 3. Switch to it?", "Switch to HDMI 3" focused and "Stay here", a bar that runs out in 10 s; OK on Switch opens HDMI 3, Stay or 10 s leaves the input; opaque over Home and over video; no text of the page shows through |
| 122 | `lumatv notifications on` | `lumatv notifications off` | `luna-send -n 1 luna://com.webos.notification/createToast '{"sourceId":"com.webos.app.home","message":"This function is not available."}'`: a text-only pill top right, not LG's grey bottom bar; with `<br>` a title over a dimmer line; also over Netflix. `python3 docs/research/perf/gate.py --surfaces toast,alert`: first frame within 50 ms, p90 render under 3 ms |
| 125 | `lumatv notifications on` | `lumatv notifications off` | An alert with two buttons (the gate's `kb.py` raises one): the card centred, the first button focused, the page's text not showing through; a modal one hides the page, ignores Back and Home, and OK runs the button's own call |
| 146 | Settings > Remote & pointer > Quick panel tiles: choices and editing, or `{"remote":{"quickPanelEdit":true}}` | the same toggle | Gear tap: the line "Hold OK on a tile to edit · Back closes" under All settings. OK on Sleep timer: its choices beside the panel with the time each would turn the TV off; pick 10 min and `tvReservationAgent` `timer/sleep/getRemainTime` reads 10; Back closes only the card (Picture mode the same). Hold OK on a tile: "Edit tiles", that tile raised; Up and Down move it, OK drops it, Right then OK takes a tile off, Add a tile lists the ones that are off; add OLED pixel brightness: its tile steps by 5 and OK opens a slider card. The order survives Back and a restart (`quickPanel.tiles` in theme.json). The hold also works over an app |
| 198 | `lumatv cover on` | `lumatv cover off` | Reboot cold and watch from about 14 s: a dark page with the first app's tile, its name at 40 px, a ring with "Starting where you left off" (HDMI 2) and the brand mark lower left, not LG's 120 px name and "Loading...". The first app still opens, keys are still swallowed until it has a window. `lumatv cover status` says `patched: true`, `lumatv log` has no "the cover patch was refused" |
| 202 | `lumatv standbyscreen off` | `lumatv standbyscreen on` (puts the old mode back) | Switch the TV off: the panel is dark within about 3 s of the power-off logo, no art and no clock, and the wake is as instant as before; `lumatv standbyscreen status` says `on: true`, Settings > Home & look > Standby shows Always Ready off |

**TV check 2026-10-28** (`48ee380` on the set, LumaTV's compositor live, the receiver muted; captures in `restyle-fixes/unbuilt-screens/tv/`): 79 passed and is on (`remote.quickAccess.on`: the toast for 2 and for an owner's choice of the store for 5). 146 passed and is on (`remote.quickPanelEdit`: the hint line, the Sleep timer card, the Picture mode card, a held OK, move and drop, add a tile, the OLED slider card; one OK on "Add a tile" added a tile at once in 2 of 3 presses). 69 and 70: Home launches draw LumaTV's card (seen); `splash` patches LG's compositor, which is not drawn while LumaTV's is live (`showSpinner` is an unknown method), so it could not be checked and stays off. 198: the cover shows only in a cold boot; no reboot was made (the runbook leaves it to Nitin), so `cover` stays off. 202, report only: `lumatv standbyscreen status` says `on:false, mode:alwaysReady`, so Always Ready is still in force; the switch changes standby behaviour and is Nitin's. 72, 95, 122, 125 and 128 were not in this check.

`lumatv revert` clears the `cover` and `splash` flags with the shadow module. Decisions followed over the mocks: Lucent v2's rules (opaque card, a backdrop that hides the page behind a question) over mock 06's dimmed page; "a digit acts on press" (mock 03, shipped) over mock 31's "hold".

## Notes on the evidence

- Alert, toast and input-popup rows share one renderer (`tools/notification_patch.py`, `payload/notification`): built, seen once on the panel on 2026-10-25 (surfaces-p2-result, slot 2), off in every `lumatv surfaces` read since. Each message row inherits that status; no individual message was raised on the set.
- Under LumaTV's compositor, live on the set since 2026-10-28 05:39 (`compositor-gate9-result`), the same screens run in the shell client (`panels/Notify.qml`, quick panel, picker, volume, saver); LG's own views (its splash and spinner, its notification view) are not drawn while it is live. Live TV does not come to the front under it (found 2026-10-28: SAM keeps Home foreground while the app's card is listed as mapped).
- Latest TV captures used: `round9/tv-2026-10-26`, `round9/r11-2026-10-26`, `lg-restyles/tv/banners-r18` (10-27), `settings-ux/tv-walk-2026-10-27`, `round8/tv-2026-10-25`; from 2026-10-28: `restyle-fixes/banners/tv`, `restyle-fixes/search/tv`, `restyle-fixes/unbuilt-screens/tv`, `settings-ux/tv-r19-2026-10-28`, `store`, `update-block`.
- Open for Nitin, not repo work: Home Assistant link (voice bar, At-home lane), Always Ready off, the no-signal photo setting (`general.noSignalScreenSaver`, which hides mock 18's block), whether to switch on `notifications`, Live TV under LumaTV's compositor.
