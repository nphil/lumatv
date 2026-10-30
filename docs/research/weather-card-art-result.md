# The weather's moving sky and the radar (result, 2026-10-29; reworked 2026-10-30)

What the card, the view and the radar look like now, and why, is `weather-redesign-result.md`. This note keeps how the sky is made and moved and what the radar
page does.

## The sky

The condition is drawn as a vector sky that moves: a sun with a warm bloom and rays that turn, a moon with twinkling stars and now and then a shooting star, clouds that cross
at three depths, rain with near and far streaks and the rings of the drops landing, snow of many sizes that sways, fog, a thunderstorm's soft flash and bolts, and streamlines of wind.
The code is `rustui/ui/src/home/weather_art.rs` (the parts and the motion) with `weather_sprites.rs` (the shapes), `weather_card.rs` and `weather_view.rs` (who shows it) and
`surface.rs` (`drive_weather`).

- Every shape is a signed-distance function evaluated per pixel into a small sprite (analytic anti-aliasing, a dithered gradient), made once for the box it is shown in and the look;
  nothing is a scaled bitmap except a soft glow, which is made at a quarter of its size and shown four times as large. The sprites are image layers (in the card's group, or on the
  view's own layers).
- The compositor moves them: every 400 ms the app restates each moving sprite's path with one linear `Animate` of 500 ms (a sine is sampled into straight segments of 4 % of a
  period at most). Clouds and streaks of wind cross the box on a saw and fade in and out at its ends, so putting them back is never seen. Rain and snow far away are one tall sheet
  each whose pattern repeats every `P` whole pixels; it slides `P` and is put back. What is near the eye is sprites on paths of their own. A splash, a flash and a shooting star are a
  raised cosine of a second or so. The sun's twelve rays are ten rings, each a sixth of a turn further round than the last, one fading in as the one before fades out: the ring turns
  once in 2.4 minutes. Nothing is drawn, allocated or sent per frame.
- The condition comes from `doc.current.code` and `isDay` (WMO code, the day flag Open-Meteo derives from sunrise and sunset), with intensity from the code (light, moderate, heavy
  rain and snow, mostly clear against partly cloudy) and wind from `windMph` (18 mph and up). Without `current` it falls back to `display.nowKind`.
- The sky moves only while it is seen: Home in front, the screen on, no screensaver, no layer over the cards and no reduced motion (`motion.reduced`); the view's, while the view
  is up. Otherwise the sprites rest on whole pixels, nothing is due and the engine is asked for nothing. The sprites are made after the card's words have settled in the glyph atlas,
  so their pixels do not hold the words back.
- The scene has 256 layers for all of the shell; the card's sky is at most 38 and the view's at most 46 (a test holds them).

Renders: `docs/mocks/host-rust/home/weather-card-{dark,light}-text{100,200}.png` (every condition). `cargo run --release --example weather_art -- <dir> [light] [view]` draws the
skies alone (a sheet of every condition at four moments, a strip of frames for the ones that move most, and with `view` the view's large sky).

## The weather view's radar and globe (2026-10-30)

OK on the weather card opens the view; Down walks Now, Radar, Details. The Radar page shows real NEXRAD frames (two hours back) and HRRR forecast frames (six hours ahead, said to be
the model's in a chip tinted with the accent, drawn at 78 %) over OpenFreeMap tiles that fill the screen, with a time bar (radar ticks, forecast ticks on a tint, a mark for now).
Left and Right scrub and hold the loop, OK plays, 1 Local, 2 Region, 3 Globe (NASA IMERG rain and satellite cloud on a lit sphere; arrows turn it, OK zooms). A frame fades out over 72 px
where its coverage ends. The helper runs LumaShow's `map` crate (`helper/app/src/wxview.rs`, `/v1/wx/*`); the shell decodes a PNG once per frame into a layer that stays on the graphics
chip, so a step of the loop is two opacity changes. The controls are plates drawn into a few pictures (glass samples the wallpaper's blur, not the map under it), so the page costs the
scene about 14 layers beside the map and the frames.

Host-measured on the real network (helper, release build, before the map was the whole screen): radar first frame 0.4 to 0.6 s cold and 20 ms warm; all 37 frames of the window 2.5 to 5.9 s
cold (varies with the service) and 0.1 to 0.3 s warm; base map 0.4 to 1.0 s (vector tiles drawn on the helper; the full-screen map is 1.8 times the pixels); globe 24 s cold (GIBS answers
slowly) and 0.3 s warm. Renders: `docs/mocks/host-rust/home/weather-view-*.png`, `weather-radar-*.png`.
