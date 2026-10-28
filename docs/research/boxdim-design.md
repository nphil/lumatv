# Box dim: dimming only the bright, still parts of an app's picture

`theme.oledcare.boxDim`, off until its TV check passes. LG's own logo-luminance adjustment, done by the compositor for an app's UI: a bright box that has not changed for minutes is drawn with less light, the rest of the screen is not. The retention Nitin saw came from Netflix's bright static boxes; the moving parts of a screen do not imprint, the still bright ones do.

The module is `src/boxdim.rs` (the grid, its clocks and the gain map; no GL), the GL pass is `render/gl/boxdim.rs`, the software renderer applies the same map on the CPU so the host tests see the picture.

## The grid

64 x 36 cells of 30 x 30 output pixels. Each cell keeps: when its content last changed, its last two sampled brightnesses, and its gain (1 = untouched). The map the renderer multiplies with is the cells' gains, as an 8-bit 64 x 36 texture sampled bilinearly in screen space.

## Still: from damage, no pixels read

The compositor already knows what changed: every commit carries its damage, which the plan turns into output rectangles. The cells a damage rectangle touches restart their clock. A frame with no commits does no work; a frame with commits costs a handful of integer operations per rectangle. Damage covering 16 cells or more (a 120 x 120 px patch) is a real change of the picture: every cell restarts (one timestamp, not 2304 writes). Smaller damage (a clock, a caret, a spinner) restarts only its own cells. A key or pointer movement, a window appearing, moving, leaving or changing stacking order, a hole changing, a transition, the screensaver, a capture being armed, and the switch going off all restart every cell.

A client that reports whole-surface damage for a still picture is never dimmed. That is the safe failure: nothing moves that should not, nothing is read to find out.

## Bright: one small sample every 4 s, only on a still screen

Brightness has to come from the pixels. Every 4 s, when the screen has been still for 4 s (no damage, no key), nothing is in transition, no capture is armed and the saver is down, the GL renderer draws the app's items at 1/30 scale into a 64 x 36 target: a box filter of 15 x 15 bilinear taps per cell covers all 900 texels of the cell once, holes are cleared to black first, and the 9 KB result is read back. Luma per cell is Rec. 709 weights on the 8-bit values. The pass is about half a million texel fetches, far less than one composed frame, and it rides on an idle tick where the GPU has been quiet for seconds, so the read stalls on nothing but the pass itself. While content changes or keys flow it never runs. A cell whose brightness moved by more than 2/255 between two samples with no damage recorded restarts anyway: belt and braces, the protocol says it cannot happen.

Video is on its own plane: the hole left for it draws nothing, samples black, and is never dimmed because there is nothing of it in the compositor's output. A hole therefore never makes its surroundings count as bright, and no gradient is ever drawn over the video's edge: the dilation below puts the feather inside the hole, where the pixels are transparent, not on the graphics beside it.

## The gain map

A cell is eligible when `boxDim` is on, it has been still for `boxDimAfterMin` (3), and its last two samples agree. An eligible cell's target gain follows its brightness, with no threshold to flap on: 1 at luma `boxDimMinLuma` (0.35) and below, falling linearly to `boxDimTo` (0.8) at luma 0.65 and above. Everything else targets 1.

The targets are then dilated by one cell (each cell takes the lowest target of its 3 x 3 neighbourhood) and blurred by a [1 2 1] kernel on both axes. The dilation is what keeps the picture looking right: the whole bright shape gets one gain (the floor across it, within 2.5 % over its outermost 15 px), so every contrast ratio inside it is what the app drew, and the feather falls on the dark surround, where a gradient of 20 % over 90 px is below what the eye picks up on near-black content. With the floor at 0.8 the map reads 0.8 on the shape, 0.85 one cell out, 0.95 two cells out and 1 beyond; the texture's bilinear sampling turns those into ramps, so no gain step between two pixels exceeds a code value on white. No rectangle is ever visible.

Each cell's gain moves toward its target: down at (1 - `boxDimTo`) per `boxDimRampSec` (60 s), stepped every 2 s (a step is under two code values); up at the full span per 150 ms, stepped every frame. A step that changes the 8-bit map damages the changed cells plus one cell of margin (the bilinear reach), so the frame redraws that region and nothing else; its items keep their own damage (no shm re-upload). A step on a screen with nothing pending is a quiet frame, as an OLED-care beat is, so the slowed idle tick stays slowed.

Hysteresis, so it never pumps: the brightness-to-gain curve is continuous; a changed cell waits the full `boxDimAfterMin` again; eligibility needs two samples 4 s apart that agree; and small changes inside a dimmed region do not restore their neighbours: a clock that ticks in a dimmed sidebar keeps the sidebar's gain through the dilation, so nothing around it pulses once a minute. A region that really changes (16 cells or more) restores everything within 150 ms and the count starts over.

## Where the map is applied

Only items of app windows: not the shell's two windows (they have `enabled`), not the wallpaper plane, not the cursor, the switcher's backdrop or the UI engine's layers. An item in a transition (posed) is drawn by the pose program, which has no gain: a transition restarts the map anyway. The GL renderer links a second copy of the item program with one more texel fetch and uses it for app items only while any cell is below 1. While no cell is, the frame path is exactly today's: the plain program, no second texture, no pass, and `Plan::sole_fullscreen` still hands the window to the plane whole. The first step that takes a cell below 1 is the one frame that leaves direct scan-out, on a screen that has been still for minutes.

Captures: `screencap shot` and `record` arm compose-only and redraw; while armed the map is snapped to 1 at once (not ramped), so what is written is the picture as the app drew it. The debug `ctl capture PATH` reads the page as composed, dim included: that is what the TV check uses to see it.

HDR: no special case. The gain multiplies the compositor's graphics output, which the display engine blends over the video plane in every mode; the video plane is never touched and the graphics layer's light is what wears the panel in SDR and HDR alike.

`appDim` (the whole-window dim after minutes of no key) multiplies with this: both restore on a key within 250 ms.

## Not worse than nothing

The gain is at most 1 everywhere, so no pixel ages faster than it would without the dim. Inside a bright shape it is uniform (dilation), so the dim adds no structure where the shape will imprint; it only lowers the imprint's depth, roughly to 0.8^1.75 = 0.68 of it for the panel's superlinear wear. The feather lies on the dark surround, where a 20 % gain change is a change of a few nits and ages nothing visibly. The dim cannot imprint its own outline because its outline is the content's outline, moved outward by one cell onto dark pixels, with less contrast than the content's own edge.

## Cost

Idle, nothing: a flag per item at plan time, no map, no program switch, direct scan-out intact. Engaged: one composed frame per 2 s during a ramp (none between steps), each with one extra texel fetch per app fragment from a 9 KB texture, and one sample pass per 4 s on an idle GPU. Restoring after a key: about nine composed frames in 150 ms with the gained program, the same frames the app's reply damages anyway. On the host (Mesa llvmpipe, 1080p) a full composed frame costs the same with the map as without, 6.8 against 7.0 ms; the sample pass's 518 000 taps take 173 ms on that CPU rasteriser and a fraction of a frame on the set's GPU (`ctl status` reports `sampleUsMax`). The TV check compares `renderUsMax` and frame deadlines against the switch off.

## Tokens

`oledcare.boxDim` (false), `boxDimAfterMin` 1-120 (3), `boxDimRampSec` 5-600 (60), `boxDimTo` 0.75-1 (0.8), `boxDimMinLuma` 0.1-0.9 (0.35). `ctl status` `oledcare.boxDim`: `on`, `engaged`, `cells` (below 1), `lowest`, `stillSec`, `samples`, `steps`, `sampleUsMax`, `restart`. Journal lines when the map first leaves 1 and when it returns, with the reason.
