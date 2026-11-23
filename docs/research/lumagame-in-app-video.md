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

## Community YouTube videos: embedded, never handed off

The owner wants them inside LumaGame (2026-11-23); the YouTube app hand-off is out.

- **Route 2, chosen to probe first: YouTube's official embed.** A small web page (served from BeastNAS through HomeLabber, internal, so it carries a Referer and avoids error 153) loads the IFrame API player. It runs in the frame web app LumaHome already uses (`org.nphil.lumaglass.frame`: starts hidden, grows from a card), placed by the compositor over the card's rectangle and grown to full screen; Back stops the player and returns to the card. Inside YouTube's terms as long as the embed rules hold: nothing over the player, 200×200 minimum (16:9 at 480×270 or more for a card), autoplay only once more than half is visible, no hover-to-play. Owner-blocked embeds (errors 101/150, likely age-restricted videos) show the still with "not available here". Unknowns: whether the browser plays on the video chip under the UI (the owner's separate-layer rule) or as pixels, which window gets keys over a card, memory and first-picture time, and whether a browser video switches the panel to the game picture preset (backlight 90 vs the owner's 80).
- **Route 1, needs the owner's explicit yes: our own pipeline.** yt-dlp on BeastNAS resolves and remuxes to H.264 + Opus for the feeder (no VP9 id there), played on SUB0 in the card (1920×1088 at 30) and MAIN full screen. Plain reading of YouTube's terms forbids this automated copying, and it breaks when YouTube changes (PO tokens, SABR-only formats, JavaScript solvers); age-restricted videos would need a YouTube login on the NAS.

## TV experiments, in order

1. One 1080p30 trailer on the second window with sound, checked with LG's video capture at 120 Hz.
2. Plain embed: one public embeddable video on the BeastNAS page at 480×270; it plays without error 153.
3. Go/no-go for Route 2: the embed's video shows as a video window and the panel stays at 120 Hz with a 30 fps video (count refresh pulses). On a fail, Route 2 stops.
4. Frame over a card, nothing bright over the player; keys and full screen with Back returning; autoplay rule; memory, first-picture time and cache size after 20 plays; picture-mode check; a blocked video.

## Embed probe 2026-11-23

Experiments 2 and 3, run 10:04 to 10:08 on the C4 with the LumaTV compositor and LumaGame in front. The page (`docs/research/youtube-embed/index.html`, IFrame API, muted autoplay, `playsinline`, `controls=0`, `disablekb=1`, `rel=0`) was served over plain http from BeastNAS's LAN address and opened in `org.nphil.lumatv.frame` with `{url, mode: "web", rect}`; the video was the Stardew Valley trailer `ot7uXNQskhs` (30 fps, public, embeddable). Receipts: `/data/home/tmp/lumagame-ytembed-20261123/` (LG display capture, LG video capture and the compositor's capture side by side, card and full screen).

**Route 2 is a go.**

- Plays without error: pass. The page loaded 1.0 s after the launch, `onReady` came 2.1 s later and `playing` 8.8 s after the page load; no `onError` (no 153, 101 or 150). WAM is Chrome 120.
- Video plane: pass. WAM decodes in hardware (AV1, a `com.webos.media` pipeline owned by the frame app, sub type `overlay`) on the main window, exports it through `wl_webos_foreign`, and the compositor lists it as a video window at the card's rectangle (720,405 480×270) and attaches the punch-through. LG's video capture shows the picture; the compositor's capture shows a transparent hole (alpha 0) there. 631 frames written, 0 dropped.
- 120 Hz: pass. `osd_irq` ran at 118.4 to 121.8 a second over 8 one-second windows in the card and 6 at full screen, as before the launch; the main window's `vFreq` read 29.9.
- Picture mode: unchanged. `eco` and `sdr` before, during and after; no `sdrALLM` or game preset. Backlight went from 27 to 35 during the run, which matches LumaTV's auto brightness's last write (35).
- Keys: the frame holds keyboard focus while open. Right and Down did not move LumaGame's focus and did not reach the page (no `keydown` logged inside the frame's iframe). Back (412) closed the frame through the compositor's overlay handling: the renderer exited, the video window was released and LumaGame came back in front with focus, the panel still at 120 Hz.
- Full screen: relaunching the frame with rect 0,0,1920,1080 played 1080p30 on the main window (`fullScreen: true`, 0 dropped, 120 Hz). The relaunch reloads the iframe, so playback restarted; growing in place needs a frame change. A small mute glyph (the receiver was muted for the run) stayed drawn above the video, which the embed rule forbids.
- First picture: 9.2 s from a cold frame launch (new renderer) to the video window, 4.7 s on a relaunch; the YouTube player's load and buffering take most of it.
- Memory: the frame's renderer 104 to 113 MB resident (64 MB anonymous), on top of a WAM that was already running (browser 85 MB, zygote 28 MB, network 35 MB). Available memory went from 696 to 454 MB with LumaGame and the frame up.
- Storage: WAM's profile `/var/lib/wam` (21.7 MB, its HTTP cache 11.7 MB) gained 40 files and 1.2 MB, YouTube's cookies and IndexedDB among them; the profile is shared by every web app.
- The page must size the player by CSS: the frame hides its iframe until load, so the page sees a 0×0 viewport when it starts. The iframe's white background shows until the page loads.

Not tested: sound, the autoplay rule, a blocked video, cache growth over 20 plays, the frame over a real card.

## Hosting the player page (2026-11-23)

The probe page loaded from `file://` inside the frame package got `onReady` and then error 153 on the C4: a file page sends no Referer, which YouTube now requires. The player page (`game/player/`) is therefore served over https: the `lumagame-player` container on BeastNAS (nginx:alpine, port 8795, files in `/mnt/user/appdata/lumagame-player/html`), published by HomeLabber as `https://lumaplayer.nateshome.net/` (internal). A change to `game/player/` is deployed by copying its two files there.

The Steam trailer's `hls_h264` playlist plays natively in WAM's `<video>` (`canPlayType` "maybe" for HLS, DASH and MP4; playing 1.1 s after load, unmuted autoplay allowed), so no HLS library is needed.
