## Hearth update: what LumaTV could lift

**Answer first:** Two things are worth lifting. (1) A slow pixel shift on the Home screen, to protect the TV's screen from burn-in. (2) Remembering the shuffle order across restarts. The rest is already covered, does not apply to native QML, or needs your decision.

Source: Hearth v1.2.0, released 2026-10-25 (https://github.com/rudra-avatar/Hearth/releases/tag/v1.2.0). Hearth's speed claims come from its own notes; I did not measure them.

### Top 5 to lift (ranked)

1. **Home-screen pixel shift.** Hearth moves the whole Home screen by 4 to 8 pixels, changing position every 60 seconds, on by default with an off switch (`src/screens/Home.tsx`). LumaTV's Home has no shift; grep of `LumaHome.qml` finds none. The screensaver already shifts, so this is a gap. Approach: a transform on the Home root, moved by a 60-second timer that stops when Home is hidden. Measure the frame rate before and after, since LumaTV tracks frame rate closely (37 to 51 fps experiments in PLAN.md:232-236). Effort: small to medium.

2. **Remember the shuffle order.** Hearth saves the shuffle deck per folder and skips files that no longer exist (commit `ffa93ec`, merged via PR #1 in `42afa1c`). LumaTV's screensaver shuffle bag is memory only (`Screensaver.qml:275, 331`), so a restart starts the bag over. Effort: small. Check whether the Home wallpaper slideshow also keeps its bag in memory; I did not check.

3. **Screensaver crossfade dip (check on the TV first).** `Screensaver.qml:387-391` fades the new picture in while fading the old one out at the same time. Hearth stacks the new layer above the old one and removes the old one after the fade (`src/components/Wallpaper.tsx`), because two layers fading together let the background show through halfway. On LumaTV's black background the effect is a brightness dip, not see-through. Only change it if the dip is visible on the TV. Effort: small.

4. **Quiet mode for Home (proposal, default off).** After 10 to 60 seconds of no input, Hearth fades its apps, and the first key or pointer move wakes them without acting (`Home.tsx`, PR #10). LumaTV deliberately leaves idle timing to the TV's power manager (`Screensaver.qml:9`). Adding this is a design choice for you, not a safety issue. It adds a timer that runs while Home is up, so it must stop when hidden, as the repo already requires.

5. **Repeatable frame and key-delay test.** Hearth added a Tests tab that measures frame time, key delay and start-up time (`src/lib/perfmarks.ts`, probe builds only). LumaTV has measured these by hand (PLAN.md:232-236). `LumaHome.qml:276-290` already logs timer gaps, so this is partly in place. Lowest priority.

### Already in LumaTV, or does not apply

- **Clock redraw once a minute:** does not apply. LumaTV's second hand uses a shared tick by design (`LumaHome.qml:500-504`, measured at 37 fps before the fix). Only revisit if the second hand is dropped.
- **Skipping unchanged tile redraws:** React-specific. QML only re-evaluates bindings whose inputs changed, and `kit/LuRow.qml` has no timers or redraw loops. Nothing to lift [INFERENCE from QML behaviour; not measured].
- **Clock shadow, tile shape, label colour, bottom fade, light themes:** cosmetic. Not assessed against the LumaTV theme system.
- **Recents panel:** already planned (`PLAN.md:1054-1057`, `SURFACES.md:72`), but not built (no `listRecents` in `payload/`). When it is built, copy Hearth's close order: close the running apps, wait up to 2 seconds for them to exit, then remove the recent entry (`src/lib/recents.ts`).
- **Replacing LG Home with a script and SAM (takeover):** needs your explicit approval. It changes the TV's home screen at boot and touches the TV's own system. LumaTV's compositor already owns the home surface [INFERENCE], so it may add little. Not recommended without a separate decision.
- **webOS 3 and webOS 11 support, loading apps without root:** LumaTV targets webOS 10.2.1 with root. Not applicable.

### Decided (Main, 2026-10-25)

- Item 1 is not built: LG's own Screen Shift (`picture.screenShift`) is `on` on this set (docs/arch/x4/all.jsonl), and it moves the whole panel output, Home included, below any compositor [INFERENCE: panel-side, not measured]. Hearth needs its own shift because it targets sets where that may be off.
- Item 2 goes to the screensaver owner: persist the shuffle bag per folder, skip files that no longer exist.
- Item 3 is checked on the TV in the next slot; changed only if the dip is visible.

### Open

1. Decide whether you want the quiet mode (item 4). It is off by default, so it changes nothing until you choose it.

### Not checked

- Whether the Home wallpaper slideshow keeps its shuffle bag in memory.
- Whether `SaverVideo.qml` has its own shift.
- The exact on-screen effect of the crossfade dip (item 3).