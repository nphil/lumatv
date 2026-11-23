# LumaGame: moving media on its own display layer

Owner's rule (2026-11-23): every moving picture in LumaGame (trailers, community videos, animated capsules, loops) plays on a hardware video layer under the UI, never as UI textures, so the UI keeps 120 Hz. Research only; nothing has run on the TV for this.

## The layer

The video wallpaper already plays under the UI through a transparent hole, positioned with `setDisplayWindow` (it needs `originalInput` beside `sourceInput`); LG's video capture matched the clip at 0.998 correlation. A YouTube video on the main window and a camera on the second window (1120×630) ran together at 30 fps with no drops (`docs/research/multiview-result.md`, M3). The second window tops out at 1920×1088 at 30 fps; 4K120 takes both scalers. With no video playing, the UI held 120 Hz.

Holes need the alpha screen format (the other format turns them black), and LG's display capture shows the hole black, so screenshots of these screens use LG's video capture.

## Routes

| Kind | Route | Open |
|---|---|---|
| Steam store trailers | The helper fetches the HLS stream and splits it into H.264 and audio; the feeder plays them on the second window (`smp` pipe, `sub_video` sink); a new compositor claim kind places it. | The feeder's FFmpeg build reads no HLS or MP4 containers; trailer audio is AAC and the feeder decodes Opus and PCM only (`docs/research/rea-ndl-audio-result.md`); sound on the second window and sharing the two window slots with camera pop-ups are untested. |
| Loops, animated WebP/GIF/WebM capsules, micro-trailers | Converted to H.264 and played silently on the wallpaper's `vplane` path in a card-sized rectangle; a still with a play mark until ready. | Card-sized placement; a converter; GIF is refused by the image decoder today. |
| Community YouTube videos | No web engine exists in LumaGame, so the official embedded player cannot run inside it. Route: launch the YouTube app (`youtube.leanback.v4`, `contentTarget`) with the video, Back returns. | Launch with an id and the return path are untested; whether the YouTube app's video can be placed inside a LumaGame card is untested and may fail. Extracting YouTube streams breaks YouTube's terms and is not done. |

Steam trailers measured 2026-11-23 for five apps (1091500, 990080, 1145360, 413150, 1245620): every one is H.264 High 4.1 (`avc1.640029`) at 30 fps, variants 1920×1080 at 5.8 Mbit/s, 720p at 2.6, 480p at 1.4 and 360p at 1.0, with one separate AAC-LC audio rendition; segments are 3 s fragmented MP4 (`.m4s`) with an init segment, VOD. 30 fps keeps the panel at 120 Hz; 25 or 50 fps video drops it to 100 Hz.

## Risks

- A 25 or 50 fps source moves the panel to 100 Hz; after a 25 fps camera pop-up the panel stayed at 100 Hz until other video played (`docs/research/compositor/slot-2026-10-30-r22/RESULT.md`).
- The camera pop-up over Home decodes into a texture today, which breaks this rule.
- Video memory had about 105 MB free in an earlier round; the cost of demuxing and audio is unmeasured.
- A play mark or card frame is a static bright element: it dims or hides after a short dwell.

## TV experiments, in order

1. One 1080p30 trailer on the second window with sound, checked with LG's video capture at 120 Hz.
2. The YouTube app launched with a known id; Back returns.
3. The YouTube app's video placed inside a card (decides whether community videos can be embedded at all).
