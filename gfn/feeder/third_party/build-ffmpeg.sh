#!/bin/sh
# FFmpeg's H.264 and H.265 decoders (libavcodec and libavutil only) as shared libraries, build/ffmpeg/arm/lib (32-bit ARM soft-float EABI, glibc
# 2.28 and NEON, like the feeder) and build/ffmpeg/host/lib (the tests), with their headers beside them. Shared on purpose: FFmpeg here is LGPL-2.1
# (no --enable-gpl, no --enable-nonfree), and separate .so files that the feeder dlopens keep the user able to replace the library. The suffix
# `-lumatv` makes the names libavcodec-lumatv.so.N and libavutil-lumatv.so.N, so they can never be mistaken for the television's own FFmpeg
# (webOS ships one of another major). The feeder loads them by absolute path from `lib/` beside itself and uses nothing else of the set's.
# The decoders alone: no demuxers, filters, scaler, network, devices, programs; the feeder converts and scales the picture itself.
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
src=$("$here/third_party/fetch-ffmpeg.sh")
out=$here/build/ffmpeg
jobs=${FFMPEG_JOBS:-$(nproc)}
common="--disable-everything --disable-programs --disable-doc --disable-avdevice --disable-avformat --disable-avfilter --disable-swscale \
 --disable-swresample --disable-network --disable-autodetect --disable-debug --disable-static --enable-shared --build-suffix=-lumatv \
 --enable-decoder=h264 --enable-decoder=hevc --enable-parser=h264 --enable-parser=hevc"
build() { # name, extra configure words
  name=$1
  shift
  if [ -f "$out/$name/lib/libavcodec-lumatv.so" ] && [ "$out/$name/lib/libavcodec-lumatv.so" -nt "$0" ]; then return 0; fi
  work=$out/work-$name
  rm -rf "$work" "$out/$name"
  mkdir -p "$work"
  (cd "$work" && "$src/configure" --prefix=/lumatv $common "$@" > configure.log 2>&1 || { tail -30 configure.log >&2; exit 1; })
  # zig's libc still links a `sysctl` symbol, so configure takes it for there, but glibc 2.32 and later have no <sys/sysctl.h> (libavutil/cpu.c includes it)
  # and nothing here wants it
  sed -i 's/^#define HAVE_SYSCTL 1/#define HAVE_SYSCTL 0/' "$work/config.h"
  (cd "$work" && make -j"$jobs" > make.log 2>&1 && make install DESTDIR="$out/$name-root" >> make.log 2>&1 || { tail -30 make.log >&2; exit 1; })
  rm -rf "$out/$name"
  mkdir -p "$out/$name"
  cp -a "$out/$name-root/lumatv/lib" "$out/$name-root/lumatv/include" "$out/$name/"
  rm -rf "$out/$name-root" "$work"
}
mkdir -p "$out/tool"
cat > "$out/tool/zcc-arm" <<'W'
#!/bin/sh
exec zig cc -target arm-linux-gnueabi.2.28 -mcpu=cortex_a9 -mfpu=neon -mfloat-abi=softfp "$@"
W
cat > "$out/tool/zar" <<'W'
#!/bin/sh
exec zig ar "$@"
W
cat > "$out/tool/zranlib" <<'W'
#!/bin/sh
exec zig ranlib "$@"
W
cat > "$out/tool/zcc-host" <<'W'
#!/bin/sh
exec zig cc -target x86_64-linux-gnu.2.36 "$@"
W
chmod +x "$out/tool"/*
build host --cc="$out/tool/zcc-host" --ar="$out/tool/zar" --ranlib="$out/tool/zranlib" --disable-x86asm
build arm --enable-cross-compile --arch=arm --target-os=linux --cc="$out/tool/zcc-arm" --ar="$out/tool/zar" --ranlib="$out/tool/zranlib" \
  --nm=true --strip=true --enable-neon --enable-pic --extra-ldflags=-Wl,--strip-all
echo "built $out/{host,arm}/lib/lib{avcodec,avutil}-lumatv.so" >&2
