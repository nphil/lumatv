# App audio on the format badge

LG's sound services see only what an app hands the TV, so an app that decodes its own sound (Marquee, a native Plex client) says what it plays and what it delivers. The format badge (top right) shows it next to the size and range.

## Sending

Loopback only (`127.0.0.1:18731`), with `X-Luma-Client` and `Content-Type: application/json` as every write.

`POST /v1/media/audio` answers 204:

```json
{"app": "org.nphil.marquee", "context": "5f43…", "codec": "truehd", "profile": null,
 "atmos": true, "channels": 8, "layout": "7.1", "delivered": "source"}
```

| Field | Value |
|---|---|
| `app` | required; the app's id, as the `foreground` topic names it |
| `context` | videooutputd's context of the picture the sound belongs to, or null for whichever picture the app has in front |
| `codec` | required; `ac3 eac3 truehd dts aac pcm opus flac mp3 other`, lowercase |
| `profile` | null, or for DTS `dts-hd ma`, `dts-hd hra`, `dts:x`, … (matched case-insensitively) |
| `atmos` | true when the delivered stream carries Atmos; false if left out |
| `channels`, `layout` | the delivered channel count (1 to 32) and its name, or null |
| `delivered` | required; `source`, `server_converted` or `app_stereo_mix` |

A bad or missing field answers 400 with the field named in the message. The helper keeps the latest entry per app (at most 4 apps; a new one past that replaces the entry posted longest ago). An entry ends 75 s after its last POST, so a sender repeats it every 30 s and when it changes. `DELETE /v1/media/audio?app=<id>` clears an app's entry when its playback stops.

## Topic

`media.audio`: `{"entries": [{app, context, codec, profile, atmos, channels, layout, delivered}]}`, published on every change and every expiry. No time is in it, so a repeat of the same entry raises no event.

## Badge

An entry is used only while its `app` is the foreground app and, when `context` is not null, the picture the badge shows is that context. HDMI inputs keep the sound `utp.extinputs` reports. While an entry applies, LG's own sound announcements do not change the sound marks (the ranges still follow them).

| Entry | Marks |
|---|---|
| `truehd` or `eac3` with `atmos` | Atmos |
| `truehd` | Dolby TrueHD |
| `eac3` | Dolby Digital Plus |
| `ac3` | Dolby Digital |
| `dts` with `dts:x` in the profile | DTS:X |
| `dts` with a profile starting `dts-hd` | DTS-HD |
| `dts` | DTS |
| `pcm` | PCM |
| `aac opus flac mp3 other` | no sound mark |
| `channels` 6, 8 | 5.1, 7.1 |

The marks stay on one row. When the row would pass half the width of the badge's zone, the speakers go first, then a Dolby Digital family sound takes the Dolby Audio mark. Atmos and DTS-HD are already the shortest marks of their formats.
