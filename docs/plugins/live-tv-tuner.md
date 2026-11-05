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
| `GET /xmltv.xml` | the TV's own guide, a day back to six days ahead, as XMLTV |
| `GET /auto/v<number>` | MPEG-TS of one channel (`/auto/v2.4`) |

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

The page has the switch, where the tuner stands (idle, streaming a channel, or busy while the TV's own Live TV has it), the three addresses
to give a player, the networks it answers and the name players show. Turning it off asks nothing.

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
