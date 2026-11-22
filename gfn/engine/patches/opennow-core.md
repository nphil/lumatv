# opennow-core

`opennow-core.patch` applies to OpenNOW's `native/opennow-core` at
[a49b124c](https://github.com/OpenCloudGaming/OpenNOW/commit/a49b124ce31e87113fd4961f6a321fbf0ad1fdc5),
the revision the workspace pins. `build-core.sh` applies it after the checkout and `build-core.sh --test`
runs the core's tests with it. To change it, edit the checkout in `gfn/build/OpenNOW` and regenerate it
there with `git diff -- native/opennow-core/src`.

Local changes, both to match the session request of NVIDIA's own client (the web client inside its LG
app). With the old values an owned Steam game opened Steam Big Picture and waited for the player to start
it, where the official client starts the game:

- `internalTitle` is null, as in every official request. The core sent the game's title.
- `accountLinked` is true only when the variant's store signs in through GeForce NOW (single sign-on),
  the link covers the variant and the account is connected and unexpired, the official client's rule.
  The core sent "the variant is in the library", so every owned Steam game went out as linked, though
  Steam offers no sign-on.

`streamer.control` also accepts `steam-overlay`, which the engine's streamer answers by typing Shift+Tab into the game.

The accepted dynamic range is read from the response's own `monitorSettings[0].sdrHdrMode`, where the
official client reads it (vendor.js `at()`, main.js `hdrStreamingMode`), before the echo of the request in
`sessionRequestData`. The echo only shows what was asked, so a refused HDR request still read as accepted
and the stream was declared HDR over SDR content. `enableHdrSource` is `server` for the response's own
fields and `request` for the echo.

The session also records `requestedHdr`: whether the create request carried `sdrHdrMode` 1 (false for WebRTC and for a game
the catalog gate downgraded), kept through polls like `hdrDowngrade`. The service's answer stays in
`negotiatedStreamProfile.enableHdr`; the two differ when the service refuses. `streamer_context` derives `settings.announceHdr` from
`requestedHdr`, NVST transport and a 10-bit HEVC or 10-bit 4:2:0 AV1 profile, and the streamer's ANNOUNCE reads it (see its
`VENDORED.md`), while `enableHdr` still carries the answer.
