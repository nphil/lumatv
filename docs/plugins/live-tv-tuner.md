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
| `GET /lineup.json`, `/lineup_status.json` | the TV's antenna channels, one per number (with `interlaced`, `height` and `fps` once a channel has been seen on air, below), and an antenna that is not scanning |
| `GET /lineup.m3u` | the same lineup as an M3U playlist; `url-tvg` names the guide |
| `GET /status.json` | network viewers, recently displaced streams, the channels that can share the tuner, the converter and the consent version; no tune or TV query |
| `GET /xmltv.xml` | the TV's own guide, a day back to six days ahead, as XMLTV; with the Guide data plugin on, the online listings laid over it ([guide-data.md](guide-data.md)) |
| `GET /auto/v<number>` | MPEG-TS of one channel (`/auto/v2.4`), starting with the program's PAT and PMT. With the tuner's messages on (the default) it is answered at once and opens with a slate while the channel tunes; otherwise it is answered once the channel's first whole picture has come, and starts with that picture |
| `GET /auto/v<number>?transcode=<profile>` | the same channel as H.264 from the TV's own video hardware, when the owner has switched that on (*H.264 on request*, below) |

A conflicting plain GET, including an unconfirmed Plex retry, receives `503 Service Unavailable`, `X-HDHomeRun-Error: 805 All Tuners In Use`,
`Content-Type: text/plain`, `Cache-Control: no-store`, and the exact body `All Tuners In Use`. It does not change the tune, the playing
streams, or the takeover tickets. It does not send a slate or prompt the TV. What a stock player displays or whether it retries is untested.

Unknown channels receive 404 with `801 Unknown Channel`. Without messages, a channel that cannot start receives `807 No Video Data`:
"no signal" when it does not lock within 3 s; "weak signal" when the tables do not come whole within 8 s, the picture does not within
5 s, or the demodulator counts packet errors; "not on air" when the multiplex is clean but has no such program or picture. Other
refusals also name their HDHomeRun error (806 tune failed, 811 protected).

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

## H.264 on request

A player that adds `?transcode=<profile>` to a channel's address (the HDHomeRun's EXTEND request, which Marquee's phone apps send) is
given H.264 in place of the station's MPEG-2, made by the TV's own video decoder and encoder. The converter is off until the owner
switches on *Make H.264 for players that ask* on the plugin's page: with it off every stream is as the station sends it.

| Profile | Picture | Video |
|---|---|---|
| `heavy` | progressive at the station's own size and rate: 1920x1080 at 29.97 from 1080i, 1280x720 at 59.94 from 720p60, the SD size at 29.97 from 480i | 9 Mbit/s at 1080, 7.5 at 720p60, 5 at 720p30, 2.5 below |
| `mobile` | progressive, at most 1280x720 (960x720 for a 4:3 station) and 30 frames a second | 3.5 Mbit/s |
| `internet540`, `internet480`, `internet360`, `internet240` | progressive, at most 30 frames a second, within 960x540, 848x480, 640x360 or 432x240 (720x540, 640x480, 480x360 or 320x240 for 4:3) | 2, 1.2, 0.8 or 0.4 Mbit/s |

Every profile is progressive: the TV's decoder hands over whole pictures (it deinterlaces in hardware), so a 1080i or 480i station comes out
at 29.97 frames a second. `heavy` keeps a 720p60 station's 59.94; the others make it 29.97. A picture whose pixels are not square (704x480
or 720x480 in 16:9 or 4:3, 1440x1080) is made at its own size and carries its pixel shape in the H.264 stream (40:33 for 704x480 in 16:9),
so a player draws it in the station's shape; a shape within 2 % of square carries none. The stream is H.264 High with a key frame every
2 s and the station's AC-3 sound passed through. A picture is never enlarged. With the setting on, a name not in the table is refused with
400 and `802 Unknown Transcode Profile`. The first picture comes about 1.5 s after the request (3 s at most) and no slate is sent: the
messages in the stream are MPEG-2. `mobile` and `heavy` are measured on the set (1080i, 720p60, 480i, 4:3 and 16:9 SD); the `internet` profiles are tested on the host only. The video chain needs about 30 MB of contiguous memory (CMA, `CmaFree`); when the apps kept ready hold it, the converter closes them for the stream (last in the Keep apps ready order first; left open at 3-7 MB free, one start in two stalled for 1.6 s on its first picture) and, when that cannot make the room, serves the station as it is sent.

The video hardware is the TV's own, so the converter gives way. It converts while Home or LumaTV is in front, or a native game (LumaGame,
Punktfunk) is in front with its own stream up, and the media server's pipelines leave it one video decoder, one MPEG-2 core and the
encoder (the TV counts four video decoders, four MPEG-2 cores and one encoder: Netflix in 4K holds all four decoders, a 1080p player two).
Any other app in front or starting ends it. When an app takes the hardware the stream ends within a second and the page says `Paused`
and why (`netflix is in front`, `org.nphil.lumagame holds the video decoders (4 of 4)`). A player reconnects when a stream ends, and a
request that comes while the hardware is taken is served as the station sends it. Beside a game's stream the chain also needs its memory
(CMA) free, and the kept apps are closed already in game mode, so a request the memory cannot hold is served as the station sends it;
this is not yet measured on the set. Also:

- a station that is H.264 already is sent as the station sends it: the converter makes H.264 from MPEG-2 only;
- a converter that cannot start, or stalls, leaves the request as MPEG-2 and rests for a minute;
- one that fails in mid-stream ends the stream, and the player's next request is MPEG-2;
- one whose hardware call does not return stops until the helper restarts, and the page says so.

The converter is a program the helper carries and writes to `/tmp/lumatv-xcode/` only while the setting is on. The engine that shares
the TV's one encoder is the `lumaxcode` crate (`helper/xcode`), with no LumaTV code in it; `docs/research/hw-transcode.md` has the
measurements.

## Players

| Player | What it is given |
|---|---|
| Plex (Live TV & DVR), Jellyfin, Emby, Channels DVR | the HDHomeRun: `http://<the TV's address>:5004`. Plex's Transcoder Quality is "Original format": the tuner sends the broadcast as is. |
| VLC, TiviMate, Kodi (IPTV Simple Client) | the playlist `http://<the TV's address>:5004/lineup.m3u` and the guide `http://<the TV's address>:5004/xmltv.xml` |

## One tuner

The tuner locks one RF channel (frequency). Requests for stations on that frequency share the capture, up to four viewers
(`hdhr.viewers`), each with its own stream. The tuner is let go when the last viewer leaves. A different frequency needs explicit
requester confirmation before any network viewer is displaced. The TV's native Live TV has no takeover ticket and can still
claim the hardware. `hdhr.shareLiveTv` allows matching frequencies to share with native Live TV; it is off by default and untested.

### Manual admission (`luma-tuner-consent/1`)

`/status.json` advertises `"consent": {"version": 1, "takeoverMs": 30000, "staleMs": 6000}`. `hdhr.staleSecs` sets the socket's
send timeout (1–9 seconds; default 6). A stalled send ends the stream; waiting for a silent broadcast does not count as a stalled send.

`GET /auto/v<number>?consent=1&device=<name>` opts into conflict details. The optional `transcode` parameter is unchanged. A
conflict receives the same status and headers as the plain GET, except `Content-Type: application/json`, with this shape:

```json
{"version":1,"error":"takeover_required","takeover":"<opaque>","holders":[{"stream":"s0","name":"iPad","kind":"app","channel":"2.4","channelName":"WEST","since":1794000000000,"rf":32}]}
```

After explicit confirmation, the requester repeats the same tune with `&takeover=<opaque>`. The ticket expires after 30 s,
is single-use, and ties the requester (peer, device, profile, and any stream key) and exact target to the same set of leases.
A holder leaving, joining or reconnecting invalidates outstanding tickets. A stale or incorrect ticket does not allow displacement;
a fresh offer requires fresh confirmation. `tuner_busy` with `takeover: null` means confirmation is unavailable, including a full
same-frequency tuner or native TV use. The latter holder has `kind: "tv"` and null fields where the TV cannot supply a value.

Each successful stream returns a public `X-Luma-Stream` and a private `X-Luma-Stream-Key`. Only `Authorization: Bearer <key>`
proves that a request from the same peer may replace that exact stream on the same frequency. A public stream ID, IP address,
device label or User-Agent never acts as proof. A channel on another frequency still needs a ticket. Both secrets are for
short-lived use on the private LAN, not for sharing, persistence or logging. Only the takeover ticket goes in the query.

`viewers` in `/status.json` lists the `holders` fields shown above plus `ip` and `shared`; `holder` is the first network viewer,
or null. `sameRf` groups channels by `rf` when at least two share it. `since` and `at` are Unix milliseconds. A `device` query
or `X-Luma-Device` header (query first; URL-decoded, at most 40 characters) supplies the name, otherwise the host's known name,
User-Agent product or address is used. `kind` is `app` with a device label, `client` without, and `plex` at the paired Plex
server's address. The latter identifies the server, not why it is tuning.

`displaced` keeps up to 32 entries for 60 s: `{stream, ip, name, by, byKind, at, channel, channelName}`. Players match the public
`stream` ID, not a name or IP. An admitted stream that is displaced ends; with messages it sends a final slate naming the taker,
or `The TV is using its tuner` if Live TV took it.

Stock Plex does not identify the exact recording or airing in its tuner request. Viewer events, DVR-specific TV prompts,
automatic priority when a prompt expires, and skipping an airing are unavailable. Manual admission never changes a Plex booking.

`/status.json` also says what the converter makes: `"transcode": {"enabled", "profiles": [{"name", "codec", "maxHeight", "progressive", "kbps", "interlacedFps"}]}`.
`enabled` mirrors *Make H.264 for players that ask*. `profiles` lists the profiles measured on the set, best picture first, and is empty while
the setting is off; a profile left out is still made when a request names it. `codec` is `h264`; `maxHeight` is the tallest picture in lines,
`null` where the station's own size is kept; `progressive` is true for every profile; `kbps` is the video rate at the largest picture;
`interlacedFps` is the frames a second the profile makes of an interlaced station (1080i, 480i): 30 (29.97) for every profile today, `null` for a
profile that keeps the field rate (59.94). Now:
`[{"name": "mobile", "codec": "h264", "maxHeight": 720, "progressive": true, "kbps": 3500, "interlacedFps": 30}]`.

`GET /lineup.json` lists, for a channel whose MPEG-2 picture the tuner has seen on air, `"interlaced"` (true for 1080i and 480i, false for 720p),
`"height"` in lines and `"fps"` in frames a second (29.97 for 1080i, whose fields come at 59.94; 59.94 for 720p60), read from the picture's
sequence header and kept across restarts. A channel not tuned or checked yet, and one whose picture is H.264, lists none of the three: a client
takes it as interlaced. A client that prefers smooth motion takes the original stream for interlaced channels and a profile whose
`interlacedFps` is null or ≥ 50 otherwise.

The tuner answers while the TV is on with any app in front, or off in Always Ready (Active Standby). A stream takes only the tuner and
the section decoder: the app in front keeps its picture and sound (Netflix in 4K, which holds every video decoder, too), and the TV's
last channel stays as it was. With Always Ready off, a TV switched off sleeps and nothing answers; the plugin's page says so.
A conversion is the exception: it also takes a video decoder and the encoder, and gives them up as soon as an app needs them.

## Channel change

A stream is on its way as soon as the channel is tuned, from what the last stream of the channel found:

- Each channel's PAT/PMT (as the program's own PMT section, with its multiplex id, program number, access flag and PMT PID) is kept in
  `tuner/channels.json` (a PMT is kept only up to 1 KiB, so the file stays about the lineup's size) and learned for every channel of a
  multiplex by the reception check. A tune routes the channel's PIDs from them at once, without waiting for any table.
- The live virtual channel table, PAT and PMT are asked for meanwhile, one section filter at a time, and compared with the cache. A
  program that moved before the first byte is routed again and goes out as the live tables say; one that moved later is corrected for
  the next request and the stream is not broken. A cached program that brings no picture is forgotten and the tune is read the long way.
- The pipeline a stream leaves is kept for 1 s with its demux, and a channel on the same RF channel is read on it without playing it
  again or waiting for the lock (a lock query only). A channel on another RF channel is played on the same pipeline.

The rest of a channel change is the station's: the stream starts at the first whole key frame (its sequence header and the I picture
with it), gathered from the moment the PIDs are routed, and that is as soon as a decoder can show a picture. Sending from a partial
GOP gives a player pictures it cannot decode; it would only show grey or green. A player should show the first picture it decodes
(a small `probesize` and `analyzeduration`, or `avformat_find_stream_info` bounded to what the first key frame holds) and open again
when a stream ends.

Examined and not done: reading the lock from `/proc` (the luna query it replaces takes 15 to 25 ms), caching the TV's own use of the
tuner (the check is what keeps Live TV first), a luna connection kept open (not measured).

## Settings

The page has the switch, where the tuner stands (idle, streaming a channel, busy while the TV's own Live TV has it, checking reception
or measuring a channel for the meter), the three addresses to give a player, the networks it answers, the name players show and the
switch for the messages in the stream, and the switch for H.264 (*Make H.264 for players that ask*, off) with a line that says whether the
converter is ready, converting, paused for an app or not available. Turning it off asks nothing.

Home's top bar shows the tuner's use as glyphs, from the helper's `tuner` topic (`{streaming, clients, converting}`, published on every change and nothing more while idle): an antenna while a player's stream has the tuner, a chip beside it while the converter is making that stream's H.264. They are drawn at 70 % of the bar's ink, only while so, and go with the bar when it hides. Settings › Home › Tuner glyphs puts them at the bar's right end (default) or left end, or off (`layout.statusBar.tuner`, a profile's own key).

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
- a file for the open broadcast pipeline's id (`WebosConfig::pipeline_file`, in the temp directory by default): LG keeps a pipeline
  after the program that opened it ends, so the next tune closes the one a run left open;
- a config file: the name players show, the port and the address or interface to bind, the allowed networks, where the TV's hardware
  address is read (the device id players keep), and the tuner's nodes;
- a logger for the `log` facade (target `lumatuner`), and its own on and off: it is a network entrance.

```rust
let tv = webos::Tv::new(webos::WebosConfig::default(), Arc::new(webos::LunaSend::new("luna-send")));
let tuner = Tuner::new(Arc::new(tv), real_clock());
tuner.serve(&Config { name, device_id: docs::device_id(&mac), addr: "192.168.1.242:5004".parse()?, allowed })?;
```
