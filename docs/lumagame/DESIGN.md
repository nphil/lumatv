---
name: LumaGame
description: A personal game collection for the living-room TV, with every way to play in one library.
colors:
  canvas: "#191724"
  sheet: "#1F1D2EE8"
  scrim: "#120E1A6B"
  glass: "#120E1A66"
  glass-raised: "#2C263A8F"
  edge: "#FFFFFF1F"
  edge-raised: "#FFFFFF33"
  ink: "#F2EFF6"
  ink-2: "#F2EFF6B1"
  ink-3: "#F2EFF66B"
  track-off: "#FFFFFF29"
  accent: "#A99DF5"
  accent-ink: "#17131F"
  positive: "#9CCFD8"
  warning: "#F6C177"
  danger: "#EF8AA6"
  info: "#9CCFD8"
  tint-8: "#F2EFF614"
  tint-9: "#F2EFF617"
  qr-plate: "#FFFFFF"
  light-canvas: "#FAF4ED"
  light-sheet: "#FFFAF3EB"
  light-glass-raised: "#FFFFFFB8"
  light-edge: "#2E2A451F"
  light-edge-raised: "#2E2A4533"
  light-ink: "#2E2A45"
  light-ink-2: "#2E2A45D5"
  light-ink-3: "#2E2A4587"
  light-accent: "#765E92"
  light-primary: "#5E4B75"
  light-accent-ink: "#FFFFFF"
  light-positive: "#286983"
  light-warning: "#925B0F"
  light-danger: "#A14D65"
  light-info: "#416F77"
typography:
  display:
    fontFamily: "Manrope"
    fontSize: "92px"
    fontWeight: 500
    lineHeight: 1.04
    letterSpacing: "-0.02em"
  title:
    fontFamily: "Manrope"
    fontSize: "40px"
    fontWeight: 600
    lineHeight: 1.2
    letterSpacing: "-0.01em"
  body:
    fontFamily: "Manrope"
    fontSize: "28px"
    fontWeight: 400
    lineHeight: 1.4
    letterSpacing: "0em"
  label:
    fontFamily: "Manrope"
    fontSize: "26px"
    fontWeight: 600
    lineHeight: 1.2
    letterSpacing: "0em"
  caption:
    fontFamily: "Manrope"
    fontSize: "24px"
    fontWeight: 500
    lineHeight: 1.35
    letterSpacing: "0em"
rounded:
  sheet: "36.4px"
  card: "31.2px"
  tile: "26px"
  row: "23.4px"
  control: "18.2px"
  pill: "999px"
spacing:
  s1: "4px"
  s2: "8px"
  s3: "12px"
  s4: "16px"
  s5: "20px"
  s6: "24px"
  s7: "32px"
  s8: "40px"
  edge-x: "96px"
  edge-y: "54px"
  gutter: "24px"
  target: "64px"
  row: "80px"
components:
  nav-group:
    backgroundColor: "{colors.glass-raised}"
    rounded: "{rounded.pill}"
    padding: "6px"
    height: "76px"
  button-primary:
    backgroundColor: "{colors.accent}"
    textColor: "{colors.accent-ink}"
    typography: "{typography.label}"
    rounded: "{rounded.pill}"
    padding: "0 34px 0 28px"
    height: "64px"
  button-secondary:
    backgroundColor: "{colors.glass-raised}"
    textColor: "{colors.ink}"
    typography: "{typography.label}"
    rounded: "{rounded.pill}"
    padding: "0 34px 0 28px"
    height: "64px"
  chip-filter:
    backgroundColor: "{colors.tint-8}"
    textColor: "{colors.ink}"
    typography: "{typography.label}"
    rounded: "{rounded.pill}"
    padding: "0 28px 0 24px"
    height: "64px"
  chip-filter-selected:
    backgroundColor: "{colors.accent}"
    textColor: "{colors.accent-ink}"
  chip-status:
    backgroundColor: "{colors.tint-9}"
    textColor: "{colors.ink}"
    typography: "{typography.caption}"
    rounded: "{rounded.pill}"
    padding: "4px 18px 4px 16px"
    height: "44px"
  cover:
    backgroundColor: "{colors.sheet}"
    rounded: "{rounded.tile}"
    width: "280px"
    height: "420px"
  row:
    textColor: "{colors.ink}"
    typography: "{typography.body}"
    rounded: "{rounded.row}"
    padding: "8px 28px"
    height: "80px"
  toggle:
    backgroundColor: "{colors.track-off}"
    rounded: "{rounded.pill}"
    width: "82px"
    height: "48px"
  toggle-on:
    backgroundColor: "{colors.accent}"
  sheet:
    backgroundColor: "{colors.sheet}"
    rounded: "{rounded.sheet}"
    padding: "40px 36px 34px"
    width: "1320px"
  sheet-confirm:
    width: "1080px"
  sheet-code:
    width: "840px"
  key:
    backgroundColor: "{colors.tint-9}"
    typography: "{typography.label}"
    rounded: "{rounded.control}"
    width: "72px"
    height: "64px"
  toast:
    backgroundColor: "{colors.sheet}"
    typography: "{typography.label}"
    rounded: "{rounded.card}"
    padding: "16px 28px 16px 20px"
---

# LumaGame design

## Overview

LumaGame is a personal game collection for the TV: full-bleed still game art, one floating nav
group, one featured game, then cover rows. The house style is Lucent v2 (TV profile) with Rosé Pine
as the default theme. Every value is laid out at a 1920x1080 reference and scaled uniformly to the
3840x2160 panel. Game art is the colour of the product; the interface around it stays tinted,
quiet and legible.

The mocks in `mock/` are the visual reference. Each page renders at exactly 1920x1080 and takes
URL parameters: `family` (rose-pine, graphite-chalk, nordic-fjord, copper-kiln), `theme`
(dark, light), `density` (compact, standard, cinematic), `text` (100, 200) and `motion=reduced`.
`mock/fetch-assets.sh` downloads the game art (not committed); `mock/render.mjs --check` renders
every screen and measures each text run against the pixels behind it. Library contents, hosts,
accounts and numbers in the mocks are illustrative.

| Screen | Mock |
|---|---|
| Home (standard, cinematic, compact) | `home.html`, `home-cinematic.html`, `home-compact.html` |
| Library | `library.html` |
| Game hub (Overview, Ways to play, Media) | `hub.html`, `hub.html?tab=ways`, `hub.html?tab=media` |
| Chooser | `chooser.html` |
| Settings (Appearance, Streaming) | `settings.html?section=appearance`, `?section=streaming` |
| Integrations, pairing, sign-in | `integrations.html`, `pairing.html`, `signin.html` |
| In-stream overlay, end-session confirm | `overlay.html`, `overlay.html?confirm=end` |
| Launch states, errors, first run | `states.html?s=checking`, `queue`, `starting`, `failed`, `offline`, `empty` |
| Component boards | `components.html`, `components.html?board=content` |

## Colors

Components name roles, never colours. The palette supplies the 20 Lucent roles for each family in
dark and light (`mock/themes.js`; Rosé Pine uses the contrast-fitted values of
`rustui/tokens/src/palettes.json`), and a theme change swaps all of them at once.

- **The stage is always dark.** Game art and logos are authored for dark backgrounds, so art sits
  on the family's dark canvas in every theme, and content on the stage resolves every role to the
  dark variant (`.on-stage`). Light themes end the art band under the hero and keep rows on the
  light canvas.
- **The art keeps its colour.** Scrims are neutral black, never tinted. The left scrim behind the
  hero text is fitted per image: the 90th-percentile luminance of the text column decides an alpha
  between .72 and .90 that keeps the secondary ink at 7:1. Natively this is baked at decode
  (`scrim_alpha`).
- **Contrast is measured on the composite.** Text reaches 7:1 against what is actually behind it;
  controls and focus reach 3:1. Secondary ink is fitted to 7:1 on the canvas and sheet; on an ink
  tint (focus wash, segmented track, filter chip, nav glass) it is lifted to 90 % ink.
- **Tone on the mark, ink on the word.** Status colour lives in a 12 px dot or an icon; the word
  stays in ink. Positive for ready and shared saves, warning for sign in and queue, danger for
  offline and failures, info for asleep, an outlined dot for unknown.
- **Primary** is the accent unless accent ink on it misses 7:1; Rosé Pine light uses `#5E4B75`.
- **QR plate exception.** The sign-in QR sits on white in every theme: phone cameras need a light
  quiet zone (four modules).

Four families are mocked to prove the token layer. The brief's 20-family list differs from the 20
families in `rustui/tokens/src/palettes.json`; one shared catalogue in `rustui/tokens` should serve
LumaTV and LumaGame alike, renamed or extended there rather than forked.

## Typography

Manrope at three weights only: Regular 400, Medium 500, SemiBold 600.

| Role | Size | Weight | Use |
|---|---|---|---|
| Display | 92 | 500 | Queue position, PIN digits |
| Title | 40 | 600 | Sheet titles, title plates, sign-in address and code |
| Body | 28 | 400; 600 for names and row heads | Descriptions, row heads, row labels, cover captions |
| Label | 26 | 500; 600 on controls | Buttons, chips, tabs, hints, meta lines |
| Caption | 24 | 500 | Secondary lines, status chips, column heads |

- Text size is 100 % or 200 %; cover, hero and tile sizes do not follow it.
- Remote-key glyphs in hints are 37 px, 1.45 times the label they precede.
- Numbers that change in place (PIN, code, timers, stats, queue position) use tabular figures.
- Sentence case everywhere. A game title too long for a title plate steps down one size; words
  never break.

## Layout

- **Safe area** 96 px left and right, 54 px top and bottom. The nav group sits on the top safe edge,
  76 high (64 px items in 6 px padding).
- **Hero** 240 / 360 / 480 high from the top safe edge (Compact / Standard / Cinematic), with a
  logo box 72 / 120 / 176 high and a text column up to 900 wide. The hero scrolls away in deeper
  rows; dark themes keep the focused game's art as a dim backdrop, light themes drop it.
- **Covers** are 2:3 at 224 / 280 / 336 wide with 24 px gaps. Row heads sit 18 px above their
  track and rows are 48 px apart. The focused row reserves room for the caption under the cover.
- **Library** fills the 1728 px safe width with whatever column count lands nearest the density's
  width: 7 x 226, 6 x 268, 5 x 326. One filter bar, no page title (the nav names the page); the
  Menu key opens sort, named by a hint at the bar's right end.
- **Hub**: art 760 high behind a 1000 px text column; logo 168 high; buttons, target summary, then
  tabs (Overview, Ways to play, Media) and four fact tiles. Moving into a tab below Overview
  scrolls the page so the tabs land on Home's focus line (y 170) under the top bar; the art stays
  as a dim backdrop in dark themes and leaves in light themes.
- **Sheets** are centred: 1320 wide for lists, 1080 for a confirm, 840 for a code; at most 972
  high. Settings uses a rail (at least 420, growing with text) beside the pane, 48 apart.
- **200 % text** reflows: chooser columns fold under the name, a sheet list that outgrows its
  sheet feathers at the bottom edge, the settings rail widens, hero meta wraps.
- **Hints**: full-screen surfaces put them in the bottom-right safe corner; sheets put them at the
  footer's left with the action on the right.

## Elevation & Depth

- **T0 by default**: canvas tints for fact tiles, ways-to-play tiles, title plates, notices and
  status chips. **Cached glass (T1)** only on the nav group, buttons over art, sheets and the toast.
- **Shadows** are neutral and vertical: rest `0 24 60 / .28`, raised `0 34 80 / .55 + 0 10 24 /
  .30`, cover at rest `0 14 30 / .34`, focus `0 22 44 / .50` (light themes about half). One
  focus shadow is active at a time; nothing glows.
- **Scrims** dim the screen behind a sheet to 74 % canvas when it stays as context and 97 % when it
  must not compete (the end-session confirm).
- **Focus** is the same everywhere: scale 1.03, lift 4 px, one shadow and a 2 px landing light in
  the element's own ink. On pills and buttons the light sits 8 px above the bottom edge, inside;
  on rows and tiles it sits on the bottom edge; on covers it sits on the canvas 12 px below the
  art, above the caption, because art behind a bar is not contrast-safe. Rows add a 9 % ink wash
  and an inset lit edge instead of growing. No outlines, no white pills.
- **Motion**: focus 150 ms, panel enter 180 ms, exit 120 ms, art dissolve 200 ms once focus
  settles. Repeated presses jump to the newest destination. Reduced motion drops lift, growth and
  dissolves and keeps colour and the landing light.

## Shapes

Radii: sheet 36.4, card 31.2, tile 26 (covers, fact and genre tiles), row 23.4, control 18.2
(keyboard keys), pills for every button, chip, tab and the nav group. Collection tiles are
448 x 252 and genre tiles 336 x 252. Dots are 12 px circles; the toggle is 82 x 48 with a 36 px
knob.

## Components

- **Nav group**: Home, Library, Search on the left; avatar and Settings on the right; the current
  page carries a 10 % ink tint. Over art in light themes the fill is the sheet, not glass, to hold
  7:1.
- **Buttons**: one size per screen, 64 high. Primary is filled with the primary colour; secondary
  is raised glass; danger keeps ink text with a danger icon; icon-only is 64 square. Play names its
  target: `Play · GeForce NOW`.
- **Cover card**: 2:3 art with a 1 px edge; focus adds the caption below: title, then where it
  plays and its status (`GeForce NOW · Ready`).
- **Title plate**: games without art get a typeset plate: the title at title size over a cropped
  initial at 9 % ink, which is decoration with no accessible name. Never a broken image.
- **Collection and genre tiles**: collections are art; genres are type over a strip of three covers.
- **Filter chip**: a 64-high control; selected is primary; engine chips show an icon and their
  game count.
- **Status chip**: 44 high, dot or icon plus a word: Ready, Asleep, Offline, Sign in, Unknown,
  Installed, Install, Queue. **Capability chip**: outlined, with a positive tick, only for verified
  capabilities.
- **Hint**: the remote-key glyph plus a verb (`Select`, `Close`, `Sort`); never the key's name.
- **Hub**: top bar with back, title, Favourite and more; then logo, meta, description, capability
  chips, `[Play · target]` and `[Choose where to play]`, and the target summary (engine,
  host/store/account, readiness). After a failed launch or for an unreachable target a notice
  replaces the target summary: an opaque card whose icon carries the tone.
- **Hub tabs**: Overview holds the fact tiles. Ways to play lists the chooser's rows at the
  chooser's width plus `Add a way to play`; OK there makes that row the game's target, which Play
  then names. Media has a Trailers row (named tiles with a play badge; a trailer plays only when
  selected) and a Screenshots row (pictures, unlabelled), both in 448 x 252 collection tiles.
- **Chooser row**: 80 high; engine badge, name and host or account, save compatibility (Shared,
  Separate, Unknown), status chip, and a radio for the selected target. OK selects a row and moves
  focus to the `Play here` action; `Remember for this game` sits under the list.
- **Settings rows**: 80 high with an icon, label, optional sub-line and one control: value, toggle,
  stepper (arrows appear on focus so values align at rest), slider, segmented choice or theme
  swatches (four palette dots and a name; selected has a 3 px accent ring).
- **Sheet**: title, body, footer with hints left and actions right. A confirm starts on the safe
  action (`Keep playing`).
- **Pairing**: four 136 x 168 PIN digits at display size and a waiting line. **Sign-in**: QR
  plate, the address and code at title size, a waiting line and the code's expiry.
- **Toast**: top-right on the safe edge, sheet material, a positive icon and one line.
- **Keyboard**: 72 x 64 keys at control radius, wide 164, space 420, Go in primary.
- **Launch curtain**: the game's art on the stage, logo, step chips (Checking, In queue,
  Starting), one status line (queue position at display size), a progress bar and the Cancel hint.
- **In-stream overlay**: the curtain's composition over still art on an opaque stage: logo, where
  and how long, stream stats, then Resume, Stream settings and End session.
- **First run**: `Add a way to play` with four tiles (GeForce NOW, Gaming PC, Desk PC, This TV),
  each with its next step.

## Do's and Don'ts

### Do:

- Name the target on every Play and show readiness before the press.
- Measure contrast on the composite, over the real art.
- Keep one button size per screen and the hints as glyph plus verb.
- Start destructive confirms on the safe option.
- Keep game art and logos in their own colours.

### Don't:

- Don't tint art, autoplay trailers, rotate the hero or add particles.
- Don't use outlines, white pills, glows, eyebrows, gradient text, side stripes, emoji or ALL CAPS.
- Don't put a card inside a card; a focused row's wash and lit edge are a state, not a container.
- Don't carry status by colour alone or in jargon (`Wakes on play`, not `Wake-on-LAN`).
- Don't use per-card live blur or more than one active focus shadow.

## Native mapping

The screens are built in `rustui/app/src/game/screens/` on the Lucent kit in
`rustui/lumashow-ui/src/lucent/`, with theme additions in `rustui/tokens/`. Toolkit facts the
design respects:

- Text is Manrope Regular, Medium and SemiBold from an integer-pixel glyph atlas.
- Glass is an SDF rounded rect with a tint or vertical gradient, at most two vertical shadows, and
  a backdrop of None, Cached, Live or Prepared. LumaGame uses Cached only.
- Layers move by x, y, scale, opacity and a feathered clip: no rotation, blend modes or filters.
- Images bake corners, strips, fades and scrims at decode (`app/src/gfn/art.rs` `Spec`).
- The render host redraws changed 16 px tiles; static art costs nothing after it settles.
- `lucent/tokens.rs` contrast tiers are BODY 4.5, GLANCE 7, CONTROL 3; LumaGame uses GLANCE for
  all text. Its switch is 68 x 40; LumaGame's 82 x 48 is that switch at 1.2 for ten-foot reading.
- The GeForce NOW screens use a 64 px safe x and 272 x 360 tiles (`app/src/gfn/layout.rs`);
  LumaGame moves to 96 px and the 2:3 cover presets above.

| Mock technique | Native equivalent |
|---|---|
| `backdrop-filter` blur | Cached glass backdrop |
| `object-view-box` logo trim | alpha-bounds trim at decode |
| Art scrims and fades (`linear-gradient`) | baked at decode, `scrim_alpha` per image |
| `mask-image` sheet feather at 200 % text | feathered clip |
| `color-mix` tints | RGBA resolved once per theme |
| Radio `radial-gradient` | ring plus filled circle |
| Toggle tick | check glyph |
| Plate initial (`::before` text) | glyph layer at 9 % ink |
| Letter-spacing on display and title | per-run advance adjustment, or dropped |
| Tabular figures | fixed digit advance |

## Name and icon

**Name: keep LumaGame.** It matches the family (LumaTV, LumaShell), says what it holds, and matches
the permanent app id `org.nphil.lumagame`. Alternatives considered:

- **LumaPlay**: short and active, but next to LumaTV it reads as a video player.
- **LumaArcade**: evocative, but promises retro and emulation rather than cloud and PC games.

**Icon**: three options, each on LumaTV's plate (200 viewBox, radius 44, `#191724` to `#1f1d2e`)
with its glass panes and sheen; SVG plus PNG at 80, 130 and 240 in `icon/`.

- **A, cover stack (recommended)**: three 2:3 glass covers fanned in love, foam and iris, with a play
  notch in the front one. It reads as games at 80 px, says "play" rather than "controller", and is
  the closest sibling to LumaTV's panes.
- **B, glass D-pad**: two crossing panes. Reads as games, but generic and close to a plus sign.
- **C, lit thumbstick**: a glass cap over a lit ring. Distinctive, but at 80 px it reads as a
  record button.
