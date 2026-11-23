# LumaGame plan budget: before the UI rework (2026-11-23, focus-graph fix)

Every screen the shots show plus the art-dip walk (191 walks), on today's UI (main `e2eeb10a`, which includes the harness's focus-graph rebuild `4901eac1`), drawn at 120 Hz on the host's software GL:

    cd rustui && GAME_BUDGET=all cargo test -p lumatv-rustui --lib budget_walks -- --nocapture

Each walk shows the screen until its art and text have settled, rests 20 frames, then presses Down, Down, Right, Right, Up, Left and OK with 100 ms between, and rests 300 ms. A frame breaks the budget of BUILD-PLAN §6.2 and §6.3 when it draws more than 44 layers, draws two full-screen layers (the canvas excluded), or captures a backdrop after the page's first frames. The fourth rule, one stage-size upload per frame, is counted. Full output, one row per screen with the first broken frames: `game-budget-baseline.txt`.

83 of 191 walks break the budget: 60 draw more than 44 layers (peak 141, `54-diagnostics-token-masked`), none draw two full-screen layers, 30 capture a backdrop after the first frames (counted from the listed broken frames, which stop at four per walk), none put more than one stage picture on the art layer. `art-dip-hub-tab` also misses its expected art dip. The per-family table of the previous run is dropped: its rows were computed before the key walks moved focus.

Read with care:

- The host draws on software GL: the counts are what the renderer is asked to draw, upload and capture; the times mean nothing. The TV at 120 Hz was not measured.
- Full-screen layers are counted from the retained plan, not from the renderer.
- A walk's peak can come from the sheet or page its OK opens, not from the screen it starts on.
- A capture counts only after the page's first frames; the sweep's OK often opens a sheet whose glass captures the backdrop.
- The first-frame upload of about 8.3 MB is one 1920 by 1080 stage picture; peaks after it are covers.
- The harness draws whole frames, so drawn and damaged areas are always the full panel and are not tabled.
- The light, bright and pad variants of a screen draw the same layers and are not walked; the `-text200` screens are.
