# Phone remote

The phone remote plugin lets paired phones drive the TV over the home network: a pointer, the remote's keys, typing into text boxes and
LumaTV's actions. It is off until the owner switches it on in Settings › Remote & pointer › Phone remote, which asks first in the
plugin's own words. Off closes the listener, withdraws the mDNS record and ends every session; the pairings are kept.

It is a plugin of the helper's own (`helper/app/src/phone_remote/`), because it drives the compositor's control socket. The engine is the
crate `lumaremote` (`helper/remote`), which has no LumaTV dependencies. The protocol, its reasoning and the client's side are in
[`docs/research/phone-remote.md`](../research/phone-remote.md) ("Protocol, version 1"); this page records what the TV implements.

## Pairing

"Pair a phone" in Settings asks on the TV first: the window opens only after OK on the physical remote (the AI gate's prompt, kind
`pair_phone`). The page then shows the code (8 characters, shown as `ABCD-EFGH`) and a QR code of
`lumaremote://pair?h=<address>&p=18733&id=<tv id>&c=<code>`. The window lasts 5 minutes, takes 5 wrong codes, and closes once a phone
pairs. At most 16 phones are paired; forgetting one ends its session and sends it `revoked`.

## Helper routes and topic

| Route | |
|---|---|
| `GET /v1/phone-remote` | the state, as the topic has it |
| `POST /v1/phone-remote/pairing` | `202` with a prompt on the TV; the code shows in the state after OK |
| `DELETE /v1/phone-remote/pairing` | closes the window |
| `DELETE /v1/phone-remote/devices/{id}` | forgets one phone |
| `DELETE /v1/phone-remote/devices` | forgets every phone |

The writes are the TV's Settings' only: without `X-Luma-Client: lumatv-ui` they are refused `403`. The topic `phone-remote` is
`{on, problem, pairing: {code, qr, expires} | null, devices: [{id, name, model, last, live}]}`, with `expires` and `last` in Unix ms.

## The listener

TCP and UDP port 18733 on the TV's private address only, announced as `_lumatv-remote._tcp.local` with TXT `v`, `id`, `name`, `mac` and
`pair`. The TV's name is LG's `network.deviceName`. A phone's session sends commands to the compositor only while the plugin is on and that
session is live; the compositor ends its phone session when the last one goes.

What a client sees, beyond the design:

- `key` `tap` and `hold` are timed on the TV: a hold is 1.1 s, 2.1 s for `mute`.
- An address is ignored while it has 8 failed handshakes within the last minute.
- The pointer: motion moves an absolute pointer the TV keeps on its 1920 × 1080 scene; vertical scroll turns the wheel one step per
  48 px; horizontal scroll is ignored. The left button clicks; the right button is Back. Clearing engaged hides the pointer.
- `fg` is the webOS id of the app in front.
- `ime` is active only for a field LumaTV's keyboard serves, which is where `text` and `edit` go.
- `volume` is `{"via": "tv"}` while the receiver plugin is off; with it on, the receiver's level, maximum and mute as it reports them.
- `camera` (one camera's popup) is not implemented: it is never in `acts` and is refused with `unavailable`.

## Compositor verbs

On the helper's persistent control connection: `phone session on|off` (a session is live; `phone text` and `phone edit` are refused
without one), `phone text BASE64`, `phone edit OP [N]`, `pointer X Y`, `click tap|press|release`, `wheel up|down` and
`remote CODE tap|press|release`. State is read with `keys` (the app in front), `ime` and `pointer`.
