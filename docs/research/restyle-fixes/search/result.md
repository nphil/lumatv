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

## TV check, 2026-10-28 (`48ee380` on the set, LumaTV's compositor, over Home)

Evidence: `search/tv/`. `voice on` took 2.3 s; the next open was a new process (pid 29092), `voice off` took 1.4 s.

1. PASS. The app draws field, rows and cards at once. The log has one load warning, `SearchView.qml:294:13: Unable to assign [undefined] to QQuickItem*`, LG's own `KeyNavigation.up` (the host tests name the same binding at :287).
2. PASS. The six cards show LG's pictures with rounded corners and a darker foot, so the shader compiled; every name reads (`patched-crops-1920.jpg`).
3. PASS. Search, history, star, the profile initial, more, close and the mic in the hint are sharp at 1920 px. The back glyph shows only in the results state (see 10).
4. PASS. Accent caret; the hint is hidden while the caret is in the field, keyboard up or hidden, and shows whole with its mic glyph as soon as focus leaves the field.
5. PASS. Five chips to a line; "Spider-Man: Brand ..." and "Tout le monde en p..." end in "...", and the focused chip's title scrolls (`patched-marquee.jpg`).
6. PASS. Back hides the keyboard; Down walks field, recent, popular, genres (LG's look walks the same order); each plate lights with its rim and grows, the words stay. Twice in about 30 keys one key moved two places (a Right, a Down): not explained; the frame gate below shows no late frame.
7. PASS with the compositor's injected pointer (`ctl pointer X Y`, `ctl click`, the Magic Remote's path): hovering a popular chip, a genre card and the more button lights each as the keys do (`patched-pointer-hover.jpg`), and a click on more opens the Options sheet, a click on Quit App leaves. One click sent while the pointer was hidden searched an empty keyword instead (the pointer has to be visible, as with the real remote); a real Magic Remote was not used.
8. PASS. Right from the field reaches profile, more, close, in that order. OK on more opens the Options sheet (hairline, switch in the accent). Back does not close it, in LG's look either (`lg-look-option-popup-back.jpg`); "Quit App" does. The results state's back key was not reached (10).
9. PASS for the suggestions: typing (evdev letters) gives five full-width rows with the typed letter in the accent, Down and OK work. The results drawer does not open: `com.webos.service.nlpmanager` is not running under `lean`, so a search ends in the loading ring (faint on the dark sheet) and Back does not leave it; only a close through SAM does. That is LG's behaviour with the voice group off.
10. SEEN ONCE, by that misdirected click: the empty search shows "No results for this keyword." centred on the sheet in Manrope; LG's "Go to Internet" button did not appear (`patched-no-results.jpg`). The "No results found." of the check's wording never came up after a typed search, which stays in the loading ring.
11. PASS. The last row (genre cards) ends at y 470 of 1080; the keyboard window is 1920x438 at y 642.
12. PASS. Frame gate (new `voice-focus` surface in `docs/research/perf/gate.py`: Down x3, Up x3, Right x2, Left x2 over the open app): first frame 15.3 ms (limit 50), p90 render 0.26 ms, 0 dropped, 0 over 20 ms, no log errors, compositor pid unchanged. RSS 77.4 MB against 73.7 MB in LG's look.
13. PASS. LG's look comes back (grey field, white focus boxes, "Hold the AI button on the remote and search with ...") in pid 4173.
14. For Nitin: unchanged. The restyled hint reads "Hold the mic key to speak"; LG's names the AI button.

`voice` was left on: every step that can run on this set passes; the Magic Remote itself and a search that finishes are not testable here.

## Offline proof

`python3 -m unittest tests.test_voice_patch tests.test_voice_stage` (every touched file's behaviour lines unchanged, every anchor fatal when
missing or doubled, no live shadow, the glyph pixels, the layout numbers, and through the host: every scene loads clean in both materials, no
overlap at 100 and 200 % text, none of the audit's kinds but `elided` at 100 %, the hint whole, the keys as LG's);
`python3 tests/qmlhost/compile_all.py`. Scenes: `python3 tests/qmlhost/scenes.py --only 09-lg-search-tv,09-lg-search-genre,...`.
