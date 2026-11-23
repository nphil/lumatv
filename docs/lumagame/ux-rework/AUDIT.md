# LumaGame TV UI audit

State of the native LumaGame UI at `main` (UI code in `rustui/app/src/game/`), before the rework. Each point is marked **(seen)** when confirmed in a 1920x1080 render, or **(code)** when read from the source only.

Renders come from the shot pass (`cargo test -p lumatv-rustui --lib every_native_screen`, `GAME_SHOTS=all`, software GL, `rustui/app/src/game/shots.rs`). They are in `$CODY_DATA_SCRATCH/ux-audit/shots/` (`/data/agent/cody-data-scratch/6e68eccca090/ux-audit/shots/`), not in git. Names below omit the directory and `.png`. Fixture data is mock data: titles, hours and queue numbers are not real.

Rules checked against: Lucent v2 TV profile and `docs/lumagame/DESIGN.md` (one button size per screen, hints are glyphs, no overlapping text, honest states, chrome leaves over media, panel care).

## 1. Navigation map

```
Home ─ Play / Game details / Refresh libraries (hero) ─ shelves (Continue, Most played, Favourites,
 │      Collections, Genres, Native on this TV)
 │        ├ game tile ──────────────────────────► Hub
 │        ├ genre / collection tile ────────────► Library (filtered)
 │        └ first run: integration tile ────────► Integration page
 ├ Library ─ chips: All, one per engine, Genre, Favourites, Sort, Refresh libraries
 │      ├ Sort / Genre chip ──────────────────► sheet
 │      └ game tile ──────────────────────────► Hub
 ├ Search ─ field + 44-key keyboard + result grid ─► Hub
 └ (sun-like glyph, top right) Settings ─ rail: Appearance, Streaming, Integrations, Controllers, Diagnostics, About
        ├ Appearance: Theme row ───────────────► Theme sheet (second theme picker)
        ├ Streaming: Preferred way to play, Stats while playing
        ├ Integrations: row ───────────────────► Integration page (enable, sign in/pair, accounts or PCs, refresh, sign out, engine fields)
        │      ├ Sign in (Steam) ──────────────► QR sheet
        │      ├ Pair a PC ────────────────────► PIN / approval sheets
        │      └ Add PC by address ────────────► Add host sheet
        ├ Controllers: Controller lab, GameSir, Leadjoy, Button labels, TV controller settings (leaves the app)
        ├ Diagnostics: server, topic, token (text sheets)
        └ About: three credit rows (open store.steampowered.com, steamgriddb.com, igdb.com outside the app)

Hub ─ Back, Heart (favourite), More (opens Chooser) ─ Play, Choose where to play (opens Chooser)
 └ tabs: Overview, Ways to play, Achievements, News, Media, Details (each only when it has data)
      ├ Ways row ─────────────────────────────► picks the way, jumps back to Overview
      ├ Ways "Add a way to play" ─────────────► Settings › Integrations (leaves the game)
      ├ News row ─────────────────────────────► opens a URL outside the app
      ├ Media: trailer ───────────────────────► plays externally; screenshot ► Media viewer
      └ Play ─────────────────────────────────► Launch curtain ─► Running (no UI) ─ Back/Guide ─► In-stream menu
In-stream menu: Resume, Stream settings (► Settings › Streaming, Back returns), Stats, Steam overlay, End session
```

Back stack: every page pushes `(page, focus)`; the top-bar tabs clear it. Back from Home with no history exits the app (code). Activation is on key release; hover never moves focus (code).

## 2. Steam features and where they live

Steam is an account integration (`game/engines/steam/`). It lists no game and launches nothing: its records only add data to games other engines list. A Steam game that no other engine lists never gets a card (`catalog.rs`: a record "never makes a card or an id").

| Feature | Where it lives today |
|---|---|
| Turn Steam on | Settings › Integrations › Steam page, first row (toggle). The list row also draws a toggle that cannot be flipped from the list |
| Sign in with the phone (QR) | Steam page › "Sign in" / "Switch account" › QR sheet (300 s) |
| Which account is linked | Steam page, one row with the account name and "Connected"; OK on it restarts sign-in |
| Sign out | Steam page, last row |
| Refresh libraries | Home hero button, Library chip, Steam page row; also automatic at start, after a stream and after 30 min away |
| What Steam gives and withholds | One note row on the Steam page ("What comes from Steam"); not in the fixture, so no render |
| Last played, play time | Home shelves Continue and Most played, hero meta line, Hub Overview tiles ("Play time on Steam"), Library sorts |
| Last two weeks played | Hub Overview tile, only when present |
| Genres and tags | Home Genres row, Library Genre sheet, Hub meta line |
| Achievements | Hub Achievements tab and an Overview tile |
| News | Hub News tab (opens a URL outside the app) |
| Reviews, Metacritic, Deck status, features, DLC, PCGamingWiki facts | Hub Details tab; Deck status also a chip on Overview |
| Trailers, screenshots | Hub Media tab |
| Steam overlay | In-stream menu button, Alt key / X on a pad, only over a Steam copy |
| Credits | Settings › About |
| Steam favourites and collections | Not available (the Steam client keeps them); only GeForce NOW favourites merge in |
| Library browse of owned games | None |

Steam has no screen, shelf, badge or status of its own outside Settings. Nothing on Home, Library or the game page says that a number or tab came from Steam, except the tile label "Play time on Steam" and the footnote on the Achievements tab.

## 3. Screens

Shots in brackets. "Missing" means shots.rs has no render (listed in section 5).

### Home
[01-home, 01-home-play-focus, 01-home-details-focus, 01-home-light, 02-home-most-played, 03-home-genres, 04-home-native-plate, 05-home-cinematic, 06-home-compact, 30-home-text200, 63-steam-home, 71-refresh-result-dark100]

Purpose: resume the last game, or pick one. Contents: nav pill (Home, Library, Search), avatar and settings glyph, the featured game with logo, meta line and three buttons, then shelves of covers.

Entry/exit: start screen; Back on row 1+ goes to row 0, Back on row 0 with no history exits the app (code). Tiles open the Hub; the hero Play launches.

Problems:
- Per-game and global actions share one row: `Play`, `Game details` and `Refresh libraries` sit together over the art (seen). Refresh is library maintenance, not an action on the featured game. At text above 100% the three stack in a column and push the shelf down (seen, 30-home-text200).
- The refresh result pill ("1 new game.") appears between the nav pill and the avatar, away from the button that caused it (seen, 71-refresh-result-dark100).
- The settings affordance is a sun-like glyph with no label (seen, crop of any screen). It reads as brightness or theme, and it is the only door to adding a service.
- No hint of any kind on Home on a remote. The pad shows glyphs only after the pad is used (seen, 07-library-pad vs 07-library).
- The hero shows a single game chosen by rule; "Game details" repeats what OK on a tile does one row below.
- Focused cover grows and shows a two-line caption (title, engine, Ready dot) under it; unfocused covers carry no label, so a row of unlabeled posters has to be read by artwork alone (seen).
- Shelves: up to 40 items in a row with no end marker; Collections and hidden games have no UI to create them (code: nothing writes `user.collections` or `user.hidden`), so the Collections shelf cannot exist for a TV user.
- First run: "Add a way to play" heading sits on its subtitle, the descenders of "play" touch "Games appear here" (seen, 22-state-first-run). It also lists Steam as a tile although Steam cannot supply a game.
- Panel care: no dwell, dim or drift handling for the nav pill, avatar or hero in the game UI code (searched `screens/`, `model.rs`). Whether the shell covers this is untested here.

Lucent: one button size per screen holds. Overlap rule broken by first run. Hints missing, not wordy.

### Library
[07-library, 07-library-sort, 07-library-pad, 72-library-refresh-dark100]

Purpose: the whole collection as a grid. Contents: filter chips, a four-to-six column grid of covers.

Problems:
- One chip row mixes four kinds of control: a filter (All, one chip per engine, Favourites), a picker (Genre, Sort) and a maintenance action (Refresh libraries) (seen). Refresh is last, so it is the third place it exists.
- Sort is hinted by the Menu or Y glyph on a pad only (seen, 07-library-pad); on a remote nothing says it exists beyond the chip.
- The chip row can run wider than the screen at 100% text with five engines and scrolls sideways (code).
- A cover with no art becomes a dark tile with the title in the middle at a different size from the captions (seen, "Celeste Classic").
- Focus caption lands between rows and shifts nothing, which is good; but the focused card overlaps its neighbours when it grows (seen, Black Myth: Wukong).
- Library and Search are the same grid with a different filter; Search adds the keyboard (code).

### Search
[37-search, 38-search-keyboard]

Purpose: find a game by title. Contents: field, a 44-key on-screen keyboard (10 x 5), the result grid below.

Problems:
- Typing means moving the D-pad across 44 keys one at a time; no voice, no system keyboard (code, seen).
- The backspace key draws as a small placeholder box (seen, bottom-left key); "abc" and "Done" are words next to glyph keys.
- Results sit under the keyboard and start about halfway down the screen; the keyboard hides only when focus moves down (seen, code).
- No empty state shot (missing).

### Game page (Hub)
[08-hub, 08-hub-light, 08-hub-ways, 73-hub-steam-dark100, 74-hub-achievements-dark100, 75-hub-news-dark100, 76-hub-details-dark100, 77-hub-gfn-only-dark100, 78-hub-native-dark100, 64-steam-hub, 31-hub-text200, 20-state-failed]

Purpose: everything about one game, and Play. Contents: Back, title, Heart, More; logo; meta line; description; capability chips; Play and Choose where to play; a target line; tabs; tab content.

Problems:
- Choosing where to play is offered four ways: the second button, the More glyph, the Ways tab, and the Chooser sheet whose rows repeat the Ways rows (seen, 09-chooser vs 08-hub-ways). The button uses the refresh glyph (seen, 08-hub), which reads as "reload".
- The sheet adds a fifth control, "Remember for this game", and a third button, "Play here", while the page already has Play (seen, 09-chooser).
- Picking a way in the Ways tab silently switches back to Overview and moves focus to Play (code); the tab the user was on disappears.
- "Add a way to play" in Ways leaves the game and opens Settings › Integrations (code).
- Tab content replaces the hero: Play and the target line scroll out of view once a tab is selected (seen, 74/75/76). Playing from a tab needs a scroll back up.
- The tab row changes length per game (2 to 6 tabs) depending on what a web lookup returned, so the same screen differs from game to game and while data loads (seen: 64-steam-hub has two tabs, 08-hub has six).
- Overview shows at most four fact tiles chosen by a fixed order; later facts are dropped silently (code `take(4)`). Details then repeats reviews, Metacritic and release date (seen, 08-hub vs 76-hub-details).
- Details is a two-column grid of equal glass tiles over the artwork. Over the character's head, "Supergiant Games" and "Steam Deck" lose contrast (seen, 76). Facts such as "HDR: No" and "Ultrawide: Yes" read as features list rows, not as findings.
- Achievements tab: shows "40 to earn" and three store highlights with rarity, not the player's own progress, and states the limit in a footnote about "the achievements the Steam store highlights" (seen, 74). With a signed-in account (code) it should show Unlocked and Not yet; no render exists.
- News rows end in the word "Open", which leaves the app to a browser URL (seen, 75; code `PlayMedia`). Rows are full width, Ways rows are 1050 wide, Details is two columns: three row widths in one tab bar.
- Failure shows as a one-sentence card between the buttons and the tabs ("The connection was interrupted. Check the network, then choose Play to try again.") with no Retry control (seen, 20-state-failed).
- Wrong data in view: Overview reads "Last played 1 days ago" (seen, Chooser backdrop in 09-chooser).
- Light theme: the artwork half stays dark and a cream panel starts mid-screen with a hard edge (seen, 08-hub-light); the tab row needs an extra capsule to stay legible.
- Text 200%: the Back control stays small while the title grows, the description takes four lines, and the second button falls off screen (seen, 31-hub-text200).

Lucent: "Choose where to play" and More are both full buttons for one action; two button sizes appear on the page (Play 64 high, tabs and tiles lower) (seen).

### Chooser sheet
[09-chooser, 32-chooser-text200, 09-chooser-pad]

Purpose: pick one of the copies of a game. Contents: rows (engine, account, saves, Ready), remember toggle, hints, Play here.

Problems: duplicates the Ways tab; the hints "Back" and "Choose" are words next to glyphs (seen); the sheet is translucent so the hub's Play button and "Last played" show through its left edge (seen); the focused row's status column shifts about 8 px from the others (seen). Sheet widths vary from 960 to 1728 px across sheets (code).

### Integrations (Settings) and integration page
[10-integrations, 60-steam-signed-out, 62-steam-signed-in, 21-state-offline-pc, 11-pairing, 12-signin, 28-enter-pin, 29-approval, 41-add-pc]

Purpose: connect services, PCs and this TV.

Problems:
- Each list row draws a live-looking toggle and a chevron, but OK opens the page; the toggle is a display (code: `Focus::Integration` opens the page; `adjust` ignores it) (seen, 10-integrations).
- The integration page has no rail, no title row with an icon or summary, and no back glyph; the Settings rail disappears and only Back returns (seen, 60). The page is mostly empty (seen, 60/62).
- The enable toggle is the first row even when nothing is signed in; "Runs only while enabled" is the only explanation of what the integration is (seen).
- Steam page signed in: `Switch account / Connect`, the account row `Nitin / Connected`, `Refresh libraries / Refresh`, `Sign out / Sign out` (seen, 62). The value words repeat the label; the account row is shown like status but OK starts a new sign-in (code `Focus::Target`).
- No sync status on the Steam page: no "last updated", no game count, no "N games matched" (seen).
- Offline PC page: the PC row says "Offline" and offers no action (wake, retry, remove) (seen, 21-state-offline-pc).
- Mixed vocabulary for one idea: Connect, Pair, Sign in, Add, Link (seen).

### Steam QR sign-in sheet
[61-steam-qr, 61-steam-qr-light]

Purpose: sign in from a phone. Contents: QR on a white plate, a two-sentence instruction, "Closes in 4:12", "Waiting for sign-in", Cancel.

Problems: the instruction is a sentence plus a reassurance sentence (seen); the countdown is text, not a ring; the "Waiting for sign-in" hint is a sentence beside an hourglass (seen). Good: the QR is large, on white, with an honest countdown, and Cancel carries the Back glyph.

### Settings
[13-settings-appearance, 13-settings-appearance-light, 14-settings-streaming, 14-settings-streaming-limited, 14-settings-streaming-wifi, 25-settings-controllers, 26-settings-about, 27-theme-picker, 33-settings-text200, 52-diagnostics-dark, 53-diagnostics-light]

Purpose: look, stream quality, services, controllers.

Problems:
- Two theme pickers: four theme cards and a "Theme · Rosé Pine · All themes" row that opens a sheet of the same cards (seen, 13 and 27).
- Diagnostics and About use the same info glyph in the rail (seen).
- Integrations is a settings section, but connecting a service is the main first-run task and Steam, the PCs and GeForce NOW are what the Home screen depends on (seen).
- Controllers mixes four tools (lab, TV settings, GameSir, Leadjoy), a segmented "Button labels" row that wraps, and a static help table (D-pad, A, B) drawn like rows that cannot be focused (seen, 25).
- "TV controller settings" opens LG's own settings and leaves the app (code).
- About rows look like list items and open URLs outside the app (code).
- Streaming rows are a flat list under two small captions (Picture, Playing); "Preferred way to play" lives here, not on the game page (seen, 14). A note "AV1 is available at 60 fps" appears as a caption under Codec (seen).
- Values are words in every row (4K, 120 fps, 80 Mb/s); this fits the "values stay words" rule (seen).
- Reduce motion is under Appearance beside theme cards (seen).

### Launch and queue states
[17-state-checking, 17-state-queue-unknown, 17-state-long-title, 18-state-queue, 18-state-queue-text200, 19-state-starting]

Purpose: show progress from OK on Play to the first frame. Contents: art, logo, three step chips (Checking, In queue, Starting), one status line, a bar, Cancel.

Problems:
- The three chips appear for every engine, including a PC that has no queue; "In queue" is then a step that never happens (code `launch.rs`).
- The bar is indeterminate unless a queue position is known; at reduced motion it disappears (code).
- A dangling "· about 2 min" after the large status line (seen, 18).
- Cancel carries the Back glyph, the only key hint on the screen (seen).
- Failure drops back to the game page with a sentence (see Game page).
- The art stage hides the page; the rest of the layout is already tested for 200% text (code tests).

### In-stream menu (overlay)
[15-overlay, 15-overlay-text200, 15-overlay-pad, 40-running-transparent, 52-idle-warning, 53-idle-ended]

Purpose: leave or tune a session. Contents: art, logo, a line of stream facts, a line of network facts, five buttons: Resume, Stream settings, Stats · Off, Steam overlay, End session.

Problems:
- Stream facts are two lines of words and numbers ("GeForce NOW · 2160p · 120 fps · H265 · HDR · 32:12 elapsed", "8 ms network · 0.0% packet loss · 78 Mb/s") (seen).
- Five buttons in one row; "Steam overlay" shows only over a Steam copy so the row changes length per game (seen, code).
- "Stats · Off" is a button that cycles a value on each press, unlike every other button (seen; Menu key also cycles it).
- "Stream settings" leaves the menu for Settings; Back returns, but the Settings page shows no hint of it (code).
- End session shows a confirm that starts on "Keep playing" (code); no render (see section 5).
- Idle warning: a pill at the top reads "Still playing? Session ends in 0:59. Press any button to keep playing". The instruction is a sentence, and the pill's right edge touches its text (seen, 52-idle-warning). A bright element over the video for up to 60 s (panel care: stated as a dwell of the countdown).

### Stats HUD
[48-stats-full-healthy, 48-stats-compact-healthy, 48-stats-full-network, 51-stats-full-stale]

Purpose: live stream measurements. Problems: the Full layout is four glass columns of text rows; texts collide in the Smoothness block ("8.3 ms" sits on the p95/p99 line) and the Latency block's legend runs into its bar (seen, 48-stats-full-healthy). It covers the right half of the picture (seen). Diagnostic words (p95, p99, vsync, feed) are not for a TV viewer.

### Controller lab and device pages
[43-controller-lab, 43-controller-sticks, 43-controller-buttons, 43-controller-timing, 43-controller-shaping, 43-controller-captured, 44-controller-flash, 45-controller-lab-stream, 46-gamesir-ready, 46-gamesir-ds4, 46-gamesir-error, 47-leadjoy-ready]

Purpose: test and configure a pad. Problems: a dense instrument panel (percentiles, report rate, stick drift) inside the user-facing Settings (seen, 43); action chips (Test buttons, Rumble left, Rumble right, Flash test) sit in a row above content and below the title with equal weight (seen); the page scrolls past the viewport with no scroll indicator (seen, 43 bottom edge). These are device-specific pages that most users never need.

### Media viewer
[39-media-image]

Full-screen screenshot with chrome. Not reviewed in depth; reached only through Hub › Media, and the Media tab has no render in the shot pass (section 5).

### Sheets (other)
[07-library-sort, 27-theme-picker, 28-enter-pin, 29-approval, 41-add-pc, 42-edit-path]

Common problems: panel widths differ per sheet (960, 1080, 1320, 1500, 1728) (code); hint lines are words plus glyphs; waiting states use an hourglass and a sentence ("Waiting for approval").

## 4. Cross-cutting findings

1. **Hints.** Full screens carry no hints on a remote. Sheets carry word-and-glyph hints. Lucent: glyphs, not words, and hints belong on every screen with a non-obvious key (Sort, Overlay, tab switch).
2. **Where things live.** The same thing is reachable from several places (Refresh libraries in three, Choose where to play in four, themes in two); other things hide in one (Steam only in Settings, Steam overlay only in the stream menu).
3. **Rows.** At least five row styles: settings row, integration row, way row, news row, tile grid. Widths and heights differ.
4. **Words.** Value words repeat labels (Sign out / Sign out, Connect / Pair a PC); status words vary (Connected, Ready, Available, Online).
5. **State honesty.** Good: Refresh shows real stages with seconds after 3 s and a self-clearing result; the QR sheet counts down. Weak: toggles that do not toggle, tab sets that change with network lookups, the "In queue" chip for a launch with no queue, failure as a sentence with no action.
6. **Light theme.** Needs extra opaque capsules over hero art (code special cases); the hub's light mode splits the screen with a hard edge.
7. **200% text.** Layouts are tested for overlap (tests), but the hub and Home push the Play button and Refresh into a column and off screen (seen).
8. **Smoothness risks (code).** Focus grows each focused item with a scale and repositions its glass layers; the sweep bar redraws at 60 Hz while a launch waits. [INFERENCE] The Hub plan changes when web data arrives (the tab set changes). Not measured.
9. **Panel care.** No dwell, dim or drift logic found in the game UI for the nav pill, avatar, hero or the Stats HUD (the stream idle notice is a session timer, not a saver). Needs a check on the TV.

## 5. Screens the shot pass lacks

No change made to shots.rs. Missing or wrong:

- End-session confirmation: `16-overlay-end-confirm` renders a blank frame (transparent) because the fixture's Overlay(3) is now Steam overlay, so OK hands off to Steam instead of opening the confirm.
- Hub Media tab: `08-hub-media` now renders the Achievements tab (the tab index shifted when Achievements and News were added); no Media render exists. Media tab, Media viewer with a trailer, and the screenshot viewer chrome beyond `39-media-image` are not covered.
- Steam in the Integrations list, in first run, and with an error or expired code; Steam page with its note row; a Steam sync that is stale or failed.
- Hub Achievements for a signed-in Steam account (Unlocked / Not yet).
- Library empty ("No games found"), Search with no results, the Genre sheet, Collections shelf, Home with Collections.
- Auth sheet failure ("Connection not completed"), "No PCs found", generic "Connecting".
- Delete sheet; Running (renders nothing by design: `40-running-transparent`).
- Real-device behaviour: remote key hints, scroll, pointer. All renders are static frames.

## 6. Priority (how much it hurts a TV user)

1. Steam has no home: a bare settings page and invisible data (section 2).
2. No hints on a remote; hidden Sort, Overlay and tab keys.
3. The game page stacks actions, hides Play under tabs, and varies by lookup result.
4. "Refresh libraries" is a hero button, a filter chip and a settings row.
5. Integrations: toggles that do not toggle, an account row that re-signs in.
6. Overlapping text: first run, Stats Full, idle pill.
7. Failures and empty states are one sentence with no action.
8. Settings structure: Integrations buried, duplicate theme pickers, duplicate icons, a sun glyph for Settings.
9. Launch curtain shows queue steps for launches with no queue.
10. Back from Home exits the app with no confirmation; text at 200% pushes primary actions off screen.
