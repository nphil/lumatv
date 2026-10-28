#!/bin/sh
# stb_image.h (public domain / MIT), pinned to one commit: the probe decodes JPEG and PNG through it.
# Usage: fetch.sh [DIR]   (default: ../build/third_party); prints the directory.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
dir=${1:-$here/../build/third_party}
commit=003501f6d7a8668ce8b6d875e34dd97ad0d5d5b5
sha256=594c2fe35d49488b4382dbfaec8f98366defca819d916ac95becf3e75f4200b3
mkdir -p "$dir"
if ! { [ -f "$dir/stb_image.h" ] && [ "$(sha256sum "$dir/stb_image.h" | cut -d' ' -f1)" = "$sha256" ]; }; then
  curl -fsSL -m 120 -o "$dir/stb_image.h.part" "https://raw.githubusercontent.com/nothings/stb/$commit/stb_image.h"
  [ "$(sha256sum "$dir/stb_image.h.part" | cut -d' ' -f1)" = "$sha256" ] || { echo "stb_image.h: checksum mismatch" >&2; exit 1; }
  mv "$dir/stb_image.h.part" "$dir/stb_image.h"
fi
echo "$dir"
