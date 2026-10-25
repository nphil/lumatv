#!/bin/sh
# Assembles the 64-bit blob bundle in gfx64/build/bundle/ (ignored by git): Rockchip's libmali plus the Debian
# bookworm arm64 libraries it runs on, all pinned and hash-checked in blob.lock. Idempotent: downloads are cached
# in build/dl/ and the bundle is rebuilt from them. Needs curl, sha256sum, tar, ar (or dpkg-deb), readelf.
#   gfx64/fetch-blob.sh [--out DIR]      (default gfx64/build/bundle)
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
OUT=$HERE/build/bundle
[ "${1:-}" = "--out" ] && OUT=$2
DL=$HERE/build/dl
LOCK=$HERE/blob.lock
MALI=libmali-bifrost-g52-g24p0-wayland-gbm.so
# Files taken from each package (soname-named; symlinks are dereferenced so the bundle has plain files).
files_of() {
  case "$1" in
    libc6) echo "ld-linux-aarch64.so.1 libc.so.6 libm.so.6 libdl.so.2 libpthread.so.0 librt.so.1" ;;
    libgcc-s1) echo "libgcc_s.so.1" ;;
    libstdc++6) echo "libstdc++.so.6" ;;
    libffi8) echo "libffi.so.8" ;;
    libwayland-client0) echo "libwayland-client.so.0" ;;
    libwayland-server0) echo "libwayland-server.so.0" ;;
    *) echo "fetch-blob: no file list for $1" >&2; exit 1 ;;
  esac
}
die() { echo "fetch-blob: $*" >&2; exit 1; }
sha() { sha256sum "$1" | cut -d' ' -f1; }

# fetch <dest> <sha256> <url...>: first URL that yields the right hash wins.
fetch() {
  dest=$1; want=$2; shift 2
  if [ -f "$dest" ] && [ "$(sha "$dest")" = "$want" ]; then return 0; fi
  for url in "$@"; do
    echo "fetch-blob: $url"
    if curl -fsSL --retry 2 -o "$dest.part" "$url" && [ "$(sha "$dest.part")" = "$want" ]; then
      mv "$dest.part" "$dest"; return 0
    fi
    rm -f "$dest.part"
  done
  return 1
}

# Debian drops superseded point releases from the pool; snapshot.debian.org keeps every file, addressed by sha1.
snapshot_url() {
  hash=$(curl -fsSL "https://snapshot.debian.org/mr/binary/$1/$2/binfiles" |
    sed -n 's/.*"architecture":"arm64","hash":"\([0-9a-f]*\)".*/\1/p')
  [ -n "$hash" ] && echo "https://snapshot.debian.org/file/$hash"
}

extract_deb() {
  if command -v dpkg-deb >/dev/null 2>&1; then dpkg-deb -x "$1" "$2"
  else (cd "$2" && ar p "$1" data.tar.xz | tar -xJ 2>/dev/null || ar p "$1" data.tar.zst | tar --zstd -x); fi
}

mkdir -p "$DL"
STAGE=$(mktemp -d "$HERE/build/stage.XXXXXX")
trap 'rm -rf "$STAGE"' EXIT
rm -rf "$OUT"; mkdir -p "$OUT/lib"

while read -r kind a b c d; do
  case "$kind" in
    mali)
      fetch "$DL/$MALI" "$b" "$a" || die "cannot fetch $MALI with the pinned hash"
      cp "$DL/$MALI" "$OUT/lib/$MALI" ;;
    deb)
      deb=${a}_${b}_arm64.deb
      fetch "$DL/$deb" "$d" "https://deb.debian.org/debian/pool/main/$c/$deb" ||
        fetch "$DL/$deb" "$d" "$(snapshot_url "$a" "$b")" || die "cannot fetch $deb with the pinned hash"
      rm -rf "$STAGE/x"; mkdir "$STAGE/x"
      extract_deb "$DL/$deb" "$STAGE/x"
      for f in $(files_of "$a"); do
        src=$(find -L "$STAGE/x" -name "$f" -type f | head -n 1)
        [ -n "$src" ] || die "$f not in $deb"
        cp -L "$src" "$OUT/lib/$f"
      done ;;
    ''|'#'*) ;;
    *) die "bad line in blob.lock: $kind" ;;
  esac
done < "$LOCK"

# Every library the bundle needs must be in it (the loader is told to look nowhere else).
missing=0
for so in "$OUT"/lib/*.so*; do
  case "$(basename "$so")" in ld-linux-*) continue ;; esac
  for need in $(readelf -d "$so" | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p'); do
    [ -e "$OUT/lib/$need" ] || { echo "fetch-blob: $(basename "$so") needs $need, not in bundle" >&2; missing=1; }
  done
done
[ $missing = 0 ] || exit 1
chmod 755 "$OUT/lib/ld-linux-aarch64.so.1"
echo "fetch-blob: bundle ready in $OUT ($(du -sh "$OUT" | cut -f1))"
