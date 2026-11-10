# Phone remote: design (PhoneRemoteDesign, 2026-11-06)

Design only; nothing was built and nothing on the TV was changed. Evidence tags: `TV:` read on this set on 2026-11-06, read-only; `REF:` this repo at `1b6d1ffe`,
Marquee at `e86aed619`, Stashy's iOS workflow and LumaShow's shared `esphome-api` crate; `PUB:` public documentation; `[INFERENCE]` not run. Compositor paths are
under `compositor/crates/lumatv-compositor/src/`.

## Verdict

- **Phone → helper → compositor.** A first-party plugin, `phone-remote`, off by default, listens on the TV's home-network address only while it is on. A phone
  pairs once with a code the TV shows and then holds its own key. The helper turns everything the phone sends into a new `phone` command family on the
  compositor's existing control socket: one ordered stream for pointer, clicks, scrolling, keys, text and LumaTV actions. The compositor gains relative pointer
  motion and scroll events and opens no network port.
- **Text goes into the field, not through key presses.** Where the compositor tracks a text field (web and Enact apps, LG Search, LG's Home, Marquee), phone
  text lands with the same `commit_string` LumaTV's keyboard uses, whichever on-screen keyboard serves the field. LumaTV's own entry boxes take it from the
  shell. YouTube, Netflix and Prime Video show no text field, so they need letter keys in the keymap the compositor serves: Nitin's decision, untested.
- **About 5 ms added at the median, under 20 ms at p95 [INFERENCE].** Pointer state rides small encrypted UDP datagrams, everything else encrypted JSON over one
  TCP connection. Wi-Fi is nearly all of it; the TV's own hop is well under a millisecond. A wired benchmark, the app's ping and TV-side stamps measure it.
- **iOS builds on GitHub's macOS runners, as Stashy's do.** `LumaRemoteKit` is a Swift package in its own public repo with the thin app beside it. CI tests it
  against byte vectors the Rust test client writes and publishes an unsigned IPA for AltStore, SideStore or Feather. Marquee's iPhone port adds the package by URL.

## Injection point

The compositor reads evdev itself and routes keys through `State::key_event(seat, code, pressed, from_network)` (`input.rs`). The Magic Remote's pointer arrives as
absolute positions (`server/seat.rs` `pointer_motion`, clamped to the 1920×1080 output); relative motion from other devices counts only as activity. `wl_seat` is
version 4 and no `wl_pointer.axis` is ever sent, so no app can be scrolled by pointer today. The control socket (`control.rs`; TV: `/tmp/xdg/lumatv-control`,
`srwxr-xr-x root`) already takes `pointer`, `click`, `wheel`, `remote CODE press|release|tap` (seat 0, as the remote), `luna` and `shell`, keeps persistent
clients, and the helper's `capture.rs` already uses it. Text reaches the focused field through `server/ime.rs` (`own_text`, `own_key`, `own_clear` →
`commit_string`, `keysym`, `delete_surrounding_text`), gated today on LumaTV's keyboard serving the field (`own_active`).

| | Helper → control socket (chosen) | uinput device | LG's input service (SSAP) |
|---|---|---|---|
| Pointer | a new relative verb, one local hop | REL events, which the compositor treats as activity: the same new code is needed | its pointer socket feeds `LGE Network Input`, a relative device (REF `arch/input-devices-01.txt`) [INFERENCE] |
| Text | `commit_string` into the field | key codes only; the served keymap has no letters | `insertText`, which feeds LG's keyboard, not LumaTV's [INFERENCE] |
| LumaTV actions | `SystemRequest`, shell panels, helper routes | none | none |
| Order of pointer, keys, text | one stream | text and actions on a second path | a separate service |
| Setup | none | a new node is opened only after a rescan, ~1.6–2 s (REF `HANDOFF.md`) | SSAP pairing prompt, more hops |

Decision: the helper holds one persistent connection to `lumatv-control` and speaks `phone` verbs over it. Pointer and keys on uinput would let an Enter overtake
the text before it, and the compositor would still need new code for relative motion; the extra local hop costs well under a millisecond [INFERENCE]. The
compositor stays off the network: authentication, pairing, mDNS and plugin state live in the helper, where privileged work belongs.

Phone keys go to seat 0, the seat `ctl remote` uses, and never pass through evdev. The AI gate and the pairing gate count presses only from `LGE RCU` and
`LGE M-RCU*` devices (`helper/app/src/ai/gate.rs`), so a phone can approve neither.

Compositor changes:

- **Relative motion.** `phone move DX DY` in 1/16 px adds to the cursor position, keeping the sub-pixel remainder, and calls `pointer_motion` (clamp, enter and
  leave, the shell's sample). Phone motion shows the pointer under either `remote.pointerShow` setting: it only flows while the owner is deliberately pointing.
  `phone pointer on` refreshes the shown time on every keepalive, so the 10 s `CURSOR_TIMEOUT` does not hide a pointer held still; `off` hides it at once, as the
  remote's cursor-off report (1199) does. The timeout stays as the backstop for a phone that vanishes.
- **Buttons.** Left is what OK does while the cursor shows (`seat::pointer_button`, BTN_LEFT to the surface under the pointer; `shell_rust_pointer_button` for
  the shell's layers). Right is BTN_RIGHT, to apps only.
- **Scrolling.** `wl_pointer.axis` with continuous values in surface pixels to the app under the pointer; version 4 has no frame, source or stop events, so the
  phone computes momentum. The shell gets `shell_rust_wheel_direction` steps, one per 64 px [INFERENCE: tuned on the TV]. `wl_seat` version 5 waits until
  Chromium, Qt, Flutter and SDL apps are checked against it on the set.
- **Keys, text, actions and events:** see [LumaTV side](#lumatv-side).

## Text entry

| Field | Examples | Path | Status |
|---|---|---|---|
| An app field the compositor tracks (a text model) | Enact and web apps, LG Search (Qt), LG's Home (Flutter), Marquee (SDL) | `commit_string`, `keysym` and `delete_surrounding_text` on the active model: `own_text`, `own_key` and `own_clear` opened beyond `own_active` | the path passed on the TV for Marquee's fields and LG Search under LumaTV's keyboard (REF `keyboard-result.md`); from a phone untested |
| LumaTV's own entry boxes | Settings fields, connection pages (the shell's Entry panel, which keeps its own drafts) | a `phonetext` event to the shell, which edits the draft; the Entry panel reports its field kind back | untested |
| No text field | YouTube (Cobalt), Netflix, Prime Video (`keyboard-result.md` lists no text field for them) | one `wl_keyboard` key per character to the focused window. The served keymap (`server/keymap.xkb`, from `compositor/tools/gen-keymap.py`) has digits, space, BackSpace and Return but no letters or Shift, so US letters must be added | Nitin's decision; untested |

- **The phone mirrors the field's kind, never its content.** The TV sends purpose, hints, Enter key, length limit and whether the field holds text (the `ime`
  state LumaTV's keyboard already publishes). The phone picks its keyboard from them: digits and number → number or decimal pad, phone → phone pad, url → URL,
  email → email, password or the hidden hint → secure entry, terminal → ASCII with corrections off, name → words capitalised; the lowercase, uppercase and
  titlecase hints → capitalisation none, all, words; sensitive → autocorrection and predictions off; Enter done, go, send, search and next → the matching
  return key.
- **Committed text only.** iOS marked text (a word still being composed: Japanese, Chinese, dictation) waits until the phone commits it. Edits are diffed into
  backspaces and insertions; one `text` message carries at most 256 bytes, the limit `own_text` already has.
- **One keyboard at a time.** While the phone's keyboard is open the TV's on-screen keyboard hides (`own_hide`, which leaves the field active) and returns when
  the phone's closes with the field still active. Under LG's keyboard (`lumatv keyboard lg`), a phone commit while LG's keyboard holds an unfinished word may
  leave its preedit stale until the field is refocused [INFERENCE]; LumaTV's keyboard is the set's persistent choice (REF `keyboard-result.md`).
- Nothing typed is logged on either side; the journal records lengths only.

## Transport and latency

Discovery: mDNS service `_lumatv-remote._tcp` from the helper's `mdns` responder, which `ble.rs` and `onhome` already use (TV: several sockets share UDP 5353).
Pairing can also start from a QR code that carries the address, port, TV id and code.

Two channels on port 18733 (TV: free; the helper's others are 18731 and 18732):

- **TCP** for everything that must arrive in order: keys, text, actions, lists, state. Noise-encrypted JSON frames, `TCP_NODELAY`.
- **UDP** for pointer and scroll. A Wi-Fi retry on TCP stalls every later sample behind the lost one. Each datagram instead carries running totals, so a lost one
  costs only its own sample; button changes are sent three times (at once, +5 ms, +20 ms).

Budget, from the phone's gyro sample to the compositor applying it:

| Stage | Median | p95 |
|---|---|---|
| Phone: filter, seal, send | < 0.5 ms | 1 ms |
| Wi-Fi uplink (5 GHz, phone awake) and wired LAN | 2–4 ms | 10–15 ms |
| Helper: receive, open, write the control line | < 0.2 ms | 0.5 ms |
| Compositor: wake, apply, move the cursor plane | < 0.2 ms | 2 ms (mid-frame) |
| Added | ~3–5 ms | under 20 ms |

All [INFERENCE]. Not counted, as the Magic Remote has equivalents [INFERENCE]: the gyro's 100 Hz sampling (5 ms on average) and the wait for the next 120 Hz
vblank (up to 8.3 ms).

Measurement:

1. **TV hop:** the helper stamps each datagram's arrival (`CLOCK_MONOTONIC`, µs) on its `phone move` line; the compositor stamps the apply on the same clock;
   `ctl phone stats` reports median, p95 and p99.
2. **Wired baseline:** `lumaremote bench 192.168.1.242` from BeastNAS sends 1,000 pointer datagrams at 100 Hz with ping probes and reports the round trip and the
   TV hop.
3. **Wi-Fi:** the app's Diagnostics screen runs the same 1,000-sample ping while pointing. Accepted when round-trip p95 plus TV-hop p95 is under 20 ms; the round
   trip includes the downlink, so this overstates the one-way figure.
4. **Feel:** a 240 fps video of phone and TV in one frame while the app flashes its screen as it jumps the pointer to the centre; 20 trials, recorded beside the
   numbers, not a gate.

Wake:

- **Active Standby** (screen off; helper, compositor and network up: the set answered SSH in that state, REF `compositor-gate9-result.md`). The phone's `power`
  key reaches `power_key` (`input.rs`), which calls `power/powerOn {"reason":"remoteKey"}` as the remote's Power key does; the warm remoteKey wake cases passed
  on the set (REF `wake-learnings.md`). The listener stays up while the screen is off and then answers only `power`, `ping` and `bye`.
- **Deeper standby or cold:** Wake-on-LAN to `74:c1:7e:bd:ae:30`, which needs LG's wake-on-network setting (`wolwowlOnOff`, REF `HANDOFF.md`); a magic packet
  booted the set from cold (REF `compositor-gate9-result.md`). iOS sends broadcasts only with Apple's multicast entitlement (PUB), which Apple grants on request
  to paid developer accounts. Without it the app sends the magic packet unicast to the TV's last address on UDP 9 [INFERENCE: it arrives only while the phone
  still holds the TV's ARP entry], then reconnects with backoff for 20 s.

## Security

- **No port while off.** The plugin's switch creates the listener and the mDNS record; off removes both and ends every session. The listener binds the TV's
  private address (TV: eth0 192.168.1.242; the AI listener's `bind` rule) and drops peers outside 10/8, 172.16/12 and 192.168/16. The set's firewall accepts
  everything (TV: `INPUT` policy ACCEPT, no rules), so this check is the only filter.
- **Nothing answers before authentication.** A bad preamble or handshake is closed without a byte; a datagram without a live session and a valid tag is dropped
  silently. Failed handshakes count per peer, as `ai/mcp.rs` counts failures: 8 in 60 s and that peer is ignored for the rest of the minute. A handshake must
  finish within 3 s.
- **Pairing needs the physical remote.** "Pair a phone" first asks for OK on the remote (the AI gate's `GateApproval`), then shows an 8-character code from the
  AI alphabet (2–9 and A–Z without I and O: 40 bits) as `XXXX-XXXX`, beside a QR code. The window lasts 5 minutes, closes after 5 wrong tries and is single use
  (the `ai/store.rs` constants).
- **What the code gives an attacker.** The pairing key is PBKDF2-HMAC-SHA256 of the code (131,072 iterations: 2^18 SHA-256 compressions per guess) and the
  handshake is Noise NNpsk0. A passive listener who records a pairing gets nothing: the session also needs both ephemeral secrets, and the code dies with the
  window. An active attacker on the LAN must recover the code from the phone's first message and finish the exchange inside the window; one high-end GPU
  (~2×10^10 compressions/s [INFERENCE]) covers about 1 in 44,000 of the 2^40 codes in 5 minutes. A PAKE would remove even that offline guess; neither CryptoKit
  nor the shared crates carry one.
- **Per-phone keys.** The TV generates each phone's 32-byte key and sends it once, inside the encrypted pairing session. The TV keeps it in
  `<data>/phone-remote/devices.json` (root, 0600); the phone keeps it in the Keychain as `AfterFirstUnlockThisDeviceOnly` (not synced, not in backups). Every
  connection runs a fresh handshake (forward secrecy); the UDP key is new per session and datagram sequence numbers only rise (no replay).
- **Revoke.** Settings removes one phone or all of them: the key goes, the live session gets `revoked` and closes at once. Switching the plugin off keeps
  pairings.
- **Narrow reach.** The phone gets a field's kind, never its text; nothing typed is logged; a phone cannot approve the AI gate or open pairing; frame and
  datagram rates are capped.

## LumaTV side

Code split, keeping the engine free of LumaTV dependencies:

- **`helper/remote`** (crate `lumaremote`): protocol types, the Noise session over `esphome-api`'s handshake, the pairing KDF (`ring::pbkdf2`; `ring` is already
  in `helper/Cargo.lock`), the datagram codec and pointer accumulator, the TXT and QR formats, and the `lumaremote` test client. `esphome-api`'s Noise module
  has its prologue as a constant (`"NoiseAPIInit\0\0"`); it gains a prologue parameter in LumaShow, where the crate is shared and never forked.
- **`helper/app/src/phone_remote/`**: the opt-in plugin (`OptIn` and `Spec` as in `receiver_plugin.rs`, a schema beside `schemas/denon-receiver.json`, a
  `firstParty` roster entry), listener and sessions, the pairing window and gate prompt, the device store, the mDNS record, the compositor link and the action
  mapping. Consent text: "Lets paired phones control this TV over the home network: the pointer, the remote's keys, typing into text boxes and LumaTV's
  actions. While on, the TV listens on port 18733."
- **Compositor verbs**, over one persistent connection the helper opens as `capture.rs` does. As built, the pointer and keys reuse the control socket's
  `pointer`, `click`, `wheel` and `remote` verbs, and the new ones are `phone session on|off`, `phone text BASE64` and `phone edit OP [N]`
  ([`docs/plugins/phone-remote.md`](../plugins/phone-remote.md)).

- **Rust shell:** the Entry panel applies `phonetext` and reports its field kind. Settings gains **Remote & pointer › Phone remote**, built like
  `rustui/ui/src/settings/pages/ai.rs` from existing rows, so it takes the theme's tokens: the plugin switch with its consent text; "Pair a phone" (gate prompt,
  then the code, a QR row from `settings/surface/code.rs` in theme colours, and the time left); one row per paired phone (name, model, last used, Connected)
  with Remove; "Remove all phones".
- **Helper routes** on the loopback API: `GET /v1/phone-remote` and hub topic `phone-remote` (`{on, pairing: {code, qr, expires} | null, devices: [{id, name,
  model, last, live}]}`); `POST` and `DELETE /v1/phone-remote/pairing`; `DELETE /v1/phone-remote/devices/{id}`; `DELETE /v1/phone-remote/devices`.
- **Scrypted:** opening one camera's popup on request is new; `helper/app/src/scrypted/mod.rs` publishes `cameras.popup` only for its own events today.

Actions:

| `act.do` | Arguments | Effect | Built on |
|---|---|---|---|
| `home` | | LumaTV Home | the Home key's tap route |
| `quick` | | Quick Settings | `SystemRequest::QuickSettings` → `shell_show("quick")` |
| `settings` | `page` (optional) | Settings, or one page of it | `SystemRequest::OpenSettings` |
| `notifications` | | notifications | `SystemRequest::OpenNotifications` |
| `input` | | input picker | `SystemRequest::InputPicker` |
| `pictureoff` | | screen off, sound on | `SystemRequest::PictureOff` |
| `cameras` | | camera panel | `shell_show("cameras")`; cameras plugin on |
| `camera` | `camera` | one camera's popup | the helper's Scrypted popup (new) |
| `camera-close` | | close the popup | `POST /v1/cameras/popup/clear` |
| `guide` | | Live TV guide | `shell_show("guide")`, as the Guide key |
| `tune` | `channel` | change channel | launch Live TV with the shell's `tune_request` parameters (`rustui/ui/src/host/channels.rs`) |
| `launch` | `app` | open an app | `SystemRequest::LaunchApp`, after checking the id against `listApps` |
| `volume` | `level`: a number on the receiver's scale, which moves by 0.5 (41.5 is a level; finer values snap to the nearest half) | set the receiver's level | `POST /v1/receiver/volume`; receiver plugin on |
| `power` | `to`: `on` or `off` | power | a Power tap when the state differs |

Volume, mute and channel steps are keys, not actions: they take the remote's own routes (the receiver while its plugin is on, LG's audio service otherwise).
Marquee playback is the media keys, which Marquee already maps (REF Marquee `rust-modules/ui/src/consts.rs`).

## Protocol, version 1

Complete for a client. Integers are little-endian unless stated; `lumaremote vectors` (below) gives exact bytes.

### Discovery

- mDNS `_lumatv-remote._tcp.local`, port 18733, instance name the TV's name. TXT: `v=1`; `id=` the TV id (8 random bytes made when the plugin is first switched
  on, as 16 lowercase hex); `name=`; `mac=` the wired MAC as 12 hex, for Wake-on-LAN; `pair=1` while a pairing window is open, else `pair=0`.
- QR code and link: `lumaremote://pair?h=192.168.1.242&p=18733&id=<tv id>&c=<code without the dash>`.
- UDP uses the same address and port as TCP.

### Connection

1. Open TCP to port 18733 and send the 6-byte preamble: `4C 52 4D 54` (`LRMT`), version `01`, mode `01` (session) or `02` (pairing).
2. Run `Noise_NNpsk0_25519_ChaChaPoly_SHA256` as initiator: prologue = the 6 preamble bytes; PSK = the phone's key (session) or the pairing key (pairing); empty
   handshake payloads. Handshake messages travel raw, without a length: the phone sends message 1 (48 bytes: ephemeral public key, 16-byte tag); the TV
   answers message 2 (48 bytes) or closes without sending anything. In session mode the TV tries each paired phone's key; a key that fails costs no
   Diffie-Hellman, since message 1's tag is checked first.
3. Split gives two ciphers, the first for phone → TV. ChaCha20-Poly1305; nonce = 4 zero bytes, then the cipher's 64-bit message counter; associated data empty.
4. Every later message is a frame: a u16 length, then that many bytes of ciphertext (plaintext plus its 16-byte tag). The plaintext is one UTF-8 JSON object
   with a string member `t`, at most 8,192 bytes.

Pairing key = PBKDF2-HMAC-SHA256(password = the 8 code characters, upper case, no dash; salt = ASCII `LumaRemote pair v1` followed by the TV id's 8 bytes;
131,072 iterations; 32 bytes). A typed code is upper-cased with spaces and dashes removed, and characters outside the alphabet are refused before connecting.

### Pairing (mode 2)

- Phone: `{"t":"pair","name":"Living room iPhone","model":"iPhone17,1","app":"LumaRemote 1.0"}`. `name` is 1–40 characters; iOS reports only the model name,
  so the app asks for one.
- TV: `{"t":"paired","device":"<16 hex>","key":"<base64, 32 bytes>","tv":{"id":"<16 hex>","name":"LG C4","mac":"74:c1:7e:bd:ae:30"}}`, then closes. The window
  closes with it.
- A refusal after a good handshake is `{"t":"error","code":"bad-message"}` and a close. A wrong code shows as a closed connection and uses one of the window's 5
  tries.

### Session (mode 1)

The phone's first frame is `hello`; the TV answers `welcome`, then pushes `state` whenever any of it changes.

Phone → TV:

| `t` | Members | Meaning |
|---|---|---|
| `hello` | `name`, `model`, `app`, `kit` | first frame; updates the phone's row in Settings |
| `key` | `k` key name; `a`: `tap`, `hold`, `down` or `up` | a remote key. `tap` and `hold` are timed on the TV, so Wi-Fi jitter cannot turn one into the other. `down` and `up` only for keys meant to be held: arrows, `volup`, `voldown`, `chup`, `chdown`, `rew`, `ff`. `home` takes `tap` only: holding Home is the physical remote's way out to LG. |
| `text` | `s`: 1–256 bytes of UTF-8, no control characters | text into the active field |
| `edit` | `op`: `backspace` (with `n`, 1–256, default 1), `enter`, `tab`, `left`, `right`, `clear`, `hide`, `show` | editing in the active field; `hide` and `show` the TV's on-screen keyboard |
| `act` | `id` (u32 the phone picks), `do`, the action's arguments | a LumaTV action (table above); answered by `done` |
| `get` | `id`, `what`: `apps`, `channels` or `cameras` | a list; answered by `list` |
| `ping` | `n` | keepalive; answered by `pong` |
| `bye` | | clean close |

TV → phone:

| `t` | Members | Meaning |
|---|---|---|
| `welcome` | `v` (1); `device`; `tv` {`id`, `name`, `mac`, `accent`: the theme's accent as `#rrggbb`}; `udp` {`session`: u32, `key`: base64 of 32 bytes}; `state` with every member | the session is open |
| `state` | the changed members | see below |
| `done` | `id`; `error` with a code when it failed | an action's result |
| `list` | `id`, `what`, `items`, `more` | one page; a long list comes as several frames with the same `id`, the last with `"more": false` |
| `pong` | `n` | |
| `error` | `code`; `about`: the offending `t` | for `key`, `text` and `edit`, which carry no `id` |
| `revoked` | | the phone was removed: the TV closes and the phone deletes its key |

State members:

- `power`: `on`, or `standby` (screen off; only the `power` key, `ping` and `bye` work).
- `fg`: the webOS id of the app holding keyboard focus (`com.beb.plxnative` is Marquee); empty while LumaTV's own layer has it.
- `ime`: `{"active": false}`, or `{"active": true, "owner": "app" | "lumatv", "purpose", "hint", "enter", "max", "text"}`. Purpose 0 normal, 1 alpha, 2
  digits, 3 number, 4 phone, 5 url, 6 email, 7 name, 8 password, 9 date, 10 time, 11 datetime, 12 terminal. Hint bits 0x8 lowercase, 0x10 uppercase, 0x20
  titlecase, 0x40 hidden text, 0x80 sensitive. Enter 0 default, 1 return, 2 done, 3 go, 4 send, 5 search, 6 next, 7 previous. `max` is the length limit (0
  none); `text` whether the field holds any text.
- `volume`: `{"via": "receiver" | "tv", "level", "max", "muted"}`, each member present when known. `level` and `max` are numbers on the receiver's scale,
  which moves by half steps (41.5), exactly as its display shows them (LumaRemoteKit 1.0.3 and later; 1.0.2 decoded them as integers).
- `acts`: the `act.do` names that work right now (`cameras` only while the cameras plugin is on, for example).

List items: `apps` `{"id", "title"}`, the visible launchable apps by title; `channels` `{"id", "no", "name"}`, the Live TV list (`id` is the channel id `tune`
takes); `cameras` `{"id", "name"}`.

Key names, with the Linux code the TV delivers:

| Group | Names (code) |
|---|---|
| Navigation | `up` (103), `down` (108), `left` (105), `right` (106), `ok` (28; a click while the pointer shows), `back` (412), `home` (773), `exit` (174) |
| Remote | `menu` (139, the gear: Quick Settings), `info` (358), `guide` (362), `list` (787), `more` (994), `input` (241), `power` (116) |
| Volume, channel | `volup` (115), `voldown` (114), `mute` (113), `chup` (402), `chdown` (403) |
| Digits, colours | `0` (11), `1`–`9` (2–10), `red` (398), `green` (399), `yellow` (400), `blue` (401) |
| Media | `play` (207), `pause` (119), `playpause` (164), `stop` (128), `rew` (168), `ff` (208) |
| App keys | `netflix` (1037), `amazon` (1038) |

Error codes: `bad-message` (not JSON, unknown `t`, a member missing or wrong), `bad-key`, `bad-action`, `too-long`, `no-field` (`text` or `edit` with no
active field; once letters are in the keymap, `text` goes out as keys instead), `off` (the plugin an action needs is off), `standby`, `not-found` (unknown app,
channel or camera), `unavailable` (LumaTV's compositor is not running), `rate`.

### Pointer datagrams

Header, 12 bytes, sent in clear and authenticated as associated data:

| Offset | Size | Field |
|---|---|---|
| 0 | 2 | `4C 52` (`LR`) |
| 2 | 1 | version, 1 |
| 3 | 1 | kind: 1 pointer (phone → TV), 2 ping (phone → TV), 3 pong (TV → phone) |
| 4 | 4 | session, u32 from `welcome` |
| 8 | 4 | sequence, u32: from 1, up by 1 per datagram, per direction |

Body: ChaCha20-Poly1305 with `udp.key`; nonce `4C 52 D 00` then the sequence as u64, where D is 0 for phone → TV and 1 for TV → phone; associated data the
header; the plaintext's ciphertext plus a 16-byte tag.

Pointer plaintext, 24 bytes:

| Offset | Size | Field |
|---|---|---|
| 0 | 4 | x: i32, total pointer motion since the session began, 1/16 TV pixel, right positive, wrapping |
| 4 | 4 | y: the same, down positive |
| 8 | 4 | scroll x: i32 total, 1/16 px, positive scrolls right (content moves left) |
| 12 | 4 | scroll y: positive scrolls down (content moves up) |
| 16 | 1 | buttons held: bit 0 left, bit 1 right |
| 17 | 1 | left presses: u8 count since the session began, wrapping |
| 18 | 1 | right presses |
| 19 | 1 | centre count: u8, wrapping; each step puts the pointer at the centre |
| 20 | 1 | flags: bit 0 engaged (show the pointer and keep it shown), bit 1 scrolling (fingers on the scroll surface) |
| 21 | 3 | zero |

The TV drops, without answering, a datagram whose header is wrong, whose session is not live, whose sequence is not above the last accepted, or whose tag fails.
From the difference to the last accepted totals (wrapping subtraction) it applies, in order: centre steps; motion, at most 4,096 px per datagram; scroll; then
each button, with k the presses since the last datagram (mod 256): k > 0 with the button's bit clear gives k clicks; k > 0 with the bit set gives k − 1 clicks
and a press; k = 0 with the bit newly clear gives a release.

Ping plaintext, 12 bytes: `id` u32, phone time u64 (µs). Pong, 16 bytes: `id` u32, the phone time echoed u64, `hold` u32 (µs the TV held it).

The phone sends a pointer datagram per motion sample while anything moves (at most 120 per second); one at once on any change of buttons, centre or flags, with
copies at +5 ms and +20 ms; and one every 250 ms while engaged or while a button is held. Clearing engaged hides the pointer at once.

### Timing and limits

- The handshake finishes within 3 s of connecting.
- The phone sends a frame at least every 2 s (`ping` when idle). The TV ends a session silent on both channels for 6 s, releasing held keys and buttons and
  hiding an engaged pointer.
- A key held `down` for 30 s is released.
- Per session: 60 frames and 250 datagrams per second; beyond that, datagrams are dropped and frames answered with `rate`.
- 4 sessions at once; 16 paired phones.

### Versions

The preamble's version byte and TXT `v` change only for incompatible changes. A TV that does not speak the version closes without answering; the phone reads `v`
before connecting, so it can say why. Within version 1, unknown JSON members are ignored; unknown `t`, keys and actions get `bad-message`, `bad-key` and
`bad-action`, so a phone can probe; unknown datagram kinds are dropped.

### Test vectors

`lumaremote vectors` writes `vectors.json`, kept in `helper/remote/tests/`: pairing keys for sample codes and TV ids; both handshake messages and the first
frames each way for fixed ephemeral keys and PSKs; sealed datagrams; and pointer-total sequences, with wrap-around and lost datagrams, beside the events they
must produce. The Rust and Swift tests both run them.

## iOS: LumaRemoteKit

- **Repo:** public `nphil/LumaRemoteKit` with `Package.swift` at the root (SwiftPM resolves a package from a repo's root) and the thin app in `App/`, generated
  by xcodegen as Stashy's is (REF Stashy `ios/project.yml`).
- **`LumaRemoteCore`** (Foundation, CryptoKit, Network; iOS 17 and macOS 14): Noise from CryptoKit's X25519, SHA-256, HMAC and ChaChaPoly; PBKDF2 from
  CommonCrypto; frames, `Codable` messages and the datagram codec; discovery (`NWBrowser`, Bonjour with TXT); the link (`NWConnection`: TCP with no delay, and
  UDP), reconnect and keepalive; the Keychain store; Wake-on-LAN.
- **`LumaRemoteKit`** (iOS): air mouse, trackpad, keyboard bridge, an `@Observable` `RemoteModel`, and SwiftUI views: `LumaRemoteView` (Pointer, Trackpad, Keys,
  Keyboard and TV tabs), `PairingView`, `TVPicker`, `DiagnosticsView`. The views use the system look with the host's tint; the standalone app takes the TV's
  `accent`.
- **Air mouse:** device motion at 100 Hz (`xArbitraryZVertical`, bias-corrected `rotationRate`) on its own queue, sending straight from that queue. Rotation
  about gravity drives x and rotation about the horizontal axis across the phone drives y, so it points the same however the phone is held. Gain about 2,750
  px/rad with an acceleration curve, a One Euro filter (PUB: Casiez, Roussel and Vogel, CHI 2012) and a soft dead zone near 0.01 rad/s, all tuned on the TV.
  Hold to point: the pointer moves while a thumb rests on the pointer pad; a tap clicks; holding past 350 ms holds the button for a drag; motion freezes for
  about 100 ms around a press so the press itself does not move the pointer; a Centre button. Lifting clears engaged after 1 s.
- **Trackpad:** an acceleration curve; a one-finger tap clicks and tap-and-drag drags; two fingers scroll, with momentum computed on the phone; a two-finger tap
  is Back.
- **Keyboard bridge:** a hidden text field configured from `ime` (keyboard type, return key, capitalisation, corrections, secure entry); committed text only;
  changes diffed into `edit backspace` and `text`; Return sends `edit enter`.
- **Keys:** a D-pad (swipe for arrows, tap for OK, hold to repeat), Back, Home, gear, volume, channel, digits, colours and media keys, which lead while `fg` is
  Marquee.
- **Thin app** `LumaRemote` (`com.nphil.lumaremote`): TV picker; pairing by the `lumaremote://` link the Camera app opens from the TV's QR code, or by a typed
  code; the remote; Diagnostics (latency); settings (air-mouse mode, speed, haptics, volume buttons). Info.plist: `NSLocalNetworkUsageDescription`,
  `NSBonjourServices` with `_lumatv-remote._tcp`, the URL scheme. The phone's own volume buttons can drive the TV's volume through `outputVolume` observation
  (a decision).
- **Marquee:** the iPhone port embeds the kit (REF Marquee `docs/marquee/ports.md`): `.package(url: "https://github.com/nphil/LumaRemoteKit", from: "1.0.0")`
  (or xcodegen `packages:`), `LumaRemoteView` in a Remote tab, and its own pairing, which shows as its own entry in the TV's list (sideloaded apps share no
  Keychain [INFERENCE]). It pairs by typed code or its own QR scanner, since the standalone app owns `lumaremote://`.

Build, test and install (BeastNAS has no Swift; REF Stashy `.github/workflows/ios-build.yml`):

1. **Every push:** `macos-15` with the latest Xcode runs `swift test` (the core on macOS, vectors included) and `xcodebuild test` on an iPhone simulator for the
   iOS parts. A step compares the kit's `vectors.json` with the copy on LumaTV's `main`, so protocol drift fails CI. Public repos use GitHub's standard runners
   free of charge (PUB).
2. **Tags:** Stashy's release flow: xcodegen, unsigned `xcodebuild` (`CODE_SIGNING_ALLOWED=NO`), `Payload/` zipped to `LumaRemote.ipa`, a GitHub release and an
   updated `apps.json`. AltStore, SideStore or Feather installs it, signed with Nitin's Apple ID.
3. **Before the TV side exists:** `lumaremote fake-tv` on BeastNAS announces itself and logs what the phone sends.

## Rust test client

`lumaremote`, built from `helper/remote`, runs on BeastNAS and on the TV: `discover`; `pair HOST CODE`; `key HOST NAME [tap|hold|down|up]`; `text HOST STRING`;
`act HOST ACTION [ARG=VALUE…]`; `get HOST apps|channels|cameras`; `move HOST --circle|--square SECONDS`; `scroll HOST DY`; `bench HOST [-n 1000]`; `fake-tv`;
`vectors FILE`. Its keys live in `~/.config/lumaremote/` (0600).

## Plan

Each phase is accepted on the TV with its receipt path recorded.

1. **Engine and client:** `helper/remote`, the LumaShow prologue parameter, the vectors. Host tests only.
2. **Compositor `phone` verbs**, driven by `ctl phone …` over SSH: relative steps move and clamp the cursor; a click opens a Home tile and a link in a web app;
   scrolling moves a web page, LG Search's results and the guide; text and Enter work in LG Search, a web app's field, Marquee's search and a LumaTV Settings
   entry; held arrows repeat as the remote's do; Home, Back and gear taps route as the remote's; the journal holds no typed text; the frame gate (`tv-gate.sh`)
   is unchanged.
3. **Helper plugin:** off, nothing listens on 18733 (`netstat -ln`) and no mDNS record exists; on, it listens on 192.168.1.242 only; `lumaremote pair` from
   BeastNAS works after the remote's OK; 5 wrong codes close the window; Remove ends the client's session at once; bad TCP and UDP get no bytes back; every
   action works, volume through the receiver and power off then on from Active Standby included.
4. **Kit and app:** CI green, IPA sideloaded. Nitin pairs by QR code, points, scrolls, types into each kind of field, uses the actions, and wakes the TV from
   Active Standby and from deep standby.
5. **Latency:** `lumaremote bench` (wired), the app's Diagnostics (Wi-Fi) and `ctl phone stats`; accepted at p95 under 20 ms; the video recorded.
6. **Letters in the keymap**, if Nitin agrees: letters through `ctl key` first, then phone typing into YouTube, Netflix and Prime Video.
7. **Marquee:** the embedding, in the Marquee session.

## Decisions (Nitin, 2026-11-06)

1. **Pairing gate:** opening pairing needs OK on the physical remote.
2. **Air mouse mode:** decided in the Marquee app design. The pointer lives on a separate remote screen in the app; the kit offers both hold-to-point and
   always-on, and the Marquee session picks the behaviour.
3. **Letters in the served keymap:** yes. The phone types into YouTube, Netflix and Prime Video; USB keyboards type there too.
4. **No standalone app:** LumaTV control is part of the Marquee iOS app. `LumaRemoteKit` stays a separate Swift package that Marquee embeds; the thin
   `LumaRemote` app above is dropped, and Marquee owns the `lumaremote://` pairing link and the QR scan.
5. **Install route:** sideloaded like Stashy (free, re-signed every 7 days, no broadcast Wake-on-LAN; unicast wake is the fallback to test).
6. **Volume buttons:** yes, while the remote screen is open.
7. **Keep it separable:** the remote must be easy to lift out of Marquee into its own app later. `LumaRemoteKit` owns everything remote-specific: the
   protocol, pairing and stored keys, discovery, wake, the air mouse, the trackpad, the keyboard bridge, and the remote screen's SwiftUI views (`LumaRemoteView`,
   `PairingView`, `TVPicker`). It takes its look from the host's tint and has no Marquee imports. Marquee reaches it only through a small host interface: the
   tab that shows `LumaRemoteView`, the URL scheme hand-off, and optional hooks such as "play this on the TV". Paired TVs live in the kit's own Keychain group
   and settings namespace, not in Marquee's. A standalone app is then a thin shell around the same views, and the kit keeps its own CI with
   `swift test`.

Untested: all of it. Inferred, not measured: every latency figure; whether Chromium, Qt, Flutter and SDL apps honour version 4 `wl_pointer.axis`; text from the
phone under LG's keyboard; YouTube, Netflix and Prime Video accepting one key per character; unicast Wake-on-LAN from iOS.
