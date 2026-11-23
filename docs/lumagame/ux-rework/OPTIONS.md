# LumaGame UI options

Two or three designs for each of the six screen types, drawn as 1920x1080 mocks in Rosé Pine
dark. Each option is a different idea, not a restyle: all of them are built from one component
kit, so they differ in layout only. The problems they answer are in `AUDIT.md`.

Pictures: `/data/agent/cody-data-scratch/6e68eccca090/ux-options/<name>.png` (not in git).
Sources: `mock/` (one HTML file per option; `rework.css` and `rework.js` are the shared kit on top
of `../mock/`). `node mock/render.mjs --check` redraws them and measures contrast, overlap and cut
text; all 43 pictures pass at 7:1 with no overlap, at 100% and 200% text.

Every count, name, post and number in the mocks is illustrative.

## Picks

| Screen | Pick | Why | 120 Hz risk |
|---|---|---|---|
| Home | A · Spotlight | One press to play, the game's art is the screen, the game's news one press down | Medium, with a named fix |
| Library | A · Bar | Most games per screen, filters about games, search that never moves the grid | Low to medium |
| Game page | B · Split | Play never leaves the screen; Steam content reads beside it | Low |
| Steam and accounts | A · Accounts page, plus B's in-place prompt | One home for Steam, and a prompt where its data is missing | Low |
| Settings | A · Rail | Same anatomy as LumaTV Settings, five places | Low |
| Launch and in-game menu | B · Drawer | The page stays readable through a queue; the game stays visible under the menu | Low to medium |

Safest at 120 Hz if fluidity outranks everything: Home C, Library C, Game B, Settings B, Launch C.

## What every option shares

**One kit.** Colour, radius, type, glass and motion come only from the Lucent v2 TV tokens
(`../mock/tokens.css`). Shared pieces: game tile, cover, the one list row (80 high: glyph tile or
picture, name, one short line, trailing state or control), glance card, side drawer, dialog, hint
bar, section strip. One button size (64) on every screen. Type is 92/40/28/26/24 only.

**Same product as LumaTV.** Rows carry LumaTV's tinted glyph tile. The side drawer sits where
LumaTV's Quick settings sits and uses the same sheet. Settings uses LumaTV's title, rail sheet and
pane sheet, with its crumb, page title and state chip. Focus is LumaTV's: lift, wash and the
landing light, never an outline. Hints use the same key glyphs.

**Navigation and input.**

| Key | Remote | Pad | Does |
|---|---|---|---|
| Move | D-pad | D-pad, left stick | Moves focus; grids do not wrap; each region remembers its last focus |
| Select | OK | A | Acts on release. On Home it plays; in Library it opens the game page |
| Back | Back | B | Closes the top layer, then returns to the screen's anchor (Play, or the last game), then goes up one screen. On Home's anchor the hint bar reads "Back again to leave" for 1.5 s; a second Back leaves |
| Menu | Menu | Y or Menu | Opens the quick menu on every screen outside a stream |
| Sections | Left, Right on the strip | LB, RB | Previous or next peer: Home, Library, Search; library views; game page sections |
| In a stream | Back | Guide, or Start held 1 s | Opens the in-game menu; the first key only shows hidden chrome |

The top bar holds three places and the profile. Hints sit bottom-right on full screens and
bottom-left in drawers and dialogs: at most three, a key glyph and one word. They follow the
device last used, so pad glyphs (and LB, RB marks at the ends of the section strip) replace the
remote's when a pad is in hand.

**Refresh libraries has one home:** the first row of the quick menu (`home-quick-menu.png`). It
is not on Home, the game page, the library bar or Settings. From any Home option, Library or the
game page: Menu, OK. Two presses. The row shows "Updated 2 min ago" and, while running, what it is
checking. Libraries still refresh by themselves at start, after a session and every 30 minutes.

**Quick menu.** Refresh libraries, Accounts and PCs (carries the one attention mark when a PC is
offline or a sign-in expired), Settings; then, for the highlighted game, Game page, Pin to Home,
Hide.

**Words.** Ready, Asleep, Offline, Sign in. A PC that is asleep wakes on Play, so it counts as
ready; only a game that cannot start now carries a mark. "Sign in" for accounts, "Pair" for PCs.

**Services stay out of the way.** GeForce NOW, the PCs and This TV never label a game or
organise a screen. The Play control carries a small glyph of where it will run; the launch screen
says it in words. Steam appears as the text "From Steam" next to what it supplied, never as a logo.

**Motion and the 120 Hz budget.** A frame is 8.33 ms. Every option keeps to these rules:

| Move | Spec | Reduced motion |
|---|---|---|
| Focus | Lift 1.03 and wash, 150 ms, `cubic-bezier(.33,1,.68,1)`; the key is acknowledged on the next frame | Wash and landing light only |
| Press | 90 ms | Same |
| Row or grid scroll | 260 ms critically damped spring, retargeted on key repeat (300 ms, then 100 ms, coalesced) | Jump |
| Drawer, dialog, layer | In 220 ms (x or y and opacity), out 180 ms | Opacity only, 120 ms linear |
| Card or section content | 180 ms opacity | 120 ms linear |
| Backdrop art | 200 ms dissolve, only after focus has rested 250 ms; never during a scroll | Cut |

Only x, y, scale and opacity move. Nothing reflows during a move: captions have reserved space,
the game page's sections are fixed, late data fills a slot that is already there. Scrims, crops
and fades are baked into the art at decode. Glass is cached or an opaque sheet; nothing blurs
live. Art decodes off the frame; the highlighted item and its neighbours (one row, two tiles) are
ready before focus reaches them; at most one texture upload per frame.

Cost figures below are counted from the mocks at 1920x1080 (a full-screen layer is 2.07 Mpx)
against the measured ceiling of 44 layers and 6.6 Mpx. They are estimates until measured on the
TV.

**Panel care.** No option adds an always-on bright element. Hints dim to 40% after 5 s without a
key and return on the next. The in-game strip and the viewer's caption leave after 5 s and 3 s.
Home A and B hold one game's art while idle; it dims with the screen after the existing idle
delay and hands over to the saver. Home C shows no backdrop at all.

## 1. Home

Each option is a LumaGame design that borrows one console idea. In all three the app opens on
the last game played, OK plays it, and the bar holds Home, Library, Search and the profile.

### A · Spotlight (pick)

`home-a-spotlight.png`, `home-a-spotlight-down.png`, `home-a-spotlight-light.png`

The highlighted game owns the screen: a row of small tiles on top, the game's art behind, its
logo and Play below.

- Borrowed: the art that follows the highlighted tile, and the game's own strip one press down
  (PS5). It suits an OLED and streamed games: the art is the reward for a short list, and the
  strip shows a cloud game's news without opening it.
- Suits: a household that plays a handful of games and wants the last one back in one press.
- First glance: nine tiles, one logo, one line (last played, hours, achievements), Play.
- One press deeper (Down): four cards for that game: achievements, last session, latest news,
  reviews. Up returns to the tiles. This strip is the Home activity surface.
- Stays uncluttered because the game's details replace the tile row instead of sitting under it,
  and a game with no Steam record shows two cards, not empty ones.
- Weak: the backdrop is the costliest thing on any Home; only nine games before scrolling.
- Fixes: Home's two rows of duplicated games, the Refresh hero button, service names on tiles.
- Motion: tiles slide under a fixed focus slot; art dissolves 200 ms after focus rests 250 ms;
  Down moves the tile row out and the cards in over 220 ms.
- Refresh: quick menu, two presses.

### B · Board

`home-b-board.png`

The last game is one large tile, recent games fill a board beside it, pinned games sit in a row
below.

- Borrowed: pins and the oversized last-played tile (Xbox). It suits a library spread over
  several services: pins put chosen games in a fixed place whichever service lists them.
- Suits: several players, or anyone who wants twenty games reachable without scrolling.
- First glance: 13 recent tiles, 7 pins, one line naming the highlighted game, Play.
- One press deeper: Menu on a tile pins, hides or opens its page.
- Stays uncluttered because tiles carry art and logo only; the name and state of the
  highlighted one live on a single line at the bottom.
- Weak: reads as a grid, so closest of the three to a media wall; the backdrop changes often.
- Motion: focus lift only; the dimmed backdrop dissolves after a 400 ms rest.
- Refresh: quick menu, two presses.

### C · Quiet row

`home-c-quiet-row.png`

One row of large tiles on the plain canvas, the game's name above, LumaTV's dock below.

- Borrowed: the single instant row (Switch) and LumaTV's own dock in place of a top bar. It
  suits panel care and the frame budget: most of the screen is black and nothing large changes.
- Suits: speed first; the fewest things on screen.
- First glance: five tiles, a name, one line, Play, the dock.
- One press deeper: the last tile opens all games; Menu holds the rest.
- Stays uncluttered because there is nothing else to draw.
- Weak: the least atmosphere; no place for news or progress on Home.
- Motion: the row slides under a fixed focus slot; no backdrop, so nothing dissolves.
- Refresh: quick menu, two presses.

| Home | Layers | Full-screen layers | Glass | Largest moving area | 120 Hz |
|---|---|---|---|---|---|
| A · Spotlight | 16 | 1 (2 for 200 ms while art changes) | 1 cached (bar) | 4.1 Mpx during the dissolve | Medium. The dissolve is the risk; the cheaper form is a dip (fade to the stage, swap, fade in), which never holds two full-screen layers |
| B · Board | 27 | 1 (2 while art changes) | 1 cached | 4.1 Mpx during the dissolve | Medium to high: 20 tiles change the backdrop far more often. Cheaper: a 960x540 pre-dimmed backdrop, or none |
| C · Quiet row | 11 | 0 | 1 cached (dock) | 0.65 Mpx | Lowest |

## 2. Library

OK on a cover opens the game page. A cover shows a mark only when its game cannot start now.

### A · Bar (pick)

`library-a-bar.png`, `library-a-bar-search.png`

One bar of views about the games (All, Favourites, Recently added, Ready now) over a dense grid.

- Suits: a library of a hundred or more, browsed by cover.
- First glance: four views with counts, the sort in words, 16 full covers at eight across.
- One press deeper: Menu opens sort and "plays on"; Search is the bar's third place.
- Search: keys on the left in a six-wide alphabet, results on the right, updated per key. The
  grid never moves under the keys; Right goes to the results.
- Stays uncluttered because services are a filter behind Menu, not chips on the bar, and a
  caption appears only under the highlighted cover.
- Weak: 34 layers on screen, the most of any option; sort is one press away.
- Fixes: the Refresh chip, hidden Sort key, services as the organising axis.

### B · Panel

`library-b-panel.png`

Filters stay on screen in a left panel, search first; five covers across beside it.

- Suits: someone who filters often and wants every choice visible.
- First glance: seven filter rows, ten covers.
- One press deeper: "Plays on" and "Sort" open their lists in the panel.
- Stays uncluttered by grouping the panel under two labels; the grid keeps a single caption.
- Weak: a third fewer games per screen; the panel is furniture most visits do not need.

### C · List

`library-c-list.png`

Titles as a list grouped by first letter; the highlighted game shown beside it.

- Suits: a very large library where the name is known; the fastest to scan and to draw.
- First glance: seven titles with hours, one game's art, state and where it plays.
- One press deeper: LB and RB jump a letter; OK opens the game page.
- Stays uncluttered because one picture is on screen at a time.
- Weak: no browsing by cover; the least like a console.

| Library | Layers | Full-screen layers | Glass | Largest moving area | 120 Hz |
|---|---|---|---|---|---|
| A · Bar | 34 (search: 12, the keys are one cached layer) | 0 | 1 cached | 1.4 Mpx (grid scroll) | Low to medium: layer count and cover uploads while scrolling; covers stay at 195x292 |
| B · Panel | 30 | 0 | 2 cached | 1.3 Mpx | Low to medium |
| C · List | 15 | 0 | 1 cached | 0.5 Mpx | Low; the preview loads only after focus rests 200 ms |

## 3. Game page

Four sections, always present, always in this order: **Activity, Community, Media, About**. A
section whose data is missing says so in its own place, so the page never changes shape when a
lookup returns. Where to play is one control. A failed start turns Play into "Try again" with the
reason on one line.

What the sections hold, and how sure the source is:

| Content | Section | Source | In the design |
|---|---|---|---|
| Own play time, last session, own achievements | Activity | The Steam account | Shown when linked; a sign-in row in its place when not |
| Achievement rarity, players now | Activity | Solid | Shown |
| Friends who play | Activity | Unconfirmed | One small line; left out if it cannot be had |
| News and announcements | Activity | Solid | Shown, no account needed |
| Review score, top reviews with play time, developer replies | Community | Solid | Shown, no account needed |
| Discussions, guides | Community | Fragile (read from Steam's pages) | Shown when they load; the group is simply absent when not |
| Official screenshots and trailers | Media | Solid | Shown; trailers play on the TV |
| Community screenshots, artwork, videos | Media | Fragile | Three extra filters when they load, absent when not. Community videos are YouTube links; how they play is undecided |
| Description, developer, release, controller support | About | Store details | Shown |
| Workshop, broadcasts | | Not available | Left out |

`game-b-split-marks.png` outlines the fragile and unconfirmed parts.

Reading is in place in every option: a post opens as one column of 28 px text, about 60
characters a line, a page at a time (`game-b-split-reader.png`). A picture opens the full-screen
viewer; Left and Right step through, the caption and hints leave after 3 s and the first key only
brings them back (`game-viewer.png`, `game-viewer-chrome-hidden.png`).

### A · Feed

`game-a-feed.png`, `game-a-feed-community.png`, `game-a-feed-text200.png`

Art, Play and the where-to-play button first; the four sections are a strip along the bottom.
Down turns the page into that section's feed under a slim Play bar.

- Suits: reading a lot about a game; closest to a Steam game page.
- First glance: logo, one line, Play, where it plays, four section names.
- One press deeper: the section in two columns, full width.
- Stays uncluttered because nothing from Steam is on the first view at all.
- Weak: reading and playing are two states of the page, so Back is needed to reach Play again
  (Back does return there first); the turn moves the whole screen.

### B · Split (pick)

`game-b-split.png`, `-community`, `-media`, `-reader`, `-where`, `-text200`, `-light`

A tall art panel holds the game, Play and the one where-to-play row; the sections sit beside it.

- Suits: nearly everyone: play in one press, or read beside it without losing Play.
- First glance: the art panel with Play and where it plays; Activity as three figures (played,
  achievements, playing now), the latest three achievements and two news items.
- One press deeper: Right enters the sections; LB and RB change section; OK opens a post or
  picture in the same pane. The where-to-play row opens its list in the panel.
- Stays uncluttered by a fixed split: the left third is only about playing, the right two thirds
  only about reading, and each section shows one screen of rows before it scrolls.
- Weak: the art is a third of the screen, not the whole of it; the first view shows the most
  text of the three.
- Fixes: stacked actions, Play hidden under tabs, tabs that vary by lookup, four ways to choose
  where to play, the "Remember" switch (the choice sticks once a session starts).
- At 200% the strip becomes the current section between two arrows and the figures wrap to two
  columns; Play and the where row stay on screen.

### C · Glance

`game-c-glance.png`, `game-c-glance-community.png`, `game-c-glance-text200.png`

One screen that never scrolls: Play, then one card per section. A card opens its section as a
layer.

- Suits: play first, a look at progress or the review score without entering anything.
- First glance: full art, Play, where it plays, four cards with one fact each.
- One press deeper: the section as a layer over the art, with the same strip to change section.
- Stays uncluttered because each card may hold one fact and no more; the cards are the ones
  Home A uses.
- Weak: the layer and its scrim over full art are the heaviest state of any game page; a card's
  one fact can be the wrong one.

| Game page | Layers | Full-screen layers | Glass | Largest moving area | 120 Hz |
|---|---|---|---|---|---|
| A · Feed | 8, then 20 in the feed | 1; 2 during the turn | 0 | 4.1 Mpx (the turn) | Medium: the turn and long feeds. Cheaper: fade the art out rather than slide it |
| B · Split | 18 | 0 (art is 0.5 Mpx) | 0 | 1.1 Mpx (pane content) | Low |
| C · Glance | 10, then 22 with a layer open | 1; 2 with the scrim | 1 sheet | 3.1 Mpx (layer and scrim) | Medium at 5.8 Mpx. Cheaper: swap to a pre-dimmed copy of the art, no scrim layer |

## 4. Steam and accounts

Steam adds information to games other services list and plays none, so it sits under "Adds to
your games", apart from the services and PCs. Sign-in is a code scanned with the Steam app, on a
white plate, with a ring that shows how long the code lasts. News and reviews need no account.
No error opens a popup: an offline PC is a word on its row, an expired sign-in is one row where
the stale data is, and the quick menu's Accounts row carries the only attention mark.

### A · Accounts page (pick, with B's prompt)

`steam-a-accounts-page.png`, `steam-a-accounts-page-qr.png`

Steam has one page under Settings, Accounts and PCs: what it adds, the account, how many games
it matched, when it last updated, a switch for news and reviews, Sign out.

- Suits: wanting one place that answers "is Steam working".
- First glance: state chip, one sentence, four facts.
- One press deeper: sign-in as a dialog on an opaque backdrop; Sign out asks first.
- Stays uncluttered: no refresh button (Refresh has its one home), no advanced options.
- Weak: reached through Menu, Accounts and PCs, Steam: four presses from Home.
- Fixes: the bare Steam page, switches that do not switch, the account row that signs in again.

### B · In place

`steam-b-in-place.png`, `-qr`, `-stale`

No Steam page. The game page asks for sign-in where the missing figures would be, and says so in
the same place when the sign-in has expired.

- Suits: never visiting Settings.
- First glance: one row in Activity, news below it as usual.
- One press deeper: the code in a side drawer; the page stays visible.
- Stays uncluttered: one row, on game pages only, gone once linked.
- Weak: alone, nowhere shows the match count or lets the owner sign out.

### C · In the menu

`steam-c-menu.png`, `steam-c-menu-qr.png`

Accounts and PCs are a level of the quick menu; the code shows in the same drawer.

- Suits: fixing a PC or linking Steam without leaving the game being looked at.
- First glance: five rows with one word of state each.
- Weak: a drawer is narrow for account detail; over Home A it is the costliest of the three.

| Steam | Layers | Full-screen layers | Glass | 120 Hz |
|---|---|---|---|---|
| A · Accounts page | 16; 4 under the dialog's opaque backdrop | 0; 1 with the dialog | 2 sheets | Low |
| B · In place | 24 with the drawer | 1 (scrim) | 1 sheet | Low to medium |
| C · In the menu | 26 over Home A | 2 (art and scrim), 5.3 Mpx | 1 sheet | Medium. Cheaper: scrim only beside the drawer |

## 5. Settings

Five places in every option: Accounts and PCs, Picture and stream, Controllers, Appearance,
System. One theme picker. The controller test lives under Controllers. Diagnostics and About live
under System. The glyph is a cog.

### A · Rail (pick)

`settings-a-rail.png`

LumaTV's settings anatomy: a rail of the five places, each with its state as a second line, and
the pane beside it.

- Suits: moving between LumaTV and LumaGame settings without relearning.
- First glance: five places with state; the first one open, one word of state per row.
- One press deeper: a row opens its own page with a crumb back.
- Weak: two columns of rows is the densest settings view.
- Fixes: buried Integrations, duplicate theme pickers and icons, the sun glyph.

### B · Tiles

`settings-b-tiles.png`

A landing of five tiles that show what is set now.

- Suits: mostly checking, rarely changing.
- First glance: five tiles, two values each. Most visits end here.
- Weak: one more press before anything can be changed.

### C · Quick panel

`settings-c-quick.png`

The five settings changed most often sit in the quick menu's drawer and change in place; the
rest is one row further.

- Suits: changing stream quality or text size without leaving the screen.
- First glance: five rows with their controls, and All settings.
- Weak: two places to change the same setting; the full settings still needs A or B.

| Settings | Layers | Full-screen layers | Glass | 120 Hz |
|---|---|---|---|---|
| A · Rail | 18 | 0 | 2 sheets | Low |
| B · Tiles | 8 | 0 | 0 | Lowest |
| C · Quick panel | 22 over Home A | 2, 5.3 Mpx | 1 sheet | Medium over Home A, low over Home C |

## 6. Launch and in-game menu

Only the steps a way of playing has are shown: GeForce NOW signs in, may queue, starts, connects;
a PC wakes if asleep, connects, starts; a game on the TV starts. End session is marked as
dangerous and asks first on an opaque backdrop, with focus on Keep playing
(`launch-b-drawer-end.png`).

### A · Stage

`launch-a-stage.png`, `launch-a-stage-menu.png`

The game's art fills the screen while it starts. The in-game menu is a list on an opaque stage.

- Suits: a clear, calm wait; a menu that is easy to read.
- First glance: steps, the queue position large, time left, Cancel.
- Weak: nothing to do while queued; the menu hides the game.

### B · Drawer (pick)

`launch-b-drawer.png`, `launch-b-drawer-menu.png`, `launch-b-drawer-end.png`

Starting happens in place on the game page, so the sections stay readable through a queue. The
in-game menu is the side drawer over the live picture.

- Suits: GeForce NOW queues; checking delay or a controller without losing sight of the game.
- First glance: the ring, queue position, Cancel; the page beside it. In a game: Resume first,
  stream and delay as two figures, Controller, End session.
- One press deeper: the drawer's Picture and Game tabs.
- Stays uncluttered: three tabs instead of one long list; the same drawer as everywhere else.
- Weak: depends on Game page B; a scrim over live video costs a blend every frame.
- Fixes: queue steps shown for launches with no queue; failure as a sentence with no action.

### C · Strip

`launch-c-strip.png`, `launch-c-strip-menu.png`

The least on screen: a ring and one line while starting; in a game, a strip along the bottom
that leaves after 5 s.

- Suits: fluidity and panel care first.
- First glance: five actions and three figures over the untouched picture.
- Weak: no room for detail; anything more opens another layer.

| Launch | Layers | Full-screen layers | Glass | 120 Hz |
|---|---|---|---|---|
| A · Stage | 8 | 1 | 0 | Low |
| B · Drawer | 18 starting; 12 in a game | 0; 1 scrim over video | 1 sheet | Low to medium. Cheaper: scrim only beside the drawer (0.9 Mpx) |
| C · Strip | 3 starting; 4 in a game | 0 | 1 sheet | Lowest |

## Known limits

- The mocks are stills; motion is specified, not shown.
- Cost figures and timings are estimates to be measured on the TV before any claim of 120 Hz.
- Friends who play is unconfirmed. Discussions, guides and community media can vanish without
  notice; the designs treat them as optional.
- How community videos (YouTube links) play is undecided.
- Light theme is drawn for the two picks that matter most (Home A, Game page B) only.
- Remote-only use of LB, RB actions relies on Left and Right on the section strip.
