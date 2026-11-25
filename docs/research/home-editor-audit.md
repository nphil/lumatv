# Home layout editor audit

State audited: lumaglass `origin/main` at `af14f083` (UI source read) and the UI installed on the TV (192.168.1.242), 2026-11-25. Source: `rustui/ui/src/home/{editor,grid,model,dock,clock,widgets}.rs`, `settings/pages/{widgets,home}.rs`. Walked on the TV with `ctl remote` keys (silent, Home edit mode only); captures in `/data/home/tmp/shots/editor-audit/` (`contact-sheet.jpg` is the overview). Facts only; "code" marks what was read but not walked.

Key codes sent: Up 103, Down 108, Left 105, Right 106, OK 28 (hold = press, 1.1 s, release), Back 158, Menu 139.

## Capabilities

| Area | What the editor does |
|---|---|
| Enter | Hold OK (700 ms theme `motion.holdMs`) on a focused card on Home; Settings > Home & look > Home widgets > "Edit on Home". A hold on a dock tile opens the tile menu instead (code). |
| Grid | 12 columns, 16 equal rows (the shipped 3 pixel rows are converted to 16 on entry), gutter 20 px; a cell is about 131 x 37 px. Cards are whole cells and never overlap; no push or swap. |
| Select mode | Arrows choose the nearest card in that direction (distance, off-axis counts double); the dock is a selectable card. OK picks the card up. Up from a card with no card above in its columns goes to the bar. Back leaves (asks first when changed). |
| Move mode | Arrows move one cell. OK turns to size. Back puts the card back to where it was at pick-up. |
| Resize mode | Arrows move the bottom-right corner (Right/Down grow, Left/Up shrink). OK puts down. Back puts back to pick-up (move and size). |
| Bar | Add, Remove, Items, Undo, Cancel, Save. Left/Right skip disabled buttons, Down returns to the grid, OK presses, Back = Cancel. |
| Add | Bar Add (or Menu key, code) opens a six-row list; Up/Down, OK adds. The card takes the first free place at the first size of its ladder that fits, is picked up in Move mode; Back takes it away. A hidden card of that type comes back instead. No "No room" case walked. |
| Remove | Removes the selected card, no confirmation. Not the dock. |
| Undo | One step per put-down, add, remove. No redo. Pick-up/put-back is separate (Back). |
| Leave | Cancel or Back with changes: "Save changes?" Keep editing (focused, also what Back does) / Save / Discard. No changes: leaves at once. Save writes the file and leaves, no confirmation or toast. |
| Items | Smart home card only: Arrows choose an item, OK opens its menu (size, place, hide), Back is done. |
| Pointer | Drag a card (snaps to cells, 10 px start), drag the corner handle, click bar/list buttons (code). |
| Limits | No count limit; duplicates allowed (`clock-2`); a refused change says why for 1.8 s ("X is in the way", "Smallest size", "No room", "The dock stays"). |
| Pages/profiles | One layout per profile (`layout` is profile-scoped; Nitin and Isabel each have `profiles/<name>/layout.json`; `theme/layout.json` is the working copy). No pages or named layouts. |
| Elsewhere | Settings > Home widgets: Show tab (Off / Small 4 / Half 6 / Wide 8 columns per card, "Reset to default"), Order tab (swap with previous/next in reading order). Pop-up placement has its own separate editor. |

Save on this TV changed three files: `profiles/nitin/layout.json`, `theme/layout.json`, `profiles/index.json` (work hash).

## Widgets

Min size and add ladder in columns x rows of the 12 x 16 grid (code, `grid.rs`).

| Widget | Min | Add ladder (first that fits) | Options |
|---|---|---|---|
| Clock | 2x3 | 4x4, 3x4, 3x3, 2x3 | None in the editor. Settings > Home & look: Clock on Home (Analogue/Digital/Off), 12-hour, Greeting; theme `clock.secondHand`; per-card `settings.style` read from the layout file, no UI. |
| At home | 2x3 | 4x5, 4x4, 3x4, 3x3, 2x3 | Title in file; Home Assistant page. Hidden in the owner's layout. |
| Weather | 2x3 | 4x6, 3x6, 3x4, 2x4, 2x3 | Settings: source (built-in/Home Assistant), entity, units, ZIP; `weather.days`. |
| News | 3x4 | 5x7, 4x6, 4x5, 3x5, 3x4 | None found. Headlines rotate while editing. |
| Cameras | 3x3 | 6x5, 4x5, 4x4, 3x4, 3x3 | Scrypted link page. |
| Smart home | 3x3 | 6x9 ... 3x3 (8 steps) | Items button; Settings > On Home. |
| Apps dock | 3x3 | not addable (fixed) | Moved/resized only. Tile size, gap, padY, fade only in `layout.json` (120/16/40/110). Tile menu on a tile (hold OK or Menu): Open, Move on the dock, Hide from the dock, Rename, Uninstall; tile move: arrows, OK done, Back cancels. |

Clock card by size (code `widgets.rs::plan`, observed on the TV): below 260 px wide or 120 px high "tiny"; under 560 wide and 300+ high "column"; 560+ wide and 230+ high "month"; 700+ wide "zoned"; else "regular". Shows: greeting, time (digital) or dial (analogue), weekday and date, a week strip with today marked, one to three months. The owner's clock is analogue, 7x4: dial, date and a month grid (`00-home-before`, `01b-home`). At 4x4 the dial and a week strip (`20-clock-span4`); at 4x3 a small dial only (`21-clock-rows-min`).

The owner's layout (before): clock 7x4 at 0,0; smart home 7x5 at 0,4; weather 5x9 at 7,0; dock 12x7 at 0,9; At home hidden; no News or Cameras card.

## Job walks (start: screensaver up, set idle)

Presses counted from the stated start; "hold" is one OK held 1.1 s.

| Job | Keys | Presses | Screen |
|---|---|---|---|
| Enter edit mode | wake key; Up/Down to a card (3 sent, ring not shown; the focused card lifts); hold OK | 1 + 1..3 + 1 hold | `00-home-before` screensaver; `01b-home` no focus mark; `04-edit-entered`: bar "Edit Home · OK moves · Add Remove Items Undo Cancel Save", ring on the focused card |
| Add News | from clock: Up (bar, on Add), OK, Down x3, OK | 6 | `27-bar-focus`, `32-add-list`, `33-add-news-row` (row highlight faint), `34-news-placed` (3x4, hint "3 × 4 · OK sizes · Back undoes"); put down: OK, OK |
| Move | dock: OK (`06-dock-pickup`), Up: toast "Weather is in the way" (`07-dock-up`); after shrinking, OK, Down moves one row (`11-dock-pickup-down`); Back returns it (`12-dock-back`) | 2 to move | hint "12 × 7 · OK sizes · Back undoes" is the only mode cue; News Right: "Weather is in the way" (`35-news-move-right`) |
| Resize | clock: OK, OK, Left x3 (7 to 4), Up (4 to 3), Up (refused "Smallest size") | 7 | `19-clock-sizemode` (corner chevron), `20-clock-span4`, `21-clock-rows-min`; News Right: refused (`37-news-size-right`); dock Up: 12x6 (`09-dock-size-up`) |
| Change options | Items needs the smart home card selected and the bar focused | unreachable | `50-onhome-items-enabled` Items lit with smart home selected; the bar is reached only by Up from a card with nothing above, and the clock is above it, so the selected card is the clock and Items is grey (`31-bar-focus`) |
| Remove | in the bar: Right (Remove), OK | 2 (plus 1 to enter the bar) | `40-removed` News gone, no confirm, selection jumps to Weather, focus stays on Remove; Right again: Undo (`41-bar-right`); OK: News back (`42-undone`) |
| Move the dock | Right/Down/Up to the dock (`05-edit-r`), OK, arrows; blocked by neighbours until the dock is shrunk first | 2 + moves | `06`..`12` |
| Change clock/calendar | not in the editor: size only (7x4, 4x4, 4x3 above); style and 12-hour live in Settings | 7 | `18`..`21` |
| Leave without saving | Back (or Cancel), Right x2 (Discard), OK | 4 | `46-ask-dirty`, `47-ask-discard-focus`, `48-after-discard` Home as before; Back on the question keeps editing (`14-keep-editing`) |
| Leave with saving | Up (bar), Right (Remove), OK, Right x3 (Save), OK | 7 | `51-clock-removed-save-focus`, `52-after-save`: clock gone, edit closed, no toast |

Menu key (139) while editing opened Quick Settings over the editor (`22-add-list` is that panel, not the add list); Down/OK then moved inside the panel and opened the OLED brightness slider (`23`, `24`; value 25 unchanged, closed with Back x2, `25`, `26`). The editor stayed in Resize mode underneath.

## Friction observed

1. Home gives no sign that a hold enters editing, and no ring on Home cards; focus shows only as a lift (`01b`, `02-focus-key`, `03b-focus-d2`). The first key after the screensaver is consumed.
2. The hint line says only what OK does ("OK moves"); it names neither the bar (Up from the top card) nor Add. The Menu key named in the code reached Quick Settings instead (`22`).
3. Select, Move and Resize look alike: the same accent ring; only the grey hint text and a small corner chevron differ (`04`, `06`, `08`).
4. "Back undoes" reverts the whole pick-up (move and size), while the Undo button reverts one put-down step; the two are separate histories.
5. A packed grid refuses nearly every move or size: no push, swap or ghost of where it would fit; the toast names one blocker only (dock Up named Weather though Smart home also blocks) (`07`, `35`, `37`).
6. Add list: types already on Home (Clock, Weather, Smart home) are offered and would duplicate; only a hidden one is marked "Hidden"; the row focus is faint (`32`, `33`). The new card appears in the first free slot, not where the list was.
7. Undo does not re-select the restored card; focus stays on the bar, selection stays elsewhere (`42`, `43`). No redo.
8. Remove has no confirmation and moves selection to another card (`40`).
9. Items is reachable only when the smart home card has nothing above it (`31`, `45`, `50`); Items and Remove grey out with no reason.
10. Per-card options are absent from the editor: clock style, weather source, news have no editor control; the clock changes shape by size with no preview of the alternatives (`20`, `21`).
11. Select moves by distance: Right from the smart home card selected the dock, not Weather (`05-edit-r`).
12. Leave: Back on the question means Keep editing, so Back, Back never leaves; the question does not list what changed; Save has no confirmation or toast (`13-ask`, `14`, `52`).
13. News headlines keep rotating while editing (`35` to `38`).
14. The dock card cannot be entered from Home by hold; the tile menu opens instead (code), so the dock is reached only inside the editor by arrows.

Untested: pointer drag, Settings "Edit on Home", digital clock style, "No room" on Add, the Items menu, entering by holding OK on the dock, standby start.

Owner files were backed up before the walk and restored after the Save pass; the SHA-256 of `theme/{layout,theme,tint}.json`, `profiles/index.json` and both profile folders matched the pre-walk values, and Home was captured as before (`53-home-restored`).
