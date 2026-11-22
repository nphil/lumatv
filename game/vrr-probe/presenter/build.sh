#!/bin/sh
# Builds presenter_test against the pinned host FFmpeg (gfn/feeder/third_party/build-ffmpeg.sh); exits 77 without that build or zig, as the feeder's own checks do.
# UBSan is on: the static libc and zig 0.16 give no working ASan here, so leaks and stale reads are judged by the decoder's own buffer counts (hevc_source.h).
#   build.sh OUTDIR        writes OUTDIR/presenter_test
set -eu
here=$(cd "$(dirname "$0")" && pwd)
feeder=$here/../../../gfn/feeder
out=${1:?output directory}
ffmpeg=$feeder/build/ffmpeg/host
[ -f "$ffmpeg/lib/libavcodec-lumatv.so" ] || { echo "no host FFmpeg (sh gfn/feeder/third_party/build-ffmpeg.sh)"; exit 77; }
command -v zig >/dev/null 2>&1 || { echo "no zig"; exit 77; }
zig cc -target x86_64-linux-gnu.2.36 -std=gnu11 -O1 -g -Wall -Wextra -Werror -fsanitize=undefined -fno-sanitize-recover=undefined \
  -isystem "$ffmpeg/include" -I"$feeder/src" -o "$out/presenter_test" \
  "$here/presenter_test.c" "$here/vrr_presenter.c" "$here/vrr_trace.c" "$here/hevc_source.c" "$feeder/src/gf_annexb.c" \
  -L"$ffmpeg/lib" -lavcodec-lumatv -lavutil-lumatv -Wl,-rpath,"$ffmpeg/lib" -lm -lpthread
