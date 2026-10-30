# Weather: the card, the view and the radar redesigned (2026-10-30)

## Resume here

State: work in progress, pushed to `main` in steps (see the commits of the weather redesign, "Weather card/view/radar: ..."). Code is in `rustui/ui/src/home/`:
`weather_art.rs` + `weather_sprites.rs` (the moving sky), `weather_card.rs` (the card), `weather_view.rs` (Now and Details pages), `weather_gfx.rs` (arcs, lines,
curves for the pictures), `radar_surface.rs` + `radar_view.rs` (the radar); helper: `helper/feeds/src/weather_more.rs` and `weather_ha.rs` (ten days), `helper/app/src/wxview.rs`
(map size). Renders: `python3 tests/render_rust_weather.py --bin <release lumatv-ui> [--only card,view,radar]` (the radar needs the helper built from this tree:
`--helper-bin <target>/debug/lumatv-helper`), into `docs/mocks/host-rust/home/`; the art alone: `cargo run --release --example weather_art -- <dir> [light] [view]`.

Done: art engine (stage, parallax clouds, rotating rays, rain with depth and splashes, snow with sizes and sway, storm flash and bolts, wind lines, stars and shooting stars);
card (plan by room, sky, smart line, hours with rain bars); view (Now: sky, reading, hours, ten days; Details: tiles that draw their number); radar (map over the whole
screen, floating controls); helper ten days.

Next, in order: (1) host renders of every page at 100 and 200 % text, dark and light, looked at and fixed; (2) `cargo test --release -p lumatv-ui --lib weather_art::tests::cost -- --ignored
--nocapture` into the numbers below; (3) the targeted tests (`cargo test -p lumatv-ui --lib weather radar surface::tests::the_weather`, helper `cargo test -p lumatv-feeds weather`) and
`cargo clippy -p lumatv-ui --all-targets -- -D warnings`; (4) before/after sheet; (5) TV check by Round22 (list below).

Open questions: none for Nitin. TV: the moving sky is more than the old one (about 25 to 40 sprites a card, up to 44 in the view); the scene has 256 layers (Home holds 122 with the old sky),
so the counts are held by a test (`a_picture_is_few_layers...`).

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

Helper: `weather.more` carries ten days (`daily[].code, hi, lo`); `weather_ha` fills them from the entity's forecast; the warm radar map is 1920 x 1080.

## Motion and its cost (the contract is the old one)

The picture moves only while it is seen (Home in front with nothing over the card, the view up), never under reduced motion, nothing is drawn, allocated or sent per frame: every
400 ms the app restates each moving sprite's path (one linear `Animate` of 500 ms); the rays turn by ten rings taking over from one another; crossings fade at their ends and are put
back unseen; rain and snow far away are sheets that slide one period and are put back; what is near is sprites on their own paths. The frames' damage is the stage (the card's 433 x 244;
in the view a window of 1,160 x 620 round the hero, the tint is still).

Numbers: see the last section.

## Tests

`weather_art` (every condition at the card's and the view's stage: whole pixels at rest, premultiplied, inside the box, sheets repeat, paths continuous, crossings put back unseen,
rings add to one light, flashes soft, layers and bytes capped), `weather_card::plan`, `weather_view` (keys, words, smart line, week, hero flow), `weather_gfx`, `radar_surface`,
`radar_view`, the Home lint with the hours at both text sizes in both looks, the helper's ten days.

## Numbers (host, release)

To be filled by `cost`.

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
