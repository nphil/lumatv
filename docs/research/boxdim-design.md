# Box dim: dimming only the bright, still parts of an app's picture

`theme.oledcare.boxDim`, off until its TV check passes. LG's own logo-luminance adjustment, done by the compositor for an app's UI: a bright box that has not changed for minutes is drawn with less light, the rest of the screen is not. The retention Nitin saw came from Netflix's bright static boxes; the moving parts of a screen do not imprint, the still bright ones do.

The module is `src/boxdim.rs` (the grid, its clocks and the gain map; no GL), the GL side is `render/gl/boxdim.rs`, the software renderer applies the same map and takes the same samples on the CPU so the host tests see the picture. The same grid tells the OLED care's app features (`appDim`, `appShift`) a still app from a changing one.

## The grid

64 x 36 cells of 30 x 30 output pixels. Each cell keeps: when it was last damaged, when its picture last changed, its last sampled numbers, and its gain (1 = untouched). The map the renderer multiplies with is the cells' gains, as an 8-bit 64 x 36 texture sampled bilinearly in screen space.

## Still: the picture, not the commits

The first rule took a cell's damage for a change. On the set (round 16) LG's browser and Plex commit the same frame again every 4 to 5 s with the whole surface damaged, a static `data:` page included, so neither the box dim nor the app features ever engaged. A picture changes through a commit or not at all, so damage still says which cells may have changed; whether they did is the picture's to say. A cell damaged since the last sample is judged by the next: its numbers moved by more than 2 code values, it changed at its last damage; they did not, it is as still as it was, however often it was committed.

A sample is begun 4 s after the last when anything was damaged since, on a tick that drew nothing (8 s on any tick, for a client that commits every frame); never within 2 s of a key. While a dimmed cell or an engaged app card has damage no sample has judged, a check is begun right after the frame that drew it, the first commit after a quiet stretch at once and the following ones of a run 250 ms apart, so a new picture under a dim shows dimmed for a frame, not longer. A screen with no commits is never sampled after the first sample. Damage left unjudged for 12 s (sampling stalled) counts as change, and an output that cannot sample at all takes every damaged cell as changed: a picture nobody looked at is never taken for still.

A sample that finds 16 cells or more changed (a 120 x 120 px patch) is a change of the picture: every cell restarts. Fewer (a clock, a caret, a spinner) restart only their own cells. 36 cells (a 64th of the screen) is a change of the app's picture for the app features, their old rule's threshold. A key or pointer movement, a window appearing, moving by more than 4 px, leaving or changing stacking order, a hole changing, a transition, the screensaver, a capture being armed, and the switch going off restart every cell without a sample.

## The sample

The app's items at the grid's resolution, on the GPU: two passes of a 15-tap box filter, taps two output pixels apart (a bilinear tap between two texels averages both, so 15 taps are 30 pixels exactly). Across, each item into a 64 x 1080 target, holes cleared black before their item; down, into 64 x 36. Each cell gets three 8-bit numbers: its mean luma (Rec. 709 weights), that of its left 14 columns and that of its lower 14 rows, so a change that keeps a cell's mean (a word replaced by one as long) still moves a part. The items are drawn where their window places them, the app features' shift taken away, and nothing of any dim is in them: neither care's own dim nor shift can look like a change of the app.

The passes are followed by a fence and read back on a later tick, once it has signalled: the loop never waits for the GPU. Round 16 measured the first single-pass sample (15 x 15 taps) at 56 ms of the loop, its build and first draw, and every later one at 2.8 ms, the read waiting for the GPU; the bar was 2 ms. Now each pass is a 15-tap shader, the programs and targets are made ahead in four pieces on ticks with nothing else to do (one compile each, then the targets with a first draw through each program), and what the loop spends on a sample is its submission and a 9 KB read of finished work. `ctl status` reports the loop's worst on both (`sampleUsMax`, `warmUsMax`).

Video is on its own plane: the hole left for it draws nothing, samples black, and is never dimmed because there is nothing of it in the compositor's output. A hole therefore never makes its surroundings count as bright, and no gradient is ever drawn over the video's edge: the dilation below puts the feather inside the hole, where the pixels are transparent, not on the graphics beside it.

## The gain map

A cell is eligible when `boxDim` is on, its picture has not changed for `boxDimAfterMin` (3) and it has been sampled. An eligible cell's target gain follows its brightness, with no threshold to flap on: 1 at luma `boxDimMinLuma` (0.35) and below, falling linearly to `boxDimTo` (0.8) at luma 0.65 and above. Everything else targets 1.

The targets are then dilated by one cell (each cell takes the lowest target of its 3 x 3 neighbourhood) and blurred by a [1 2 1] kernel on both axes. The dilation is what keeps the picture looking right: the whole bright shape gets one gain (the floor across it, within 2.5 % over its outermost 15 px), so every contrast ratio inside it is what the app drew, and the feather falls on the dark surround, where a gradient of 20 % over 90 px is below what the eye picks up on near-black content. With the floor at 0.8 the map reads 0.8 on the shape, 0.85 one cell out, 0.95 two cells out and 1 beyond; the texture's bilinear sampling turns those into ramps, so no gain step between two pixels exceeds a code value on white. No rectangle is ever visible.

Each cell's gain moves toward its target: down at (1 - `boxDimTo`) per `boxDimRampSec` (60 s), stepped every 2 s (a step is under two code values); up at the full span per 150 ms, stepped every frame. A step that changes the 8-bit map damages the changed cells plus one cell of margin (the bilinear reach), so the frame redraws that region and nothing else; its items keep their own damage (no shm re-upload). A step on a screen with nothing pending is a quiet frame, as an OLED-care beat is, so the slowed idle tick stays slowed.

Hysteresis, so it never pumps: the brightness-to-gain curve is continuous; a changed cell waits the full `boxDimAfterMin` again; a commit of the same picture changes nothing, so a client's redundant frames neither restore nor restart anything; and small changes inside a dimmed region do not restore their neighbours: a clock that ticks in a dimmed sidebar keeps the sidebar's gain through the dilation, so nothing around it pulses once a minute. A region that really changes (16 cells or more) restores everything within 150 ms of the sample that sees it and the count starts over.

## Where the map is applied

Only items of app windows: not the shell's two windows (they have `enabled`), not the wallpaper plane, not the cursor, the switcher's backdrop or the UI engine's layers. An item in a transition (posed) is drawn by the pose program, which has no gain: a transition restarts the map anyway. The GL renderer links a second copy of the item program with one more texel fetch and uses it for app items only while any cell is below 1. While no cell is, the frame path is exactly today's: the plain program, no second texture, no pass, and `Plan::sole_fullscreen` still hands the window to the plane whole. The first step that takes a cell below 1 is the one frame that leaves direct scan-out, on a screen that has been still for minutes.

Captures: `screencap shot` and `record` arm compose-only and redraw; while armed the map is snapped to 1 at once (not ramped), so what is written is the picture as the app drew it. The debug `ctl capture PATH` reads the page as composed, dim included: that is what the TV check uses to see it.

HDR: no special case. The gain multiplies the compositor's graphics output, which the display engine blends over the video plane in every mode; the video plane is never touched and the graphics layer's light is what wears the panel in SDR and HDR alike.

`appDim` (the whole-window dim after minutes of no key) multiplies with this: both restore on a key within 250 ms.

## Not worse than nothing

The gain is at most 1 everywhere, so no pixel ages faster than it would without the dim. Inside a bright shape it is uniform (dilation), so the dim adds no structure where the shape will imprint; it only lowers the imprint's depth, roughly to 0.8^1.75 = 0.68 of it for the panel's superlinear wear. The feather lies on the dark surround, where a 20 % gain change is a change of a few nits and ages nothing visibly. The dim cannot imprint its own outline because its outline is the content's outline, moved outward by one cell onto dark pixels, with less contrast than the content's own edge.

## Cost

Idle, nothing: a flag per item at plan time, no map, no program switch, direct scan-out intact; nothing of the grid runs while neither the box dim nor an app feature is on. Tracking: a cell mark per damaged cell per frame, and one sample per burst of commits (none on a screen that commits nothing), each a submission and a 9 KB read of finished work on the loop. Engaged: one composed frame per 2 s during a ramp (none between steps), each with one extra texel fetch per app fragment from a 9 KB texture, and a check after the first commit of a run. Restoring after a key: about nine composed frames in 150 ms with the gained program, the same frames the app's reply damages anyway. On the host (Mesa llvmpipe, 1080p) a full composed frame costs the same with the map as without (3.7 against 4.0 ms); the warm-up's pieces and a sample's submission are in `boxdim-result.md`. The TV check compares `renderUsMax` and frame deadlines against the switch off.

## Tokens

`oledcare.boxDim` (false), `boxDimAfterMin` 1-120 (3), `boxDimRampSec` 5-600 (60), `boxDimTo` 0.75-1 (0.8), `boxDimMinLuma` 0.1-0.9 (0.35). `ctl status` `oledcare.boxDim`: `on`, `tracking`, `engaged`, `cells` (below 1), `lowest`, `stillSec`, `unjudged` (damaged cells no sample has judged yet), `samples`, `checks`, `steps`, `warm`, `blind` (the output cannot sample), `sampleUsMax`, `warmUsMax`, `restart`. Journal lines when the map first leaves 1 and when it returns, with the reason.
