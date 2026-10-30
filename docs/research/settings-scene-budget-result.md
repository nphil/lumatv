# Settings over a full Home: the scene's groups

## What the TV showed

Settings > Inputs & devices drew Live TV and HDMI 1 (HDMI 2 and 3 after 15 s) and never HDMI 4, Input banner or Show input previews; the Rust shell's log printed `settings: at most 96 groups` every few hundred ms. Sound and Remote & pointer drew whole.

## Cause

The scene is one for every kind of the process: 96 groups and 256 layers (`compositor-protocol`). Home over a full dock (19 tiles, 3 cards, the status bar) holds 64 groups and 122 layers, because every focusable holds two groups (the lift group and the lit group) from the moment it is built. Settings gave each of its rows a plate: the rail's ten and the page's seven took 43 groups, the 32 that were left ran out part-way down the page, and the error went to the shell, which logs it at every redraw and leaves the rows made so far. Sound took 31 of the 32 and fitted by one.

The gear's panel, closed a moment ago, stays built for four seconds (26 groups, 45 layers): Settings opened from "All settings" found 6 groups.

## Change

- A Settings row keeps what its plate is made of and makes the plate (two groups and a layer) when the focus comes to the row; it goes when the row's lift is at rest again (`Flow::resting`, `settings/surface/list.rs`). The plate is the only thing the focus lifts in a row, so nothing on screen changes. The Inputs page takes 9 groups instead of 43, and the rail and a page of any length hold two or three plates each.
- While Settings is open a panel that is only kept built for its next opening (the gear's, the picker's, the drawer's) gives its layers back at once, as it does when another panel opens (`Panels::give_way`).
- `rust.ui` of `ctl shell` has `room` (layers, groups, tracks left in the scene) and `settings.built` (the rows of the page that were drawn).
- Test hook: `ctl shell show toast [TEXT]` raises a toast by the feed's own path (an underscore in TEXT is a space, a bar a line break, so quote TEXT for the shell; the first line is the title; it stays up 8 s). The report's `can` has `toast` while the kind is on.

## Numbers (host, a TV-like Home, every kind on)

| | groups used | layers used |
|---|---|---|
| Home and the resident kinds | 64 | 122 |
| Settings > Inputs, a plate per row | 43 (cap: `at most 96 groups`) | 113 |
| Settings > Inputs, now | 9 | 97 |
| Settings > Sound, a plate per row | 31 | 98 |
| the gear's panel | 26 | 45 |
| the picker, the drawer | 3 each | 10, 6 |

## Left as it is

Home's 64: a dock tile costs two groups, so a dock of 30 apps would pass the ceiling on its own (the protocol's note says the resting Home holds at most 40). A lit group made only for the tile the focus is on, or a dock that builds the tiles in view, would give back 20 or more. The screens that give a plate to every row (the gear's panel, the menu, the drawer's rows) fit today in what Home leaves and share the same ceiling.

## TV check

1. Settings > Inputs & devices: the seven rows of the first tab (Live TV, HDMI 1 to 4, Input banner, Show input previews) are drawn at once; `rust.ui.settings.built` is 7, `rust.ui.room.groups` is 8 or more, and the shell log has no `at most 96 groups`.
2. Gear, All settings, Inputs & devices: the same, and `rust.ui.panels.quick` is `gone` while Settings is up.
3. Down through the rows: the lift follows the focus row by row, with no blink as it changes.
4. `ctl shell show toast 'Placement_check|Second_line'`: a toast with a title and a body in the toast corner for 8 s.
