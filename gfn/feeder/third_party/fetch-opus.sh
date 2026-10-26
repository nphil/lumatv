#!/bin/sh
# Fetches the pinned libopus source (BSD-3-Clause, opus-codec.org) into gfn/feeder/build/opus/src and prints that directory. The feeder
# decodes Opus to PCM with it when NDL refuses the Opus load (gf_opus.c); the tarball's SHA-256 is checked, nothing is built here.
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
ver=1.5.2
sha=65c1d2f78b9f2fb20082c38cbe47c951ad5839345876e46941612ee87f9a7ce1
dir=$here/build/opus
if [ ! -f "$dir/src/include/opus.h" ]; then
  mkdir -p "$dir"
  tgz=$dir/opus-$ver.tar.gz
  [ -f "$tgz" ] || curl -fsSL -o "$tgz" "https://downloads.xiph.org/releases/opus/opus-$ver.tar.gz"
  echo "$sha  $tgz" | sha256sum -c - >&2
  rm -rf "$dir/src"
  mkdir -p "$dir/src"
  tar xzf "$tgz" -C "$dir/src" --strip-components=1
fi
echo "$dir/src"
