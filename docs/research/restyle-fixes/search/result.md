# Search app restyle (surface rows 181, 182)

LG's Search and voice results app (`com.webos.app.voice`) takes mock 09 for its field, keyword chips, genre cards and mic hint, in Manrope and the
palette. `tools/voice_patch.py` changes style and layout only: the behaviour lines of every touched file, LG's results and every string but the
hint are as LG has them (tested). The switch is `lumatv voice on|off`, off by default; it stays off until the TV check below has passed.
Renders: `before/` (the patch of commit 58453f5) and `after/`, the top 620 px of 1920x1080 (the keyboard is maliit's, under it). Both are drawn
by `tests/qmlhost/voice.py` with the set's content: five recent searches, five popular ones, six genre cards, no AI keywords. LG's own pictures are not
in the census copy, so `before/` lacks the close and more glyphs and the row marks that the set shows.

## What the app looks like

- **Field**: 84 px well (the mock's `--well` with its inset lips and a 1 px edge) from x 176, 1384 wide; search glyph, the words in 32 px
  semibold, accent caret, and, where LG shows its hint (an empty field that is not being typed in), the mic hint at the right end, whole, in dim
  ink: "Hold the mic key to speak" (LG's string named the AI button and was cut short).
- **Chips** (recent, popular): 312 x 64 tiles on the 10 % ink wash, 27 px semibold, five to the line from the field's edge to the safe edge; a
  history or star glyph marks each row. The AI keywords keep LG's row, dot and words in the accent read at 4.5:1.
- **Genre cards**: 258 x 96 photographs, corners rounded and a scrim under the foot in one shader quad
  (`payload/voice/Views/Components/LumaPhoto.qml`), the name in 28 px semibold over it. A card whose picture is not there shows its plate.
- **Focus** (chips, cards, suggestions, top row, popups): the plate lights (18 % ink), a 1 px rim in dim ink, the plate grows (chips 4 %, cards
  3 %, `theme.json focus`) and the words stay where they are. No line under the item, no halo, no live shadow.
- **Typed suggestions**: full-width 64 px rows with a search glyph and the typed words in the accent (mock 09's mark).
- **Top row**: back, profile, more and close are 64 px round targets on the field's centre line, Lucide glyphs; the profile's initial is
  in whichever of the two inks reads better on its colour.
- **No-results notice**: the button keeps its whole name ("Go to Internet" was "Go to Int..." in Manrope under LG's 250 px ceiling).

## Where the mock and a later decision differ

- Focus follows Nitin's 2026-10-23 decision (PLAN.md: no line under the focused item, lifted and brighter glass) and the shipped theme
  (`tileGlow` 0): the mock's accent halo is not drawn. The rim is 1 px at 66 % ink so that the cue still reads 3:1 on the sheet.
- The hint is dim ink (66 %), not the mock's ink-3 (42 %, 3.9:1 on the well; 4.5:1 is needed).
- A chip is 64 px (the TV's target), not the mock's 108 px result tile: LG's keywords carry no second line and no glyph of their own.
- The mock draws field, results and keyboard inside one floating sheet; here the keyboard is maliit's own window (LumaKeyboard's files), so the
  app's full-width dark sheet carries the field and rows and the keyboard panel sits under it.
- LG's marquee for a title that does not fit its chip is kept as asked; Lucent's `motion.label` says no focus-triggered marquee.

## TV check before the switch is turned on

Evidence to `docs/research/restyle-fixes/search/tv/`; `lumatv voice on`, open Search, `lumatv voice off` after.

1. The app starts from the patched tree and draws field, rows and cards: no black screen, no QML error in `/tmp/app.voice.log`.
2. The photographs: the cards show LG's pictures with rounded corners and a darker foot (if the shader does not compile on the set the plates and
   names show instead: report which); the name reads on every card; a card with no picture from the server shows LG's default one.
3. The eight glyphs (search, mic, history, star, back, profile, more, close) are sharp and in the ink, at 32, 28 and 26 px.
4. The field: caret accent and blinking while typing; with the keyboard up the hint is hidden (as LG's); with the keyboard hidden (Back) and the field empty it shows whole at the right end.
5. Rows: five chips fit a line; a long title ends in "..." and scrolls while its chip has the focus.
6. Focus: Back hides the keyboard, then Down walks field, recent, popular, genres; Left and Right along a row; each plate lights, takes its rim and
   grows without the words moving or flickering.
7. The Magic Remote: hovering a chip, card or top-row target focuses it as the keys do; a click searches or acts as before.
8. Top row: Right from the field reaches profile, more, close in that order; OK on more opens the option popup, on close leaves; the back key
   (results state) sits clear of the field.
9. Typing through LG's keyboard: five suggestion rows, the typed words in the accent, Down and OK as before; the results drawer opens.
10. "No results found." with its button whole.
11. Against the keyboard: nothing of the app is under the keyboard's top edge with `keys` on (the last row ends at y 475, or 520 with AI keywords; LG's keyboard starts at y 642).
12. Frame time while opening and moving focus (the frame gate) and the process's memory against LG's own look.
13. `voice off` and the next open: LG's look again, a new pid.
14. For Nitin: the hint promises the mic key; until voice search is linked it does nothing in LG's Search, as with LG's own hint ("Hold the AI button"), and whether LumaHome's listener answers the key while this app is up is not established offline. The copy is the mock's; hiding the hint is one line of `patch_searchinputbox` (its `visible`).

## Offline proof

`python3 -m unittest tests.test_voice_patch tests.test_voice_stage` (every touched file's behaviour lines unchanged, every anchor fatal when
missing or doubled, no live shadow, the glyph pixels, the layout numbers, and through the host: every scene loads clean in both materials, no
overlap at 100 and 200 % text, none of the audit's kinds but `elided` at 100 %, the hint whole, the keys as LG's);
`python3 tests/qmlhost/compile_all.py`. Scenes: `python3 tests/qmlhost/scenes.py --only 09-lg-search-tv,09-lg-search-genre,...`.
