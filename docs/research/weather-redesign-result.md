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

To be written with the last push.
