# Batch 2 mocks: the rest of the shell

## Resume here

Status: complete. 42 shots in `shots/` (11 from batch 1, 31 from this batch), all in the `index.html`
gallery with captions and the live viewer. Nothing left to build; nothing applied to the TV.

If more screens are asked for: copy the nearest HTML file, add it to `SHOTS` in `render.mjs` and in
`index.html`, run `cd /data/home/lumaglass/docs/mocks && node render.mjs <prefix>` and look at the PNG
at 1:1. Variants are query strings on one file: `?theme=light`, `?v=…`, `?tab=…` (see the list below).
Detector: `/data/home/.agents/skills/impeccable/scripts/impeccable detect --json docs/mocks/lucent.css docs/mocks/*.html`
(from `/data/home/lumaglass`). Remaining findings are intentional: `dark-glow` is the accent focus halo
Nitin chose; `nested-cards` is Lucent's sheet → well → raised-control stacking.

## Files

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

## Rationale, one line each

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

## Kit changes (lucent.css, applied)

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

## Lucent proposals (not applied to the skill)

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

## Progress log

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
