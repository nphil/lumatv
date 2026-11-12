#!/bin/sh
# Fetches Beszel's agent for the TV (its release for linux arm64, the hub's version), hash-checked, into beszel/build/beszel-agent
# (ignored by git), where tools/build-ipk.sh takes it from. Idempotent: the release is cached beside it.
#   beszel/fetch-agent.sh
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
VERSION=0.21.1
SHA256=a6ea544976b9a74398ac88d5da5bb96813d9a2e353a37bbfdb6a903fa8ba1838
URL=https://github.com/henrygd/beszel/releases/download/v$VERSION/beszel-agent_linux_arm64.tar.gz
OUT=$HERE/build
TARBALL=$OUT/beszel-agent_linux_arm64-$VERSION.tar.gz
sha() { sha256sum "$1" | cut -d' ' -f1; }

mkdir -p "$OUT"
if ! [ -f "$TARBALL" ] || [ "$(sha "$TARBALL")" != "$SHA256" ]; then
  curl -fsSL --retry 2 -o "$TARBALL.part" "$URL"
  if [ "$(sha "$TARBALL.part")" != "$SHA256" ]; then
    rm -f "$TARBALL.part"
    echo "fetch-agent: $URL is not the release pinned here" >&2
    exit 1
  fi
  mv "$TARBALL.part" "$TARBALL"
fi
tar -xzf "$TARBALL" -C "$OUT" beszel-agent
chmod 755 "$OUT/beszel-agent"
echo "$OUT/beszel-agent"
