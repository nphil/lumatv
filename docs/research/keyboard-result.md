# LumaTV's own on-screen keyboard (LumaKeyboard)

## Resume here

Built and proven on the workstation; nothing has run on the TV. Switch: `lumatv keyboard luma` (LumaTV's own) and `lumatv keyboard lg` (LG's
maliit), default `lg`. It matters only while LumaTV's compositor is live and its shell client is up; under LG's compositor, with the switch off
or with the shell down, maliit serves every field exactly as before. Next: the TV check below, then Nitin's pick of the look
(mockups in `docs/mocks/keyboard/`).

CHANGELOG line for the main agent: *LumaTV's own on-screen keyboard, off by default (`lumatv keyboard luma`, back with `lumatv keyboard lg`): the
shell client draws it and LumaTV's compositor binds it to the text field, so there is no maliit start-up; layouts follow the field (letters,
e-mail, url, symbols, number pad), the D-pad wraps and repeats at its own pace, the Magic Remote pointer works, the microphone key asks LG's voice,
nothing typed is shown or logged, and LG's keyboard stays the fallback.*

## What it adds over LG's keys in the Lucent restyle (`lumatv keys`)

The look is the restyle's on purpose (`tools/maliit_patch.py`: opaque sheet of the material's glass, key plates one hairline wash above it,
function keys one wash below, focus a lit plate with an ink ring and the same ink, accent on the one primary key, Manrope Medium). What the own
keyboard adds is not the paint:

| | LG's maliit, restyled | Own keyboard |
|---|---|---|
| First open | a separate process (`MaliitServer`) started on demand by `initctl emit ime-activate`, 85 QML files read; `maliit-server.service` was inactive before every gate run | already built in the shell client; showing is a property change |
| Motion | LG's 600 ms window slide; the motion patch (`lumatv keyboard on`) acts only under LG's compositor | 110 ms slide in and out, the shell's one panel motion (`panels/PanelMotion.qml`) |
| Layout | one layout; a number field gets maliit's pad on its own terms | the field's purpose picks letters, e-mail, url, symbols or a number pad; the enter key names what the field does with it |
| D-pad | LG's focus box, no wrap rule of ours | wrap on both axes, the column kept through the space bar, own repeat pace |
| Pointer | LG's | hover moves the focus, a press let go on the same key types it |
| Text | preedit then commit through a context | plain `commit_string` per key; nothing composes, so nothing has to be reset |
| Hidden | the server runs on | the item is invisible, nothing in it runs (`busy` false), the system layer is not drawn |
| Logs | the protocol log prints what is typed | no typed text in any log; only the length of the field's text is kept |

Not in the own keyboard: word suggestions, accents and language switching, CJK composition, preedit, emoji. maliit stays for those
(`lumatv keyboard lg`); the Chinese and Japanese plugins are not touched by either keyboard switch.

## Architecture

Three ways to bind a keyboard to a field were weighed:

1. The shell client as an `input_method` client (as maliit is). Not possible from QML: `qml-runner`'s Qt 5.12 has no way to bind `input_method`
   or `input_panel`, and a helper process to do it would add a process and a hop for nothing.
2. Drawing in the compositor (the Rust UI engine). Would fork the look and the motion away from the shell's kit, which owns every other panel.
3. **The compositor is the input method, the shell draws.** Chosen.

```
app ── text_model: activate, set_content_type, set_enter_key_type, set_surrounding_text, show/hide_input_panel
        │                                                        ▲ commit_string, keysym, delete_surrounding_text,
        ▼                                                        │ input_panel_state / input_panel_rect
  compositor server/ime.rs  (own_*)  ── topic `ime` ──────────►  shell client (Shell.qml) ── panels/Keyboard.qml
        ▲   `imekey` events: arrows, OK, Back, digits  ───────►        │ in the system layer, under the overlay
        └────────── POST /v1/ime: text, key, clear, shown, hide ◄──────┘
```

- **Which keyboard serves a field** is decided at its activation: the flag `<data dir>/.keyboard-luma` exists, the shell client has said it can show
  a `keyboard` (`can`), and no keyboard failed to come up in the last minute. Anything else is LG's path, unchanged (`initctl emit ime-activate`).
  The flag is read at every field, so the switch needs no restart and no rebuild.
- **The field's description** is kept per model whether or not it is active (an app may describe a field before it activates it): content hint
  and purpose, enter key type, maximum length, and of the surrounding text only its length and the cursor's byte offset. The shell hears it as the
  `ime` topic (`active`, `show`, `model`, `app`, `hint`, `purpose`, `enter`, `max`, `text`: whether the field has any). The words of the field never
  leave the compositor.
- **Why the remote's keys travel as events and not on `wl_keyboard`.** A panel that takes the keys takes the keyboard focus, and the compositor
  ends a text field whose surface loses it (`ime::focus_changed`, as LG's does; an app is likely to end its own field on the `leave` too
  [INFERENCE]). The keyboard must leave the focus where it is, as maliit's `grab_keyboard` does. The compositor therefore routes the arrows, OK,
  Back and the digits of a key table to the shell as `imekey` events `{k, d, r}` (name, down, the remote's repeat) while the keyboard is on the
  screen, and nothing else: a hardware keyboard's letters reach the app as before. The cost is one loopback hop over the channel that already
  carries `show` and `homekey`; a
  secondary `wl_keyboard.enter` to the shell's window is the alternative if the TV measures the hop too slow (step 8).
- **What the shell sends back** goes one request at a time (two made in one moment travelled on connections of their own and arrived in the wrong
  order): `text` (`commit_string`), `key` (`keysym` for Backspace, Enter, the cursor keys: press and release), `clear` (one
  `delete_surrounding_text` from the start of the text the field said it holds), `shown` with the sheet's rectangle, and `hide`. Each names its
  field; a request for another field than the active one is dropped. `commit_string` carries the serial the field was activated with.
- **The field is told** `enter` at once, and `input_panel_state(1)` with `input_panel_rect` when the shell says the sheet is on its way in (the
  rectangle the sheet will have, so the app can move its field clear as it does for maliit), `input_panel_state(0)` when it goes.
- **Pointer.** The system layer's window is transparent and takes the pointer nowhere; while the keyboard is up it takes it inside the sheet's
  rectangle only (`Scene::shell_pointer`), so the Magic Remote can type and still reaches the app above the sheet.
- **Failure.** The shell not connected, or `keyboard` not in its `can`: LG's. A keyboard that does not report itself on the screen within 2 s of
  the field asking: LG's keyboard is shown for that field at once and serves for a minute. The shell client going while it serves a field: the
  field leaves. Nothing waits on the shell for ever.
- **Trace markers.** `Q:keyboard-show` (the field asked) and `Q:keyboard-shown` (the shell said the sheet is up) go to the kernel trace buffer like
  `Q:<panel>-show`, so the frame gate reads the shell's reaction and the slide from one capture.

Files: `compositor/crates/lumatv-compositor/src/server/ime.rs` (`own_*`), `shell.rs` (`Msg::Ime`, `/v1/ime`, `keyboard` in `can`,
`com.webos.service.voiceconductor` on the luna allowlist), `input.rs` (`ImeState::grabs`, `keyboard_up`), `scene.rs` (`shell_pointer`);
`compositor/shell/Shell.qml`, `imports/LumaShell/Link.qml`; `payload/compositor/lumatv/panels/Keyboard.qml`, `KeyboardLayouts.js`;
`tools/lumatv` (`keyboard luma|lg`, the registry row `keyboard-luma`, `revert` removes the flag).

## How the apps ask for a keyboard

From the interface census (`shell-architecture.md`, read off the set's client libraries) and the gate runs; nothing here was captured per field.

| Client | Text path | What is known |
|---|---|---|
| LG's Search (`com.webos.app.voice`, Qt) | `text_model` | An overlay window; its field activates on launch (`launchMode: vkb`), `show_input_panel` follows; maliit typed through `preedit_string` and `commit_string` (3 keys: 2 preedits, 1 commit). Its enter key type is Search [INFERENCE: label "Search" in the restyle's render]. |
| Web apps and LG's Enact apps (WebAppMgr, Chromium) | `text_model_factory`, `input_method`, `input_panel` | Content type per `<input type>` [INFERENCE: password to purpose 8 with the hidden-text and sensitive-data hints, email 6, url 5, number 3, tel 4]. |
| LG Home (Flutter) | `text_model_factory`, `input_panel` | |
| Netflix, YouTube (Cobalt), Prime | none | Their interface lists have no `text_model_factory`, `input_method` or `input_panel`: their search keyboards are drawn by the apps and no system keyboard is asked for. [INFERENCE until step 7] |

The content type is two numbers and a bitmask of the text protocol (`compositor/protocol/webos/text.xml`): purpose (0 normal, 1 alpha, 2 digits,
3 number, 4 phone, 5 url, 6 email, 7 name, 8 password, 9 date, 10 time, 11 datetime, 12 terminal), hint bits (0x7 default, 0x8 lowercase, 0x10
uppercase, 0x20 titlecase, 0x40 hidden text, 0x80 sensitive data) and the enter key type (0 default, 1 return, 2 done, 3 go, 4 send, 5 search, 6 next,
7 previous).

## Layouts

One grid of 12 columns by 5 rows for the letter layouts so that a hand that knows one knows the others; a number pad of 4 by 4.

| Field | Layout | Differences |
|---|---|---|
| normal, alpha, name, terminal, password (and no content type) | letters | left column `?123`, Shift, `'`, `@`; a hide key where the microphone would be |
| email | e-mail | left column `?123`, Shift, `@`, `.com`; `_` after `l`; `. - +` after `m` |
| url | url | left column `?123`, Shift, `/`, `.com`; `:` after `l`; `. - _` after `m` |
| (`?123` from any of them) | symbols | all 32 printable ASCII symbols and `€`, `ABC` to return |
| digits, number, phone, date, time, datetime | number pad | 1 to 9, 0, Backspace, Enter, Clear; corners none, `. -`, `+ -`, `/ :` |

- Rows 1 to 4: `1234567890`, `qwertyuiop`, `asdfghjkl` and one more key, `zxcvbnm` and three more. Right column: Backspace, the enter key, the
  microphone (or the hide key), Clear. Bottom row: cursor left, the space bar over ten columns, cursor right.
- The enter key is the one accent key: a picture for default and return, otherwise the word (Done, Go, Send, Search, Next, Prev). After Search, Go,
  Send and Done the keyboard goes away and the field stays; after the others it stays.
- Shift cycles off, one capital, caps lock; a letter uses up one capital, a digit does not; a dot marks one capital and a dot and a bar caps lock
  (not colour alone). A field that asks for uppercase opens in caps lock, a name or a title in one capital; nothing else capitalises.
- Clear is dim while the field has no text and then deletes it: it needs the field to send its surrounding text (Qt's does; the TV check
  records whether each app does). Backspace and Enter are key events, which every app that reads keys handles, rather than
  `delete_surrounding_text`, which an app has to implement.
- The focus starts on `g` (the 5 on a pad), the middle of the layout, at every opening.

Sizes (1080p, the TV profile): keys 104 by 68 with 10 between (target 64 and up), the grid 1358 by 380, the sheet the screen's width and 454
high at y 626 (20 above the keys, the 54 of the safe area under them; LG's is 438 at y 642); a pad 176 by 84 with 12 between, the sheet 868 by 446
at x 526. At 200 % text the keys grow to at most 1.5 times in height and 134 wide (sheet 624 high), the words shrink into the key and keep 8 px
of air, the pictures grow with the text.

## Remote

- **D-pad.** Left and Right walk a row and wrap to its other end; Up and Down go to the key above or below at the column the walk started from
  and wrap top to bottom; the column is kept through the space bar, so down, down, up lands where it began. A hole (the pad's) is walked over to
  the nearest key on the row and the column is remembered.
- **Repeat.** A held key reaches the keyboard as presses with the remote's own repeats marked. The first press moves; repeats are let through
  after 300 ms (`focus.repeatDelay`) and then at most one in 100 ms (`repeatInterval`), one in 60 ms after 8 steps. No timer runs, so a key whose
  release is lost cannot run away. OK repeats only on Backspace and the cursor keys, never on a letter.
- **OK acts on its press**, an exception to Lucent's keyup rule (and to PLAN's) for text entry: both halves of the key are the keyboard's, so no
  release reaches the app, and a key of a keyboard is its press. The release of the OK that opened the keyboard goes to the app as it did for maliit.
  If the TV shows it feels wrong the change is one line (`Keyboard.imeKey`).
- **Digits** on the remote type themselves into the field on any layout.
- **Back** puts the keyboard away and the field stays active (the app shows it again on OK); with the keyboard away the remote steers the app
  again at once, a second Back is the app's. **Exit**, or Back held, closes the keyboard and the field, as for maliit. Home is the compositor's.
- **Pointer.** Hover moves the focus and types nothing; a press let go on the key it began on types it (Lucent: commit on release inside); the
  first D-pad key hides the cursor and the focus stays where the pointer left it.
- **Focus** is the PLAN's rule: the lit plate moves at once; after 150 ms of rest it lifts (`kit/FocusLift.qml`, 1.04), a typed key shrinks it to
  .97 for 90 ms. Reduced motion: no lift, no shrink, a fade of at most 120 ms for the slide.

## Voice

The microphone key is there only where the TV says it has voice recognition (`com.webos.service.config` `system.supportVoiceRecognition`, asked once
when the channel is up); otherwise a key that hides the keyboard takes its place. A press calls LG's voice service the way maliit's dictation card
does (`com.webos.service.voiceconductor/system/recognizeVoiceWithDetails`, `runVoiceUi: false`, a subscription), says "Listening" and then
"Working" on the space bar, types the first answer into the field and ends the call (`cancelRecognition` with the ticket). Back while listening
ends the listening and not the keyboard; so does the keyboard going away. A service that is not there (the `lean` profile stops LG's voice
services) answers with an error and the space bar says "Voice isn't available" for 3 s. 20 s without an answer ends it the same way.
Not verified: the call's answer shapes are read from maliit's dictation card (`sttStart` with `voiceTicket`, `sttConnecting`, `sttResult` with
`text`, `sttError`).

## Password and PIN fields

A field is secret when its purpose is password or its hint has hidden text or sensitive data; a PIN is a digit field that is secret and gets the
pad. The keyboard shows nothing typed in any field (no preview strip, no enlarged key). The compositor keeps only the length of the field's text,
journals no text, and the protocol log (`--log-protocol`) prints byte counts for `commit_string`, `preedit_string`, `set_surrounding_text` and
`set_platform_data`. The topic carries only whether the field has text. This closes a hole LG's path had: the protocol log printed the field's text.

## Motion and cost

- The sheet slides from the bottom edge in 110 ms and back, the shell's panel motion (the layer token's 220 ms capped at 110 ms for the gate's
  burst margin, `PanelMotion.qml`), a transform on one item; reduced motion fades. Slide in and out are under the 200 ms asked for.
- The drawing order is the cost design: the plates first, the one focus plate, then the words and the pictures, so that the renderer can batch each
  group and a focus move changes one item. No cached texture: a focus move would draw it again. The 51 plates, 51 labels and 8 pictures are made
  once at the shell's start (an asynchronous loader) and a layout change rebinds them.
- Hidden costs nothing at rest: the item is invisible, the rest timer and the animations are stopped (`state().keyboard.busy` false), the shell
  reports the system layer not live and the compositor leaves it out of the stack. Host-proven: 2.4 s with the keyboard away, the system layer
  swapped no frame. Nothing was kept warm after a hide (`lingerMs` 0): the first frame of a show pays for the nodes joining the scene, which is the
  first number the TV check reads.
- The keyboard sits under the overlay and under the notifications: the volume display (foot of the screen, where the keys are) stays over it.

## Verified on the workstation

- `cargo test` (also `--features ui`), `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`: `tests/ime_own.rs` 8 tests against a real
  `wayland-client` text model: the field's description reaches the shell and nothing of its text does; text, Backspace, Enter and Clear reach the
  field; the arrows, OK, Back and digits reach the shell and a hardware letter the app; the release of the opening OK goes back to the app; hide and
  Exit; a field switch is one topic change; no flag, no `keyboard` in `can` or a silent shell gives LG's keyboard; the pointer inside the sheet and
  nowhere else; the shell going ends the field; the late request of a field that left is dropped.
- `bun test tests/keyboard.test.js` 17 tests: content type to layout, the grids, wrap, the column through the space bar, holes, key effects, the repeat
  gate, no layout beyond the keys the QML makes.
- `tests/test_keyboard_own.py` 6 host runs of the real shell client against the channel stand-in (`tests/qmlhost/shellhost.py`): the client offers
  the keyboard; a typed sequence with Shift, Backspace, Clear, Enter and Back; nine content types; hidden runs nothing and swaps no frame; the
  microphone path with an answer and a failure; the pointer; reduced motion.
- `tests.test_lumatv_inputcommon` (the verb), `tests.test_surfaces_registry`, `python3 tests/qmlhost/compile_all.py` (142 compiled, 0 failed).
- The renders, `docs/mocks/keyboard/` (`python3 tests/qmlhost/scenes.py --only keyboard-letters,keyboard-letters-200,keyboard-symbols,
  keyboard-symbols-200,keyboard-numbers,keyboard-numbers-200,keyboard-letters-voice --out docs/mocks/keyboard`), dark and light, 100 % and 200 %
  text, over a still of video: `keyboard-letters`, `keyboard-symbols`, `keyboard-numbers` (each `-light`, `-200`, `-200-light`) and
  `keyboard-letters-voice` (`-light`), 14 files. Each render runs the layout audit of its picture (words cut, outside their key, overlapping,
  too faint): none at 100 %, no overlap at 200 %. The audit learned that the underscore hangs under the baseline and the grave accent stands above
  the capitals (`layoutaudit.py`).

## TV check (one slot; not run)

LumaTV's compositor live, Home in front, receiver muted, evidence to `docs/research/keyboard/tv/` (`log.txt` first, PASS, FAIL or INFO a line).
`TOOL`, `APP`, `shot` and `kb` as in `lg-restyles-result.md`'s "TV check, overnight round"; `ctl` is the compositor's control socket.

1. **Before (read only).** `$TOOL keyboard status` has `"keyboard":"lg"`; `ctl shell` has `keyboard` in `can`; `ctl ime` has `own.flag` false
   and `own.canShow` true. LG's own for comparison: launch Search (`luna-send -n 1 luna://com.webos.applicationManager/launch
   '{"id":"com.webos.app.voice"}'`, PTY), `shot lg-search` at 3 s; INFO the seconds from the launch to the first keyboard frame (the on-demand
   maliit start); `kb BACK` twice.
2. **On.** `$TOOL keyboard luma`: `{"ok":true,"keyboard":"luma"}`; `ctl ime` has `own.flag` true; `$TOOL surfaces` has `keyboard-luma` on.
3. **Search.** Launch it again. PASS: `shot own-search` at 1.5 s shows the sheet flush at the bottom, 454 high, the Search key in the accent;
   `ctl ime` has `own.serving` true, `own.up` `{"x":0,"y":626,"w":1920,"h":454}`, `asked` unchanged (no input method asked for), `grabbing` true;
   the journal has `entered; the shell's keyboard serves it` and `the shell's keyboard is on the screen`. INFO the seconds to the first frame
   against step 1.
4. **Typing.** `kb OK` types `g`, `kb RIGHT` `kb OK` types `h`: PASS the field shows `gh` and `own.counts.texts` is 2. Backspace (`kb UP`
   `kb UP`, `kb RIGHT` six times, `kb OK`): PASS the field loses one character; FAIL if Search ignores the key event (then Backspace becomes
   `delete_surrounding_text`, a one-line change in `own_key`). Clear: INFO whether it deletes (`ctl ime` `active.surround` says whether Search sends
   its text's length). `?123` and `ABC`, Shift once: INFO the picture. Search key: PASS the search runs and the keyboard goes away, the field stays
   (`own.up` null, the model active); `kb BACK` with the keyboard up: PASS it goes away and a second `kb BACK` leaves Search; `kb EXIT`: PASS the
   keyboard and the field close.
5. **A web app's field.** One with a password and one with a number (LG Settings, Network, a Wi-Fi password and an IP address field
   [INFERENCE: web]). PASS: the keyboard shows, letters for the password and a pad for the number, what is typed reaches the app (masked by the app);
   INFO `ctl ime` `active.purpose`, `hint` and `enter` for each (the table above guesses them); PASS the typed characters are in neither the
   compositor's log nor `ctl journal` (`grep -c` of what was typed).
6. **A hardware keyboard.** If one is paired: its letters reach the field while the keyboard is up, and the arrows and OK steer the keyboard.
7. **Netflix and YouTube search.** Open each, open its search. INFO `ctl ime` `counts.activations` before and after: unchanged means the app draws
   its own keyboard and there is nothing here to check; changed means check steps 3 and 4 there.
8. **Typing speed.** (a) Hold the real RIGHT for 2 s and read `ctl ime` `own.counts.nav` before and after: INFO the events a second (the remote's
   repeat rate); PASS the focus crossed the row and wrapped at about 100 ms a step after the first 300 ms. (b) `kb` the keys of "the quick brown
   fox" at 150 ms and read the field: PASS every character arrived in order; INFO the time from the last `kb OK` to the character in the field,
   then the same on `keyboard lg`. If the hop over the channel is the slow part, the fix is the secondary `wl_keyboard.enter` (Architecture).
9. **Frame gate.** Capture around the launch of Search and the first OK (`capture.sh`): PASS the slide's frames (110 ms, at most 7 at 60 Hz) each
   on its refresh with none late, `Q:keyboard-show` to `Q:keyboard-shown` INFO in ms; INFO the first frame's cost (the nodes joining the scene) and
   whether a typed key's frame is one frame. Then 60 s with the keyboard hidden over an app: `ctl shell` `systemLive` false and no frame of the
   layer. If the first frame is late, the keyboard can be parked in the scene at activation (`PanelMotion.warm`) at a cost the gate must then
   measure for the app's frames.
10. **Fall back.** `$TOOL keyboard lg`: PASS the next field is maliit (`asked` goes up, `own.flag` false). With `luma` on and the shell killed
    (`ctl shell restart` and a field before it is back): PASS a keyboard still appears, LG's. `$TOOL compositor off`: PASS the flag changes
    nothing under LG's compositor.
11. **Voice.** With voice services on (`lean`'s `voice` group): hold nothing, press the microphone key and speak: INFO whether `Listening` shows
    and the words arrive; with them off: PASS the space bar says "Voice isn't available" and the keyboard is unharmed.
12. **End.** `$TOOL keyboard lg` unless Nitin wants his own; Home in front; `$TOOL keyboard status` and `$TOOL surfaces` in the log.

The gate's own `ime` step (`gate.sh ime`) asserts maliit's start (`MaliitServer` pid, an input method bound, `asked 1`): run it with `keyboard lg`.

## Not done

- Suggestions and the candidate row (not required), accents, a language key, CJK input, emoji, key-press popups, sound.
- Auto-capitalisation after a full stop; `max_text_length` is not enforced by the keyboard (the app's field is).
- A preview of the typed text in the sheet: the app's field is the preview; a reveal-password key would need the app's text and is left out.
- Parking the keyboard in the scene before it is asked for (see step 9).
