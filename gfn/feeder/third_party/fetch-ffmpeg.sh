#!/bin/sh
# Fetches the pinned FFmpeg source (LGPL-2.1+, ffmpeg.org) into gfn/feeder/build/ffmpeg/src and prints that directory. The feeder decodes a camera's
# low-resolution H.264 or H.265 stream with it when no video window is free (gf_swdec.c); the tarball's SHA-256 is checked, nothing is built here.
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
ver=9.0.2
sha=8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e
dir=$here/build/ffmpeg
if [ ! -f "$dir/src/libavcodec/avcodec.h" ]; then
  mkdir -p "$dir"
  tar=$dir/ffmpeg-$ver.tar.xz
  [ -f "$tar" ] || curl -fsSL -o "$tar" "https://ffmpeg.org/releases/ffmpeg-$ver.tar.xz"
  echo "$sha  $tar" | sha256sum -c - >&2
  rm -rf "$dir/src"
  mkdir -p "$dir/src"
  tar xJf "$tar" -C "$dir/src" --strip-components=1
fi
echo "$dir/src"
