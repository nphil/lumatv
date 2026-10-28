# Surface status: LG screens and menus replaced, restyled or switched off

State on 2026-10-27, from the repository only (code, CHANGELOG, HANDOFF, PLAN, `docs/research/*-result.md`, TV captures and logs). The TV was not touched. Scope: the 111 rows of `docs/SURFACES.md` whose plan is Replace (62), Restyle (25) or Off (24); the 67 Keep rows are out of scope. Row numbers are SURFACES.md line numbers.

## Answer

No. 35 of 111 are live on the TV. Of the 28 daily and sometimes surfaces in scope, 12 are live (Home, volume, mute, power-off logo, quick panel and its tiles, input picker, Apps library, Settings, keyboard keys, saver); the other 16 are not, and the LG screens a viewer still meets are listed below.

## Status legend

- **LIVE**: built, on by default on the TV and seen working there. For Off rows: the off state is in place and was checked by the declutter or Home-stub runs (no picture exists of an absence).
- **BUILT-OFF**: built and deployed to the set, but switched off by default (`lumatv surfaces`: `notifications`, `inputcommon`, `voice` are off), or reachable only under LumaTV's compositor, which is not live (frame timing fails, `compositor-gate9-result`; Nitin parked the move).
- **BUILT-UNTESTED**: code exists, the surface was never seen on the TV.
- **NOT BUILT**: no code, or the plan was dropped.
- **BLOCKED**: built but cannot run without something outside the repo (here: Nitin's Home Assistant link).
- **Look** judges a built surface against the approved mock (`docs/mocks`) and Lucent's TV profile (Manrope, glass and ink tokens from `payload/theme/theme.json`, focus lift, 4.5:1 text, 200 % text). **MATCHES** and **GAPS** rest on a capture viewed for this note; **UNSEEN** means no TV capture or kept render exists. Off, not-built and dropped rows have no look.

## Counts

| Status | Daily | Sometimes | Rare | Never | Total |
|---|---|---|---|---|---|
| LIVE | 3 | 9 | 5 | 18 | 35 |
| BUILT-OFF | 1 | 4 | 25 | 3 | 33 |
| BUILT-UNTESTED | 1 | 5 | 21 | 2 | 29 |
| NOT BUILT | 1 | 3 | 9 | 0 | 13 |
| BLOCKED | 0 | 1 | 0 | 0 | 1 |
| **Total** | **6** | **22** | **60** | **23** | **111** |

Look, built Replace and Restyle rows (76): MATCHES 8, GAPS 12, UNSEEN 56.

## Still LG, met in normal use (daily and sometimes, not LIVE)

| How often | What a viewer meets | Why |
|---|---|---|
| daily | LG's grey launch splash with the dots on app launches | LumaTV's splash exists only for a Home tile launch and was never seen; LG's compositor view is not swapped (69) |
| daily | LG's grey toast bar, bottom centre ("not available" messages and the rest) | notification view patch is off by default (122) |
| daily | LG's Always Ready art and clock in standby | Nitin's "panel dark in standby" is not applied; the setting is still on (202) |
| sometimes | LG's grey input banner on every HDMI switch and boot, and LG's No Signal screen | `inputcommon` restyle is off (88, 89) |
| sometimes | LG's alerts, and the "device connected, switch?" popup | notification view patch is off or unseen (125, 128, 95) |
| sometimes | LG's Search app with LG's shelves; only the keyboard keys are restyled | `voice` restyle is off (182) |
| sometimes | LG's launch spinner, Quick Access toasts, boot animation and startup cover | not swapped or never seen (70, 79, 196, 198) |
| sometimes | no app switcher at all | not built; LG's lived in the removed Flutter Home (72) |
| sometimes | no quick-panel slider popup or edit mode | not built (146) |
| sometimes | voice bar | needs the Home Assistant link (183) |

Also LG-side but rarer: the Home Hub tile in the apps grid (86), LG Channels on its remote key (238), the LG sub-apps (wizards, scheduler, user guide) until the web theme is run on the set.

## Look gaps, most visible first

| Surface | What differs | File |
|---|---|---|
| Input INFO card and channel INFO card (88, 165) | fixed offline, not yet on the set (it showed grey `#575E66`): every file reads LumaStyle, checked with LG's stock sheet in memory; the Guide chip is now a sheet pill (`restyle-fixes/banners/`) | `tools/inputcommon_patch.py` |
| No-signal screen (89) | rebuilt offline to mock 18 on an HDMI input (glyph, input name, title, line, Inputs and Home); not yet on the set | `tools/inputcommon_patch.py` (Screensaver.qml), `payload/inputcommon/` |
| Search app and keyboard (181, 182) | keys are Lucent, but field, chips, genre photo cards, cloud suggestions and the "Hold the AI button" hint are LG's; layout is not mock 09 | `tools/voice_patch.py`, `tools/maliit_patch.py` |
| Volume bar (110) | bottom-centre bar lands on Netflix subtitle rows; bar stays at Nitin's bottom-centre choice, a clear spot needs his pick | `panels/VolumeOsd.qml` |
| Quick panel (145) | fixed offline 16abb15: docked right at mock 10's place, steppers beside their words, no "LG" button; Ambient tile still needs the link | `panels/QuickPanel.qml` |
| Apps library (68) | rail wrap and stale clock fixed offline (34bfc16), rendered clean at 100 and 200 % text, not re-seen on the set | `panels/AppsGrid.qml` |
| Settings (149, 150) | selected tab and segment marks now an accent plate with a wider mark (16abb15, offline); disabled rows measure 6:1 offline | `kit/LuSegmented.qml`, `kit/LuRow.qml` |
| Notification drawer (133) | footer text fix 0a2d782 renders clean; head, times and body fixed at 200 % text and contrast (16abb15) | `panels/Drawer.qml` |
| LG Settings app (151) | themed, but LG's two-pane layout and focus pill | `payload/webapps/lumatv.css` |

## Daily (6)

| Row | Surface | Plan | Status | Evidence | Look |
|---|---|---|---|---|---|
| 66 | Home | Replace | LIVE | homestub-result; round9/tv-2026-10-26/10-home-after-reboot.png | MATCHES (glass, Manrope, wallpaper tint, dock); the At-home lane shows a placeholder until the Home Assistant link is made |
| 69 | Launch splash | Replace | BUILT-UNTESTED | `panels/Splash.qml` wired to a Home tile launch (`Layers.launchSplash`); no capture; LG's compositor splash is not swapped (no Shadow-module patch) | UNSEEN |
| 110 | Volume OSD | Replace | LIVE | round8-result (a) PASS; round9/r11 `mutefix-*` captures | GAPS: bottom-centre bar lands on Netflix subtitle rows (`mutefix-format-unmuted-vol0.png`; Main owns, round9-result) - `panels/VolumeOsd.qml`; otherwise matches mock 11 |
| 122 | Toast: standard | Replace | BUILT-OFF | registry 10-26: `notifications` off; the patched alert and toast drew on the panel 10-25 (surfaces-p2-result), this message never raised | UNSEEN (no image kept; offline render only) |
| 197 | Power-off logo | Replace | LIVE | `logo-events` 10-25/26: `power-off logo 1 launch`, `answered tvpowerd` (round9/tv-2026-10-26/reboot) | UNSEEN (panel is dark; nothing can capture it) |
| 202 | Always Ready and Gallery+ | Replace | NOT BUILT | PLAN: Off, decided by Nitin 2026-10-23; not applied: `lifeOnScreenMode` still `alwaysReady` (saver-own-result 10-26) and the Always Ready switch reads on in the 10-27 walk (`home-look-standby.png`) | n/a |

## Sometimes (22)

| Row | Surface | Plan | Status | Evidence | Look |
|---|---|---|---|---|---|
| 68 | Apps tile (LG Content Store) | Replace | LIVE | round8-result (e) PASS; `round8/tv-2026-10-25/45-apps-grid.jpg` | GAPS: rail rows wrapped into the next row and the bar clock was stale on the set; fixed offline (34bfc16), not re-seen - `panels/AppsGrid.qml` |
| 70 | Launch spinner and infinite spinner | Replace | BUILT-UNTESTED | `Splash.qml` spinner mode and the Home launch spinner in `LumaHome.qml`; never seen; LG's spinner view not swapped | UNSEEN |
| 72 | Recents (app switcher) | Replace | NOT BUILT | no switcher in `payload/` (LG's lived in the Flutter Home the stub removed) | n/a |
| 79 | Quick Access (number keys) | Replace | BUILT-UNTESTED | `LumaHome.numberKey`, `Dock.openTile`; shell-host-result: not run on the set | UNSEEN |
| 85 | Input picker | Replace | LIVE | round8-result (d): Input key (evdev 241) opens it; gates r12-r14 over Home, HDMI and Netflix (round9-result) | MATCHES in `round8/tv-2026-10-25/41-key241.jpg` (glass sheet, Manrope, icons, eARC mark); later layout changes (round 15 cached texture) not re-captured |
| 88 | HDMI input banner | Restyle | BUILT-OFF | `inputcommon` off; round 18 on the TV 10-27: dark glass pill 1.68-6.37 s (lg-restyles-result); stock grey pill still shown at boot (`reboot/boot-002-up16.33.png`) | GAPS: pill matches; INFO card was LG grey `#575E66` (`banners-r18/info-1.png`), fixed 3ba7b16/95a66db and re-checked offline with LG's stock sheet in memory (`restyle-fixes/banners/after/detail.png`), TV check pending - `tools/inputcommon_patch.py` (DetailInfoBanner.qml) |
| 89 | No-signal screen | Restyle | BUILT-OFF | `inputcommon` off; seen round 9 and round 18 (7 s frame, "unchanged, dim words") | Rebuilt offline to mock 18 (`restyle-fixes/banners/after/nosignal.png`): glyph, input name, title, line, Inputs (focused) and Home on an HDMI input with nothing connected; the tuner's no signal, LG's photo mode and LG's static text mode keep LG's block; TV check pending - `tools/inputcommon_patch.py` (Screensaver.qml) |
| 95 | HDMI or CEC device connected | Replace | BUILT-UNTESTED | `AdvancedAlertDelegate.qml` in the notification view; only toast and alert were seen on the set | UNSEEN |
| 111 | Mute bubble | Replace | LIVE | round8-result (a) mute; round9 `mutefix3` (mark returns after restart); r11 `saver-own-step2.png` | MATCHES (slashed-speaker disc, hops every 10 s, clear of subtitles after 00dbca3) |
| 125 | Alert: standard and modal | Replace | BUILT-OFF | registry: `notifications` off; alert drew on the panel 10-25 (surfaces-p2-result) | UNSEEN (no image kept) |
| 128 | Input alert | Replace | BUILT-UNTESTED | `AdvancedAlertDelegate.qml` (mock 18); never seen | UNSEEN |
| 145 | Quick panel | Replace | LIVE | round8-result (d): gear tap opens it; `40-gear-tap-home.jpg` | GAPS vs mock 10: docked left (mock right; no Decided entry found), no second lines (minimal-text rule), extra "LG" button, no Ambient tile until the plugin is linked - `panels/QuickPanel.qml` |
| 146 | Quick panel popups and edit mode | Replace | NOT BUILT | `QuickPanel.qml` has steppers only; no popup or edit mode in `panels/` | n/a |
| 147 | Quick panel tile set | Replace | LIVE | `40-gear-tap-home.jpg`: Picture mode, Receiver, Sleep timer, Wi-Fi, AI access, All settings | MATCHES |
| 149 | Settings | Replace | LIVE | round8-result (b); `settings-ux/tv-walk-2026-10-27` (27 captures) | GAPS: selected tab and segment marks are faint (`picture-mode.png`: Mode tab, Off segment); disabled rows (Aspect ratio, Just Scan) read dim, TV contrast not measured (offline audit passes) - `kit/LuSegmented.qml`, `kit/LuRow.qml` |
| 150 | Settings sections without a mock | Replace | LIVE | tv-walk-2026-10-27 (Inputs, Network, Ambient light, Home & look, Remote, AI access, System) | MATCHES; Ambient light reads Off and Home Assistant "Not switched on" until Nitin links them |
| 181 | On-screen keyboard | Restyle | LIVE | `keys` on; round9-result 10-26 PASS (`lg-restyles/tv/keys-final-on.png`); `keyboard` motion patch off | GAPS: key caps are Lucent (dark, lavender Search); the Search app's field, chips and backdrop around them stay LG grey and the layout is LG's, not mock 09 - `tools/maliit_patch.py`, `tools/voice_patch.py` |
| 182 | Search and voice results | Restyle | BUILT-OFF | `voice` off; round9-result 10-26 PASS; own Search panel (`panels/Search.qml`) never seen on the TV | GAPS: restyled app keeps LG's shelves: genre photo cards, trending chips, "Hold the AI button" hint (`round9/r11/voice-search.png`, `voice-own.png`) - `tools/voice_patch.py` |
| 183 | Voice listening bar | Restyle | BLOCKED | `panels/VoiceBar.qml` built; needs Nitin: Home Assistant token and the "Search by voice" switch (voice-result) | UNSEEN |
| 196 | Boot logo | Replace | BUILT-UNTESTED | BootLogo built, `logos` on; the 10-26 cold boot capture starts black at 14.6 s with no logo (`reboot/boot-000-up14.60.png`) | UNSEEN |
| 198 | Startup cover | Restyle | NOT BUILT | no CoverUI patch in `payload/` or `tools/` | n/a |
| 200 | Screensaver | Replace | LIVE | saver-own-result 10-26 PASS (`r11/saver-own-step2.png`); `screensaver.own` on | MATCHES for the clock (Manrope, dim ink, burn-in mover); art and video modes not seen on the panel; LG's Flutter clock still draws in LG's Screen Saver state (`round8/54-saver-whose.jpg`) |

## Rare (60)

| Row | Surface | Plan | Status | Evidence | Look |
|---|---|---|---|---|---|
| 73 | Multi View window frames | Restyle | NOT BUILT | dropped: PLAN Surfaces table keeps LG's Multi View (Nitin 2026-10-23) | n/a |
| 74 | Multi View picker | Restyle | BUILT-UNTESTED | in `payload/webapps/apps.txt`; its comment: web theme "not run on the set yet" | UNSEEN |
| 77 | Pointer right-click menu | Replace | NOT BUILT | shell-slice-result: pointer right-button menu not done; no code | n/a |
| 78 | "More Actions" panel (on-screen remote) | Replace | BUILT-UNTESTED | `panels/ScreenRemote.qml`, Home only (the ScreenRemote key); over apps not wired (shell-host-result) | UNSEEN |
| 86 | Home Hub | Off | NOT BUILT | no `keyaction.inputHub` repoint in `tools/`; Home Hub tile is in the apps grid (`45-apps-grid.jpg`) | n/a |
| 90 | Banner position editor | Restyle | BUILT-OFF | `inputcommon` off; rendered Lucent in round 9 | Rendered again offline in both materials, no overlap at 100 and 200 % text (`restyle-fixes/banners/after/banner-edit.png`); not seen on the set |
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
| 164 | Channel banner | Restyle | BUILT-OFF | `inputcommon` off; round 9 and 18 PASS (`banners-r18/channel-ok.png`) | The "Guide" chip below was flat LG grey: now a sheet pill with a ring on focus at the banner's inset (`restyle-fixes/banners/after/channel-guide.png`), checked with LG's stock sheet in memory; TV check pending - `tools/inputcommon_patch.py` |
| 165 | Channel detail banner | Restyle | BUILT-OFF | `inputcommon` off; channel INFO seen round 9, `banners-r18/channel-info.png` | Shares the INFO card with 88 (grey on the TV, fixed 3ba7b16, re-checked offline); TV check pending |
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

## Notes on the evidence

- Alert, toast and input-popup rows share one renderer (`tools/notification_patch.py`, `payload/notification`): built, seen once on the panel on 2026-10-25 (surfaces-p2-result, slot 2), off in every `lumatv surfaces` read since. Each message row inherits that status; no individual message was raised on the set.
- Under LumaTV's compositor the same screens run in the shell client (`panels/Notify.qml`, quick panel, picker, volume, saver): the shell gate passed 11 of 11 and the keys gate passed (`compositor-surfaces-result`, slot 10-27), but frame timing fails, so LG's compositor stays live.
- Latest TV captures used: `round9/tv-2026-10-26`, `round9/r11-2026-10-26`, `lg-restyles/tv/banners-r18` (10-27), `settings-ux/tv-walk-2026-10-27`, `round8/tv-2026-10-25`. The 10-27 Settings walk and banner run were on package 9efedab; later commits (3ba7b16, 95a66db, 34bfc16, 0a2d782) have not been on the set.
- Open for Nitin, not repo work: Home Assistant link (voice bar, At-home lane), Always Ready off, the dock side of the quick panel, whether to switch on `notifications`, `inputcommon` and `voice`.
