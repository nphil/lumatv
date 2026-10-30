# Home icons and text against LG's Home

Nitin (2026-10-29): the app icons on LumaTV's Home looked much less sharp than on LG's, and LG's text sharper. Evidence: the TV's page
captures of both Homes (`compositor/slot-2026-10-29-r20/img/r20-lg-home-full.png`, `r20-luma-home-full.png`, 1:1 crops beside them) and the
icon files from the TV's launch points. The page (1920x1080) is the same x2 scaled by the display engine for both, so the difference is in the
pixels each Home draws.

## Cause

The host render of the dock with the TV's own icon files reproduces the TV capture's edge rise to within 0.1 px, so the host is the test bed.

| Suspect | Finding |
|---|---|
| Source file | Not it. The dock already takes the largest (`extraLargeIcon` 192, else `largeIcon` 130); LG's tile fits the 192 px file best (a fit to the capture) |
| Resample | **Cause.** Shrinking was an area mean on the CPU to a side between the box and the lifted box (192 to 135), then the GPU's bilinear read of that at 120/135: two filters, the second one smearing |
| Scale at rest | **Cause.** The icon layer was shown at 120/135 or 120/130, never 1:1 at rest |
| Sub-pixel placement | The tile's origin is whole; the lifted tile was not (120 px at 1.12 is 134.4, edges at -7.2) |
| Mipmaps, premultiplied alpha, glass over the icon | Ruled out (UiSharp turned mipmaps off; the plate and glass sit under the icon) |
| LG's tile | 126 px with the icon at about 116 (a fit of the 192 px file to the capture), i.e. a 0.60 reduction of a 192 px source; LumaTV's tile is 120 with the icon edge to edge |

## Fix (`rustui/ui`)

- `Picture::fitted_sharp`: one Lanczos-3 pass from the file's pixels to the exact on-screen size, then a light unsharp mask (0.45 over a 3 by 3
  blur, colour only) that gives back the edge the x2 scaler will soften. Used for the dock tiles and the Apps library's icons.
- A dock tile holds two pictures: the box's size, drawn 1:1 at whole pixels while the tile rests, and the lifted box's size, drawn 1:1 at full
  lift and crossfaded in by the lift (the tile's lit group). Each is made once, when the tile is built.
- The lift scale is nudged so the lifted tile grows by an even number of pixels (120 to 134, not 134.4): its edges stay on whole pixels.
- Text keeps its face and weights: the shipped theme turns on `text.gridFit` (TextRender's grid-fitted glyphs; layout unchanged).

## Numbers

Edge rise = the 10 to 90 % width of an edge, in page pixels, over the logo of each tile (lower is sharper).

| Tile | LG Home | LumaTV before | LumaTV after |
|---|---|---|---|
| Netflix | 1.82 | 2.15 | 1.66 |
| PLX | 1.69 | 1.78 | 1.41 |
| Apple TV | 2.06 | 3.03 | 1.87 |
| YouTube | 1.44 | 1.79 | 1.46 |
| Apple TV, focused (134 px) | | 3.24 | 1.92 |

Text, median edge rise over the same words: LG 1.5 px and 28 % of edges within 1.2 px; LumaTV before 1.48 and 26 %; with `text.gridFit` 1.05
and 53 %. By that measure the text edges were already level; LG's text reads bolder from stems about 10 % wider (3.3 px against 3.0 for a 26 px
label) and from white on black (230 on 0) where LumaTV's is 242 on a glass of 44 to 84. Grid fitting widens the stems by 3 to 5 % and lifts the
fraction of whole-pixel edges, which is the closest a change that keeps the face can come; the contrast is the glass's.

Pictures: `ui-sharpness/icon-lg-before-after.png` (LG, before, after; 1:1 crops x4) and `ui-sharpness/text-gridfit-before-after.png`
(off, on). Plain text weight, face and size were not touched.

## TV check

Capture the same four tiles (Netflix, PLX, Apple TV, YouTube) at rest, and Apple TV focused, at 1:1 from `r20-luma-home-full.png`'s positions;
the edge rise should read at or under the table's "after" column, and a two-line text block ("At home", "Home Assistant") at or under 1.1 px median.

## TV result

On the set (forge bc641553, ctl capture), edge rise of the same logos: Netflix 1.66, PLX 1.41, Apple TV 1.87, YouTube 1.46, Apple TV focused 1.97; text "At home" 1.08 and "Home Assistant" 1.05 px median. Each equals the host render, and four tiles are at or under LG's. Picture: `ui-sharpness/icon-tv-lg-vs-after.png` (LG above, LumaTV below).
