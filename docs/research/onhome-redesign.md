# Smart home card redesign (2026-11-03)

Home Assistant things as raised glass items inside the Smart home card, in three sizes, with a sheet for what an item has more to say.
The helper's `onhome` topic and the integration path are unchanged; what is new is what the card does with them.

## Items

An item is one thing on the card: a shared entity (the helper's tile) or one of the TV's cameras (the `cameras` topic, at most two, first).
Each item is a glass mini card (the dock's raised material): quiet glass at rest, and when it is in front its glass grows about 9 px a side,
rises 4 px and takes the lit glass (a brighter edge and lip, sheen, a deeper shadow) on the flowing-focus spring. The words and pictures on it
stay on whole pixels and do not scale; only the glass grows. A press (OK down) shrinks the glass to `focus.pressScale` with the pressed
shadow until OK is released.

In the light theme the card is already a pale veil, so a white wash is a slab. There the items at rest keep 60 % of the card's veil, and the
lit item is glass lit from above: white at its top, almost clear at its foot, a rim, a short soft shadow (host render: lit 235 at the top and
219 at the foot, items 214, card 210). The viewport fades the shadow where it clips it, so no seam shows under a one-row card.

The scene holds 96 groups and Home's dock and bars take most of them, so an item costs layers and no groups at rest: its lift (two groups) is
made when it comes to the front and goes once it has settled back, never more than five at a time, however many items the card holds.

| Kind | Default size | OK | Hold OK | Shows |
|---|---|---|---|---|
| Light | small | toggle | colour sheet (brightness, temperature, colour, effects) when the lamp has more than on/off | the bulb glyph lit in the lamp's own colour with a glow that follows its brightness; dim neutral when off |
| Switch, fan | small | toggle | fan: speed sheet | a switch (track and thumb); the thumb travels |
| Scene, script | small | run | | glyph, name |
| Climate | wide | controls sheet | | current temperature large, set point, mode |
| Cover, media, lock | small / wide / small | sheet / sheet / lock, or ask to unlock | | glyph, state |
| Sensor, number | small | history sheet | | value large, unit, name |
| Sensor, on/off | small | history sheet | | glyph and state word |
| Camera | wide | the Cameras app's full view | | the live still, name, offline when it is |

The owner can set any item to small (1 x 1 cells), wide (2 x 1) or large (2 x 2). Menu on an item opens a small glass strip: three size
glyphs, Move, Hide. Left and Right move over the sizes and the item takes each at once (Back puts it back), OK keeps one; Move reorders with the
arrows. The same menu is reached from Home's layout editor: its bar has an Items button for a chosen Smart home card, which hands the keys to
the card's items. Sizes and order are kept per entity by the helper (`onhome/picks.json`, `sizes` beside `picks`) and come back on the topic
as `size`. A size the card cannot show falls back one step at a time: large to wide when the card has one visible row or one visible column,
wide to small when it has one visible column.

## Grid and scrolling

The card's content box (inside the card's usual padding) sets the grid. Rows: as many whole rows as the least item height lets in; columns:
whole items of about the row height; item size whole pixels, gutter `space.3`, leftover pixels split at the two ends. One visible row scrolls
sideways, several rows scroll up and down, by whole strides (item + gutter) on the flow's scroller spring, so a focus that moves past the
edge scrolls only as far as it must and the focused item is whole. The next stride shows as a deliberate peek of 45 % of an item (a row at the
foot, an item at the right of a one-row card) behind the compositor's clip feather; it is never a sliver and a peeked item cannot be pressed.
Nothing is drawn once scrolling has settled. The Magic Remote's wheel scrolls the card while the pointer is over it: the compositor sends
`wheelup` and `wheeldown`, and the UI lists them in its `takes` only while a card scrolls and the pointer is over it.

The heading is the title and, while an item with a menu is in front, one quiet Menu glyph; there are no hint rows. It is drawn when the card
is tall enough for it.

## Focus

OK on the card enters it and the focus lands on the item last focused (the first at first). Arrows move by position (the kit's beam rule:
the item that overlaps in line, nearest first; no wrap). Back leaves the card; Back inside a sheet closes the sheet. The focus is the lift
itself; there is no outline.

## States

On, off, pending, unavailable. A toggle shows the new state in the same frame as the key (optimistic), marks the item pending (a dimmed
glyph is not used; the control keeps moving) and keeps it until the helper answers: a 200 keeps it, a refusal or no answer eases the item
back and says why in the heading for a few seconds. An unavailable item has dim ink, "Unavailable" and no lift to press. A lamp's colour is
data; the glyph is darkened in lightness only as far as it needs to reach 3:1 on the glass, the glow stays true. A short OK on a lamp, a fan,
a switch or a scene that Home Assistant lets the TV do nothing to (the helper's tile has no `toggle` or `run`, and says why in `why`) sends
nothing and opens no sheet, which would hold nothing: the heading says why for a few seconds.

## Sensor sheet

OK on a sensor opens a sheet over a stepped back Home. The helper serves `GET /v1/onhome/history?id=&range=` from Home Assistant's own
recorder through the TV's Home Assistant link (hourly statistics beyond a day where they exist). By kind:

- number: the current value large, a smooth line with a soft fill under it, light grid lines at nice values with the unit and the times on
  the axes, the extremes marked, minimum, average and maximum below. Humidity and other percentages read 0 to 100; others take a padded range.
- on/off: bands of on and off over the range with when it last changed.
- text: the recent changes with their times.

Left and Right change the range (1 h, 6 h, 24 h, 7 d, 30 d; 24 h at first). Without the link the sheet says so in one line and keeps the
current value.

## Colour sheet

Hold OK on a lamp with more than on/off: the colour wheel with a marker, hue and saturation rows, brightness, colour temperature (lamps that
have it), presets and, when Home Assistant lists them, effects. Left and Right change the row in front, Up and Down move between rows. The
pointer picks on the wheel, the tracks and the presets. A track whose end the sheet's ground hides (white on the light sheet) and a preset too
near the ground are drawn with a hairline of ink. What the sheet shows is the lamp's own: the marker stands at the hue and saturation it says
(`hs_color`), a lamp showing a white (`color_mode` `color_temp`) has no colour on the ring, the warmth track runs between the lamp's own limits
(a Hue bulb's 2000 to 6535 K, a Canvas's 1200 K) and the effect chips are all its effects (a floor lamp lists 111). The integration holds a Show
to 10 calls in 10 seconds, so the sheet's calls are paced as a bucket of three that fills one in 1.5 s: the first three of a turn go a quarter of
a second apart, then one in 1.5 s with the latest value (at most nine in any ten seconds).

## Thermostat sheet

OK on a thermostat: the room now (large, with what the thermostat is doing and the humidity it reads), its set point with a step down and a step
up (two of them, heat to and cool to, for a range), and the modes as chips with the one it is in filled. Up and Down move between the set
points and the modes, Left and Right step the set point in front by the thermostat's own step (a set point rests 300 ms and is sent once, as
`temperature` or `range`, which the helper makes `climate.set_temperature`) or walk the chips, and OK chooses the chip the cursor is on
(`hvac_mode`, `climate.set_hvac_mode`): a mode is not switched in passing. The pointer steps with the buttons and chooses with a chip. A
thermostat that is off has no set point and shows its mode; one Home Assistant lets the TV do nothing to shows what it reads and says why.

## Motion

Compositor tracks and layer animations only: the lift and scroll on the flow's spring, the lit glass, a bulb's light-up and a switch's
track as opacity cross-fades of prepared layers, the thumb's travel, the sheet's arrival (`motion.layer`) and a chart's reveal as a clip
that opens. Colour changes between two lit colours go through the OKLab midpoint (a prepared third layer), not the RGB mix. Nothing is
repainted, re-laid out or uploaded while an animation runs; the card draws nothing at rest.

## Measured

`python3 tests/host_budget.py --surfaces onhome` on the host harness (Mesa, 120 Hz, the workspace built at opt-level 2). A frame's work is the
harness's estimate of its GPU time (1.5 ms + 0.085 ms a layer) plus this process's time to apply it; the set's per-pixel cost is not in it.
The budget is 8.3 ms.

| Window | Frames that drew | Most layers | Work, max | p99 |
|---|---|---|---|---|
| enter the card | 114 | 121 | 11.8 ms | 11.6 ms |
| along a row | 178 | 48 | 5.6 ms | 4.7 ms |
| toggle a switch | 44 | 22 | 3.4 ms | 3.4 ms |
| toggle a lamp | 155 | 36 | 4.6 ms | 4.6 ms |
| long scroll | 306 | 63 | 6.9 ms | 6.9 ms |
| sensor history | 322 | 127 | 12.3 ms | 12.2 ms |
| colour sheet | 306 | 127 | 12.3 ms | 12.3 ms |
| rest | 0 | 0 | 1.5 ms | 1.5 ms |

The card's own frames (along a row, both toggles, the long scroll) hold the budget. The windows over it change Home whole, as the cards of
`home-widgets` do (`frames-120-result.md`): the focus coming into a card is 3 frames of 119-121 layers, and opening or closing a sheet is a run
of frames of 122-127 layers (5 over 80 layers in the sensor window, 16 in the colour window); an open sheet draws 35-57 layers a frame. At 60 Hz
every window fits 16.6 ms. The rest window draws no pixel: the shell's commit once a second changes nothing.

## Open

- Home Assistant must allow the TV to control a kind of thing (*Configure, What the Show may control* in the LumaTV entry: light, fan, climate)
  or its items are shown and say why they do nothing. Effects, the lamp's white range and its hue need lumashow-ha 0.3.1.
- History needs the Home Assistant link (`lumatv-helper ha-link`).
- The wheel's direction follows evdev (a turn away is up); untested on the TV. Nothing here has been checked on the TV.
