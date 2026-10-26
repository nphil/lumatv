# Home widgets: the editor and the size-aware cards (2026-10-25)

## Resume here

Status: built and pushed (c2d52bc and the commits after it); checked in the offline host only (renders, unit tests, the layout audit). Nothing was run on the set.

What exists:

- **The model**: `LayoutGrid.js`, a port of Nitin's September simulator (`/data/home/lgtv/sim/sim.js`): the same area, cell, collision, add and place-a-copy rules, so
  the simulator and the TV agree on what a layout means. `tests/layoutgrid.test.js` (bun) carries copies of the simulator's own functions and checks parity.
- **Settings › Home & look › Home widgets** (`settings/PageWidgets.qml`): per card Off | Small | Half | Wide, Add rows for the cards that are not on Home, an Order
  tab (move up / down), "Edit on Home" and "Reset to default". A change that does not fit is refused and the row says why for a moment ("News is in the way").
- **The layout editor on Home** (`LayoutEdit.qml`, opened from that page or by holding OK on a card): a grid over Home; OK picks a card up, the arrows move it by
  the cell, OK again turns to the size, the arrows size it, OK puts it down; Back in a pick-up puts the card back. A bar: Add, Remove, Undo, Cancel, Save. Menu
  opens the add list. Back with nothing picked up asks "Save changes?" when something changed. Home draws the layout being edited, nothing is written before Save.
- **The file**: only Save, a Settings change or `lumatv layout` write `/var/lib/lumatv/theme/layout.json`, and only what differs from the shipped layout
  (`Grid.forFile`; "Reset" writes `{}`). Home applies it live (LumaHome polls the file; its own writes are applied at once and the poll finds nothing new).
  A hidden card is `"hidden": true`, so it keeps its place; the dock cannot be hidden or removed.
- **The cards use the room they get** (see the table): Clock, Weather, News and At home each plan their layout from their own width and height.
- **Workstation**: `tools/layout.py` (the on-TV tool, `lumatv layout get|check|put|set|reset`) and `tools/layout_push.py` (from a workstation, over SSH, atomic
  rename, no backup). The simulator's Save calls it (`server.py` with `LUMATV_PUSH=1`, POST `/api/push`).

Way back: `lumatv layout reset` on the TV (or "Reset to default" on the page). No copy of the previous file is kept (Nitin, 21:07: no backups for small reversible changes).

Next, in this order: (1) the TV check below; (2) the items under "Not done".

## Deploy (for Main)

1. `tools/build-ipk.sh` (it ships `tools/layout.py`; the build's QML compile gate loads every file).
2. Install as usual; the compositor restarts for new QML. `lumatv layout get` should print the current file; Nitin's file (halves, no news) is valid as it is.
3. The shipped `payload/theme/layout.json` is unchanged. The first Save in the editor converts the shipped pixel rows to the editor's 16 uniform rows, as the simulator does.

## TV check (pass criteria)

1. Home: hold OK on the Weather card for about a second: the grid appears, the status bar is gone, the card has a ring. Pass: it opens once and OK's release is not an OK.
2. OK, Left twice, OK, Right, OK (pick up, move two cells left, turn to size, a column wider, put down), Back, choose Save. Pass: Home keeps the new arrangement; `lumatv layout get` shows `"grid": {"rows": 16}` and the cards.
3. Settings › Home & look › Home widgets: set News to Half. Pass: News appears on Home at once, or the row names what is in the way.
4. Same page, Reset to default: Pass: Home shows the shipped arrangement, `lumatv layout get` prints `{}`.
5. Frame budget: `python3 docs/research/perf/gate.py --surfaces home-idle,home-dpad` (p90 render under 3 ms, as before this change). The editor is hidden when closed, so Home's own surfaces carry its cost; there is no gate surface for the editor itself.

## What each card does at each size

Sizes are the card's own width and height in pixels (the 12-column grid is 1792 px wide, Small 4 columns, Half 6, Wide 8).

| Card | What it draws |
|---|---|
| Clock (analog and digital) | under 260 × 120: the dial or the time with the date beside it; the mock's own boxes otherwise; under 560 wide and 300 high or more: dial or time, date and the week strip stacked; 700 wide and under 230 high: face, date and week strip side by side; 560 × 230 or more: face and one to three months with today marked |
| Weather | under 620 wide: now, the stats line, the week as rows; 620 to 1100 wide and 240 high or more: now with a stat a line, the week as a list with range bars; wider or lower: the week as columns with range bars; under 130 high: icon and temperature only |
| News | portrait: the hero (46 % of the height, 96 to 262 px, none under 240 high), the headline in two or three lines, the summary where it fits, the meta line (none under 96 high); 700 wide and under 420 high: the hero on the left, the words on the right |
| At home | one chip column per 250 px of width up to four, as many rows as the height holds; no title under 130 high; the empty state centred when the lane is 180 high or more |

A card drops the lines that would not fit (the title, the summary, the stats); nothing is clipped or overlaps, and an empty source says so in a short line
("No weather reading yet", "No stories yet") instead of drawing a blank card.

## Decisions

- **Workstation tool, not a LAN listener.** PLAN allows no settings listener: the helper's API is loopback only (127.0.0.1:18731) and the one LAN listener is
  AI access, off by default and a human-only decision (Principle 0). So the simulator stays a workstation tool: Save → `server.py /api/push` →
  `tools/layout_push.py` over SSH. On the TV the same file is written by `lumatv layout`. Serving the simulator from the helper would need that listener.
- **One grid model.** `LayoutGrid.js` is the simulator's model; LumaHome's own geometry calls it, so a card is drawn where the editor and the page put it.
- **16 uniform rows while editing** (as the simulator): the shipped 222 / 238 / 392 px rows are converted by `toUniform` when the editor opens; the Settings page
  works on whatever grid the file has. The file stores the difference from the shipped layout only, the cut `theme.py minimize` already makes at every apply.
- **Hide is a flag**, so a card keeps its place and comes back to it. Adding a type that has a hidden entry shows that entry.
- **Delegates are keyed by card id**: a move in the editor moves and sizes the delegate; a widget is built once per card and never rebuilt by an edit.
- **The editor is a Resident in LumaHome** (z 59, built after the layers, under the confirm), not a layer, so Layers.qml has no new step.
- **Plugin cards**: the At-home card is where plugins show; a widget pack's own card appears on the page only if its entry is already in `layout.json`.

## Not done

- **Widget-pack discovery.** QML cannot list directories and the helper has no list of installed packs, so the Add list offers the four built-in types.
- **Hourly weather.** The helper publishes seven daily days and no hours; the wide Weather card shows the week. Hours need a helper change.
- **The simulator's groups and second dock** are not supported on the TV: the editor is single-dock, as Home is.
- **Not measured on the set**: the 3 ms p90 gate (the editor surface is hidden when not editing, the grid is one ShaderEffect over its own rectangle, no
  ShaderEffectSource is built on show, widgets are built once per card id), the text at 200 %, the pointer drag (written, not exercised in the host).
- **Fixed pixel sizes in the cards**: no text-scale hook exists for the 200 % check; the audit runs the doubled pass on the scenes.

## Evidence

- `tests/layoutgrid.test.js` (bun: 22 tests, including parity with the simulator's functions), `tests/test_layout_tool.py` (the tool's check/put/set/get/reset),
  `tests/test_widgets.py` (host: the shipped layout drawn where it was, a hidden card not drawn and not focused, Up from a card with nothing over it reaches the
  bar, Settings page choices and refusals, the editor's pick-up, move, size, Save and what it writes, Back asking and Discard, hold of OK).
- Scenes (`tests/qmlhost/scenes_widgets.py`, dark and light): `37-home-*` the cards at each size and the Home arrangements, `38-settings-home-widgets*`,
  `38-home-edit*`. Renders in `docs/research/widgets/`; `before-halves-on-the-set.png` is the set before this work (the empty clock and weather cards).
