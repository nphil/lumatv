# Visual QA: every LumaTV screen professional (no cut, crammed, moved or misread words)

## Resume here

Done (2026-10-25, offline host; nothing deployed to the set):

- The set's Settings screenshot (`docs/research/visual-qa/tv-settings-picture-2018.png`, 20:18) showed the Picture page with cut
  mode cards, "Not available on this TV" on rows the TV has, the focused row's words shifted left, and the title repeated in
  the head and the rail. Every cause is fixed below; the after renders are `docs/research/visual-qa/after-*.png` (dark and light),
  the before ones `before-*.png` (the offline host's render of the same scenes before the change).
- `tests/qmlhost/layoutaudit.py`: the automated layout check. It walks the item tree of a rendered surface and the grabbed
  picture and reports: `elided` (an ellipsis), `overflow`, `clipped` (cut by a clip or the window), `scaled` (words inside a
  focus-scaled item), `outside` and `tight` (words leaving or crammed against their pill, card or row: 8 px beside, 4 above and
  below), `overlap` (words over words), `off-centre` (words not centred in a pill up to 160 tall) and `icon-level` (an icon not level
  with the words beside it), and `contrast` (Lucent's measured rule: the median of the ink box is the background, the farthest pixel
  the ink; 4.5:1 under 48 px, 3:1 from 48 px, 3:1 for muted ink under .9 alpha). A full-window veil (a layer's scrim) hides what was
  drawn before it. `python3 tests/qmlhost/scenes.py --audit [--only ...]` runs it on every snap and then once more with the type at
  200 % (overlap only, as Lucent asks), writing `<scene>.layout.json`, `<scene>.layout-200.json` and `<scene>-200.png` beside the
  PNGs; `tests/test_layout.py` holds the rules on a surface built for them and runs one scene per kind of screen.
- Kit: a focus lift scales the plate only (`LuRow`, `LuCard`, `LuButton`, the picker's tiles, the drawer's items, the app grid's
  cells, the remote's keys, the search's keys and answers, banner buttons): words never move or blur under focus. `LuText` sets
  its own `lineHeight` (1 for one line, the role's pitch between lines; an instance names its own pitch as `line`) and reports
  the leading under a wrapped text's last line as `trailing`, so a pill, a tab, a chip, a card and a row centre their glyphs.
  `LuSegmented` measures its labels as drawn (widest label + 16 px of air a side, never under a target). `LuCard` with a second
  line is as tall as a two-line row (88). `LuRow`'s detail wraps to a second line and the row grows.
- Settings engine (`PageList`): the mode strip shows whole cards only, inset like the rows (24 px) with a chevron in the inset
  when more cards are that way, and moves by whole cards; the list scrolls by whole rows (a row the viewport would cut is not
  drawn; chevrons at either end say there is more); the back pill and the slider's value have their air. The rail rows are a row
  tall (80); the bar is a target tall (64).
- The settings service (`kit/Sio.qml`), read against the set on 2026-10-25: a read with a key the TV lacks answers `returnValue:
  false` with the keys it has in `settings` and the rest in `errorKey` (the old code dropped the whole answer, which is why OLED
  pixel brightness and Energy saving said "Not available on this TV"); `getSystemSettingDesc` answers `results[]`, one per key;
  `truMotionMode` and `arcPerApp` answer only with `app_id`; `localeInfo` has no category; the C4 has no `filmmakerAmbientLight`
  (LG's own app hides the row: so does LumaTV, `Sio.missing`). A row not yet answered says "No answer from the TV". The stub
  (`tests/qmlhost/stubs/WebOSServices/FakeSettings.js`) now answers in the set's shape.
- Picture page: head "Picture · Mode on HDMI 2" (the tab), rail "Filmmaker · Brightness 80" (the mode in force); TruMotion and
  the aspect ratio are per-input rows. AI access: rows size to their words, short group labels, levels Always / Ask / Never.

Not done, needs the TV: nothing of this has run on the set. Deploy needs a LumaHome reload (`tools/lumatv` install of the ipk, or
the QML copy and `systemctl restart surface-manager-daemon` as the handoff describes); then the Picture page should show the
mode cards whole with a chevron at the right, OLED pixel brightness and Energy saving live, no Filmmaker ambient light row, and
the focused row's tile and words where the unfocused rows' are.

Contrast (Lucent 3, measured on the picture): the segmented tray was 16 % white in the dark material and the unselected labels
(ink2) read 2.4–2.9:1 on it; the tray is 10 % now (`theme.json` dark `trackOff` #1AFFFFFF) and every segment label is in the
reading ink (the wash and the accent mark say which is chosen).

Open (2026-10-25 22:55, the log of the after run is `docs/research/visual-qa/audit.txt`): at 200 % text the five-tab bar of
System and the input picker's tiles overlap (a tab bar cannot take doubled type; `tests/test_layout.py` holds the 200 % pass on
the scenes where it passes, `AT_200`). Findings in files owned by other workers were sent to them: HomeAssistantWidget (state line
1 px from its box, contrast), Weather and Clock widgets (muted lines under 3:1, since fixed by WidgetEditor), Screensaver (its
`lineHeight: 0.95` must become `line: 0.95`). The full-run log of every scene before these fixes is `/tmp/vqa/full/audit.log`
on the workstation (not kept).

## How to run

    python3 tests/qmlhost/scenes.py --out /tmp/vqa --audit            # every scene, dark and light, with the audit
    python3 tests/qmlhost/scenes.py --out /tmp/vqa --audit --only 01-settings-picture --dark
    python3 -m unittest tests.test_layout                             # the rules, then one scene per kind of screen

A finding reads `[layout] kind: item path (why)`; the 200 % pass is tagged `[layout 200%]`.
