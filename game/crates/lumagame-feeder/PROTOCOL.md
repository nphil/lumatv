# Feeder protocol 2

The 32-bit app `main` owns LG's player. Engines connect to its Unix stream socket (`/tmp/org.nphil.lumagame/feed.sock`; the old GFN app uses `/tmp/lumatv-gfn/feed.sock`). One connection owns the player; another receives ERROR 3 and closes. Disconnect unloads. No TV libraries are linked into an engine. GPL engines implement this protocol in their own process; permissive Rust engines use `lumagame-feeder`.

The authoritative layouts are [`gf_proto.h`](../../../../gfn/feeder/src/gf_proto.h). All integers are little-endian, signed where marked `i`, with no implicit wire padding. Every frame starts with `u8 type, u8 flags=0, u16 reserved=0, u32 payload_length`, followed by exactly that many bytes. Maximum payload: 8 MiB. Text is UTF-8 without a trailing NUL, except HELLO's two fixed NUL-padded strings. Readers must handle partial frames and ignore unknown types.

## Engine → feeder

| Type | Payload |
|---|---|
| LOAD 1 | 100 bytes: `u32 codec,width,height,fps,audio,channels,sample_rate,hdr`; 60-byte HDR metadata; `i32 drop_threshold,pts_unit` |
| VIDEO 2, AUDIO 3 | `i64 pts_ns, u8 key, u8 zero[7]`, then one whole encoded unit/packet |
| FLUSH 4 | Empty; reports loss, **not** a request to flush the decoder |
| UNLOAD 5 | Empty |
| STATE 6 | `u8`: 1 foreground, 0 background |
| HDR 7 | 60-byte HDR metadata |
| PING 8 | `i64 echo` |
| APP 9 | App-id text for the next load; empty restores the window's app id |
| TEXTURE 10 | `u32 width,height`; nonzero selects CPU decoding into a camera texture, zero restores the player |
| PIPE 11 | `ndl`, `es`, or `smp`, for the next load |
| SINK 12 | Empty for the main video window; `sub_video` for Starfish's secondary window |
| LOAD_OPUS51 13 | 108 bytes: the 100-byte LOAD, then `u8 streams,coupled,mapping[6]` in Vorbis channel order |

Codec: H.264=1, H.265=2, AV1=4. H.264/H.265 units use Annex B start codes; AV1 uses a low-overhead temporal unit beginning with its temporal delimiter. Audio: none=0, Opus=1, interleaved signed 16-bit little-endian PCM=2. Ordinary LOAD uses mono or stereo Opus. LOAD_OPUS51 requires six-channel Opus at 48000 Hz, 1–6 streams, coupled no greater than streams, streams plus coupled equal to six, and a mapping that is a permutation of 0–5. It requires NDL and compressed audio; it never falls back to PCM. `key=1` marks a decoder-safe keyframe or recovery anchor. `drop_threshold=-1` leaves LG's default. `pts_unit` is a legacy field; send 1000000. Every NDL load uses milliseconds regardless of that field.

HDR is 15 `u32`s: green x/y, blue x/y, red x/y, white x/y, maximum/minimum mastering luminance, maximum content light level, maximum frame-average light level, transfer characteristics, colour primaries, matrix coefficients. Chromaticity units are 1/50000; mastering luminance units are 1/10000 nit. `Hdr::hdr10(1000,400)` supplies BT.2020/PQ defaults. SDR loads set `hdr=0`; HDR messages cannot turn them into HDR sessions.

## Feeder → engine

| Type | Payload |
|---|---|
| LOADED 16 | `i32 ok`, then window-id text on success or error text on failure |
| EVENT 17 | `i32 kind, i32 zero, i64 number`, then diagnostic text |
| ACCEPT 18 | `i64 original_pts_ns, i32 queued_frames, i32 full_buffer_wait_us` |
| DROP 19 | `i64 original_pts_ns, i32 reason, i32 zero` |
| STATUS 20 | 104-byte `gf_status` counters, once a second; counters reset each interval |
| ERROR 21 | `i32 code`, then error text |
| PONG 22 | `i64 echo, i64 feeder_CLOCK_MONOTONIC_ns` |
| HELLO 23 | First frame: `u32 version,library_available,pid,owns_window`, `char window_id[32],library_name[64]` |
| KEYFRAME 24 | Empty; request a decoder-safe recovery anchor |

DROP reasons: buffer full=1, no pipeline=2, refused unit=3, held for an anchor=4, fatal player failure=5. ACCEPT means submitted, not displayed. NDL exposes no on-glass timestamp. Event 22 is LOADCOMPLETED; 26 is PLAYING; 18 is a player error. ERROR 1 is malformed input, 2 a fatal player/load failure, 3 an occupied player.

## Load, clocks and recovery

1. Read HELLO and require version 2. Send LOAD and check LOADED. `ok=1` means the load call was accepted, **not** that loading finished. Feed video immediately: some TVs report LOADCOMPLETED only after ingesting video.
2. VIDEO timestamps are absolute `CLOCK_MONOTONIC` nanoseconds on the TV, optionally mapped from the source clock by the engine. The feeder subtracts its exact load-call timestamp, rounds up to milliseconds, and clamps each plane to its last timestamp. ACCEPT/DROP echo the original number unchanged. AUDIO's supplied timestamp is not used for presentation: audio uses the feeder's clock since load.
3. The feeder loads and primes an Opus plane even for video-only NDL sessions. Silent packets carry the load window; real audio packets before event 22 are discarded. Priming stops on that event. The old GFN feeder retains its explicit PCM fallback for TVs that reject Opus.
4. Loss or buffer rejection holds dependent video and sends one KEYFRAME per hold. FLUSH never calls NDL's flush: that destroys session audio. Submit the next safe anchor with `key=1`; subsequent video resumes. A real software-decode error may flush the texture decoder.
5. HDR is applied after the first accepted video unit, then only when changed. It is never sent to LG for SDR.
6. Eight seconds without LOADCOMPLETED, or a fatal player event, latches failure. Exactly one ERROR 2 is emitted; subsequent video gets DROP 5 and audio is refused. The engine must end the session as failed (`video`), then disconnect or UNLOAD. Only a new LOAD clears the latch.

## App lifecycle channel

The UI inherits descriptor 3, a separate newline-delimited JSON channel. Requests: `{"kind":"launch","seq":1,"app_id":"com.example.game","parameters":{}}` or `{"kind":"apps","seq":2}`. Sequences are unsigned 64-bit integers. Launch parameters are an object under 1024 bytes; input lines are under 2048 bytes.

Replies: `{"kind":"launched","seq":1,"app_id":"com.example.game","success":true,"message":""}` and `{"kind":"apps","seq":2,"success":true,"apps":[{"id":"com.example.game","title":"Game","icon":"icon.png"}]}`. Each app of the list carries `id`, `title` and, when it has one, `icon`, as the application manager wrote them, whatever their length; an app without an id or title is left out. Failures set `success:false` and include `message`; failed lists carry `apps:[]`, as does a list that is not an array of apps or whose reply would not fit the line bound. The native LS2 handle calls `applicationManager/launch` with `id` and `params`, and `listApps`. Foreground subscription updates are `{"kind":"foreground","app_id":"org.nphil.lumagame"}`. Reply lines are bounded at 128 KiB. Registration, relaunch and close messages retain the existing lifecycle shapes.

`{"kind":"frames","on":true}` enables frame telemetry; `false` stops and clears it. While enabled, `frames` replies arrive at most every 250 ms: `fed` and `accepted` are totals since enabling (or LOAD), `dropped` counts reasons 1–5, `omitted` counts samples omitted when the UI falls behind, and `samples` contains `[arrival_gap_us, arrival_to_feed_us, queued_frames, drop_reason]` for each video unit. The first gap and an unavailable queue are `-1`; reason `0` means accepted. Arrival is the feeder receiving a complete VIDEO message, feed is its pipeline submission returning: neither measures decoding or presentation. At most 256 samples fit a batch; totals include omitted samples. Lifecycle messages take precedence over telemetry.

Build settings `GF_APP_ID`, `GF_RUNTIME_DIR`, `GF_UI_PROGRAM`, and `GF_UI_GAME` select the application identity and UI arguments. `gfn/feeder/build.sh` produces both `lumatv-gfn-feeder` and `lumagame-main` for ARM and host; `GF_BINARY_NAME` names a custom variant. The helper's existing live-TV/camera executable keeps its old defaults.
