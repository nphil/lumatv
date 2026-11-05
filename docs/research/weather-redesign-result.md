# Weather: the card, the view and the radar redesigned (2026-10-30)

The map's and the radar's pictures are cut and held differently since 2026-11-05 (`GET /v1/wx/pic`, four pictures held, everything freed on close): `weather-map-result.md`. Where this note describes `/v1/wx/map`, `/v1/wx/frame` or a layer for every frame, that note is the current one.

## Resume here

State: built, tested and pushed to `main` (commit "Weather card, view and radar redesigned"); what is left is the TV check below (Round22) and whatever it finds. Code is in `rustui/ui/src/home/`:
`weather_art.rs` + `weather_sprites.rs` (the moving sky), `weather_card.rs` (the card), `weather_view.rs` (Now and Details pages), `weather_gfx.rs` (arcs, lines,
curves for the pictures), `radar_surface.rs` + `radar_view.rs` (the radar); helper: `helper/feeds/src/weather_more.rs` and `weather_ha.rs` (ten days), `helper/app/src/wxview.rs`
(map size). Renders: `python3 tests/render_rust_weather.py --bin <release lumatv-ui> [--only card,view,radar,sheet]` (the radar needs the helper built from this tree:
`--helper-bin <target>/debug/lumatv-helper`; it takes the local view over a whole loop and keeps the frame with rain nearest Now), into `docs/mocks/host-rust/home/`; the before/after
sheet is `weather-redesign-before-after.png` there; the art alone: `cargo run --release --example weather_art -- <dir> [light] [view]`.

If the TV check finds the sky too dear: thin it in `weather_art.rs` (the `few(..)` counts of near rain, splashes and snow first, then the far sheets); if the view's window is too dear, shrink
`Stage` in `WeatherSurface::sky`. If a page's words are cut at a large text size: `weather_view::fit` takes the first of its variants that fits, else cuts with an ellipsis; the test
`the_weather_view_cuts_no_words_at_either_text_size_in_either_look` holds it.

Open questions: none for Nitin. The scene has 256 layers (Home holds 122 with the old sky, up to about 140 with the new); the counts are held by a test (`a_picture_is_few_layers...`).

Round 2 (the last section: every card size, the storm and snow moments, the sky in the theme's palette, one rectangle for a scattered sky) is built and tested; its TV check is owed too.

## What was wrong

- Card: the picture was a 130 px icon in a corner, the reading and four rows of 20 to 23 px text left the right of the card empty and the bottom unread from the couch; no hours,
  no rain timing, no line that says what is next, range bars 50 px wide.
- View: Now was two boxed cards (a 700 px column and a chart) with half their glass empty and the art 210 px in a corner; This week was one 1,792 px card of seven rows with sunrise and
  sunset times nobody asked for; Details was eight same-size tiles of one number each; the radar was a framed card 744 px tall whose left third had no radar (the data's edge drawn as
  an edge), hints written as sentences.
- Motion: sheets of rain moved 34 px/s, flakes 5 px/s, clouds floated 10 px over 30 s, the sun only pulsed: too subtle to read as weather.

## What it is now

Card (433 x 480 by default): the sky of the day behind the reading (a tint from the top, clouds that cross at three depths, the sun with a warm bloom and twelve rays that turn once in
2.4 minutes, rain in three depths with rings where it lands on a hairline, snow in many sizes swaying, fog bands, stars that twinkle and a shooting star, a storm's soft flash and
bolts, wind lines); the place, the temperature at 112 px, the condition, how it feels with the day's high and low, one line for what is next (an alert, rain from or until, the sunset),
and the next five hours with their picture, temperature and a bar for the chance of rain (larger pictures when it is dry). Taller or wider cards add the week as rows; smaller ones
keep the reading. Planned by room (`weather_card::plan`), at the text size's reduction.

View: Now is one picture across the screen: the sky's tint over all of it, the moving sky in the open between the reading (230 px temperature, the facts of now) and the ten days
(range bars on the week's span, a dot for now on today's), the next twelve hours as a glass band (bars of rain, or the temperature's curve when it is dry). Radar is the map over the
whole 1920 x 1080 with four floating controls (what is shown, in a chip tinted with the accent for the model's forecast; the three views; the time bar with the radar's and the forecast's
ticks and a tint over the forecast's part; the colours' legend); a frame fades out where its coverage ends. Details are eight tiles that draw their number: the day's span with the actual and
the felt temperature, a compass, the UV scale, 24 hours of the chance of rain, the sun's arc with its place now, a ring for the humidity, the pressure's scale.

Type and fit: words are 24 px at the least (the TV's caption size) and the pages stay readable at 200 % text: the hero drops its facts, the hours and the ten days show fewer columns and rows, a
Details line takes a shorter way of saying it before it is cut (`weather_view::fit`), and the radar's controls grow with the words to 1.3 times and are as wide as their words need (`radar_surface::Geo`).
The page behind the view is hidden by a fully opaque scrim (the theme's is 97 %, which let Home's words show through the open sky as ghosts).

Helper: `weather.more` carries ten days (`daily[].code, hi, lo`); `weather_ha` fills them from the entity's forecast; the warm radar map is 1920 x 1080.

## Motion and its cost (the contract is the old one)

The picture moves only while it is seen (Home in front with nothing over the card, the view up), never under reduced motion, nothing is drawn, allocated or sent per frame: every
400 ms the app restates each moving sprite's path (one linear `Animate` of 500 ms); the rays turn by ten rings taking over from one another; crossings fade at their ends and are put
back unseen; rain and snow far away are sheets that slide one period and are put back; what is near is sprites on their own paths. The frames' damage is the stage (the card's 433 x 244;
in the view a window of 1,160 x 620 round the hero, the tint is still).

Numbers: see Numbers below.

## Tests

`weather_art` (every condition at the card's and the view's stage: whole pixels at rest, premultiplied, inside the box, sheets repeat, paths continuous, crossings put back unseen,
rings add to one light, flashes soft, layers and bytes capped), `weather_card::plan`, `weather_view` (keys, words, smart line, week, hero flow), `weather_gfx`, `radar_surface`,
`radar_view`, the Home lint with the hours at both text sizes in both looks, every page of the view with no word wider than its box at both text sizes in both looks, the helper's ten days.

## Numbers

Not measured on the workstation: the release test build for `weather_art::tests::cost` (`cargo test --release -p lumatv-ui --lib weather_art::tests::cost -- --ignored --nocapture`, which prints
sprites, KiB of pixels and the time of one tick for every condition at the card's and the view's stage) was left for the TV check, which measures the real cost (A3, A4, B1 below). What the
tests hold: a card's sky is at most 38 layers and 2.2 MB of pixels, the view's at most 46 layers and 6 MB.

## TV check (round 22)

`T` runs a command on the set, `H` is the helper's loopback base URL as the earlier probes use it. The shell, the compositor kinds as shipped; the helper rebuilt (the ten days and the
full-screen map are in it: `weather.more` gains `daily[].code, hi, lo`; an older helper still works, the view then lists the card's seven days and the radar's map is 1792 x 744 stretched
over the screen, so rebuild it first).

A. The card, Home in front, a rain reading (`T curl -s $H/v1/weather` shows `current.code` 61 to 65; otherwise push one with `ctl` or wait for rain).

1. `T ctl shell` gives `rust.ui.room`: layers and groups left. Note the numbers with a clear sky and with rain; the sky costs at most 38 layers (Home held 122 of 256 before it).
2. `T shot home`: the card has the sky behind the reading, the line that says what is next in the accent (a warning colour for an alert), five hours with a bar for the chance of rain (larger
   pictures when dry). Look at it for 10 s: clouds cross, rain falls at two speeds with rings where it lands on the hairline, the sun's rays turn (a ring takes over from the last every 4 s).
3. `ctl` ring D/L lines for 60 s on Home with rain: the card's frames draw its sky box only (433 x 244, about 106 k px) at the refresh; and 0 draws of it with an app over Home, the
   screensaver up, the screen off, and with `motion.reduced` true (then the sprites rest and nothing is due).
4. `python3 docs/research/perf/gate.py --surfaces quick --tag weather-sky-rain` over Home with rain and again with a clear sky: both PASS. If the compositor's frame work is over the gate with
   rain, thin the near rain and splashes first (`weather_art.rs`, the `few(..)` counts), then the far sheets.
5. OLED: over 30 s no pixel of the sky stays bright in one place (the rays turn, the bloom breathes, the sun drifts); `T shot home` twice 10 s apart and compare the sun's disc position.
6. 200 % text (`ctl` text scale 2): the card keeps the place, the reading and the next line; nothing overlaps; the sky is behind.

B. The view (OK on the card).

1. The Now page draws in under a second; the sky is in a window of 1,160 x 620 px, the rest of the page is still. `ctl shell` room: at most 46 layers for the sky and about 15 more for the
   page; rain, wind and snow are the dearest. Ring D/L lines for 60 s: the damage is that window; GPU time per frame at 60/120 Hz (the gate's report); if it is over, shrink `Stage` in
   `WeatherSurface::sky` (the window) before touching the sprites.
2. Down: the radar. With the helper's frames: the map is the whole screen, a frame fades out at the edge of its coverage, the chip says RADAR (what the radar saw) or FORECAST (model, not
   radar) in the accent, the bar has the ticks and a tint over the forecast's part. Keys 1, 2, 3, Left, Right, OK. The first map takes (helper log `wx map`) up to 2 s cold at 1920 x 1080.
   Layers: the 37 frames and about 14 of controls; note `room` while it plays.
3. Down: Details. With an alert in force (`T curl -s $H/v1/weather.more`) the alert card shows above the tiles; the compass points where the wind goes; the sun's arc has the sun's place now.
4. 200 % text on each page: nothing overlaps; the hero drops the facts, the hours show fewer columns, the ten days fewer rows.

C. After a day of use, `ctl shell` room and the shell's memory are what they were at the start (the sky's layers are dropped and made again only when the condition or the look changes).

## Round 2 (2026-11-04): every card size, moments, the sky in the theme's palette

Code: `weather_card.rs` (`plan`), `weather_art.rs` (moments, hold, anchor), `weather_sprites.rs` (`Pal::of`). Frames: `python3 tests/weather_frames.py --bin <release lumatv-ui> --cases rain,storm [--window FROM:TO] [--rest]`,
a weather-only Home on the host harness at 120 Hz in virtual time (`moment_clock` puts a run where a moment is).

- Sizes: the card is planned for the room it has, at the text size's reduction: thirteen sizes of reading; the next thing under it or in a column of its own; the hours in three heights (compact, plain, with the bars of the
  rain); the week with range bars only where they have room; a wide short card puts the forecast beside the reading. No part overlaps or leaves its box at any grid size (2 x 3 to 12 x 16) at 100, 150 and 200 % text, and a
  condition has a shorter word before it is cut ("Storm"). The sky's picture stays beside the temperature, never on it.
- Blend: the tint behind the sky runs to nothing (down on a stacked card, left to right on a split one), so the glass has no edge where the sky ends. The sky's colours are the theme's palette roles (`Pal::of`): Rosé Pine
  comes out within a tenth of the old hand-picked colours in each channel; the light look's sun, snow and tints are lifted off its deep text roles.
- Moments: a thunderstorm (4.8 s) and snow (8 s) take over the whole card, one to three minutes apart (`Schedule`: a hash of the slot, a function of the sky's clock, nothing kept). Storm: a dark veil, heavy rain over the
  card, two flashes 650 ms apart with a bolt each (never more than two in a second). Snow: a cool veil and big soft flakes drifting across it. A moment's layers (at most ten sprites and two veils, besides the steady sky's 38) are lit only in the
  moment and are not asked about between moments; ticks come every 50 ms while one is on, 400 ms otherwise.
- Hold: with Settings, a panel or the screensaver over Home the sky stops on whole pixels with its own clock held and a moment's light out, and nothing is asked of the engine; it goes on from there, a moment coming in over
  half a second. Under reduced motion it does not move.
- One rectangle: a clear night, thin clouds and wind are many small things that move apart, and the compositor draws a rectangle for each, every one a pass over all the layers it meets. A veil over the sky's box of
  four-thousandths of the glass (`Builder::anchor`; no pixel shows it) makes the damage the box, one pass. Rain, snow, fog and the sun already move as one region and have none.

Numbers: layers a frame at p50 / p99 / max on the default 433 x 480 card (the set draws about 80 layers in 8.3 ms, `frames-120-result.md`; est. = 1.5 + 0.085 x layers ms, at the max). Before is `dffa764f`, after this round; a moment
is measured where it has the card (before, the same window of the old sky). The host's own CPU per frame is the software renderer's and says nothing of the TV's.

| sky | before | after | est. before to after |
|---|---|---|---|
| clear night | 115 / 116 / 116 | 34 / 35 / 35 | 11.4 to 4.5 ms |
| partly cloudy day | 85 / 90 / 90 | 28 / 30 / 30 | 9.2 to 4.1 |
| windy | 110 / 159 / 159 | 31 / 33 / 33 | 15.0 to 4.3 |
| overcast | 75 / 80 / 80 | 20 / 20 / 20 | 8.3 to 3.2 |
| rain | 31 / 51 / 52 | 31 / 33 / 33 | 5.9 to 4.3 |
| heavy rain | 36 / 50 / 50 | 35 / 38 / 38 | 5.8 to 4.7 |
| drizzle | 23 / 42 / 42 | 23 / 24 / 24 | 5.1 to 3.5 |
| clear day, fog | 21 / 23 / 23, 18 / 18 / 18 | the same | 3.5, 3.0 |
| thunderstorm | 36 / 38 / 38 | 36 / 39 / 39 | 4.8 |
| snow | 24 / 25 / 25 | 24 / 25 / 25 | 3.6 |
| storm moment | 37 / 39 / 39 | 45 / 49 / 49 | 4.8 to 5.7 |
| snow moment | 24 / 25 / 25 | 33 / 33 / 33 | 3.6 to 4.3 |

At rest (the sky's motion off) rain, storm and snow draw 0 layers and 0 px at 1.1 frames a second, as before. The pixels a frame draws grow with the anchor (a night's box, 146 k px, where its many rectangles were 87 k) and
in a moment (the whole card, 400 k px for a storm); the model above counts layers, which is what the set's frames cost (`frames-120-result.md`), and the TV check below settles it.

Tests: `weather_card::tests` (the plan at every grid size at three text sizes and five data sets: parts inside the box and apart; the default card, a shorter one, a wide one, the picture beside the temperature, the sky
covering the reading), `weather_art::tests` (moments one to three minutes apart, brief and bounded; the clock alone says when one is on; two flashes at most, never faster; covered, a moment goes dark and asks for nothing,
and comes back in softly, also at a sky's first step; the anchor is over the box, never seen, never at rest, and only on a scattered sky), `weather_sprites::tests` (the palette's colours), the Home lint at both text sizes.

TV check (round 2, not run: the set was not used; install only a commit that is on `main` and name the hashes):

1. Home in front with a clear night, a partly cloudy day and a windy sky (`ctl` can push a reading): `ctl shell` room, and ring D/L lines for 60 s: the damage is the sky's box (433 x 244) in one rectangle, about 30 layers.
   If the GPU time per frame is not lower than before, remove the anchor for the skies it does not help (`Builder::anchor`).
2. `python3 docs/research/perf/gate.py --surfaces quick --tag weather-sky-night` and `weather-sky-rain`: both PASS; the same with a storm moment on (a thunderstorm reading, wait one to three minutes, or start the run where
   `tests/weather_frames.py` `moment_clock` says): no frame over 8.3 ms in the gate's report while it has the card.
3. A storm moment and a snow moment by eye: the flash lights the whole glass twice, 650 ms apart, no strobing; the bolts fall between the clouds and the reading; the snow drifts across the whole card and the words stay
   legible; both fade in and out over seconds; nothing outside the card is lit.
4. A palette other than Rosé Pine (Settings): the sky's clouds, sun, rain and snow follow it, in the dark look and the light one; the sun is amber in the light look.
5. Settings or a panel over Home, then back: the sky stops where it was and goes on from there, no jump, no moment starting in the middle; 200 % text: nothing overlaps at 2 x 3 to 12 x 9.
