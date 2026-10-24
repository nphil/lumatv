# Shell mocks: batches 2 and 3

## Resume here (batch 3)

Status: complete. 72 shots in `shots/` (11 batch 1, 31 batch 2, 30 batch 3), all in the `index.html`
gallery with captions and the live viewer. Nothing left to build; nothing applied to the TV.

If more screens are asked for: copy the nearest HTML file, add it to `SHOTS` in `render.mjs` and in
`index.html`, run `cd /data/home/lumaglass/docs/mocks && node render.mjs <prefix>` and look at the PNG at
1:1. Variants are query strings on one file (`?theme=light`, `?v=…`, `?tab=…`). Detector:
`/data/home/.agents/skills/impeccable/scripts/impeccable detect --json docs/mocks/lucent.css docs/mocks/*.html`
from `/data/home/lumaglass`. Remaining findings are intentional: `dark-glow` (the accent focus halo),
`nested-cards` (sheet → well → raised control), and `low-contrast` on 27 (standby text is dimmed to 42 % on
purpose for the panel; the detector measures against the palette canvas, the real ground is #000).
Gotcha learned twice: a state class on a container must not reuse a kit class name (`art`, `slider`
clashed); state classes are now `artmode`, `sliderpop`.

### Files (batch 3)

| Screen | File (variants) | Shots |
|---|---|---|
| HDMI input banner family | `25-input-banner.html` (`?v=ok`, `?v=info`, `?v=update`) | 25-input-pill, 25-input-banner, 25-input-info, 25-input-update |
| Power-off logo, startup cover | `26-power-off.html` (`?v=cover`) | 26-power-off-logo, 26-startup-cover |
| Standby (Always Ready) | `27-standby.html` (`?v=art`, `?v=off`) | 27-standby-clock, -art, -off |
| Voice bar, mic indicator | `28-voice.html` (`?v=heard`, `?v=results`, `?v=mic`) | 28-voice-listening, -heard, -results, 28-mic-indicator |
| Quick-panel popups, edit | `29-quick-panel-popups.html` (`?v=picker`, `?v=edit`, `?v=add`) | 29-qp-slider, -picker, -edit, -add |
| Settings sections | `30-settings-inputs.html`, `-network`, `-ambient`, `-home`, `-system` | one shot each |
| Quick Access | `31-quick-access.html` (`?v=assign`) | 31-quick-access-toast, -assign |
| Pairing prompt | `32-pairing-prompt.html` | 32-pairing-prompt |
| Pointer menu | `33-pointer-menu.html` | 33-pointer-menu |
| Game Dashboard | `34-game-dashboard.html` | 34-game-dashboard |
| App prompts, advanced toast | `35-app-prompts.html` (`?v=unavailable`, `?v=toast`) | 35-app-update-required, 35-app-not-available, 35-toast-advanced |

Shared: `lucent.css` (batch-3 kit section: `.ipill/.ibanner/.facts/.fact/.badge`, `.offlogo/.cover`,
`.standby-*`, `.vbar/.vresults/.micind`, `.qpop/.srow/.opt`, `.qpanel.edit` + `.grip`, `.dialog.pair` +
`.who-row/.grants`, `.pmenu/.cursor`, `.gdash/.gtile`, `.toast.link`, generic `.src`), `render.mjs` (30 new
entries, checks extended), `index.html` (30 new entries), `img/game.jpg` (generated game frame, provenance
embedded).

### Rationale, one line each (batch 3)

- **25 Input banner family**: one lower-left pill on every switch (device glyph, port · CEC name, signal in
  one line); OK grows the same pill into three actions instead of opening a second surface; INFO is a card
  of eight facts with two actions; a background update is a small opposite-corner badge, never a dialog.
- **26 Power-off / cover**: the wordmark at 72 % on black for the ~3 s power-off still; the cover names what is
  starting (HDMI 2 · Denon) with one arc and a corner mark, replacing LG's bare "Loading…".
- **27 Standby**: three answers to Needs-Nitin #1 side by side: clock block at 42 % drifting with glance
  facts, the art frame at half, or nothing but the set's own standby light.
- **28 Voice**: a lower-left bar, one accent ring around the mic (no 485-frame animation), words in quotes
  when heard; a one-word command acts, anything else lists three local choices above the bar; the mic
  indicator is a small disc with a green dot.
- **29 Quick-panel popups**: values open beside the panel (slider with marks, picker with the resulting
  time) while the source tile stays raised; edit mode swaps controls for move/remove on every tile and an
  "Add a tile" row; the add list sits in the same popup slot.
- **30 Settings sections**: the Live-list page pattern with real items from settings-map.md; each page fits
  without scrolling (two tabs hold the rest), and every page ends in the LG escape row where LG keeps
  something deep.
- **31 Quick Access**: a toast with the number plate on launch; an empty slot asks "Nothing on 4 yet.
  Assign it?" with five picks and Not now.
- **32 Pairing prompt**: who (name, host, address, first time), what it can do, see and cannot; Don't allow
  focused; "Only this remote can answer" (Principle 0).
- **33 Pointer menu**: a 300 px menu beside the cursor with Back, Home, LumaTV menu, then Inputs and Quick
  settings; the hover state is the pointer's.
- **34 Game Dashboard**: a bottom-centre strip over the game (tinted, no blur): six live stats, then genre,
  dark room and black stabiliser; All settings in the head.
- **35 App prompts / advanced toast**: sam's three messages in plain words with the safe action focused
  and "updates only when you choose"; the advanced toast is icon + a Play link that a pointer click runs.

### Progress log (batch 3)

- 20:05 BATCH2.md → BATCH3.md with a resume section; kit section appended; 14 HTML files written; first
  render 30 shots.
- 20:20 fixes: fact/stat labels 21 px; five Settings pages trimmed to fit (removed a row or label each);
  info banner 1240 wide; slider popup and standby art had class clashes (`slider`, `art`) → renamed;
  game dashboard 1700 wide, three tiles; pairing and assign copy shortened; designer commentary removed
  from the update prompt.
- 20:30 gallery entries added; full re-render (72 shots); detector run (two mechanical fixes); live
  navigation checked.

## Batch 2 (complete)

### Resume notes kept from batch 2

Status: complete. 42 shots in `shots/` (11 from batch 1, 31 from this batch), all in the `index.html`
gallery with captions and the live viewer. Nothing left to build; nothing applied to the TV.

If more screens are asked for: copy the nearest HTML file, add it to `SHOTS` in `render.mjs` and in
`index.html`, run `cd /data/home/lumaglass/docs/mocks && node render.mjs <prefix>` and look at the PNG
at 1:1. Variants are query strings on one file: `?theme=light`, `?v=…`, `?tab=…` (see the list below).
Detector: `/data/home/.agents/skills/impeccable/scripts/impeccable detect --json docs/mocks/lucent.css docs/mocks/*.html`
(from `/data/home/lumaglass`). Remaining findings are intentional: `dark-glow` is the accent focus halo
Nitin chose; `nested-cards` is Lucent's sheet → well → raised-control stacking.

### Files (batch 2)

| Screen | File (variants) | Shots |
|---|---|---|
| Quick panel | `10-quick-panel.html` (`?theme=light`, `?v=video`) | 10-quick-panel, -light, -video |
| Volume OSD | `11-volume-osd.html` (`?v=abs`, `?v=muted`, `?v=offline`) | 11-volume-osd, -abs, -muted, -offline |
| All-apps library | `12-all-apps.html` (`?theme=light`, `?v=menu`) | 12-all-apps, -light, -menu |
| Notification drawer | `13-notifications.html` | 13-notifications |
| App switcher | `14-recents.html` | 14-recents |
| Splash, ring, spinner | `15-launch-splash.html` (`?v=ring`, `?v=spinner`) | 15-launch-splash, 15-long-press-ring, 15-spinner |
| Context menu | `16-context-menu.html` | 16-context-menu |
| Power menu | `17-power-menu.html` (`?v=countdown`) | 17-power-menu, 17-power-countdown |
| No signal, device connected | `18-no-signal.html` (`?v=connected`) | 18-no-signal, 18-device-connected |
| PIN prompt | `19-pin-prompt.html` | 19-pin-prompt |
| Channel banner | `20-channel-banner.html` | 20-channel-banner |
| Screensaver | `21-screensaver.html` (`?v=art`) | 21-screensaver-clock, -art |
| Bluetooth, update | `22-bluetooth-update.html` (`?v=update`) | 22-bluetooth-pair, 22-update-prompt |
| Boot | `23-boot.html` | 23-boot |
| Receiver settings | `24-settings-receiver.html` (`?tab=levels|audyssey|zones`) | 24-settings-receiver, -levels, -audyssey, -zones |

Shared: `lucent.css` (kit + depth step + over-video rule), `mock.js` (`?v=` → `data-v`), `render.mjs`
(31 new entries, overflow checks extended to the new text classes), `index.html` (31 new entries),
`img/apps/*.png` (13 tiles cropped from the Home capture), `img/art-hokusai.jpg` (public domain).
Batch-1 edits: `02-settings-sound.html` Sound out row now says it opens the receiver's controls.

### Rationale, one line each (batch 2)

- **10 Quick panel**: one right-edge pane as tall as its tiles (no empty slab), steppers so left/right
  changes picture mode and receiver volume in place; the receiver tile shows input · sound mode with
  the volume in the Denon's own dB. Over video the pane is tinted, not blurred.
- **11 Volume OSD**: lower-left safe corner away from subtitles, 640×132, the receiver's number large
  with its unit, one meta line (input · sound mode), a short track; muted swaps the number for the
  word and keeps the last level readable; "Receiver not answering" hides the value instead of showing a
  stale one (PLAN: reads older than 6 s are hidden).
- **12 Library**: the Settings rail pattern re-used for categories with live lines (counts, the apps
  inside), Hidden pinned at the bottom; 168 px tiles with names; key hints replace a toolbar; the hold-OK
  menu opens under its tile with the rest dimmed and the tile kept raised as the menu's source.
- **13 Notifications**: right drawer as tall as its content, grouped by source with counts, inline
  action where one exists (Resume Cody), Clear all in the head.
- **14 Recents**: a row of frames (live thumbnail for the playing app, a plate with the icon otherwise),
  focused frame forward, close control per frame, hints in a pill over the darker scrim.
- **15 Splash / ring / spinner**: identity on the darkened canvas with one static arc; the ring is an
  accent arc around the held tile with a label; the spinner is the same arc at 56 px with a label pill.
- **16 Context menu**: anchored beside the raised tile, the rest of Home dimmed, five verbs, Uninstall
  in danger ink, the dock-number shortcut named in the header.
- **17 Power**: four rows with plain consequences (the Denon follows over CEC; screen off keeps sound),
  the sleep timer as a segmented row; the countdown is a toast with a draining bar and Cancel.
- **18 No signal / device connected**: no-signal is black with 62 %-dim text that drifts (burn-in rule)
  and two ways out; the device popup is a wide toast over video with Switch focused.
- **19 PIN**: four well cells, a fixed 3×4 pad the D-pad walks, Clear/0/Delete in the last row.
- **20 Channel banner**: lower-left strip, channel plate, programme, time span with a bar, what is next,
  format chips; tinted glass over the broadcast.
- **21 Screensaver**: clock block at 60 % that drifts, two glance facts; art mode is a public-domain
  print in a cream matte on a dark wall, dimmed.
- **22 Bluetooth / update**: both state the consequence in one sentence; the safe choice is focused;
  the update prompt says it installs only on Nitin's choice.
- **23 Boot**: near-black canvas, a gradient mark plus the Manrope wordmark (Luma light, TV bold), one
  state line; static, T0-safe.
- **24 Receiver**: a drill-in under Sound with a back chip and an "Answering" chip; tabs Sound mode
  (power, input stepper, mode categories as tiles with the live mode, DSP styles under More, Quick
  Select), Levels (volume, dialog, sub, bass, treble, Cinema EQ), Audyssey (MultEQ, Dynamic EQ,
  reference offset, Dynamic Volume), Zones & power (Zone 2, ECO, escape to the Denon's menu).

### Kit changes (lucent.css, applied in batch 2)

- Dark depth step (Nitin, 2026-10-23): edge `#FFFFFF1F` → `#FFFFFF2B` (12 → 17 % white), raised edge
  `#FFFFFF33` → `#FFFFFF3D`, rest highlight `.07` → `.10`, rest shadow `0 24px 60px .28` →
  `0 28px 72px .40` + contact layer `0 4px 14px .20`, raised shadow `0 18px 44px .42 / 0 6px 14px .28`
  → `0 22px 52px .48 / 0 6px 14px .30`. Glass fill unchanged (pane tint still matches the live Home).
- Over video: `[data-video]` on `<html>` turns off `backdrop-filter` on every pane/overlay and swaps the
  fill for `--glass-video` (dark `rgba(22,17,32,.90)`, light `rgba(255,250,243,.94)`): tinted glass only,
  no live blur over a changing frame. Used by 10 (video variant), 11, 18 (device connected), 20.
- Focus stays as Main left it: lift + brighter glass + `--focus-halo`; no underline anywhere.
- Dialog footnotes moved from ink-3 to ink-2 (3.7:1 → 6.6:1 on the overlay).
- New kit pieces: `.qpanel/.qtile/.stepper`, `.osd`, `.lib-rail/.lib/.apps-grid/.app(.src)`, `.menu/.mi`,
  `.drawer/.ngroup/.nitem`, `.recents/.rcard`, `.splash/.arc/.ring`, `.prow`, `.toast.wide` + `.toast .bar`,
  `.nosig`, `.cells/.cell/.kp/.kpk`, `.banner`, `.ss/.ss-clock/.art`, `.boot/.wordmark`.

### Lucent proposals (not applied to the skill)

- `material.video`: the tinted no-blur reading surface for overlays over video (values above), as a
  named material beside `overlay`.
- `component.stepper`: a well tray with chevrons and a value, for left/right in-place changes on TV rows
  (quick panel, receiver input and volume).
- `component.osd`: compact lateral overlay, 640×132 at 1080p, number + unit + meta + 8 px track; states
  value / muted / unavailable.
- `component.contextMenu`: 420 px anchored overlay, header with source identity, 68 px items, danger
  item after a hairline; the source tile stays raised, the rest dims `.46`.
- `component.ring`: long-press arc, 6 px stroke, accent on ink `.18`, label pill below.
- Screensaver tokens: dim `.60`, drift period 60 s; art matte `#E9E2D3` with 64 px border, frame `#1F1A1B`.
- Wordmark: mark gradient `#B7ADF8 → #A79BF5 → #7FB3C9` at 135°, 28 px radius at 96 px; word Manrope 400 +
  700, −.03em.

### Progress log (batch 2)

- 19:05 kit additions written; 14 HTML files written; first render of all 25 shots OK.
- 19:15 fixed variant selectors (`html:not([data-v=…])`), OSD wrapping (640 wide, meta on the right),
  banner wrapping (1240 wide, chips 21 px); quick panel is content-height and wider so the live lines fit.
- 19:20 library: 5 columns of 168 px tiles, menu anchored under the Netflix tile, source tile `.src`,
  the rest dimmed; one focus owner (the menu).
- 19:30 full-size review: recents scrim `.74` + hint pill, power dialog 860 wide, PIN footnote one
  line, no-signal button icons sized; gallery entries 10-23 added; SURFACES.md read → spinner added.
- 19:45 Nitin's Denon addition: OSD states dB / 0-98 / muted / not answering; `24-settings-receiver`
  with four tabs; quick panel receiver tile; gallery entries; Sound out row text.
- 19:55 detector run; footnote contrast fixed; stepper/rail/thumb padding; full re-render (42 shots);
  gallery and live navigation checked (receiver page D-pad walk OK).
