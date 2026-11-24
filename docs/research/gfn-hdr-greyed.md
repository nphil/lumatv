# GFN: the game's HDR option greyed out

Hogwarts Legacy and Cyberpunk show their HDR option greyed in a GeForce NOW session that looked like HDR10.

## What the logs showed

- The 2026-11-22 reports ("feeder logs `hdr 1`", TV `hdr_type 1`) came from builds up to `09d301eb`, which took "HDR accepted" from the echo of the
  session request, not from the service. They show that HDR was asked for, not that the stream was HDR.
- Since `d5842a40` the answer is read from `monitorSettings[0].sdrHdrMode` and the stream's own tags (VUI/SPS). Hogwarts under the default Linux
  identity: answer 0, VUI transfer 1 (BT.709), `hdr_missing service_sent_sdr`. So the cloud PC was not given an HDR display, and the game's option
  followed.
- Client identity test (build `64b7545d`, Hogwarts, 4K120 HEVC, HDR on): Windows PC identity was answered HDR10 (PQ) and went back to SDR tags
  about 16 s in, about when the game started; Linux PC and LG webOS stayed SDR. Not recorded: whether the game's option was greyed or merely off.
- The display fields are complete since `b260254c` (`displayData`: 1000 nit peak, 200 nit frame average, 0.0005 nit black, P3-D65) and the Linux
  identity still got SDR, so they are not what decides the answer. The official LG client sends `clientDisplayHdrCapabilities: null` too.

## Cause

The service grants HDR by client platform, and the Windows PC identity (`clientPlatformName` `Windows`, `nv-device-os` `WINDOWS`) is set on the TV.
The TV's streamer.log of 2026-11-24 under that identity: "the stream's tags say HDR10 (transfer 16) and the session answered hdr true", HEVC
3840x2160 at 120 fps, 10-bit, moving between SDR (transfer 1) and HDR10 as the game switches. So the service does grant HDR to this client and the
display fields are not the cause. The official LG GeForce NOW app has HDR, so the LG webOS identity is expected to work as well (untested here).
What is left is the game's own option, which the test below checks.

## One-session TV test (Hogwarts Legacy, Settings › Stream › Client identity)

1. With Client identity Windows PC, HDR on, 4K 120, HEVC, launch the game. Read the core's `CloudMatch session created ... hdr in the response`
   line and `stream_format` `vui.transfer` (16 is PQ).
2. In the game's Graphics menu, note whether HDR is available, off, or greyed. Turn it on if it is available; the stream's tags should stay PQ.
3. If it is greyed while the tags were PQ at launch: the game started before Windows HDR was on. Quit to Steam Big Picture (tags PQ there) and
   start the game from it.
4. If still greyed, change one field per session, in this order, and keep the diagnostics of each:
   - `clientDisplayHdrCapabilities`: an object instead of `null` (unverified shape: `version`, `hdrEdrSupportedFlagsInUint32`,
     `staticMetadataDescriptorId`, `displayData` as in the monitor entry). The official clients seen send `null`; try only if 1-3 fail.
   - `hdr10PlusGamingData`: `null` today.
   - `requestedStreamingFeatures.trueHdr`: `false` today; `true` asks for the service's own SDR-to-HDR filter, which is not what the game needs.
5. Watch the refresh rate at the SDR-to-HDR switch (the 60 Hz drop noted for build `b260254c`).
