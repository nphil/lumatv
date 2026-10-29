#!/bin/sh
# Gate for the placement crate, the same way ambient/build.sh does it.
#
#   ./build.sh test    host tests and clippy -D warnings
#   ./build.sh check   fmt --check, test, then a release build of the library for aarch64-unknown-linux-musl (the set's target)
#   ./build.sh clean   removes target/
#
# Build scripts of the dependencies are compiled for the host and need a C linker; when `cc` is missing (this container
# has none) a shim that runs `zig cc` through cargo-zigbuild is created under target/ and put first on PATH.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
cd "$HERE"
say() { printf '%s\n' "$*" >&2; }
die() { say "build.sh: $*"; exit 1; }
command -v cargo >/dev/null 2>&1 || die "cargo is not on PATH"

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

case "${1:-test}" in
  test)  cargo test --all-targets && cargo clippy --all-targets -- -D warnings ;;
  check) cargo fmt --check && "$0" test && cargo build --release --lib --target aarch64-unknown-linux-musl ;;
  clean) rm -rf target ;;
  *) die "unknown command: $1" ;;
esac
