#!/bin/sh
# Runs one of the app's 64-bit programs (bin64/) on the set: the TV's userland is 32-bit, so the program is started through the
# private Debian loader and glibc beside it (gfx64/bundle/lib, the same runtime the card app uses, without the GPU driver here).
#   run64.sh opennow-core ARGS...
here=$(cd "$(dirname "$0")" && pwd)
lib=$here/gfx64/bundle/lib
prog=$1
shift
exec "$lib/ld-linux-aarch64.so.1" --library-path "$lib" "$here/bin64/$prog" "$@"
