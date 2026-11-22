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

The setting `clientIdentity` (`desktop`, the default, `lgWebos` or `windows`) says what the CloudMatch requests claim the client is. A 4K 120
HEVC session that asked for HDR (`sdrHdrMode` 1) was answered in SDR under the desktop identity; the other two identities test whether
the answer depends on who asks, one session each. `lgWebos` is the web client of NVIDIA's LG app as that app's bundle builds it,
`windows` the native Windows client. Each changes only the fields below. The `desktop` column is the host OS of the build (Linux on the
TV); a Windows or Mac build sends its own values there, and a Mac build's `dpi` 144 becomes 96 under `windows`.

| Field | `desktop` | `lgWebos` | `windows` |
|---|---|---|---|
| `User-Agent` | `GFN-PC/30.0 (Linux) BifrostClientSDK/4.9 (38495286)` | `Mozilla/5.0 (Web0S; Linux/SmartTV) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.6099.270 Safari/537.36 WebAppManager`, what the TV's web view reports | `GFN-PC/30.0 (Windows NT 10.0) BifrostClientSDK/4.9 (38495286)` |
| `x-nv-client-identity` | the user agent | not sent (the web client never sends it) | the user agent |
| `Content-Type` | `text/plain` | `application/json` | `text/plain` |
| `nv-client-type` | `NATIVE` | `BROWSER` | `NATIVE` |
| `nv-device-os` | `LINUX` | `WEBOS` | `WINDOWS` |
| `nv-device-type` | `DESKTOP` | `TV` | `DESKTOP` |
| `nv-device-make` | `UNKNOWN` | `LG` | `UNKNOWN` |
| `nv-browser-type` | not sent | `CHROMIUM` | not sent |
| create body `clientPlatformName` | `Linux` (`SteamOS` with the Steam Deck flag) | as `desktop` | `Windows`, also over the Steam Deck flag |
| create body `enhancedStreamMode` | `0` | `1` | `0` |

The Windows values follow two sources. GeForce NOW's own web app bundle (its shared header service) sends `NATIVE` and `NVIDIA-CLASSIC`
from a native client and `WINDOWS` for a Windows OS, and derives the device type from the OS (`DESKTOP`); upstream OpenNOW's former
Electron client (`clientHeaders.ts`, `deviceIdentity.ts` and their header test) carries the Windows profile `WINDOWS`, `DESKTOP`,
`UNKNOWN`, `UNKNOWN` and `Windows`, `text/plain`, and an `x-nv-client-identity` equal to the user agent. `GFN-PC/30.0` and
`BifrostClientSDK/4.9 (38495286)` come from an official Mac client's request (2026-08-19), not from Windows. Assumed, because no
Windows client request was captured: the `Windows NT 10.0` token (a real client may add a build number), `UNKNOWN` make and model (the
native web app sends the PC's own manufacturer and model when it knows them), `DESKTOP` (a laptop may send `LAPTOP`) and `Windows` as
the body's platform name.

The request keeps its NVST/RTSP transport fields and its resolution, frame rate and HDR mode under every identity, so the service's
answers to the same request can be compared; the core's WebRTC request carries no HDR. The fields that choose the streamer therefore
keep their desktop values: `nv-client-streamer` (`NVIDIA-CLASSIC`; the LG app sends `WEBRTC`), `clientPlatformName` (`browser` in the
LG app), `sdkVersion` (`2.0`; `1.0`), `streamerVersion` (`14`; `1`), `secureRTSPSupported`
(`true`; `false`) and no `GSStreamerType` metadata. `nv-device-model` stays `UNKNOWN`; the LG app sends its launcher's CPU name, which
is not known here. The identity applies to every CloudMatch request of a session (create, server info, network test, poll, resume, ad
report, end, the seat list); a running session keeps the one it was created under (`clientIdentity` in the session), and a resume
takes the setting as it then stands. The catalog, subscription and LCARS requests keep the desktop headers, which are what select the
account's 4K 120 entitlements.

Risks: the service may grant less than 4K 120 to a TV profile, or answer or refuse a browser client on the classic transport (no real
client sends that pair), and an unusual identity on the account may be noticed; `windows` on a Linux build claims an OS the stream does
not come from. Whether the identity gates HDR at all is not known.
The core logs one line per session, `CloudMatch session created as the <identity> client: hdr requested <bool>, hdr in the response
<bool> (<source>)`; the streamer's `session answered hdr` line and the diagnostics' `stream_format` and `vui` events show the answer
the stream carries.
