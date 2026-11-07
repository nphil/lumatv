# Live TV Tuner

The Live TV Tuner plugin shares the TV's antenna tuner on the home network as a one-tuner HDHomeRun, with an M3U playlist and an XMLTV
guide made from the TV's own broadcast listings. It is off until the owner switches it on in Settings › Inputs & devices › Live TV Tuner,
which asks first in the plugin's own words ("Shares this TV's antenna tuner on the local network…").

It is a plugin of the helper's own, not a program of its own: tuning, the demux tap and the PVR download's certificate need root, LG's
luna services and LG's HAL, which a sandboxed plugin process cannot be given. It asks first like a plugin on the socket: it is listed
off with its consent words, and the owner's approval of exactly those words is kept in `plugins/enabled.json` (`firstParty`).

## Endpoints

On the TV's private address, port 5004 (`hdhr` in `helper.json`: `port`, `bind`, `allowedNets`). Only the allowed networks are answered
(the private ranges by default); any other peer is dropped unanswered.

| Request | Answer |
|---|---|
| `GET /discover.json` | the device: name, id, `TunerCount` 1, `BaseURL`, `LineupURL` |
| `GET /lineup.json`, `/lineup_status.json` | the TV's antenna channels, one per number, and an antenna that is not scanning |
| `GET /lineup.m3u` | the same lineup as an M3U playlist; `url-tvg` names the guide |
| `GET /status.json` | who has the tuner, without tuning or asking the TV: `{"holder": {"ip", "name", "channel", "channelName", "since"}}` (`name` is `Plex` for the signed-in Plex server's address or a Plex user agent, else the user agent's product; `since` in Unix ms), or `{"holder": null}` while no client's stream has it |
| `GET /xmltv.xml` | the TV's own guide, a day back to six days ahead, as XMLTV; with the Guide data plugin on, the online listings laid over it ([guide-data.md](guide-data.md)) |
| `GET /auto/v<number>` | MPEG-TS of one channel (`/auto/v2.4`), starting with the program's PAT and PMT. With the tuner's messages on (the default) it is answered at once and opens with a slate while the channel tunes; otherwise it is answered once the channel's first whole picture has come, and starts with that picture |

A request the tuner cannot take is refused with the HDHomeRun error a player expects: 503 with `805 All Tuners In Use` (another
client's stream, or the TV's own Live TV), 404 with `801 Unknown Channel`. With the messages on, a tuner in use is answered with 200
and 6 s of a slate in the channel's shape instead of 503, then the end: `Tuner busy: Plex is recording <programme> on <channel>` (the
TV's guide title when it has one), `Tuner busy: in use by <client>`, or `The TV is using its tuner`. Without the messages, a channel that cannot start is refused too, with
`807 No Video Data` and the reason saying which: "no signal" when it does not lock within 3 s; "weak signal" when the multiplex locks
but its tables do not come whole within 8 s, its picture (an MPEG-2 sequence header, an H.264 SPS or IDR) does not within 5 s, or the
demodulator counts packet errors on it; "not on air" only when the multiplex is clean and its whole tables do not list the channel, or
it carries none of the channel's picture. Nothing of a refusal is kept but the channel's reception: the next request tunes again.
Other refusals name their HDHomeRun error too (806 tune failed, 811 protected). The helper logs a refusal once a second per client,
channel and reason, with how many more came (VLC asks again about 20 times a second while the tuner is busy).

## Messages in the stream

With the messages on, what the tuner is doing is a picture in the stream itself (a slate), so a player shows it instead of a spinner,
a frozen picture or Plex's Error 16, and a weak channel's stream outlasts a fade:

| When | The slate says | Then |
|---|---|---|
| the channel tunes | `Tuning 16.9 WXYZ…`, with a bar: locked, tables found, picture waited for | the channel, from its first whole picture |
| no picture after 8 s, the picture stops for 1.5 s, or a second loses a fifth of its packets | `Weak signal on 16.9`, how long it has waited, and the signal as the TV's demodulator reads it | the channel again at its next whole picture, once a second loses at most a twentieth |
| no picture 45 s after the request or 2 min after it stopped; no lock; the channel not on the multiplex | `No signal on 16.9`, for 6 s | the end |
| the TV's own Live TV takes the tuner | `The TV is using its tuner`, for 6 s (the TV has it at once) | the end |

A slate is MPEG-2 in the channel's own size, rate and shape, as its last stream or *Check every channel* saw it (kept in
`tuner/channels.json`). A channel not seen yet sends nothing until its first sequence header gives the shape (at most 4 s after the
request, then 1920x1080 interlaced): Plex restarts its transcode ("Error reinitializing filters") when the size changes within a stream.
A slate is an I picture twice a second with skipped pictures between, with silent AC-3 on each of the channel's sounds, drawn in the
active theme's colours and type. The channel's own packets go on in the same program: its PIDs moved onto the program's (0x101 the
picture, 0x102 on the sounds), its times onto the slate's clock, its counters carried on. Plex's transcoder drops a stream that appears
after the start and passes a time jump through, which is why (`docs/research/tuner-slates.md`). A channel whose picture is not MPEG-2
(H.264, HEVC) is served without slates, from its first IDR picture: at once when it was not seen yet (nothing has gone out while its
shape was waited for), and from the start of every stream once its type is known.

## Players

| Player | What it is given |
|---|---|
| Plex (Live TV & DVR), Jellyfin, Emby, Channels DVR | the HDHomeRun: `http://<the TV's address>:5004`. Plex's Transcoder Quality is "Original format": the tuner sends the broadcast as is. |
| VLC, TiviMate, Kodi (IPTV Simple Client) | the playlist `http://<the TV's address>:5004/lineup.m3u` and the guide `http://<the TV's address>:5004/xmltv.xml` |

## One tuner

The TV has one tuner, so one stream runs at a time and the TV's own Live TV comes first: a stream does not start while Live TV has the
tuner, and a running one ends when Live TV takes it. The client whose stream has the tuner takes it over with its next request (a channel
change); another client is refused (503) while a stream runs.

The tuner answers while the TV is on, or off in Always Ready (Active Standby). With Always Ready off, a TV switched off sleeps and nothing
answers; the plugin's page says so.

## Settings

The page has the switch, where the tuner stands (idle, streaming a channel, busy while the TV's own Live TV has it, checking reception
or measuring a channel for the meter), the three addresses to give a player, the networks it answers, the name players show and the
switch for the messages in the stream. Turning it off asks nothing.

Its antenna meter reads the TV's demodulator about once a second while the page is open, of an RF channel picked (the helper tunes its
first channel) or of the one the tuner is on: a bar of the signal to noise from 16 to 30 dB in the verdict's colour, a tick at the best
reading since the channel was picked, and the lock, MSE, dB, packet errors and BER (n/a when the TV's 24-bit count is full). Good is no
packet errors at 23 dB or more, Fair a few or less margin, Poor 20 or more packet errors a reading or under 19 dB. The meter tunes only a
free tuner and gives it up at once to a stream, a reception check or the TV's own Live TV, then reads the channel they tuned and says
why; it lets go when the page closes, and 5 s after the page last asked for a reading. The page asks the helper for it on loopback
(`GET`/`POST /v1/tuner/meter`, `POST /v1/tuner/meter/best` and `/stop`, the TV's Settings only).

Its channel list has every antenna channel under its RF channel (the channels that share a signal come in alike), each saying how it
came in the last time it was watched or checked: good, some glitches, weak or no signal, with the signal in dB, the share of the
picture lost and the dropouts. A channel switched off there is left out of the lineup, the playlist and the guide players see. *Check
every channel* tunes each RF channel in turn for a few seconds and listens for each channel's MPEG-2, H.264 or HEVC picture; a stream
asked for meanwhile comes first. The list, the channels left out and each channel's picture type and shape are kept in
`tuner/channels.json`.

## Standalone app

The engine is the `lumatuner` crate (`helper/tuner`), with no LumaTV host in it. A homebrew app for a stock rooted LG TV runs it with:

- a root service started by the Homebrew Channel: the DVB nodes (`/dev/dvb/adapter0`, demux channel 2) and LG's HAL need root;
- luna access: `webos::LunaSend` runs the set's `luna-send`, and the set's 32-bit `python3` reads the PVR seed key from LG's HAL;
- a config file: the name players show, the port and the address or interface to bind, the allowed networks, where the TV's hardware
  address is read (the device id players keep), and the tuner's nodes;
- a logger for the `log` facade (target `lumatuner`), and its own on and off: it is a network entrance.

```rust
let tv = webos::Tv::new(webos::WebosConfig::default(), Arc::new(webos::LunaSend::new("luna-send")));
let tuner = Tuner::new(Arc::new(tv), real_clock());
tuner.serve(&Config { name, device_id: docs::device_id(&mac), addr: "192.168.1.242:5004".parse()?, allowed })?;
```
