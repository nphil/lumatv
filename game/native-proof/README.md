# LumaGame native graphics proof

A standalone SAM app, not a LumaGame engine or launcher change. A GLES 3 triangle rotates for up to 15 seconds or 3,600 frames. Back, Exit, Escape and SDL quit end it sooner. It tears down its GL context and SDL before exiting. C keeps this external hardware proof minimal; LumaGame remains Rust. The icon and geometry are original; no game assets are included.

## Build on Linux

The ARMv7 recipe follows [Web-Mario-64](https://github.com/pustal/Web-Mario-64/tree/b8eb36b6da25118fd3f7ca07f84929914fa97c54), in particular `scripts/build-webos.sh`, `cmake/SDL2WebOSBackport.cmake` and `cmake/WebOSPackage.cmake`. This is not an assertion that every webOS app is 32-bit; LumaGame itself uses 64-bit components.

- A relocated [webosbrew Buildroot SDK](https://github.com/openlgtv/buildroot-nc4/releases) for `arm-webos-linux-gnueabi`, GCC, GLES headers, Python 3, curl and sha256sum. The SDK used for this build was already installed; its original archive `/data/home/webos-ndk/ndk.tar.bz2` has SHA-256 `45a2d12ff557457d92cde4fddaa77a6f1090fca03adc43bb74397e5e0c379501`. Its exact upstream release was not established; do not mistake that local hash for a verified upstream release hash. Buildroot and toolchain components have their respective upstream licences.
- The build fetches the [SDL-webOS release 2.30.12-webos.6](https://github.com/webosbrew/SDL-webOS/releases/tag/release-2.30.12-webos.6) ARM archive, SHA-256 `63dc9ea268bdf64801fce5e0c19585ee6aaf79f0b08486c853c47f922385c698`, and verifies it. The SDL fork is an altered SDL version under the zlib licence; the archive's complete licence is bundled as `SDL-LICENSE.txt`. SDL and its headers are cached outside the repository; only the shared library is packaged.
- `ares-package` from [`@webos-tools/cli` 3.2.6](https://www.npmjs.com/package/@webos-tools/cli/v/3.2.6), Apache-2.0; npm tarball SHA-512 integrity `VkCV8oBycgd0UTT3n8xOCUKjzIFaJM5MzJWr9BqmHfplUemtMsTBx+1GNOyxio6Ky1mj/iyqAUx4VcUy2nQu3Q==`. It is a build tool, not packaged.

```sh
npm install --prefix "$HOME/.cache/lumagame-native-proof/ares" --ignore-scripts --no-audit --no-fund @webos-tools/cli@3.2.6
WEBOS_SDK="$HOME/webos-ndk/arm-webos-linux-gnueabi_sdk-buildroot" sh game/native-proof/build.sh
```

Set `ARES_PACKAGE` to use an existing `ares-package`; set `WEBOS_SDK` to another relocated SDK as needed. Output is in `game/native-proof/build/dist/`. The SDK headers supply GLES declarations; functions are resolved from the TV's driver through SDL at runtime, not supplied by a packaged driver.

## Timing and acceptance

The storage path follows Web-Mario-64's [`Storage::Resolve`](https://github.com/pustal/Web-Mario-64/blob/b8eb36b6da25118fd3f7ca07f84929914fa97c54/src/port/Storage.cpp), and remote scancodes follow [SDL-webOS's mapping](https://github.com/webosbrew/SDL-webOS/blob/release-2.30.12-webos.6/src/events/SDL_scancode_webos.c); HOME and its writability are untested on this TV.

Each launch overwrites `$HOME/data/native-proof-timing.csv` in the app's writable data directory. Startup records the actual GLES version and renderer, swap-interval request and result, and drawable size. A bounded in-memory sample records monotonic frame start, swap start and swap completion timestamps in nanoseconds; it is written after the loop with a swap-completion cadence summary. The log is bounded and has no per-frame console output. If the swap interval request fails, reports a value other than one, or eight consecutive frames complete in under 2 ms despite the reported interval, an 8 ms sleep prevents an uncontrolled busy loop; the log records when and why pacing was added. Those samples measure the fallback too, not an unpaced display. A process killed before normal exit may leave only startup information.

Swap completion is **not** a timestamp of pixels appearing on the panel: buffering, scheduling, compositor and scanout can differ. Neither requesting vsync nor a calculated cadence proves 120 Hz panel output. TV launch, real cadence, Back/Exit and return to LumaGame require separate hardware acceptance and are not established by a host build.
