# Product

<!-- impeccable:product-schema 1 -->

Source: the shared LumaGame brief and `ARCHITECTURE.md` beside this file. Lines marked
**[Assumption]** were inferred, not confirmed by the owner.

## Platform

web

LumaGame is a native LG webOS TV app drawn by the Rust toolkit in `rustui/`. The schema has no TV
value; `web` is recorded because the design mocks are HTML. **[Assumption]**

## Users

- Nitin, the owner, on the living-room LG OLED65C4 (webOS 10.2.1, 3840x2160 panel at 120 Hz). He is
  not a programmer and has ADHD: copy stays short and every state is obvious at a glance.
- **[Assumption]** Other household members and guests use their own profiles (the saved state keeps
  per-profile favourites, collections and remembered targets) and pick up the remote with no setup
  knowledge.

## Product Purpose

One app to play every game the household can reach, wherever it runs: GeForce NOW in the cloud,
the home PCs through Punktfunk and Sunshine/Apollo, and native webOS games on the TV; Steam Remote
Play and emulators later. Their games merge into one library with one card per game. No separate
client app is ever launched.

Success: a game is chosen and playing in a few presses, the screen always says where it will run,
and nothing about the session (PC, account, store, engine) changes without being shown.

## Positioning

A personal game collection rather than a launcher directory. One card per game across engines, an
explicit play target on every Play button, and readiness for every target (ready, asleep, sign in,
offline) shown before the press, drawn natively at 120 Hz on the TV.

## Operating Context

- Ten-foot viewing on a 65-inch OLED, often in a dim room, with HDR content.
- Input: LG Magic Remote (d-pad, OK, Back, Menu, pointer) in the app; a game controller in play.
  During play the controller's Back belongs to the game; the overlay opens with Guide or a hold, or
  with the remote's Back, which never reaches the game.
- Engines are integrations, off until switched on in Integrations, each with its own sign-in or
  pairing: GeForce NOW signs in with a code entered on a phone; Sunshine/Apollo pairs with a PIN
  entered on the PC; PCs that sleep are woken on play.
- Launch passes through Checking, then Sign in / Queue / Starting, then Running; a failure returns
  to the same game hub.

## Capabilities and Constraints

- Screens: Home, Library, game hub, chooser, Integrations, Settings, sign-in, pairing, in-stream
  overlay, launch and error states, first run.
- Play target order: pinned target, global preferred engine, last successful target, the only
  verified-ready target, then the chooser. A target is remembered only after its session started.
- Terminology: an **engine** is a way to play (GeForce NOW, Punktfunk, Sunshine/Apollo, This TV); a
  **target** is one engine plus its host, store or account.
- The UI is laid out at a 1920x1080 reference and scaled uniformly to the panel. Frame budget
  8.33 ms at 120 Hz with the critical path at or under 6.25 ms; rows are virtualised; no per-card
  live blur.
- Undecided: the final display name and icon (options in `DESIGN.md`); the app id
  `org.nphil.lumagame` is permanent once installed. The theme catalogue: the brief names 20
  families that differ from the 20 in `rustui/tokens/src/palettes.json`.

## Brand Commitments

- Lucent design language v2 with Rosé Pine as the default theme; one token layer, components read
  tokens only; every theme family in light and dark.
- Manrope. Sentence case, few words, plain language.
- Part of the Luma family (LumaTV, LumaShell); the icon is a sibling of LumaTV's.
- Game art and logos keep their original colours. No autoplay trailers, no auto-rotating hero, no
  particles.

## Evidence on Hand

- Game art in the mocks comes from Steam's public CDN through `mock/fetch-assets.sh` and is not
  committed; it shows realistic art only. Library contents, play times, achievements, hosts
  (BEAST-PC, Desk PC) and the account (nitin, Ultimate) in the mocks are illustrative.
- No LumaGame screen has run on the TV yet; frame-time claims need TV measurements.

## Product Principles

1. **Say where it plays.** Every Play names its target, and nothing switches silently.
2. **The collection is the product.** One card per game; the game's own art leads every screen.
3. **Obvious state, few words.** Status is a word with a mark, never colour alone or jargon.
4. **Smooth before rich.** An effect that costs frame time ships only after it is measured.
5. **Engines stay optional.** Each is off until switched on; the core stays minimal.

## Accessibility & Inclusion

- Text at 7:1 against what is actually behind it (art, glass, tints); controls and focus at 3:1.
- Text size 100 % and 200 %; layouts reflow at 200 %.
- Reduced motion drops lift, growth and dissolves and keeps the focus colour and landing light.
- Activation on release; the pointer never steals focus.
- **[Assumption]** No colour-vision or screen-reader requirement beyond these was stated; status
  never relies on colour alone.
