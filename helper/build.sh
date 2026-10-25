#!/bin/sh
# Builds lumatv-helper, the native helper, as the static aarch64 musl binary the set runs.
#
#   ./build.sh            release build for aarch64-unknown-linux-musl -> dist/lumatv-helper (+ .sha256)
#   ./build.sh test       host tests and clippy (the gate before a commit)
#   ./build.sh check      fmt --check, clippy -D warnings, host tests, then the release build
#   ./build.sh clean      removes target/ and dist/
#
# The helper shares LumaShow OS's crates (fetch, settings-schema) by path, so the LumaShow checkout must sit beside this
# repository: /data/home/LumaShow/lumashow next to /data/home/lumaglass. They are never copied or edited.
# The compiler is the one LumaShow pins (helper/rust-toolchain.toml); rust-lld links the target, no C toolchain is needed for it.
# Build scripts of the dependencies are compiled for the host and need a C linker: when `cc` is missing (this container has
# none) a shim that runs `zig cc` through cargo-zigbuild is created under target/ and put first on PATH.
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
TARGET=aarch64-unknown-linux-musl
BIN=lumatv-helper
cd "$HERE"

say() { printf '%s\n' "$*" >&2; }
die() { say "build.sh: $*"; exit 1; }

[ -d "$HERE/../../LumaShow/lumashow/crates/fetch" ] ||
  die "the LumaShow checkout is not at $HERE/../../LumaShow/lumashow (the helper depends on its crates by path)"
command -v cargo >/dev/null 2>&1 || die "cargo is not on PATH (install rustup; the toolchain is pinned in rust-toolchain.toml)"

if ! command -v cc >/dev/null 2>&1; then
  shim="$HERE/target/cc-shim"
  mkdir -p "$shim"
  if command -v cargo-zigbuild >/dev/null 2>&1; then
    printf '#!/bin/sh\nexec cargo-zigbuild zig cc -- "$@"\n' > "$shim/cc"
  elif command -v zig >/dev/null 2>&1; then
    printf '#!/bin/sh\nexec zig cc "$@"\n' > "$shim/cc"
  else
    die "no C linker for the build scripts: install gcc, or zig with cargo-zigbuild"
  fi
  chmod +x "$shim/cc"
  PATH="$shim:$PATH"
  export PATH
fi

host_tests() {
  cargo clippy --workspace --all-targets -- -D warnings
  cargo test --workspace
}

release() {
  cargo build --release --target "$TARGET" -p lumatv-helper
  out="target/$TARGET/release/$BIN"
  [ -x "$out" ] || die "the build produced no $out"
  mkdir -p dist
  cp -f "$out" "dist/$BIN"
  (cd dist && sha256sum "$BIN" > "$BIN.sha256")
  verify "dist/$BIN"
}

# A static AArch64 executable: ELF64, machine 183, no interpreter and no dynamic section.
verify() {
  bin=$1
  size=$(wc -c < "$bin")
  if command -v file >/dev/null 2>&1; then
    desc=$(file -b "$bin")
    say "$desc"
    case "$desc" in
      *aarch64*|*ARM\ aarch64*) : ;;
      *) die "not an AArch64 binary: $desc" ;;
    esac
    case "$desc" in
      *statically\ linked*|*static-pie\ linked*) : ;;
      *) die "not statically linked: $desc" ;;
    esac
  fi
  if command -v readelf >/dev/null 2>&1; then
    header=$(readelf -h "$bin")
    printf '%s\n' "$header" | grep -q 'Class:.*ELF64' || die "not ELF64"
    printf '%s\n' "$header" | grep -q 'Machine:.*AArch64' || die "not AArch64"
    if readelf -l "$bin" | grep -q 'INTERP'; then die "it names a dynamic loader (not static)"; fi
    if readelf -d "$bin" 2>/dev/null | grep -q 'NEEDED'; then die "it needs shared libraries (not static)"; fi
    say "ELF64 AArch64, static: no interpreter, no needed libraries"
  fi
  say "dist/$BIN: $size bytes ($((size / 1024)) KiB)"
  say "sha256 $(cut -d' ' -f1 "dist/$BIN.sha256")"
}

case "${1:-release}" in
  release) release ;;
  test) host_tests ;;
  check)
    cargo fmt --all --check
    host_tests
    release
    ;;
  clean) rm -rf target dist ;;
  *) die "usage: build.sh [release|test|check|clean]" ;;
esac
