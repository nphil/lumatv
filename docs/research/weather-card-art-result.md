# The weather card's animated vector picture (result, 2026-10-29)

The large weather card draws the current condition as a vector illustration that moves: a sun whose glow breathes and whose rays pulse in two
interleaved sets, a moon with twinkling stars, clouds that float, rain, snow and fog, a thunderstorm whose flash is a slow soft pulse, and
wind streaks. The code is `rustui/ui/src/home/weather_art.rs` (the shapes and the motion) with `widgets.rs` (`Weather::set_art`, `animate`)
and `surface.rs` (`drive_weather`).

## How it is drawn and moved

- Every shape is a signed-distance function evaluated per pixel into a small sprite (analytic anti-aliasing, a dithered gradient), made once
  for the card's box and look; nothing is a scaled bitmap. The sprites are image layers in the card's group.
- The compositor moves them: every 400 ms the app restates each moving sprite's path with one linear `Animate` of 500 ms (a sine is sampled
  into straight segments of 4 % of a period at most). Rain and snow are one tall sheet each, whose pattern repeats every `P` pixels; it
  slides exactly `P` and is put back, which cannot be seen. The lightning is a raised cosine of 1.2 s every 6 to 8 s on two sprites.
  Nothing is drawn, allocated or sent per frame.
- The condition comes from `doc.current.code` and `isDay` (WMO code, the day flag Open-Meteo derives from sunrise and sunset), with
  intensity from the code (light, moderate, heavy rain and snow, mostly clear against partly cloudy) and wind from `windMph` (18 mph and up).
  Without `current` it falls back to `display.nowKind`.
- The picture moves only while the card is seen: Home in front, the screen on, no screensaver, no layer over the cards and no reduced
  motion (`motion.reduced`). Otherwise the sprites rest on whole pixels, nothing is due and the engine is asked for nothing. The sprites
  are made after the card's words have settled in the glyph atlas, so their pixels do not hold the words back.

## Numbers (host, release, x86)

| | clear | partly | rain | storm | snow |
|---|---|---|---|---|---|
| sprites made (once) | 1.6 ms | 1.2 ms | 1.7 ms | 3.1 ms | 0.9 ms |
| layers made (once) | 2.1 ms | 2.0 ms | 1.9 ms | 4.4 ms | 1.4 ms |
| pixels held | 248 KiB | 174 KiB | 166 KiB | 356 KiB | 135 KiB |
| one tick (two a second) | 0.4 us | 0.8 us | 0.5 us | 1.1 us | 0.3 us |

`cargo test --release -p lumatv-ui --lib weather_art::tests::cost -- --ignored --nocapture`. On the host harness (llvmpipe) a frame between
two wallpaper frames with the rain moving drew 30,976 px in one rectangle (1.5 % of the page, 8 layers); the clip of every sheet and the
box of the picture bound it. GPU time is for the set (below).

## Host renders

`docs/mocks/host-rust/home/weather-card-{dark,light}-text{100,200}.png` (every condition), `weather-card-anim-{rain,storm}.gif` and
`weather-card-strip-*.png` (frames from the harness). `cargo run --example weather_art -- <dir> [side] [light]` draws the pictures alone.

## For the set

`ctl` ring D/L lines for 60 s on Home with a rain reading: the weather card's frames should cost the widget's box only (about 31 k px),
and 0 widget draws with an app over Home, the screensaver up or the screen off.
