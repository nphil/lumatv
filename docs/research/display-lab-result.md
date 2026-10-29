# The display lab: judging the display chip's options on the TV screen

A guided, on-screen test driven with the Magic Remote: one short step at a time, the zone to look at marked on a test card, a line on
what the difference looks like, the look on the screen named in big letters (A or B), and the person's answer. Built on the results
of `layer-deep-result.md` § 8 (registers, safe values, restore) and `display-chip-direct.md`. No TV access in this round: nothing here
has run on the panel.

## Resume here

Done (2026-10-29, host only): the lab's screens (`rustui/ui/src/lab/`), the helper's door with its hard list
(`helper/app/src/displaylab/`), the compositor's `displaylab` kind and the wheel turn as the key `wheel`
(`compositor/.../shell_rust.rs`, `server/seat.rs`), the wheel press as key 1199 (`3ff3b31`), the Settings row (Picture, Expert tab),
host renders (`docs/mocks/host-rust/displaylab/`) and the tests below.

After Round 19's run on the TV (2026-10-29 18:09-18:14, `slot-2026-10-29-r19/displaylab/`): the dimming steps are gone (VSP is the firmware's, §
1), **Medium** (gain `0x40`) is in, with a Medium / Strong step, and a look that does not show says "Couldn't apply" and offers no pick (§ 1). The
run itself: Strong 92 switches and Normal better, Soft preferred in one step, the window answered Yes with the registers holding.

Next, in order:

1. Main installs (Face ID); the kind is already on (`ctl shell rust +displaylab`).
2. The TV check (§ 5), Round19 at the set: Medium and Medium / Strong are new; the window's frame against the real hole.
3. From what the TV shows: decide the gain to keep; set `Card::Window`'s zone (`lab/plan.rs`) to where the hole really is, and whether the
   second window setting (`on_b`) is worth keeping.
4. Follow-ups, not built: taking `dimming` and the VSP target off the helper's list (the unit tests use them as their register example), and
   an out-of-process guard that restores when the helper itself is killed (§ 4, last row).

## 1. What Nitin sees

Opened from Settings > Picture > Expert > **Display lab**, or `ctl shell show displaylab`. Not on Home.

| Step | Looks compared (A is the set's own state, bar step 3) | Where to look (the tag on the frame) | Said on the sheet | Asked |
|---|---|---|---|---|
| 1 Sharpness 1 of 5 | Normal / Strong (OSD sharpener gain `0x80`) | Small letters: 12 to 24 px text, white and dark | Sharper edges, or bright halos. | Which looks better? |
| 2 Sharpness 2 of 5 | Normal / Medium (gain `0x40`, between the set's `0x10` and Strong) | Small letters | Crisper edges. Jagged up close? | Which looks better? |
| 3 Sharpness 3 of 5 | Medium / Strong, against each other | Small letters | Crisp, without jagged edges? | Which looks better? |
| 4 Sharpness 4 of 5 | Normal / Soft (gain `0x00`) | Thin lines: 1-px lines and a checkerboard | Lines turn fainter and blurrier. | Which looks better? |
| 5 Sharpness 5 of 5 | Normal / Sharpener off | Letters and colours: bars, ramps, text in colour | Plainer letters, colours may shift. | Which looks better? |
| 6 Window 1 of 2 | Window off / Window on (mixer hole, select 1) | The window: a grey rectangle in the middle | The picture behind shows through. | Does the window show the picture cleanly? |
| 7 Window 2 of 2 | Window off / Window on (select 3) | the same | the same | the same; left out when step 6 was answered Yes |

On every step: a frame round the zone with a tag (an eye, then two or three words) and, where there is room, a large arrow from the
sheet to it. The sheet beside it: the group and counter, the one line on the difference, the option on the screen as a big letter and its
name, the two switches (**A** and **B**, for the pointer), the question, the answers one under the other (**This one is better**, **Can't
tell**, **Skip**; or **Yes**, **No**, **Can't tell**, **Skip**; or just **Next** when the look could not be put on the screen, see below), and the keys as glyphs.

| Press | Does |
|---|---|
| Left or Right, the wheel turned | switches between A and B (the TV is changed to it, the big letter follows) |
| Up, Down | moves between the answers; the first answer has the focus on arrival |
| the wheel pressed (OK) | answers with what is on the screen, or presses the focused button |
| Back | leaves; everything is put back |
| the pointer | hover is the cue, a click is OK on that control; the switches only switch |

After the last step: the results (one row a step, "Strong looks better", "OK", "Not OK", "Can't tell", "Skipped"), saved on the TV
(`<data dir>/displaylab/results.json`, and the last ten runs dated) and sent to the helper, from which Main reads them
(`GET /v1/displaylab/results`). Each step records the answer, the time spent, whether what the helper wrote still held, **how often the
person switched and whether B was on the screen at all** (an answer given without looking at both looks says so). Done leaves; Again
starts a new run.

**A look that could not be applied is said so.** When B is asked for and the helper's answer says the register does not hold what was
written (the firmware put it back), or the write is refused, the sheet says "Couldn't apply Strong. The TV puts it back." (in the helper's own
words when it refused), the answers about a picture that was never on the screen go, and the one that is left is **Next**, which records
`failed` ("Couldn't apply") and goes on. Two identical pictures are never offered for a pick. Switching to A and back to B tries again, and
the answers return when B shows. A look that showed and was taken back later keeps its answers and says so ("The TV put Strong back by
itself."). The helper says whether things hold per control (`heldBy`), so a register of the window taken back does not fail a sharpness
look. Each step records `bApplied` besides `held`.

Not in the lab, by decision: the page-size probe (the driver refused it with EINVAL, that route is closed), the orbit (measured, not judged by
eye) and **OSD0's dimming and grey (VSP, `0xCC480360`)**. Round 19 read it on the TV (`slot-2026-10-29-r19/displaylab/`): the firmware owns
it and puts `0x02000001` back at once when `0x01000001` is written (the session read `drifted ["osd0_vsp"]`, `held false`, 89 sets, and
the watch never saw `0x01000001`), and grey `0x02000003` stayed in the register but showed nothing on LumaTV's plane. The helper still
lists the `dimming` control (its tests are built on it as the register example); no step offers it. Taking it off the list is a follow-up.

## 2. How it is built

- **The screens** (`rustui/ui/src/lab/`, no QML): `plan.rs` the steps (five of sharpness, two of the window), `card.rs` the test cards (painted once into a full-screen
  image, theme-independent), `model.rs` the state (one request to the helper at a time, a ping every second while open, the exits),
  `surface.rs` the sheet and the frame, `host.rs` the part the shell feeds. Layer band `kit::z::LAB` (400): over Settings, which stays
  behind it for the way back, under the layout editor and the system layer.
- **Large text.** The sheet grows with the text size and gives up, in order, what it needs least: the answers go side by side, then the
  A/B switches (the keys and the wheel switch too), then the line on the difference, then the key glyphs. The big letter follows the text
  size up to 125 %. The window step's sheet stays right of the window (592 wide at most): the hole would cut a sheet that reached into it.
- **The helper's door** (`helper/app/src/displaylab/`): `GET /v1/displaylab`, `POST begin|set|ping|end|results`. The UI sends a control
  and an option (`sharpness: normal|strong|soft|off`, `dimming: normal|dimmed|grey`, `window: off|on_a|on_b`), never an address or a
  value.
- **The compositor** (`shell_rust.rs`): the kind `displaylab`; the wheel press is key 1199 and is OK; the pointer's keep-alive 1198 is
  no key; a turn of the wheel is sent to the client as the key `wheel` only while the lab is open and has said it takes it.

## 3. Tests

- Rust UI: model (step flow: switch, pick, can't tell, skip, exit, every way out, one request at a time, the last switch wins),
  plan (ids held to the helper's list by reading its source), cards, surface (lint clean at 100 % and 200 %, dark and light, every
  step, the intro, the results, a long note), shell (keys, wheel, the release of a key the lab took, pointer, Home key, another app,
  screen off, the kind off).
- Helper: `allow.rs` (the table; every address and value outside it refused; implausible originals), `lab.rs` (journal first, read-back,
  every exit restores, the dead-man fires on silence, restore at start, a restore that fails is retried and blocks new sessions),
  `tests/displaylab.rs` (the door: only the shell, no browser, the list), `tests/ai.rs` (the AI-access listener has no route to it and no
  tool for it).
- Compositor: the wheel press is key 1199, the keep-alive is no key, the wheel turn reaches only a client that took it.
- Host, against the real engine on Mesa: `tests/test_rust_displaylab.py` (what the keys send the helper, what is kept, lint clean);
  `tests/render_rust_displaylab.py` makes the pictures.

## 4. Safety

The lab writes the display chip's registers, so what it may write is one short list and every write is undone.

| Rule | How |
|---|---|
| A hard list | `allow.rs`: six targets, no others: `/proc/lg/fbdev/chip/osd_sr_gain` (`0x00`, `0x40`, `0x80`), `/proc/lg/fbdev/chip/osd_sr` (0, 1), `0xCC480360` (`0x01000001`, `0x02000003`), `0xCC4802C8` (`0x10B403C0`), `0xCC4802CC` (`0x0032A21C`), `0xCC48029C` (`0x1`, `0x3`). Besides these only the target's **original**, read from the target itself, is ever written |
| No address or value from a client | a request names a control and an option; `plan` turns it into writes; `hw.rs` checks every write against the list again at the bottom |
| Original first | the value read before the first write is the one restored; one that is not plausible (a lab value left by a crash, a register that holds nothing of this block) makes its control unusable instead of being guessed |
| Journal before write | the originals and the values about to be written are on disk before the first byte; it is removed only when every touched target reads its original again |
| Read-back | after every write, and after every restore; a mismatch is `verify_failed` and the restore runs |
| Dead-man | six seconds without a word from the shell (every request and the one-second ping renews the lease): everything is put back, by the helper's own thread |
| The UI dies, freezes, or the screen goes off | no ping: the dead-man |
| Nobody touches the lab for three minutes | the lab ends itself (a key, a click or the pointer moving over it is somebody), so a walked-away look is not left on for hours or under the saver |
| The lab ends by any path | Back, Home key, another app in front, screen off, the last step, a scene that cannot be drawn: the model sends `end` and the helper puts everything back; a lab that is closed asks for nothing more |
| The helper stops | SIGTERM and `lumatv-helper stop` restore before exit |
| Boot, or the helper restarted | `recover()` at start restores from the journal, and only the targets that still hold what the lab wrote (a reboot since means the register is not the lab's to change) |
| A restore that fails | kept as pending, retried every two seconds; no new session until it is done |
| Who can ask | the TV's own shell only: a loopback peer, `X-Luma-Client: lumatv-ui`, no `Origin` or `Sec-Fetch-Site` (a browser). The AI-access listener is a separate server whose handler has no route to it; `tests/ai.rs` lists the paths and tool names and pins that |
| Off switch | `displayLab.enabled` in `helper.json` (on by default: the lab can write its list and nothing else) |

Residual risk: the helper killed by SIGKILL, or crashing, in the minutes a lab is open leaves the registers changed until the helper is
started again (at boot, or by hand), when `recover()` restores them. Every value on the list is one the set accepted in the 2026-10-29
session and a reboot clears them; an out-of-process guard would close this and is not built.

## 5. The TV check (next round, Round19 at the set)

Install needs Nitin's Face ID. `tools/displaylab-tv.sh` (copied to the set, read-only, writes no register) keeps the six readings
(`baseline`), compares them again (`check LABEL`), times a way out (`watch`) and asks the helper's door (`door`, `results`). Then:

1. `ctl shell rust +displaylab`; `ctl shell` lists `displaylab` in the kinds and in `can`.
2. With the lab closed: `displaylab-tv.sh baseline` (expected on the set of 2026-10-29: gain `0x10`, sharpener on, the VSP `0x02000001` or
   `0x0` as the boot left it, holes `0`), then `displaylab-tv.sh door`: `available true`, the three controls `usable` (`why` says what is not).
3. Open it (Settings > Picture > Expert > Display lab, and `ctl shell show displaylab`). Press OK with the wheel exactly once: **one**
   step happens (the wheel press arrives as 1199 and may also arrive as another key on a device that also sends KEY_ENTER; if a step is
   skipped on one press, that is it). Turn the wheel: A and B switch. Move the remote without pressing: nothing happens (1198 is ignored).
4. Step through all seven with a video playing behind (the window step); on Medium and Medium / Strong say whether Medium is crisp without the jagged edges close up. Watch for: whether the frame on the card is where the hole
   opens, whether the hole shows the picture cleanly, and what each sharpness and dimming look is (Nitin's answers).
5. Every way out, with a look that changes a register on the screen (step 4, 5 or 6 at B), then `displaylab-tv.sh check LABEL`: Back; the
   Home key; another app in front; kill the UI client (`kill -9 lumatv-ui`, with `watch` running: back within about six seconds);
   `lumatv-helper stop` mid-lab; leave it alone for three minutes. **Expected: every time `everything is as it was`.**
6. Read the results: `displaylab-tv.sh results`, and `<data dir>/displaylab/results.json`.

## 6. Known limits

- The lab starts no video. With Home in front the window step says so and a playing video shows the window best; open the lab over a
  video for the real test. Whether Home's own moving wallpaper counts is not known to the lab.
- The window's frame is where the hole is expected (the middle, 480 by 270). The hole's units come from HwTest's notes and are not
  confirmed on the panel; the first window setting may be the wrong one, which is why there are two.
- At 200 % text the window step's sheet (592 wide) shows no switches and no line on the difference.
- A turn of the wheel is one switch however many notches arrive in one read (the compositor merges a spun wheel into one event), and
  the turn has no direction: with two looks that is no loss.
- Wheel press 1199 is mapped to OK at the top of the key path; the TV check confirms exactly one OK per press.

## 7. Design review

Native platform, so the automated detector does not apply: Assessment A only (a read of the host renders against the Lucent language
and the craft floor), done inline since no sub-agents were allowed in this round.

- **Hierarchy.** Where to look is on the card itself (frame, eye tag, arrow), not in a sentence; what differs is one line; the state
  is the biggest thing on the sheet. The first version had a sentence for where to look; the tag replaced it, which also saved the height
  large text needed.
- **Order.** The two switches sit under the big letter they change, the question under them, the answers last, as the person's hands go.
  The first version had the question between the letter and the switches.
- **Copy.** Every line is a few words; the keys are glyphs (the wheel press, the check, the return arrow), no "OK" or "Back" words.
- **Honest states.** A helper that says no, a control the set cannot do, a restore the TV made by itself and Home behind the window each
  say so in the helper's or the model's own words, in the note position, and never block the way out.
- **Contrast and overlap.** The host lint reads clean at 100 % and 200 %, dark and light, on every step; the sheet never reaches over the
  frame or the window.
- **To look at on the TV.** The focus lift of the first answer is the language's own and quiet at ten feet; the light material's answer
  pills have faint edges on the black test page.
