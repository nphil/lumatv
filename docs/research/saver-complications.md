# Screensaver: complications and what shows while it is up

Code: `saver/` (crate `lumatv-saver`: the contract, the board, the rules), `helper/app/src/saver.rs` (routes, topic, enforcement),
`rustui/ui/src/system/saver/` (the Rust saver: numbers, clock raster, model).

## Brightness policy (Rust saver)

The saver is black with a few dim marks. No pixel is brighter than `PEAK` = 0.38 of full white (97 of 255); the theme's
`screensaver.clockLevel` can only lower it (`cap = min(0.38, clockLevel)`). The inks are fractions of the cap: primary 1.0 (hands, date,
temperature, titles), secondary 0.78 (condition, values, ages), tertiary 0.52 (ring, ticks); a glow is at most 0.10 alpha. The block dims to
`dimTo` (floor 0.25) over `dimOverMin`, drifts in whole pixels (±30 × ±20, two periods that do not meet, a step every `shiftEverySec`) and moves
to another spot every `moveEverySec` through a black fade. Reduced motion keeps the moves (they are burn-in care) with the short fade and no
drift. Contrast is traded on purpose: 3.4:1 on true black at the first minute, about 1.4:1 after the dim; Lucent's night rule is "dim clock and
urgent state only". `style: art` is not carried over: the Rust saver shows the clock (a matte is a large bright fill).

With `oledcare.panelDim` on (the default) and the panel reachable, the dim over time is the panel's: the compositor tells the Rust client `paneldim {carry: true}`, the
block is drawn at its first brightness, `saverState.level` carries the curve and the compositor lowers LG's OLED pixel brightness to that fraction of the user's value
(`panel-dim-result.md`). When the panel cannot be dimmed the client draws the dim as above.

## Complication contract

`{id, source, glyph, text ≤ 28, value ≤ 12, priority 0..9, expiresAt (ms), tone: plain|info|warning|danger}`. A glyph is one of the module's
Lucide names (anything else draws `circle`). No expiry means one day from when it was last said. The board holds 12 (4 per source) and shows 6,
most urgent first, newest of equals first. No plugin-specific code: a plugin on the plugin API has complications through its cards
(`lumatv_saver::from_card`), a tool that is no plugin posts one.

| Request | Answer |
|---|---|
| `GET /v1/saver` | the topic's document: `{items, rules, lines, wake}` |
| `GET/POST/DELETE /v1/saver/complications[/{source}[/{id}]]` | the board |
| `GET /v1/saver/rules` | `{answers: [{id, glyph, title}], classes: [{id, title, glyph, choice, default, choices}]}` |
| `POST /v1/saver/rules` | `{id, choice}`, `{id, reset: true}`, `{reset: true}`; owner only; 400 `not_offered`, `unknown_class`, `unknown_choice` |

## What shows while the saver is up

Source classes: `doorbell` (wake), `timer` (wake), `camera.popup` (hide; over, wake, hide), `camera.still` (inside; inside, wake, hide), `ha` (inside),
`ntfy` (inside), `toast` (hide; over, inside, hide), `complication` (inside; inside, hide) and one `plugin:<id>` per installed plugin (inside), which
appears by itself. The answers: `over` (shows over the saver), `inside` (one dim line in the saver's notification area), `wake` (the screen is
woken, then it shows as usual), `hide` (it does not show; it stays in the drawer). The owner's answers are the file `saver-rules.json` of the
data directory.

Enforced where a thing is delivered, so both shells obey: the helper withholds a camera pop-up (`cameras.popup` is not published), turns it into
a line of the topic (`lines`), or publishes it with `overSaver: true`, and for `wake` asks tvpower to turn the screen on (`luna-send`, only when the
saver is up, never from standby) and counts it in `wake.seq` for a shell that acts on it. A doorbell is the `doorbell` class whatever sent it; a
camera video that is hidden leaves the event to `camera.still`. Plugin notes are classed by `lumatv_saver::rules::note_class` (kind, channel and
icon, as the queue does). Whether the saver is up: LG's power state `Screen Saver`, or `oledcare.saver` of the compositor's `status`, asked only
when something is about to be delivered.

QML shell: a card waits while the saver is up and comes back with the time it has left (as before), the camera pop-up does the same (Home counts
as busy while its saver is up), `over` and `inside` behave as a wait, and `wake` reaches LG's own saver (power state `Screen Saver`) but not a QML
saver that LumaTV owns (state `Active`), which ends on the next key. The Rust shell is the one that draws `inside` and `over`.
