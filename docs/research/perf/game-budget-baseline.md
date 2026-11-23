# LumaGame plan budget: before the UI rework (2026-11-23)

Every screen the shots show (190 walks), on today's UI (main `939dd34d`) with only the budget harness added, drawn at 120 Hz on the host's software GL:

    cd rustui && GAME_BUDGET=all cargo test -p lumatv-rustui --lib budget_walks -- --nocapture

Each walk shows the screen until its art and text have settled, rests 20 frames, then presses Down, Down, Right, Right, Up, Left and OK with 100 ms between, and rests 300 ms. A frame breaks the budget of BUILD-PLAN §6.2 and §6.3 when it draws more than 44 layers, draws two full-screen layers (the canvas excluded), or captures a backdrop after the page's first frames. The fourth rule, one stage-size upload per frame, is not counted on this build. Full output, one row per screen with the first broken frames: `game-budget-baseline.txt`.

87 of 190 walks break the budget: 63 draw more than 44 layers (peak 126), 25 draw two full-screen layers, 53 capture a backdrop after the first frames.

| family | screens | layers, min to max (worst screen) | full-screen max | screens that capture | screens over the budget | mock layers / full-screen |
|---|---|---|---|---|---|---|
| Home | 13 | 45 to 68 (`06-home-compact`) | 2 | 10 | 13 | 16 / 1 |
| Home with a refresh sheet | 6 | 66 to 71 (`71-refresh-result-dark100`) | 2 | 6 | 6 | 16 / 1 |
| Library | 4 | 63 to 74 (`72-library-refresh-dark100`) | 1 | 2 | 4 | 30 / 0 |
| Game page | 19 | 20 to 47 (`76-hub-details-dark100`) | 2 | 11 | 12 | 10 to 22 / 1 |
| Chooser | 2 | 37 to 38 (`32-chooser-text200`) | 2 | 2 | 2 | |
| Settings, diagnostics | 18 | 25 to 126 (`54-diagnostics-token-masked`) | 1 | 4 | 13 | 18 / 0 |
| Dialogs and text entry | 16 | 39 to 118 (`38-search-keyboard`) | 1 | 3 | 13 | |
| Controller pages | 63 | 9 to 121 (`47-leadjoy-rename`) | 1 | 5 | 14 | |
| Launch states | 14 | 14 to 40 (`20-state-failed`) | 1 | 2 | 2 | 3 to 4 / 0 |
| Steam sign-in | 6 | 19 to 32 (`62-steam-signed-in`) | 1 | 3 | 3 | |
| In-stream overlays | 29 | 0 to 57 (`15-overlay`) | 1 | 5 | 5 | |

Read with care:

- The host draws on software GL: the counts are what the renderer is asked to draw, upload and capture; the times mean nothing. The TV at 120 Hz was not measured.
- Full-screen layers are counted from the retained plan, not from the renderer.
- A walk's peak can come from the sheet or page its OK opens, not from the screen it starts on.
- A capture counts only after the page's first frames; the sweep's OK often opens a sheet whose glass captures the backdrop.
- The first-frame upload of about 8.3 MB is one 1920 by 1080 stage picture; peaks after it are covers.
- The harness draws whole frames, so drawn and damaged areas are always the full panel and are not tabled.
- The light, bright and pad variants of a screen draw the same layers and are not walked; the `-text200` screens are.
- The run predates the harness rebuilding each walk's focus graph: on screens the shots set up after the fixture, the D-pad keys moved nothing, so those rows hold the first frames and the OK on the starting focus. Their numbers are a floor; a new `GAME_BUDGET=all` run replaces them.
