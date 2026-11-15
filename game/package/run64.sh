#!/bin/sh
# Runs one of the package's 64-bit programs on the set: the TV's userland is 32-bit, so the program starts through the private Debian
# loader and glibc beside it (gfx64/bundle/lib). The UI starts each engine this way, by absolute path:
#   run64.sh <package>/engines/<id>/<exec> ARGS...
lib=$(cd "$(dirname "$0")" && pwd)/gfx64/bundle/lib
exec "$lib/ld-linux-aarch64.so.1" --library-path "$lib" "$@"
