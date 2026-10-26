# Settings UX: critique, copy rules and the help pattern (2026-10-25)

## Resume here

Status: done and pushed; checked in the offline host only (renders, the unit tests and the layout audit). Nothing was run on the set.

What exists:

- **The rule** (Nitin, 21:13): a row is its name and its value. A second line only for state the person needs, at most four words. Anything else sits behind
  one info mark, opened by holding OK (or Menu). Spec and copy rules below.
- **The code**: `kit/LuInfo.qml` (the mark), `kit/LuHelp.qml` (the sheet), `LuRow.help`, `help:` on a Rows.js or PageList row; Home's key path routes the held OK
  to Settings (`Layers.hold()` and `holdReady()`).
- **The pages**: every Settings page follows the rule, the ones SettingsCoverage added too. Remote & pointer is tabs Pointer | Keys | Voice; the input banner
  lives on Inputs & devices; the rail's second line is one short fact or nothing.
- **The guard**: `tests.test_settings.Help` (held OK opens help, Back closes it before leaving, the Menu key, and a text lint: a second line over 32 characters or
  with a sentence, a help over 170, a note over 45 fails).
- **The mocks**: the ten settings mocks follow the rule, `docs/mocks/36-settings-help` draws the pattern, `docs/mocks/help.css` styles it.

Next, in this order: (1) on the set, the items under "Needs the TV" below; (2) `/impeccable init` so PRODUCT.md and DESIGN.md exist for the next critique;
(3) the follow-ups under "Not done".

## Critique (impeccable `critique`, Operate mode: a settings surface used from a sofa with a D-pad or the Magic Remote's pointer)

Method: `⚠️ DEGRADED: single-context (leaf worker, no sub-agents allowed)`. Assessment B (the web detector) does not apply to QML; its deterministic
counterpart here is `tests/qmlhost/layoutaudit.py` (overlap, clipping, contrast on the composited picture), run on the scenes. The impeccable context
step ran; the project has no PRODUCT.md or DESIGN.md (`NO_PRODUCT_MD`), so the incumbent implementation and Lucent (`LANGUAGE.md`, TV profile, 2026-10-25
revision) were the authority. `/impeccable init` is the follow-up.

Who uses it: Nitin (ADHD, not a programmer, remote in hand, ten feet away, wants one or two presses to a result and is derailed by walls of text
and jargon), his family (never open Settings; must not be able to break things), and the AI assistants only through the page that rules them.

### What works

- Rail plus pane is the right shape for the 10-foot use: nine sections, a live fact on the rail, one focus zone at a time, Back and Left return one step.
- The rows keep LG's real keys and names, so nothing is invented; an unanswered or inactive row is dimmed and left in place, never faked.
- Focus is a wash and a lit edge on the row, never a new colour block; one control kind per row (toggle, segments, slider, value).

### Design health score (before the pass; after in brackets, from the renders only)

| # | Heuristic | Score | Key issue (before) |
|---|---|---|---|
| 1 | Visibility of system status | 3 (3) | Toggles flip at once, receiver and Home Assistant have state chips. But the bar drew a Wi-Fi glyph on a cable and a pointer glyph with the pointer off, and an unanswered level read 0 |
| 2 | Match with the real world | 2 (3) | eARC, DHCP, "accent", "dimension" leaked into sentences; LG's names were mostly kept |
| 3 | User control and freedom | 3 (3) | Back and Left work everywhere; the pointer had no way back (the "Settings" chevron and the child page's back pill were not clickable) |
| 4 | Consistency and standards | 2 (3) | Remote & pointer was one 16-row scroll with five group titles while every other page has tabs; "More in LG settings" was on the rail and on every tab; the CEC switch twice; the AI page is its own layout |
| 5 | Error prevention | 3 (3) | Remove and lock-out need a second press; AI "more" asks on the remote (Principle 0) |
| 6 | Recognition over recall | 2 (3) | The tab bar is only reached by Up; a nine-mode chooser showed four; no way to ask what a row means |
| 7 | Flexibility and efficiency | 2 (3) | 0 to 100 sliders move one step a press |
| 8 | Aesthetic and minimalist design | 1 (3) | A sentence under every row, a second line on every rail item, a subtitle in every head, paragraphs in notes. On the real set (docs/research/visual-qa/tv-settings-picture-2018.png) three of six rows were also dimmed with "Not available on this TV" |
| 9 | Recognise, diagnose, recover | 2 (3) | Dimmed rows say why, but "No answer from the TV" is five words and offers nothing; a refused write is corrected silently on the next read |
| 10 | Help and documentation | 1 (3) | Help was the sentence under the row, always there, never asked for |
| | **Total** | **21/40 (30/40)** | Acceptable, then Good. The after score is a design judgement of the renders, not a measurement on the TV |

### Priority issues

1. **[P1] A sentence under every row.** Cost: eight rows per tab became eight rows plus eight sentences; the eye reads all of them every time and the ones
   that matter (a dimmed row's reason) drown. Fix: name and value only; a second line only for state; the sentence behind the help mark. Command: clarify, distill. Done.
2. **[P1] No one way to ask "what is this?".** Fix: one mark on the row (an "i"), opened by holding OK (the dock's own "options" gesture), the Menu key or a
   click; the focused row says "Hold OK" so nothing has to be remembered. Done (`LuInfo`, `LuHelp`).
3. **[P1] States that were not true.** The bar's glyphs, `0` for an unanswered level, "Not available on this TV" for "the TV did not answer". Fix: the
   glyph follows the link; an en dash for no reading; "TV not answering" and "Not available now". Done.
4. **[P1] Information architecture drift.** Remote & pointer (one long scroll), the input banner under Remote, CEC twice, "More in LG settings" on every
   tab and on the rail with a list of what is behind it. Fix: tabs Pointer, Keys, Voice; the banner on Inputs & devices; one CEC switch (HDMI tab); LG rows
   are a label only. Done; the LG rows go away as Settings coverage lands.
5. **[P2] Pointer.** A click on a segment of a row, and on the info mark, was swallowed by the row's own click area; the bar's back chevron and the child
   page's back pill did nothing. Fix: the row's click area sits under its controls; both back targets act as Back. Done; needs the Magic Remote to confirm.
6. **[P2] Long ranges one step at a time.** Fix: a held Left or Right runs the step up on ranges of more than 40 steps (2 after ten presses, 5 after
   twenty-four). Done.
7. **[P3] The picture-mode strip shows four of nine.** Not changed (VisualQA owns the strip). A 3 x 3 grid like the sound modes would show all nine.
8. **[P3] The rail has no group titles** (LG's own: Picture, Sound; LumaTV's: Ambient light, Home & look, Remote & pointer, AI access). Not changed:
   Settings coverage keeps it at nine rows and a heading would take one of the ten that fit.

### Persona red flags

- **Nitin (project persona)**: before, every row was a paragraph to read; the one decision on the page (what to change) was lost. Now a page of eight rows
  reads in a second, and the sentence is there on a held OK.
- **Alex (power user with the pointer)**: segment clicks did nothing; sliders took a hundred presses. Fixed both.
- **Jordan (family member who never opens Settings)**: jargon (eARC, DHCP, accent) and "More in LG settings" rows with lists of terms. The terms are now in help
  sheets and the LG rows are labels.
- **Sam (low vision / cognitive load)**: ink3 hairlines carried meaning (the old lock note, the section captions); the info mark is ink2 at rest (above 3:1 on
  both materials) and the sheet is the reading surface with an opaque backdrop.
- **Riley (stress)**: a TV that does not answer: three rows dimmed with the same sentence; now one short reason each, an en dash for the number.

### Minor observations

- The mode cards had a second line ("Warm, dark room"): dropped; the names are LG's.
- Energy saving's choices use LG's words (Minimum, Medium, Maximum).
- A tab's first stop was sometimes a row that only shows; rows that only show are skipped unless they have help.

### Questions to consider

- Should the rail's second line exist at all for rows nobody glances at (Home & look, System)? It is empty now, and nothing is lost.
- Is a five-state segmented control (Energy saving) the right size for a D-pad, or a stepper?

## The help pattern (everyone reuses this; one pattern on every screen)

*Rule*: a row shows its name and its value. A second line only for state the person needs now. Anything that is not obvious has its sentence behind the row's
info mark.

- **The mark**: `kit/LuInfo.qml`, a 26 px "i" after the row's name, `ink2` at rest, a pill with "Hold OK" while the row has the focus. `LuRow` draws it
  when `help` is set (`LuRow { help: "…"; onHelpAsked: … }`; `helpAsked()` is the pointer's click on the mark).
- **The sheet**: `kit/LuHelp.qml`, one per host, above its panes: the row's name, one or two plain sentences, an opaque backdrop (Lucent `scrim.hidesMin`),
  under the row or above it when there is no room. Any key closes it (Back and OK included); it never moves the focus; a click anywhere closes it.
  `helpSheet.show(title, text, anchorRect, boundsRect)`, rectangles in the host's own coordinates.
- **The gesture**: OK held for `motion.holdMs` (700 ms, the dock's own long press). The Menu key does the same, but the Magic Remote has none (it is the gamepad's and
  the plain IR remote's), so the hint on the focused row says "Hold OK". Home's key path asks the surface that has the keys:
  `Layers.holdReady()` and `Layers.hold()` call the surface's `holdReady()` and `hold()`; Settings answers for the focused row of its page.
- **In a Settings page**: put `help: "…"` (or a function) on a row descriptor; `Rows.js` builders take `help:` too. `PageList` does the rest
  (`helpFor`, `helpReady`, `openHelp`). A page that is not a PageList (AI access) implements `helpReady()` and `openHelp()` itself.
- **Elsewhere** (panels, banners, cards): use `LuRow.help` for rows; for another kind of element draw a `LuInfo` where the label ends and open a `LuHelp`;
  the surface's `holdReady()` and `hold()` give it the held OK and the Menu key.
- **Writing a help**: at most two sentences and about 120 characters; what it changes and the one thing to know; no list of settings; no "click"; LG's words.

## Copy rules (clarify)

| Part | Rule | Example |
|---|---|---|
| Label | LG's name where Nitin knows it, even when long ("Auto Power Off after 4 Hours"); else at most three words; sentence case except names; no full stop | "AI Picture Pro", "Match screen and sound" |
| Value | what you would say aloud; a number only with its reading; an en dash for no reading | "Auto", "Strong", "80", "–" |
| Second line | state only, at most four words | "TV not answering", "No input open", "Needs the receiver", "Press OK again" |
| Help | the sentence that used to sit under the row | "Lifts dark scenes a little when the room is bright." |
| Notes and heads | a note only for a state that blocks the whole tab; a head only to name a group | "Sound is on the receiver" |
| LG rows | the label of LG's screen; no list of what is behind it | "More in LG settings" |

Words kept: Colour (LG's British), Filmmaker, Receiver (the Denon), TV speakers, the remote, Home. Words that left the screen: accent, dimension, DHCP and
eARC outside a help sheet, "themed", "everything else".

## Done / not done

### Done

- **Minimal text on every Settings page.** Counted from what each page hands to its rows: before, 12 pages and 134 rows carried 1230 words in a second line
  (9.2 per row) and 103 in notes; now 23 pages (the ones Settings coverage added follow the same rule) and 291 rows carry 126 words in a second line (0.4
  per row) and 21 in notes. The sentences did not go: 1024 words in 91 help sheets, one gesture away.
- **The help pattern** (above), in code and in the mocks (`36-settings-help`), with a lint test so a new page cannot bring the sentences back.
- **The rail**: one short fact each, or nothing. Picture "Filmmaker", Sound "Receiver", Inputs & devices "HDMI 2", Network "Nate's Home · Strong" or "Wired",
  Ambient light "On" or "Off", Remote & pointer "Pointer on", AI access "On", "Off" or "Not running", System the sleep timer while one runs; Home & look and "More
  in LG settings" none.
- **Information architecture**: Remote & pointer is tabs (Pointer | Keys | Voice); the input banner switch is on Inputs & devices ("Input banner"); one CEC switch;
  "More in LG settings" is a label only. The sections stay at nine (agreed with Settings coverage): rare LG items go into tabs or child pages, at most six
  tabs and nine rows a tab.
- **Honest states**: the bar shows the link that is up (Ethernet or Wi-Fi) and the pointer only when it is on; an unanswered level shows an en dash; a dimmed row
  says "TV not answering" or "Not available now".
- **Pointer and keys**: the row's click area sits under its controls (a click on a segment or on the mark lands); the bar's chevron and the child page's back pill
  act as Back; a held Left or Right accelerates on ranges over 40 steps.
- **Tests**: `tests.test_settings.Help` (held OK, no help on an ordinary row, Back closes the sheet first, Menu key, the sheet names its row, the text lint), the
  held-key slider test, the rail-summary test; the others in that file follow the new tab and row counts.
- **Mocks**: the ten settings mocks, `help.css`, `36-settings-help`, the rail in `mock.js`, `settings-map.md`.

Before (left) and after (right), from the offline host (`tests/qmlhost/scenes.py`), in `docs/research/settings-ux/`:
[Picture](settings-ux/picture.jpg) ([light](settings-ux/picture-light.jpg)), [Sound](settings-ux/sound.jpg), [Ambient light](settings-ux/ambient.jpg)
([light](settings-ux/ambient-light.jpg)), [Home & look](settings-ux/home.jpg), [Network](settings-ux/network.jpg) ([light](settings-ux/network-light.jpg)),
[AI access](settings-ux/ai.jpg). The help sheet is new, so it has no before: [dark](settings-ux/help.jpg), [light](settings-ux/help-light.jpg). The after renders
come from a copy of the tree in which the host picks another wallpaper file, so only the Settings surface compares; Inputs & devices, Remote & pointer, Receiver
and System have no montage yet.

### Needs the TV (nothing here ran on the set)

1. Held OK on the Magic Remote: the 700 ms threshold, and that the key filter lets the release through (the offline host sends the key events itself).
2. The Menu key does not exist on the Magic Remote; the plain IR remote and the gamepad send it.
3. Pointer clicks: on the mark, on a segment, the bar's chevron, the child page's back pill, and a click anywhere closing the sheet.
4. Why three Picture rows were dimmed in `docs/research/visual-qa/tv-settings-picture-2018.png` ("not available" rows on Filmmaker): LG's own description marks
   them unavailable, or the service did not answer; the wording is honest either way ("Not available now") but the cause is not known.
5. Write failure: a refused write is corrected on the next read, silently.

### Not done

- **Write verification and a failure toast.** Needs the TV to tell a refusal from a slow answer; a false alarm on every slider would be worse than silence.
- **The picture-mode strip as a 3 x 3 grid** (it shows four of nine; VisualQA owns it).
- **Rail group titles** (LG's and LumaTV's own); the rail is full at nine rows.
- **PRODUCT.md and DESIGN.md** (`/impeccable init`): the next critique should read them instead of inferring the audience.
- **The help pattern in the Lucent catalogue** (`languages/lucent`): the mark, the pill and the sheet belong there as one element.
- **Other surfaces** (quick panel, input picker, prompts, toasts, banners): other workers own them and have the rule; check each for a sentence under a name.
- **Contrast the layout audit found before this pass** (not from the help mark or sheet, which passed): unselected tab labels read 2.4:1 and 2.6:1 on the
  segmented track, and at 200 % text the rail rows overlap. Reported to VisualQA.
